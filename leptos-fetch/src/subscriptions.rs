use std::{
    any::TypeId,
    collections::HashMap,
    sync::{Arc, Weak},
};

use leptos::prelude::{ArcRwSignal, ArcSignal, Get, GetUntracked, Set};

use crate::utils::{new_subscription_id, KeyHash};

type Subs = Arc<parking_lot::Mutex<HashMap<TypeId, HashMap<u64, Sub>>>>;

#[derive(Debug, Default)]
pub(crate) struct Subscriptions {
    subs: Subs,
    // Used to initialise as true when a subscriber is created whilst a fetch is ongoing:
    // (bool = loading_first_time)
    actively_fetching: HashMap<(TypeId, KeyHash), bool>,
}

impl Subscriptions {
    pub fn add_is_fetching_subscription(
        &mut self,
        cache_key: TypeId,
        keyer: ArcSignal<KeyHash>,
    ) -> ArcSignal<bool> {
        let initial = self
            .actively_fetching
            .contains_key(&(cache_key, keyer.get_untracked()));
        self.add_subscription(
            cache_key,
            keyer,
            |sub| {
                if let SubVariant::IsFetching(signal) = &sub.variant {
                    Some(signal)
                } else {
                    None
                }
            },
            move || ArcRwSignal::new(initial),
            SubVariant::IsFetching,
        )
    }

    pub fn add_is_loading_subscription(
        &mut self,
        cache_key: TypeId,
        keyer: ArcSignal<KeyHash>,
    ) -> ArcSignal<bool> {
        let initial = if let Some(loading_first_time) = self
            .actively_fetching
            .get(&(cache_key, keyer.get_untracked()))
        {
            *loading_first_time
        } else {
            false
        };

        self.add_subscription(
            cache_key,
            keyer,
            |sub| {
                if let SubVariant::IsLoading(signal) = &sub.variant {
                    Some(signal)
                } else {
                    None
                }
            },
            move || ArcRwSignal::new(initial),
            SubVariant::IsLoading,
        )
    }

    fn add_subscription<T>(
        &mut self,
        cache_key: TypeId,
        keyer: ArcSignal<KeyHash>,
        match_existing: impl Fn(&Sub) -> Option<&ArcRwSignal<T>> + Send + Sync + 'static,
        new_signal: impl Fn() -> ArcRwSignal<T> + Send + Sync + 'static,
        new_variant: impl Fn(ArcRwSignal<T>) -> SubVariant + Send + Sync + 'static,
    ) -> ArcSignal<T>
    where
        T: Clone + Send + Sync + 'static,
    {
        let subs_arc = self.subs.clone();
        let sub_guard_holder = parking_lot::Mutex::new(None);
        ArcSignal::derive(move || {
            // Making sure not to hold the subs lock when replacing the value in sub_guard_holder (for it's use in the Drop impl):
            let (return_value, new_sub_guard) = (|| {
                let key = keyer.get();
                let mut subs_guard = subs_arc.lock();
                let subs = subs_guard.entry(cache_key).or_default();

                // Use an existing one if available:
                for sub in subs.values() {
                    if sub.key_hash.get_untracked() == key {
                        if let Some(signal) = match_existing(sub) {
                            if let Some(sub_guard) = sub.drop_guard.upgrade() {
                                return (signal.get(), sub_guard);
                            }
                        }
                    }
                }
                let signal = new_signal();
                let sub_id = new_subscription_id();
                // By including the guard in the derived signal, we can hook into the signal itself being dropped, at which point we can GC the subscriber.
                let new_sub_guard = Arc::new(SubDropGuard {
                    subs: subs_arc.clone(),
                    cache_key,
                    sub_id,
                });
                let new_sub = Sub::new(new_variant(signal.clone()), keyer.clone(), &new_sub_guard);
                subs.insert(sub_id, new_sub);
                (signal.get(), new_sub_guard)
            })();
            // By including the guard in the derived signal, we can hook into the signal itself being dropped, at which point we can GC the subscriber:
            sub_guard_holder.lock().replace(new_sub_guard);
            return_value
        })
    }

    pub fn notify_fetching_start(
        &mut self,
        cache_key: TypeId,
        key_hash: KeyHash,
        loading_first_time: bool,
    ) {
        self.actively_fetching
            .insert((cache_key, key_hash), loading_first_time);
        if let Some(subs) = self.subs.lock().get_mut(&cache_key) {
            for sub in subs.values() {
                if sub.key_hash.get_untracked() == key_hash {
                    match &sub.variant {
                        SubVariant::IsFetching(signal) => {
                            // Don't want to trigger if not changing:
                            if !signal.get_untracked() {
                                signal.set(true);
                            }
                        }
                        SubVariant::IsLoading(signal) => {
                            if loading_first_time {
                                // Don't want to trigger if not changing:
                                if !signal.get_untracked() {
                                    signal.set(true);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn notify_fetching_finish(
        &mut self,
        cache_key: TypeId,
        key_hash: KeyHash,
        loading_first_time: bool,
    ) {
        self.actively_fetching.remove(&(cache_key, key_hash));
        if let Some(subs) = self.subs.lock().get_mut(&cache_key) {
            for sub in subs.values() {
                if sub.key_hash.get_untracked() == key_hash {
                    match &sub.variant {
                        SubVariant::IsFetching(signal) => {
                            // Don't want to trigger if not changing:
                            if signal.get_untracked() {
                                signal.set(false);
                            }
                        }
                        SubVariant::IsLoading(signal) => {
                            if loading_first_time {
                                // Don't want to trigger if not changing:
                                if signal.get_untracked() {
                                    signal.set(false);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.subs.lock().values().map(|v| v.len()).sum()
    }
}

#[derive(Debug)]
struct Sub {
    variant: SubVariant,
    key_hash: ArcSignal<KeyHash>,
    drop_guard: Weak<SubDropGuard>,
}

impl Sub {
    fn new(
        variant: SubVariant,
        key_hash: ArcSignal<KeyHash>,
        drop_guard: &Arc<SubDropGuard>,
    ) -> Self {
        Self {
            variant,
            key_hash,
            drop_guard: Arc::downgrade(drop_guard),
        }
    }
}

#[derive(Debug)]
enum SubVariant {
    IsFetching(ArcRwSignal<bool>),
    IsLoading(ArcRwSignal<bool>),
}

struct SubDropGuard {
    sub_id: u64,
    cache_key: TypeId,
    subs: Subs,
}

impl Drop for SubDropGuard {
    fn drop(&mut self) {
        let mut subs_guard = self.subs.lock();
        if let Some(subs) = subs_guard.get_mut(&self.cache_key) {
            subs.remove(&self.sub_id);
            if subs.is_empty() {
                subs_guard.remove(&self.cache_key);
            }
        }
    }
}
