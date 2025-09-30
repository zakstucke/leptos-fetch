use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Weak},
};

use leptos::prelude::{ArcRwSignal, ArcSignal, Get, GetUntracked, Set};
use parking_lot::Mutex;

use crate::{
    cache::ScopeLookup,
    maybe_local::MaybeLocal,
    query_scope::ScopeCacheKey,
    utils::{KeyHash, new_sub_listener_id, new_value_modified_id},
};

// cache_key -> (variant_id, key_hash) -> Sub
type SubsInner = HashMap<(ScopeCacheKey, &'static str, KeyHash), Sub>;
type Subs = Arc<parking_lot::Mutex<SubsInner>>;

#[derive(Debug)]
pub(crate) struct ScopeSubs {
    subs: Subs,
    // Used to initialise as true when a subscriber is created whilst a fetch is ongoing:
    // (bool = loading_first_time)
    actively_fetching: Arc<Mutex<HashMap<(ScopeCacheKey, KeyHash), bool>>>,
    #[allow(dead_code)]
    scope_lookup: ScopeLookup,
}

const IS_FETCHING_ID: &str = "IsFetching";
const IS_LOADING_ID: &str = "IsLoading";
const VALUE_SET_UPDATED_OR_REMOVED_ID: &str = "ValueSetUpdatedOrRemoved";
#[cfg(any(
    all(debug_assertions, feature = "devtools"),
    feature = "devtools-always"
))]
const ACTIVE_RESOURCES_ID: &str = "ActiveResources";
#[cfg(any(
    all(debug_assertions, feature = "devtools"),
    feature = "devtools-always"
))]
const EVENTS_UPDATED_ID: &str = "EventsUpdated";

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
        cache_key: ScopeCacheKey,
        keyer: MaybeLocal<ArcSignal<Option<KeyHash>>>,
    ) -> ArcSignal<bool> {
        let actively_fetching = self.actively_fetching.clone();
        let signal = self.add_subscription(
            cache_key,
            keyer.clone().map(|keyer| move || keyer.get()),
            IS_FETCHING_ID,
            move || {
                ArcRwSignal::new(
                    if let Some(initial_key) = keyer
                        .value_if_safe()
                        .and_then(|keyer| keyer.get_untracked())
                    {
                        actively_fetching
                            .lock()
                            .contains_key(&(cache_key, initial_key))
                    } else {
                        false
                    },
                )
            },
            SubVariant::IsFetching,
            move |sub| {
                if let SubVariant::IsFetching(signal) = &sub.variant {
                    signal
                } else {
                    panic!("Expected IsFetching variant")
                }
            },
        );
        ArcSignal::derive(move || signal.get().unwrap_or(false))
    }

    pub fn add_is_loading_subscription(
        &mut self,
        cache_key: ScopeCacheKey,
        keyer: MaybeLocal<ArcSignal<Option<KeyHash>>>,
    ) -> ArcSignal<bool> {
        let actively_fetching = self.actively_fetching.clone();
        let signal = self.add_subscription(
            cache_key,
            keyer.clone().map(|keyer| move || keyer.get()),
            IS_LOADING_ID,
            move || {
                ArcRwSignal::new(
                    if let Some(initial_key) = keyer
                        .value_if_safe()
                        .and_then(|keyer| keyer.get_untracked())
                    {
                        if let Some(loading_first_time) =
                            actively_fetching.lock().get(&(cache_key, initial_key))
                        {
                            *loading_first_time
                        } else {
                            false
                        }
                    } else {
                        false
                    },
                )
            },
            SubVariant::IsLoading,
            move |sub| {
                if let SubVariant::IsLoading(signal) = &sub.variant {
                    signal
                } else {
                    panic!("Expected IsLoading variant")
                }
            },
        );

        ArcSignal::derive(move || signal.get().unwrap_or(false))
    }

    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    pub fn add_active_resources_subscription(
        &mut self,
        cache_key: ScopeCacheKey,
        keyer: MaybeLocal<ArcSignal<Option<KeyHash>>>,
    ) -> ArcSignal<usize> {
        let scope_lookup = self.scope_lookup;
        let signal = self.add_subscription(
            cache_key,
            keyer.clone().map(|keyer| move || keyer.get()),
            ACTIVE_RESOURCES_ID,
            move || {
                ArcRwSignal::new(
                    if let Some(initial_key) = keyer
                        .value_if_safe()
                        .and_then(|keyer| keyer.get_untracked())
                    {
                        scope_lookup
                            .scopes()
                            .get(&cache_key)
                            .and_then(|scope| {
                                scope
                                    .get_dyn_query(&initial_key)
                                    .map(|query| query.active_resources_len())
                            })
                            .unwrap_or(0)
                    } else {
                        0
                    },
                )
            },
            SubVariant::ActiveResources,
            move |sub| {
                if let SubVariant::ActiveResources(signal) = &sub.variant {
                    signal
                } else {
                    panic!("Expected ActiveResources variant")
                }
            },
        );

        ArcSignal::derive(move || signal.get().unwrap_or(0))
    }

    pub fn add_value_set_updated_or_removed_subscription(
        &mut self,
        cache_key: ScopeCacheKey,
        key_get: MaybeLocal<impl Fn() -> Option<KeyHash> + 'static>,
    ) -> ArcSignal<Option<u64>> {
        self.add_subscription(
            cache_key,
            key_get,
            VALUE_SET_UPDATED_OR_REMOVED_ID,
            move || ArcRwSignal::new(new_value_modified_id()),
            SubVariant::ValueSetUpdatedOrRemoved,
            |sub| {
                if let SubVariant::ValueSetUpdatedOrRemoved(signal) = &sub.variant {
                    signal
                } else {
                    panic!("Expected ValueSetUpdatedOrRemoved variant")
                }
            },
        )
    }

    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    pub fn add_events_subscription(
        &mut self,
        cache_key: ScopeCacheKey,
        keyer: MaybeLocal<ArcSignal<Option<KeyHash>>>,
    ) -> ArcSignal<Vec<crate::events::Event>> {
        let scope_lookup = self.scope_lookup;
        let signal = self.add_subscription(
            cache_key,
            keyer.clone().map(|keyer| move || keyer.get()),
            EVENTS_UPDATED_ID,
            move || {
                ArcRwSignal::new(
                    if let Some(initial_key) = keyer
                        .value_if_safe()
                        .and_then(|keyer| keyer.get_untracked())
                    {
                        scope_lookup
                            .scopes()
                            .get(&cache_key)
                            .and_then(|scope| {
                                scope
                                    .get_dyn_query(&initial_key)
                                    .map(|query| query.events().to_vec())
                            })
                            .unwrap_or_default()
                    } else {
                        vec![]
                    },
                )
            },
            SubVariant::EventsUpdated,
            move |sub| {
                if let SubVariant::EventsUpdated(signal) = &sub.variant {
                    signal
                } else {
                    panic!("Expected EventsUpdated variant")
                }
            },
        );

        ArcSignal::derive(move || signal.get().unwrap_or_default())
    }

    // Returns None when keyer is local and wrong thread or no key.
    fn add_subscription<T>(
        &mut self,
        cache_key: ScopeCacheKey,
        key_get: MaybeLocal<impl Fn() -> Option<KeyHash> + 'static>,
        variant_id: &'static str,
        new_signal: impl Fn() -> ArcRwSignal<T> + Send + Sync + 'static,
        new_variant: impl Fn(ArcRwSignal<T>) -> SubVariant + Send + Sync + 'static,
        signal_from_variant: impl Fn(&Sub) -> &ArcRwSignal<T> + Send + Sync + 'static,
    ) -> ArcSignal<Option<T>>
    where
        T: Clone + Send + Sync + 'static,
    {
        let listener_id = new_sub_listener_id();

        // By including the guard in the derived signal, we can hook into the signal itself being dropped, at which point we can GC the subscriber.
        let new_sub_drop_guard = Arc::new(SubDropGuard {
            listener_id,
            variant_id,
            subs: self.subs.clone(),
            cache_key,
            key_hash: Mutex::new(None),
        });
        let subs = self.subs.clone();

        ArcSignal::derive(move || {
            // Need to keep it alive till the signal is dropped:
            let _ = new_sub_drop_guard.clone();

            if let Some(key_get) = key_get.value_if_safe() {
                if let Some(key_hash) = key_get() {
                    let mut guard = subs.lock();
                    // The drop guard handles the number of listeners a Sub has, and gc's it when it reaches 0.
                    new_sub_drop_guard.set_key_hash(&mut guard, key_hash);
                    let sub = guard
                        .entry((cache_key, variant_id, key_hash))
                        .or_insert_with(|| {
                            Sub::new(new_variant(new_signal()), &new_sub_drop_guard)
                        });
                    // Add the listener to the sub in case it wasn't already there:
                    sub.add_listener(listener_id);
                    Some(signal_from_variant(sub).get())
                } else {
                    None
                }
            } else {
                None
            }
        })
    }

    /// NOTE: use with_notify_fetching instead.
    pub fn notify_fetching_start(
        &mut self,
        cache_key: ScopeCacheKey,
        key_hash: KeyHash,
        loading_first_time: bool,
    ) {
        self.actively_fetching
            .lock()
            .insert((cache_key, key_hash), loading_first_time);
        let subs_guard = self.subs.lock();
        for sub in [
            subs_guard.get(&(cache_key, IS_FETCHING_ID, key_hash)),
            subs_guard.get(&(cache_key, IS_LOADING_ID, key_hash)),
        ]
        .into_iter()
        .flatten()
        {
            if matches!(
                sub.variant,
                SubVariant::IsFetching(_) | SubVariant::IsLoading(_)
            ) {
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

    /// NOTE: use with_notify_fetching instead.
    pub fn notify_fetching_finish(
        &mut self,
        cache_key: ScopeCacheKey,
        key_hash: KeyHash,
        loading_first_time: bool,
    ) {
        self.actively_fetching.lock().remove(&(cache_key, key_hash));
        let subs_guard = self.subs.lock();
        for sub in [
            subs_guard.get(&(cache_key, IS_FETCHING_ID, key_hash)),
            subs_guard.get(&(cache_key, IS_LOADING_ID, key_hash)),
        ]
        .into_iter()
        .flatten()
        {
            if matches!(
                sub.variant,
                SubVariant::IsFetching(_) | SubVariant::IsLoading(_)
            ) {
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

    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    pub fn notify_active_resource_change(
        &mut self,
        cache_key: ScopeCacheKey,
        key_hash: KeyHash,
        active_resources: usize,
    ) {
        if let Some(sub) = self
            .subs
            .lock()
            .get_mut(&(cache_key, ACTIVE_RESOURCES_ID, key_hash))
            && let SubVariant::ActiveResources(signal) = &sub.variant
        {
            // Don't want to trigger if not changing:
            if signal.get_untracked() != active_resources {
                signal.set(active_resources);
            }
        }
    }

    pub fn notify_value_set_updated_or_removed(
        &mut self,
        cache_key: ScopeCacheKey,
        key_hash: KeyHash,
    ) {
        if let Some(sub) =
            self.subs
                .lock()
                .get_mut(&(cache_key, VALUE_SET_UPDATED_OR_REMOVED_ID, key_hash))
            && let SubVariant::ValueSetUpdatedOrRemoved(signal) = &sub.variant
        {
            signal.set(new_value_modified_id());
        }
    }

    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    pub fn notify_events_updated(
        &mut self,
        cache_key: ScopeCacheKey,
        key_hash: KeyHash,
        new_events: &[crate::events::Event],
    ) {
        if let Some(sub) = self
            .subs
            .lock()
            .get_mut(&(cache_key, EVENTS_UPDATED_ID, key_hash))
            && let SubVariant::EventsUpdated(signal) = &sub.variant
        {
            signal.set(new_events.to_vec());
        }
    }

    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.subs.lock().len()
    }
}

#[derive(Debug)]
struct Sub {
    variant: SubVariant,
    listeners: HashSet<u64>,
    _drop_guard: Weak<SubDropGuard>,
}

impl Sub {
    fn new(variant: SubVariant, drop_guard: &Arc<SubDropGuard>) -> Self {
        Self {
            variant,
            listeners: HashSet::new(),
            _drop_guard: Arc::downgrade(drop_guard),
        }
    }

    fn add_listener(&mut self, listener_id: u64) {
        self.listeners.insert(listener_id);
    }

    fn remove_listener(&mut self, listener_id: u64) {
        self.listeners.remove(&listener_id);
    }

    fn orphaned(&self) -> bool {
        self.listeners.is_empty()
    }
}

#[derive(Debug)]
enum SubVariant {
    IsFetching(ArcRwSignal<bool>),
    IsLoading(ArcRwSignal<bool>),
    ValueSetUpdatedOrRemoved(ArcRwSignal<u64>),
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
    listener_id: u64,
    variant_id: &'static str,
    key_hash: Mutex<Option<KeyHash>>,
    cache_key: ScopeCacheKey,
    subs: Subs,
}

impl SubDropGuard {
    fn set_key_hash(&self, subs_guard: &mut SubsInner, key_hash: KeyHash) {
        let mut guard = self.key_hash.lock();
        if let Some(old_key_hash) = guard.take()
            && old_key_hash != key_hash
            && let Some(sub) = subs_guard.get_mut(&(self.cache_key, self.variant_id, old_key_hash))
        {
            sub.remove_listener(self.listener_id);
            if sub.orphaned() {
                subs_guard.remove(&(self.cache_key, self.variant_id, old_key_hash));
            }
        }
        *guard = Some(key_hash);
    }
}

impl Drop for SubDropGuard {
    fn drop(&mut self) {
        let mut subs_guard = self.subs.lock();
        if let Some(key_hash) = self.key_hash.lock().take()
            && let Some(sub) = subs_guard.get_mut(&(self.cache_key, self.variant_id, key_hash))
        {
            sub.remove_listener(self.listener_id);
            if sub.orphaned() {
                subs_guard.remove(&(self.cache_key, self.variant_id, key_hash));
            }
        }
    }
}
