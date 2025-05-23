use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
    sync::Arc,
    time::Duration,
};

use leptos::prelude::{ArcRwSignal, Set};
use parking_lot::Mutex;
use send_wrapper::SendWrapper;

use crate::{
    QueryOptions, SYNC_TRACK_UPDATE_MARKER,
    cache::ScopeLookup,
    debug_if_devtools_enabled::DebugIfDevtoolsEnabled,
    maybe_local::MaybeLocal,
    options_combine,
    query_scope::{QueryScopeInfo, ScopeCacheKey},
    safe_dt_dur_add,
    utils::{KeyHash, new_buster_id},
    value_with_callbacks::{GcHandle, GcValue, RefetchHandle},
};

pub(crate) struct Query<K, V: 'static> {
    key: MaybeLocal<K>,
    value_maybe_stale: GcValue<V>,
    pub combined_options: QueryOptions,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    invalidation_prefix: Option<Vec<String>>,
    invalidated: bool,
    /// Will always be None on the server, hence the SendWrapper is fine:
    gc_cb: Option<Arc<SendWrapper<Box<dyn Fn() -> bool>>>>,
    /// Will always be None on the server, hence the SendWrapper is fine:
    refetch_cb: Option<Arc<SendWrapper<Box<dyn Fn()>>>>,
    active_resources: Arc<Mutex<HashSet<u64>>>,
    pub buster: ArcRwSignal<u64>,
    scope_lookup: ScopeLookup,
    cache_key: ScopeCacheKey,
    key_hash: KeyHash,
    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    pub events: crate::events::Events,
}

impl<K, V> Drop for Query<K, V> {
    fn drop(&mut self) {
        self.scope_lookup
            .scope_subscriptions_mut()
            .notify_value_set_updated_or_removed(self.cache_key, self.key_hash);
        #[cfg(any(
            all(debug_assertions, feature = "devtools"),
            feature = "devtools-always"
        ))]
        self.scope_lookup
            .scope_subscriptions_mut()
            .notify_active_resource_change(self.cache_key, self.key_hash, 0);
    }
}

#[cfg(any(
    all(debug_assertions, feature = "devtools"),
    feature = "devtools-always"
))]
pub(crate) trait DynQuery {
    fn key_hash(&self) -> &KeyHash;

    fn debug_key(&self) -> crate::utils::DebugValue;

    fn debug_value_may_panic(&self) -> crate::utils::DebugValue;

    fn combined_options(&self) -> QueryOptions;

    fn updated_at(&self) -> chrono::DateTime<chrono::Utc>;

    fn events(&self) -> &[crate::events::Event];

    /// Option when already stale.
    fn till_stale(&self) -> Option<Duration>;

    fn is_invalidated(&self) -> bool;

    fn active_resources_len(&self) -> usize;
}

#[cfg(any(
    all(debug_assertions, feature = "devtools"),
    feature = "devtools-always"
))]
impl<K, V> DynQuery for Query<K, V>
where
    K: DebugIfDevtoolsEnabled + 'static,
    V: DebugIfDevtoolsEnabled + 'static,
{
    fn key_hash(&self) -> &KeyHash {
        &self.key_hash
    }

    fn debug_key(&self) -> crate::utils::DebugValue {
        // SAFETY: should only be called from single threaded frontend (devtools)
        crate::utils::DebugValue::new(self.key.value_may_panic())
    }

    fn debug_value_may_panic(&self) -> crate::utils::DebugValue {
        // SAFETY: should only be called from single threaded frontend (devtools)
        crate::utils::DebugValue::new(self.value_maybe_stale.value().value_may_panic())
    }

    fn combined_options(&self) -> QueryOptions {
        self.combined_options
    }

    fn updated_at(&self) -> chrono::DateTime<chrono::Utc> {
        self.updated_at
    }

    fn events(&self) -> &[crate::events::Event] {
        &self.events
    }

    /// Option when already stale.
    fn till_stale(&self) -> Option<Duration> {
        if self.stale() {
            None
        } else {
            let stale_after = safe_dt_dur_add(self.updated_at, self.combined_options.stale_time());
            let now = chrono::Utc::now();
            let till_stale = stale_after - now;
            if till_stale < chrono::TimeDelta::zero() {
                return None;
            }
            Some(
                till_stale
                    .to_std()
                    .expect("Could not convert to std duration"),
            )
        }
    }

    fn is_invalidated(&self) -> bool {
        self.invalidated
    }

    fn active_resources_len(&self) -> usize {
        self.active_resources.lock().len()
    }
}

impl<K, V> Debug for Query<K, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Query").finish()
    }
}

