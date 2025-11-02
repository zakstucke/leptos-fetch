use std::{
    any::Any,
    collections::{HashMap, hash_map::Entry},
    future::Future,
    ops::{Deref, DerefMut},
    sync::Arc,
};

use leptos::prelude::{ArcRwSignal, ArcSignal};

use crate::{
    cache_scope::{QueryAbortReason, Scope},
    debug_if_devtools_enabled::DebugIfDevtoolsEnabled,
    global::{GLOBAL_INVALIDATION_TRIE, GLOBAL_SCOPE_LOOKUPS, GLOBAL_SCOPE_SUBSCRIPTION_LOOKUPS},
    maybe_local::MaybeLocal,
    query::Query,
    query_scope::{QueryScopeInfo, ScopeCacheKey},
    subs_scope::ScopeSubs,
    trie::Trie,
    utils::{KeyHash, OnDrop},
};

pub(crate) trait Busters: 'static {
    fn invalidate_scope(
        &mut self,
        invalidation_type: QueryAbortReason,
    ) -> Box<dyn FnOnce(&mut Scopes) -> Option<Box<dyn FnOnce()>>>;

    fn busters(&self) -> Vec<ArcRwSignal<u64>>;
}

impl<K, V> Busters for Scope<K, V>
where
    K: DebugIfDevtoolsEnabled + Clone + 'static,
    V: DebugIfDevtoolsEnabled + 'static,
{
    fn invalidate_scope(
        &mut self,
        invalidation_type: QueryAbortReason,
    ) -> Box<dyn FnOnce(&mut Scopes) -> Option<Box<dyn FnOnce()>>> {
        let mut cbs_scopes = vec![];
        for query in self.all_queries_mut_include_pending() {
            let cb_scopes = query.invalidate(invalidation_type);
            cbs_scopes.push(cb_scopes);
        }
        Box::new(move |scopes| {
            let mut cbs_external = vec![];
            for cb in cbs_scopes {
                if let Some(cb_external) = cb(scopes) {
                    cbs_external.push(cb_external);
                }
            }
            if cbs_external.is_empty() {
                None
            } else {
                Some(Box::new(move || {
                    for cb in cbs_external {
                        cb();
                    }
                }))
            }
        })
    }

    fn busters(&self) -> Vec<ArcRwSignal<u64>> {
        self.all_queries()
            .map(|query| query.buster.clone())
            .collect::<Vec<_>>()
    }
}

pub(crate) trait ScopeTrait: Busters + Send + Sync + 'static {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn clear(&mut self);
    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    fn get_dyn_query(&self, key_hash: &KeyHash) -> Option<&dyn crate::query::DynQuery>;
    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    fn iter_dyn_queries(&self) -> Vec<&dyn crate::query::DynQuery>;
    fn invalidate_queries(
        &mut self,
        key_hashes: Vec<KeyHash>,
        invalidation_type: QueryAbortReason,
    ) -> Box<dyn FnOnce(&mut Scopes) -> Option<Box<dyn FnOnce()>>>;
    fn cache_key(&self) -> ScopeCacheKey;
    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    fn title(&self) -> &Arc<String>;
    #[cfg(test)]
    fn total_cached_queries(&self) -> usize;
}

