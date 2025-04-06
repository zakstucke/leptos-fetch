use std::{
    any::TypeId,
    collections::HashMap,
    sync::{Arc, Weak},
};

use leptos::prelude::{ArcRwSignal, ArcSignal, Get, GetUntracked, Set};

use crate::{
    cache::ScopeLookup,
    maybe_local::MaybeLocal,
    utils::{new_subscription_id, new_value_modified_id, KeyHash},
};

// cache_key -> sub_id -> Sub
type Subs = Arc<parking_lot::Mutex<HashMap<TypeId, HashMap<u64, Sub>>>>;

#[derive(Debug)]
pub(crate) struct ScopeSubs {
    subs: Subs,
    // Used to initialise as true when a subscriber is created whilst a fetch is ongoing:
    // (bool = loading_first_time)
    actively_fetching: HashMap<(TypeId, KeyHash), bool>,
    #[allow(dead_code)]
    scope_lookup: ScopeLookup,
}

impl ScopeSubs {
    pub fn new(scope_lookup: ScopeLookup) -> Self {
        Self {
            subs: Default::default(),
            actively_fetching: Default::default(),
            scope_lookup,
        }
    }

    pub fn add_is_fetching_subscription(
        &mut self,
        cache_key: TypeId,
        keyer: MaybeLocal<ArcSignal<KeyHash>>,
    ) -> ArcSignal<bool> {
        // WONTPANIC: this is called from the initial thread the keyer was provided from:
        let initial_key = keyer.value_may_panic().get_untracked();
        let initial = self
            .actively_fetching
            .contains_key(&(cache_key, initial_key));
        let signal = self.add_subscription(
            cache_key,
            keyer,
            move || ArcRwSignal::new(initial),
            SubVariant::IsFetching,
        );
        ArcSignal::derive(move || signal.get().unwrap_or(false))
    }

    pub fn add_is_loading_subscription(
        &mut self,
        cache_key: TypeId,
        keyer: MaybeLocal<ArcSignal<KeyHash>>,
    ) -> ArcSignal<bool> {
        // WONTPANIC: this is called from the initial thread the keyer was provided from:
        let initial_key = keyer.value_may_panic().get_untracked();
        let initial = if let Some(loading_first_time) =
            self.actively_fetching.get(&(cache_key, initial_key))
        {
            *loading_first_time
        } else {
            false
        };

        let signal = self.add_subscription(
            cache_key,
            keyer,
            move || ArcRwSignal::new(initial),
            SubVariant::IsLoading,
        );

        ArcSignal::derive(move || signal.get().unwrap_or(false))
    }

    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    pub fn add_active_resources_subscription(
        &mut self,
        cache_key: TypeId,
        keyer: MaybeLocal<ArcSignal<KeyHash>>,
    ) -> ArcSignal<usize> {
        // WONTPANIC: this is called from the initial thread the keyer was provided from:
        let initial_key = keyer.value_may_panic().get_untracked();
        let initial = self
            .scope_lookup
            .scopes()
            .get(&cache_key)
            .and_then(|scope| {
                scope
                    .get_dyn_query(&initial_key)
                    .map(|query| query.active_resources_len())
            })
            .unwrap_or(0);

        let signal = self.add_subscription(
            cache_key,
            keyer,
            move || ArcRwSignal::new(initial),
            SubVariant::ActiveResources,
        );

        ArcSignal::derive(move || signal.get().unwrap_or(0))
    }

    pub fn add_value_set_updated_or_removed_subscription(
        &mut self,
        cache_key: TypeId,
        keyer: MaybeLocal<ArcSignal<KeyHash>>,
        v_type_id: TypeId,
    ) -> ArcSignal<Option<u64>> {
        self.add_subscription(
            cache_key,
            keyer,
            move || ArcRwSignal::new(new_value_modified_id()),
            move |signal| SubVariant::ValueSetUpdatedOrRemoved { v_type_id, signal },
        )
    }

    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    pub fn add_events_subscription(
        &mut self,
        cache_key: TypeId,
        keyer: MaybeLocal<ArcSignal<KeyHash>>,
    ) -> ArcSignal<Vec<crate::events::Event>> {
        // WONTPANIC: this is called from the initial thread the keyer was provided from:
        let initial_key = keyer.value_may_panic().get_untracked();
        let initial = self
            .scope_lookup
            .scopes()
            .get(&cache_key)
            .and_then(|scope| {
                scope
                    .get_dyn_query(&initial_key)
                    .map(|query| query.events().to_vec())
            })
            .unwrap_or_default();

        let signal = self.add_subscription(
            cache_key,
            keyer,
            move || ArcRwSignal::new(initial.clone()),
            SubVariant::EventsUpdated,
        );

        ArcSignal::derive(move || signal.get().unwrap_or_default())
    }

