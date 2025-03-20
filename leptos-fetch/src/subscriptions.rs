use std::{
    any::TypeId,
    collections::HashMap,
    sync::{Arc, Weak},
};

use leptos::prelude::{ArcRwSignal, ArcSignal, Get, GetUntracked, Set};

use crate::{
    utils::{new_subscription_id, KeyHash},
    QueryClient,
};

#[derive(Debug, Default)]
pub(crate) struct Subscriptions {
    subs: HashMap<TypeId, HashMap<KeyHash, HashMap<u64, Sub>>>,
    // Used to initialise as true when a subscriber is created whilst a fetch is ongoing:
    // (bool = loading_first_time)
    actively_fetching: HashMap<(TypeId, KeyHash), bool>,
}

impl Subscriptions {
    pub fn add_is_fetching_subscription(
        &mut self,
        client: QueryClient,
        cache_key: TypeId,
        key: KeyHash,
    ) -> ArcSignal<bool> {
        let initial = self.actively_fetching.contains_key(&(cache_key, key));
        self.add_subscription(
            client,
            cache_key,
            key,
            |sub| {
                if let SubVariant::IsFetching(signal) = &sub.variant {
                    Some(signal)
                } else {
                    None
                }
            },
            || ArcRwSignal::new(initial),
            SubVariant::IsFetching,
        )
    }

    pub fn add_is_loading_subscription(
        &mut self,
        client: QueryClient,
        cache_key: TypeId,
        key: KeyHash,
    ) -> ArcSignal<bool> {
        let initial =
            if let Some(loading_first_time) = self.actively_fetching.get(&(cache_key, key)) {
                *loading_first_time
            } else {
                false
            };

        self.add_subscription(
            client,
            cache_key,
            key,
            |sub| {
                if let SubVariant::IsLoading(signal) = &sub.variant {
                    Some(signal)
                } else {
                    None
                }
            },
            || ArcRwSignal::new(initial),
            SubVariant::IsLoading,
        )
    }

    fn add_subscription<T>(
        &mut self,
        client: QueryClient,
        cache_key: TypeId,
        key: KeyHash,
        match_existing: impl Fn(&Sub) -> Option<&ArcRwSignal<T>>,
        new_signal: impl Fn() -> ArcRwSignal<T>,
        new_variant: impl Fn(ArcRwSignal<T>) -> SubVariant,
    ) -> ArcSignal<T>
    where
        T: Clone + Send + Sync + 'static,
    {
        println!("B");
        let subs = self
            .subs
            .entry(cache_key)
            .or_default()
            .entry(key)
            .or_default();
        println!("C");

        // Use an existing one if available:
        for sub in subs.values() {
            if let Some(signal) = match_existing(sub) {
                if let Some(sub_guard) = sub.drop_guard.upgrade() {
                    let signal = signal.clone();
                    return ArcSignal::derive(move || {
                        let _ = sub_guard.clone(); // Forces the closure to hold onto the guard until the closure itself is dropped.
                        signal.get()
                    });
                }
            }
        }

        println!("D");

        let signal = new_signal();
        println!("E");
        let sub_id = new_subscription_id();
        println!("F");
        // By including the guard in the derived signal, we can hook into the signal itself being dropped, at which point we can GC the subscriber.
        let new_sub_guard = Arc::new(SubDropGuard {
            client,
            cache_key,
            key,
            sub_id,
        });
        println!("G");
        let new_sub = Sub::new(new_variant(signal.clone()), &new_sub_guard);
        println!("H");
        subs.insert(sub_id, new_sub);
        println!("I");
        ArcSignal::derive(move || {
            println!("INNER RUNNING!");
            let _ = new_sub_guard.clone(); // Forces the closure to hold onto the guard until the closure itself is dropped.
            signal.get()
        })
    }

    pub fn gc_sub(&mut self, cache_key: TypeId, key: KeyHash, sub_id: u64) {
        if let Some(scope) = self.subs.get_mut(&cache_key) {
            if let Some(subs) = scope.get_mut(&key) {
                subs.remove(&sub_id);
                if subs.is_empty() {
                    scope.remove(&key);
                }
            }
            if scope.is_empty() {
                self.subs.remove(&cache_key);
            }
        }
    }

    pub fn notify_fetching_start(
        &mut self,
        cache_key: TypeId,
        key: KeyHash,
        loading_first_time: bool,
    ) {
        self.actively_fetching
            .insert((cache_key, key), loading_first_time);
        if let Some(scope) = self.subs.get_mut(&cache_key) {
            if let Some(subs) = scope.get_mut(&key) {
                for sub in subs.values() {
                    match &sub.variant {
                        SubVariant::IsFetching(signal) => {
                            // Don't want to trigger if not changing:
                            if !signal.get_untracked() {
                                println!("Setting fetching to true");
                                signal.set(true);
                            }
                        }
                        SubVariant::IsLoading(signal) => {
                            if loading_first_time {
                                // Don't want to trigger if not changing:
                                if !signal.get_untracked() {
                                    println!("Setting loading to true");
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
        key: KeyHash,
        loading_first_time: bool,
    ) {
        self.actively_fetching.remove(&(cache_key, key));
        if let Some(scope) = self.subs.get_mut(&cache_key) {
            if let Some(subs) = scope.get_mut(&key) {
                for sub in subs.values() {
                    match &sub.variant {
                        SubVariant::IsFetching(signal) => {
                            // Don't want to trigger if not changing:
                            if signal.get_untracked() {
                                println!("Setting fetching to false");
                                signal.set(false);
                            }
                        }
                        SubVariant::IsLoading(signal) => {
                            if loading_first_time {
                                // Don't want to trigger if not changing:
                                if signal.get_untracked() {
                                    println!("Setting loading to false");
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
        self.subs
            .values()
            .flat_map(|v| v.values().map(|v| v.len()))
            .sum()
    }
}

#[derive(Debug)]
struct Sub {
    variant: SubVariant,
    drop_guard: Weak<SubDropGuard>,
}

impl Sub {
    fn new(variant: SubVariant, drop_guard: &Arc<SubDropGuard>) -> Self {
        Self {
            variant,
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
    key: KeyHash,
    client: QueryClient,
}

impl Drop for SubDropGuard {
    fn drop(&mut self) {
        println!("DROPPING!");
        self.client
            .scope_lookup
            .subscriptions_mut()
            .gc_sub(self.cache_key, self.key, self.sub_id);
    }
}