impl<K, V> ScopeTrait for Scope<K, V>
where
    K: DebugIfDevtoolsEnabled + Clone + 'static,
    V: DebugIfDevtoolsEnabled + Clone + 'static,
{
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn clear(&mut self) {
        self.clear_cache()
    }

    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    fn get_dyn_query(&self, key_hash: &KeyHash) -> Option<&dyn crate::query::DynQuery> {
        self.get(key_hash)
            .map(|query| query as &dyn crate::query::DynQuery)
    }

    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    fn iter_dyn_queries(&self) -> Vec<&dyn crate::query::DynQuery> {
        self.all_queries()
            .map(|query| query as &dyn crate::query::DynQuery)
            .collect::<Vec<_>>()
    }

    fn invalidate_queries(
        &mut self,
        key_hashes: Vec<KeyHash>,
        invalidation_type: QueryAbortReason,
    ) -> Box<dyn FnOnce(&mut Scopes) -> Option<Box<dyn FnOnce()>>> {
        let mut cbs_scopes = vec![];
        for key_hash in key_hashes {
            if let Some(query) = self.get_mut(&key_hash) {
                let cb_scopes = query.invalidate(invalidation_type);
                cbs_scopes.push(cb_scopes);
            }
        }
        Box::new(move |scopes| {
            let mut cbs_external = vec![];
            for cb in cbs_scopes {
                if let Some(cb_external) = cb(scopes) {
                    cbs_external.push(cb_external);
                }
            }
            if cbs_external.is_empty() {
                None
            } else {
                Some(Box::new(move || {
                    for cb in cbs_external {
                        cb();
                    }
                }))
            }
        })
    }

    fn cache_key(&self) -> ScopeCacheKey {
        self.query_scope_info.cache_key
    }

    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    fn title(&self) -> &Arc<String> {
        &self.query_scope_info.title
    }

    #[cfg(test)]
    fn total_cached_queries(&self) -> usize {
        self.cache_size()
    }
}

/// Internalising the sharing of a cache, which needs to be sync on the backend,
/// but not on the frontend where LocalResource's are used.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ScopeLookup {
    // Storing key mapping to a global lookup to allow it and the QueryClient to be Copy.
    // Shouldn't be an issue as there should be only 1 ScopeLookup per QueryClient,
    // and only one QueryClient, or at most a few per app.
    // Using this rather than e.g. a leptos StoredValue, just to have no chance of external disposed errors.
    pub scope_id: u64,
}

#[derive(Default)]
pub(crate) struct Scopes {
    scopes: HashMap<ScopeCacheKey, Box<dyn ScopeTrait>>,
    pub(crate) refetch_enabled: Option<ArcSignal<bool>>,
}

impl Deref for Scopes {
    type Target = HashMap<ScopeCacheKey, Box<dyn ScopeTrait>>;

    fn deref(&self) -> &Self::Target {
        &self.scopes
    }
}

impl DerefMut for Scopes {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.scopes
    }
}