    // Returns None when keyer is local and wrong thread.
    fn add_subscription<T>(
        &mut self,
        cache_key: TypeId,
        keyer: MaybeLocal<ArcSignal<KeyHash>>,
        new_signal: impl Fn() -> ArcRwSignal<T> + Send + Sync + 'static,
        new_variant: impl Fn(ArcRwSignal<T>) -> SubVariant + Send + Sync + 'static,
    ) -> ArcSignal<Option<T>>
    where
        T: Clone + Send + Sync + 'static,
    {
        let signal = new_signal();
        let sub_id = new_subscription_id();
        // By including the guard in the derived signal, we can hook into the signal itself being dropped, at which point we can GC the subscriber.
        let new_sub_drop_guard = Arc::new(SubDropGuard {
            subs: self.subs.clone(),
            cache_key,
            sub_id,
        });
        let new_sub = Sub::new(
            new_variant(signal.clone()),
            keyer.clone(),
            &new_sub_drop_guard,
        );
        self.subs
            .lock()
            .entry(cache_key)
            .or_default()
            .insert(sub_id, new_sub);

        ArcSignal::derive(move || {
            // Need to keep it alive till the signal is dropped:
            let _ = new_sub_drop_guard.clone();

            Some(signal.get())
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
                if matches!(
                    sub.variant,
                    SubVariant::IsFetching(_) | SubVariant::IsLoading(_)
                ) && sub
                    .key_hash
                    .value_if_safe()
                    .map(|signal| signal.get_untracked())
                    == Some(key_hash)
                {
                    match &sub.variant {
                        SubVariant::IsFetching(signal) => {
                            // Don't want to trigger if not changing:
                            if !signal.get_untracked() {
                                signal.set(true);
                            }
                        }
                        SubVariant::IsLoading(signal) => {
                            // Don't want to trigger if not changing:
                            if loading_first_time && !signal.get_untracked() {
                                signal.set(true);
                            }
                        }
                        _ => {}
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
                if matches!(
                    sub.variant,
                    SubVariant::IsFetching(_) | SubVariant::IsLoading(_)
                ) && sub
                    .key_hash
                    .value_if_safe()
                    .map(|signal| signal.get_untracked())
                    == Some(key_hash)
                {
                    match &sub.variant {
                        SubVariant::IsFetching(signal) => {
                            // Don't want to trigger if not changing:
                            if signal.get_untracked() {
                                signal.set(false);
                            }
                        }
                        SubVariant::IsLoading(signal) => {
                            // Don't want to trigger if not changing:
                            if loading_first_time && signal.get_untracked() {
                                signal.set(false);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    pub fn notify_active_resource_change(
        &mut self,
        cache_key: TypeId,
        key_hash: KeyHash,
        active_resources: usize,
    ) {
        if let Some(subs) = self.subs.lock().get_mut(&cache_key) {
            for sub in subs.values() {
                if let SubVariant::ActiveResources(signal) = &sub.variant {
                    if sub
                        .key_hash
                        .value_if_safe()
                        .map(|signal| signal.get_untracked())
                        == Some(key_hash)
                    {
                        // Don't want to trigger if not changing:
                        if signal.get_untracked() != active_resources {
                            signal.set(active_resources);
                        }
                    }
                }
            }
        }
    }

    pub fn notify_value_set_updated_or_removed<V>(&mut self, cache_key: TypeId, key_hash: KeyHash)
    where
        V: 'static,
    {
        let current_v_type_id = TypeId::of::<V>();

        if let Some(subs) = self.subs.lock().get_mut(&cache_key) {
            for sub in subs.values() {
                if let SubVariant::ValueSetUpdatedOrRemoved { v_type_id, signal } = &sub.variant {
                    if sub
                        .key_hash
                        .value_if_safe()
                        .map(|signal| signal.get_untracked())
                        == Some(key_hash)
                        && v_type_id == &current_v_type_id
                    {
                        signal.set(new_value_modified_id());
                    }
                }
            }
        }
    }

    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    pub fn notify_events_updated(
        &mut self,
        cache_key: TypeId,
        key_hash: KeyHash,
        new_events: &[crate::events::Event],
    ) {
        if let Some(subs) = self.subs.lock().get_mut(&cache_key) {
            for sub in subs.values() {
                if let SubVariant::EventsUpdated(signal) = &sub.variant {
                    if sub
                        .key_hash
                        .value_if_safe()
                        .map(|signal| signal.get_untracked())
                        == Some(key_hash)
                    {
                        signal.set(new_events.to_vec());
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
    key_hash: MaybeLocal<ArcSignal<KeyHash>>,
    _drop_guard: Weak<SubDropGuard>,
}

impl Sub {
    fn new(
        variant: SubVariant,
        key_hash: MaybeLocal<ArcSignal<KeyHash>>,
        drop_guard: &Arc<SubDropGuard>,
    ) -> Self {
        Self {
            variant,
            key_hash,
            _drop_guard: Arc::downgrade(drop_guard),
        }
    }
}

#[derive(Debug)]
enum SubVariant {
    IsFetching(ArcRwSignal<bool>),
    IsLoading(ArcRwSignal<bool>),
    ValueSetUpdatedOrRemoved {
        v_type_id: TypeId,
        signal: ArcRwSignal<u64>,
    },
    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    ActiveResources(ArcRwSignal<usize>),
    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    EventsUpdated(ArcRwSignal<Vec<crate::events::Event>>),
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