impl<K, V> Query<K, V> {
    pub fn new(
        client_options: QueryOptions,
        scope_lookup: ScopeLookup,
        query_scope_info: &QueryScopeInfo,
        invalidation_prefix: Option<Vec<String>>,
        key_hash: KeyHash,
        key: MaybeLocal<K>,
        value: MaybeLocal<V>,
        buster: ArcRwSignal<u64>,
        scope_options: Option<QueryOptions>,
        active_resources: Option<Arc<Mutex<HashSet<u64>>>>,
        #[cfg(any(
            all(debug_assertions, feature = "devtools"),
            feature = "devtools-always"
        ))]
        event: crate::events::Event,
    ) -> Self
    where
        K: DebugIfDevtoolsEnabled + Clone + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        let cache_key = query_scope_info.cache_key;
        let combined_options = options_combine(client_options, scope_options);
        let active_resources =
            active_resources.unwrap_or_else(|| Arc::new(Mutex::new(HashSet::new())));

        // Add to the invalidation prefix trie/hierarchy on creation:
        if let Some(invalidation_prefix) = &invalidation_prefix {
            scope_lookup
                .invalidation_trie()
                .insert(invalidation_prefix, (cache_key, key_hash));
        }

        let gc_cb = if cfg!(any(test, not(feature = "ssr")))
            && combined_options.gc_time() < Duration::from_secs(60 * 60 * 24 * 365)
        {
            let active_resources = active_resources.clone();
            // GC is client only (non-ssr) hence can wrap in a SendWrapper:
            let invalidation_prefix = invalidation_prefix.clone();
            Some(Arc::new(SendWrapper::new(Box::new(move || {
                if active_resources.lock().is_empty() {
                    scope_lookup.gc_query::<K, V>(&cache_key, &key_hash);

                    // Remove from the invalidation prefix trie/hierarchy on gc:
                    if let Some(invalidation_prefix) = &invalidation_prefix {
                        scope_lookup
                            .invalidation_trie()
                            .remove(invalidation_prefix, &(cache_key, key_hash));
                    }

                    true
                } else {
                    false
                }
            })
                as Box<dyn Fn() -> bool>)))
        } else {
            None
        };

        let refetch_cb = if cfg!(any(test, not(feature = "ssr")))
            && combined_options.refetch_interval().is_some()
        {
            // Refetching is client only (non-ssr) hence can wrap in a SendWrapper:
            let query_scope_info = query_scope_info.clone();
            Some(Arc::new(SendWrapper::new(Box::new(move || {
                scope_lookup.with_cached_scope_mut::<K, V, _>(
                    &query_scope_info,
                    false,
                    |maybe_scope| {
                        // Invalidation will only trigger a refetch if there are active resources, hence fine to always call:
                        if let Some(scope) = maybe_scope {
                            if let Some(cached) = scope.get_mut(&key_hash) {
                                cached.invalidate();
                                #[cfg(any(
                                    all(debug_assertions, feature = "devtools"),
                                    feature = "devtools-always"
                                ))]
                                {
                                    cached.events.push(crate::events::Event::new(
                                    crate::events::EventVariant::RefetchTriggeredViaInvalidation,
                                ));
                                }
                            }
                        }
                    },
                );
            }) as Box<dyn Fn()>)))
        } else {
            None
        };

        let created_at = chrono::Utc::now();
        Self {
            #[cfg(any(
                all(debug_assertions, feature = "devtools"),
                feature = "devtools-always"
            ))]
            events: crate::events::Events::new(&scope_lookup, cache_key, key_hash, vec![event]),
            key,
            value_maybe_stale: GcValue::new(
                value,
                GcHandle::new(gc_cb.clone(), combined_options.gc_time()),
                RefetchHandle::new(refetch_cb.clone(), combined_options.refetch_interval()),
            ),
            combined_options,
            updated_at: created_at,
            invalidation_prefix,
            invalidated: false,
            gc_cb,
            refetch_cb,
            active_resources,
            buster,
            scope_lookup,
            cache_key,
            key_hash,
        }
    }

    #[cfg(test)]
    pub fn is_invalidated(&self) -> bool {
        self.invalidated
    }

    pub fn mark_resource_active(&self, resource_id: u64) {
        let total_active = {
            let mut guard = self.active_resources.lock();
            guard.insert(resource_id);
            guard.len()
        };
        #[cfg(any(
            all(debug_assertions, feature = "devtools"),
            feature = "devtools-always"
        ))]
        {
            self.scope_lookup
                .scope_subscriptions_mut()
                .notify_active_resource_change(self.cache_key, self.key_hash, total_active);
        }
        let _ = total_active;
    }

    pub fn mark_resource_dropped(&self, resource_id: u64) {
        let total_active = {
            let mut guard = self.active_resources.lock();
            guard.remove(&resource_id);
            guard.len()
        };
        #[cfg(any(
            all(debug_assertions, feature = "devtools"),
            feature = "devtools-always"
        ))]
        {
            self.scope_lookup
                .scope_subscriptions_mut()
                .notify_active_resource_change(self.cache_key, self.key_hash, total_active);
        }
        let _ = total_active;
    }

    pub fn invalidate(&mut self) {
        if !self.invalidated {
            self.invalidated = true;
            // To re-trigger all active resources automatically on manual invalidation:
            self.buster.set(new_buster_id());

            // Invalidate any linked children through the invalidation prefix trie/hierarchy on creation:
            if let Some(invalidation_prefix) = &self.invalidation_prefix {
                let trie = self.scope_lookup.invalidation_trie();
                let mut invalidation_map = HashMap::new();
                for (cache_key, key_hash) in trie.find_with_prefix(invalidation_prefix) {
                    if cache_key == &self.cache_key && *key_hash == self.key_hash {
                        continue;
                    }

                    invalidation_map
                        .entry(*cache_key)
                        .or_insert_with(Vec::new)
                        .push(*key_hash);
                }
                if !invalidation_map.is_empty() {
                    // Not ideal having to spawn, but need to get access to the global lock we'll already be holding in this .invalidate() fn:
                    let scope_lookup = self.scope_lookup;
                    leptos::task::spawn(async move {
                        let mut scopes = scope_lookup.scopes_mut();
                        for (cache_key, key_hashes) in invalidation_map {
                            if let Some(scope) = scopes.get_mut(&cache_key) {
                                scope.invalidate_queries(key_hashes);
                            }
                        }
                    });
                }
            }

            #[cfg(any(
                all(debug_assertions, feature = "devtools"),
                feature = "devtools-always"
            ))]
            {
                self.events.push(crate::events::Event::new(
                    crate::events::EventVariant::Invalidated,
                ));
            }
        }
    }

    pub fn stale(&self) -> bool {
        if self.invalidated {
            true
        } else {
            chrono::Utc::now()
                > safe_dt_dur_add(self.updated_at, self.combined_options.stale_time())
        }
    }

    pub fn key(&self) -> &MaybeLocal<K> {
        &self.key
    }

    pub fn value_maybe_stale(&self) -> &MaybeLocal<V> {
        self.value_maybe_stale.value()
    }

    pub fn set_value(
        &mut self,
        new_value: MaybeLocal<V>,
        track: bool,
        #[cfg(any(
            all(debug_assertions, feature = "devtools"),
            feature = "devtools-always"
        ))]
        event: crate::events::Event,
    ) where
        V: DebugIfDevtoolsEnabled + 'static,
    {
        self.update_value(
            |value| {
                // Only need to update on false, always defaults to true:
                if !track {
                    SYNC_TRACK_UPDATE_MARKER
                        .with(|marker| marker.store(false, std::sync::atomic::Ordering::Relaxed));
                }
                *value = new_value;
            },
            #[cfg(any(
                all(debug_assertions, feature = "devtools"),
                feature = "devtools-always"
            ))]
            event,
        );
    }

    /// Respects SYNC_TRACK_UPDATE_MARKER if set to false during the modifier:
    pub fn update_value<T>(
        &mut self,
        cb: impl FnOnce(&mut MaybeLocal<V>) -> T,
        #[cfg(any(
            all(debug_assertions, feature = "devtools"),
            feature = "devtools-always"
        ))]
        event: crate::events::Event,
    ) -> T
    where
        V: DebugIfDevtoolsEnabled + 'static,
    {
        // Default to true instead overriden during the modifier:
        SYNC_TRACK_UPDATE_MARKER
            .with(|marker| marker.store(true, std::sync::atomic::Ordering::Relaxed));

        let result = cb(self.value_maybe_stale.value_mut());

        let should_track = SYNC_TRACK_UPDATE_MARKER
            .with(|marker| marker.load(std::sync::atomic::Ordering::Relaxed));

        self.value_maybe_stale.reset_callbacks(
            GcHandle::new(self.gc_cb.clone(), self.combined_options.gc_time()),
            RefetchHandle::new(
                self.refetch_cb.clone(),
                self.combined_options.refetch_interval(),
            ),
        );

        #[cfg(any(
            all(debug_assertions, feature = "devtools"),
            feature = "devtools-always"
        ))]
        {
            self.events.push(event);
        }

        self.invalidated = false;
        self.updated_at = chrono::Utc::now();

        if should_track {
            // To update all existing resources:
            self.buster.set(new_buster_id());

            self.scope_lookup
                .scope_subscriptions_mut()
                .notify_value_set_updated_or_removed(self.cache_key, self.key_hash);
        }

        result
    }
}