impl ScopeLookup {
    // Returns None if scope not found, likely because it was cleaned up and this is happening during drop:
    pub fn try_scopes(&self) -> Option<parking_lot::MappedRwLockReadGuard<'_, Scopes>> {
        let guard = GLOBAL_SCOPE_LOOKUPS.read();
        if !guard.contains_key(&self.scope_id) {
            return None;
        }
        Some(parking_lot::RwLockReadGuard::map(
            guard,
            |scope_lookups: &HashMap<u64, Scopes>| {
                scope_lookups
                    .get(&self.scope_id)
                    .expect("leptos-fetch bug: scope not found, even though just checked")
            },
        ))
    }

    #[track_caller]
    pub fn scopes(&self) -> parking_lot::MappedRwLockReadGuard<'_, Scopes> {
        let location = std::panic::Location::caller();
        parking_lot::RwLockReadGuard::map(
            GLOBAL_SCOPE_LOOKUPS.read(),
            |scope_lookups: &HashMap<u64, Scopes>| {
                scope_lookups
                    .get(&self.scope_id)
                    .unwrap_or_else(|| panic!("leptos-fetch bug: scope not found at {location}",))
            },
        )
    }

    pub fn scopes_mut(&self) -> parking_lot::MappedRwLockWriteGuard<'_, Scopes> {
        parking_lot::RwLockWriteGuard::map(
            GLOBAL_SCOPE_LOOKUPS.write(),
            |scope_lookups: &mut HashMap<u64, Scopes>| {
                scope_lookups
                    .get_mut(&self.scope_id)
                    .expect("leptos-fetch bug: scope not found")
            },
        )
    }

    pub fn invalidation_trie(
        &self,
    ) -> parking_lot::MappedMutexGuard<'_, Trie<(ScopeCacheKey, KeyHash)>> {
        parking_lot::MutexGuard::map(
            GLOBAL_INVALIDATION_TRIE.lock(),
            |invalidation_trie: &mut HashMap<u64, Trie<(ScopeCacheKey, KeyHash)>>| {
                invalidation_trie
                    .get_mut(&self.scope_id)
                    .expect("invalidation_trie not found (bug)")
            },
        )
    }

    pub fn try_scope_subscriptions_mut(
        &self,
    ) -> Option<parking_lot::MappedMutexGuard<'_, ScopeSubs>> {
        let guard = GLOBAL_SCOPE_SUBSCRIPTION_LOOKUPS.lock();
        if !guard.contains_key(&self.scope_id) {
            return None;
        }
        Some(parking_lot::MutexGuard::map(
            guard,
            |sub_lookups: &mut HashMap<u64, ScopeSubs>| {
                sub_lookups
                    .get_mut(&self.scope_id)
                    .expect("Scope not found (bug)")
            },
        ))
    }

    pub fn scope_subscriptions_mut(&self) -> parking_lot::MappedMutexGuard<'_, ScopeSubs> {
        parking_lot::MutexGuard::map(
            GLOBAL_SCOPE_SUBSCRIPTION_LOOKUPS.lock(),
            |sub_lookups: &mut HashMap<u64, ScopeSubs>| {
                sub_lookups
                    .get_mut(&self.scope_id)
                    .expect("Scope not found (bug)")
            },
        )
    }

    pub async fn with_notify_fetching<T>(
        &self,
        cache_key: ScopeCacheKey,
        key_hash: KeyHash,
        loading_first_time: bool,
        fut: impl Future<Output = T>,
    ) -> T {
        self.scope_subscriptions_mut().notify_fetching_start(
            cache_key,
            key_hash,
            loading_first_time,
        );
        // Notifying finished in a drop guard just in case e.g. future was cancelled to make sure still runs:
        // Not sure if this is actually needed, added it whilst trying to fix a different bug, may as well keep it:
        let _notify_fetching_finished_guard = OnDrop::new({
            let self_ = *self;
            move || {
                if let Some(mut subs) = self_.try_scope_subscriptions_mut() {
                    subs.notify_fetching_finish(cache_key, key_hash, loading_first_time);
                }
            }
        });
        fut.await
    }

    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    pub fn client_subscriptions_mut(
        &self,
    ) -> parking_lot::MappedMutexGuard<'_, crate::subs_client::ClientSubs> {
        parking_lot::MutexGuard::map(
            crate::global::GLOBAL_CLIENT_SUBSCRIPTION_LOOKUPS.lock(),
            |sub_lookups: &mut HashMap<u64, crate::subs_client::ClientSubs>| {
                sub_lookups
                    .get_mut(&self.scope_id)
                    .expect("Scope not found (bug)")
            },
        )
    }

    pub fn mark_resource_dropped<K, V>(
        &self,
        key_hash: &KeyHash,
        cache_key: &ScopeCacheKey,
        resource_id: u64,
    ) where
        K: DebugIfDevtoolsEnabled + Clone + 'static,
        V: 'static,
    {
        if let Some(query) = self
            .try_scopes() // Called from drop handlers, might have already been cleaned up
            .as_ref()
            .and_then(|scope| scope.get(cache_key))
            .and_then(|scope_cache| {
                scope_cache
                    .as_any()
                    .downcast_ref::<Scope<K, V>>()
                    .expect("Cache entry type mismatch.")
                    .get(key_hash)
            })
        {
            query.mark_resource_dropped(resource_id);
        }
    }

    pub fn fetcher_mutex<K, V>(
        &self,
        key_hash: KeyHash,
        query_scope_info: &QueryScopeInfo,
    ) -> Arc<futures::lock::Mutex<()>>
    where
        K: DebugIfDevtoolsEnabled + Clone + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        self.scopes_mut()
            .entry(query_scope_info.cache_key)
            .or_insert_with(|| Box::new(Scope::<K, V>::new(*self, query_scope_info.clone())))
            .as_any_mut()
            .downcast_mut::<Scope<K, V>>()
            .expect("Cache entry type mismatch.")
            .fetcher_mutexes
            .entry(key_hash)
            .or_insert_with(|| Arc::new(futures::lock::Mutex::new(())))
            .clone()
    }

    pub fn with_cached_query<K, V, T>(
        &self,
        key_hash: &KeyHash,
        cache_key: &ScopeCacheKey,
        cb: impl FnOnce(Option<&Query<K, V>>) -> T,
    ) -> T
    where
        K: DebugIfDevtoolsEnabled + Clone + 'static,
        V: DebugIfDevtoolsEnabled + 'static,
    {
        let guard = self.scopes();
        let maybe_query = guard.get(cache_key).and_then(|scope_cache| {
            scope_cache
                .as_any()
                .downcast_ref::<Scope<K, V>>()
                .expect("Cache entry type mismatch.")
                .get(key_hash)
        });
        cb(maybe_query)
    }

    pub fn with_cached_scope_mut<K, V, T, P>(
        &self,
        scopes: &mut Scopes,
        scope_cache_key: ScopeCacheKey,
        on_scope_missing: OnScopeMissing,
        mut scopes_prehook: impl FnMut(&mut Scopes) -> P,
        cb: impl FnOnce(Option<&mut Scope<K, V>>, P) -> T,
    ) -> T
    where
        K: DebugIfDevtoolsEnabled + Clone + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        let prehook_result = scopes_prehook(scopes);
        let maybe_scope =
            match scopes.entry(scope_cache_key) {
                Entry::Occupied(entry) => Some(entry.into_mut()),
                Entry::Vacant(entry) => match on_scope_missing {
                    OnScopeMissing::Skip => None,
                    OnScopeMissing::Create(query_scope_info) => Some(entry.insert(Box::new(
                        Scope::<K, V>::new(*self, query_scope_info.clone()),
                    ))),
                },
            };

        if let Some(scope) = maybe_scope {
            cb(
                Some(
                    scope
                        .as_any_mut()
                        .downcast_mut::<Scope<K, V>>()
                        .expect("Cache entry type mismatch."),
                ),
                prehook_result,
            )
        } else {
            cb(None, prehook_result)
        }
    }

    pub fn gc_query<K, V>(&self, cache_key: &ScopeCacheKey, key_hash: &KeyHash)
    where
        K: DebugIfDevtoolsEnabled + Clone + 'static,
        V: DebugIfDevtoolsEnabled + 'static,
    {
        let mut guard = self.scopes_mut();
        let remove_scope = if let Some(scope) = guard.get_mut(cache_key) {
            let scope = scope
                .as_any_mut()
                .downcast_mut::<Scope<K, V>>()
                .expect("Cache entry type mismatch.");
            scope.remove_entry(key_hash);
            scope.is_empty()
        } else {
            false
        };
        if remove_scope {
            guard.remove(cache_key);
        }
    }

    pub fn prepare_invalidation_channel<K, V>(
        &self,
        query_scope_info: &QueryScopeInfo,
        key_hash: KeyHash,
        maybe_local_key: &MaybeLocal<K>,
    ) -> futures::channel::oneshot::Receiver<QueryAbortReason>
    where
        K: DebugIfDevtoolsEnabled + Clone + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        let (query_abort_tx, query_abort_rx) =
            futures::channel::oneshot::channel::<QueryAbortReason>();
        self.with_cached_scope_mut::<K, V, _, _>(
            &mut self.scopes_mut(),
            query_scope_info.cache_key,
            OnScopeMissing::Create(query_scope_info),
            |_| {},
            |scope, _| {
                let scope = scope.expect("provided a default");
                if let Some(cached) = scope.get_mut(&key_hash) {
                    cached.set_query_abort_tx(query_abort_tx);
                } else {
                    scope.insert_pending((*maybe_local_key).clone(), query_abort_tx, key_hash);
                }
            },
        );
        query_abort_rx
    }
}

pub(crate) enum OnScopeMissing<'a> {
    Skip,
    Create(&'a QueryScopeInfo),
}

pub(crate) struct CachedOrFetchCbInput<'a, K: 'static, V: 'static> {
    pub cached: &'a Query<K, V>,
    pub variant: CachedOrFetchCbInputVariant,
}

#[derive(Debug)]
pub(crate) enum CachedOrFetchCbInputVariant {
    CachedUntouched,
    CachedUpdated,
    Fresh,
}
pub(crate) enum CachedOrFetchCbOutput<T> {
    Refetch,
    Return(T),
}
