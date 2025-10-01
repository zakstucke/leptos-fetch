use std::{
    any::Any,
    collections::{HashMap, hash_map::Entry},
    future::Future,
    hash::Hash,
    ops::{Deref, DerefMut},
    sync::{Arc, LazyLock},
};

use futures::FutureExt;
use leptos::prelude::{ArcRwSignal, ArcSignal};

use crate::{
    QueryOptions,
    cache_scope::{QueryAbortReason, Scope},
    debug_if_devtools_enabled::DebugIfDevtoolsEnabled,
    maybe_local::MaybeLocal,
    query::Query,
    query_scope::{QueryScopeInfo, ScopeCacheKey},
    subs_scope::ScopeSubs,
    trie::Trie,
    utils::{KeyHash, OnDrop, OwnerChain, ResetInvalidated, new_buster_id, new_scope_id},
};

pub(crate) trait Busters: 'static {
    fn invalidate_scope(&mut self, invalidation_type: QueryAbortReason);

    fn busters(&self) -> Vec<ArcRwSignal<u64>>;
}

impl<K, V> Busters for Scope<K, V>
where
    K: DebugIfDevtoolsEnabled + 'static,
    V: DebugIfDevtoolsEnabled + 'static,
{
    fn invalidate_scope(&mut self, invalidation_type: QueryAbortReason) {
        for query in self.all_queries_mut_include_pending() {
            query.invalidate(invalidation_type);
        }
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
    fn invalidate_queries(&mut self, key_hashes: Vec<KeyHash>, invalidation_type: QueryAbortReason);
    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    fn title(&self) -> &Arc<String>;
    #[cfg(test)]
    fn size(&self) -> usize;
}

impl<K, V> ScopeTrait for Scope<K, V>
where
    K: DebugIfDevtoolsEnabled + 'static,
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
    ) {
        for key_hash in key_hashes {
            if let Some(query) = self.get_mut(&key_hash) {
                query.invalidate(invalidation_type);
            }
        }
    }

    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    fn title(&self) -> &Arc<String> {
        &self.query_scope_info.title
    }

    #[cfg(test)]
    fn size(&self) -> usize {
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

static SCOPE_LOOKUPS: LazyLock<parking_lot::RwLock<HashMap<u64, Scopes>>> =
    LazyLock::new(|| parking_lot::RwLock::new(HashMap::new()));

static SCOPE_SUBSCRIPTION_LOOKUPS: LazyLock<parking_lot::Mutex<HashMap<u64, ScopeSubs>>> =
    LazyLock::new(|| parking_lot::Mutex::new(HashMap::new()));

static INVALIDATION_TRIE: LazyLock<
    parking_lot::Mutex<HashMap<u64, Trie<(ScopeCacheKey, KeyHash)>>>,
> = LazyLock::new(|| parking_lot::Mutex::new(HashMap::new()));

#[cfg(any(
    all(debug_assertions, feature = "devtools"),
    feature = "devtools-always"
))]
static CLIENT_SUBSCRIPTION_LOOKUPS: LazyLock<
    parking_lot::Mutex<HashMap<u64, crate::subs_client::ClientSubs>>,
> = LazyLock::new(|| parking_lot::Mutex::new(HashMap::new()));

impl ScopeLookup {
    pub fn new() -> Self {
        let scope_id = new_scope_id();

        let result = Self { scope_id };

        SCOPE_LOOKUPS.write().insert(scope_id, Default::default());
        SCOPE_SUBSCRIPTION_LOOKUPS
            .lock()
            .insert(scope_id, ScopeSubs::new(result));

        INVALIDATION_TRIE
            .lock()
            .insert(scope_id, Default::default());

        #[cfg(any(
            all(debug_assertions, feature = "devtools"),
            feature = "devtools-always"
        ))]
        CLIENT_SUBSCRIPTION_LOOKUPS
            .lock()
            .insert(scope_id, crate::subs_client::ClientSubs::new(result));

        result
    }

    pub fn scopes(&self) -> parking_lot::MappedRwLockReadGuard<'_, Scopes> {
        parking_lot::RwLockReadGuard::map(
            SCOPE_LOOKUPS.read(),
            |scope_lookups: &HashMap<u64, Scopes>| {
                scope_lookups
                    .get(&self.scope_id)
                    .expect("Scope not found (bug)")
            },
        )
    }

    pub fn scopes_mut(&self) -> parking_lot::MappedRwLockWriteGuard<'_, Scopes> {
        parking_lot::RwLockWriteGuard::map(
            SCOPE_LOOKUPS.write(),
            |scope_lookups: &mut HashMap<u64, Scopes>| {
                scope_lookups
                    .get_mut(&self.scope_id)
                    .expect("Scope not found (bug)")
            },
        )
    }

    pub fn invalidation_trie(
        &self,
    ) -> parking_lot::MappedMutexGuard<'_, Trie<(ScopeCacheKey, KeyHash)>> {
        parking_lot::MutexGuard::map(
            INVALIDATION_TRIE.lock(),
            |invalidation_trie: &mut HashMap<u64, Trie<(ScopeCacheKey, KeyHash)>>| {
                invalidation_trie
                    .get_mut(&self.scope_id)
                    .expect("invalidation_trie not found (bug)")
            },
        )
    }

    pub fn scope_subscriptions_mut(&self) -> parking_lot::MappedMutexGuard<'_, ScopeSubs> {
        parking_lot::MutexGuard::map(
            SCOPE_SUBSCRIPTION_LOOKUPS.lock(),
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
                self_.scope_subscriptions_mut().notify_fetching_finish(
                    cache_key,
                    key_hash,
                    loading_first_time,
                );
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
            CLIENT_SUBSCRIPTION_LOOKUPS.lock(),
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
        K: DebugIfDevtoolsEnabled + 'static,
        V: 'static,
    {
        if let Some(query) = self.scopes().get(cache_key).and_then(|scope_cache| {
            scope_cache
                .as_any()
                .downcast_ref::<Scope<K, V>>()
                .expect("Cache entry type mismatch.")
                .get(key_hash)
        }) {
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
        K: DebugIfDevtoolsEnabled + 'static,
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
        query_scope_info: &QueryScopeInfo,
        create_scope_if_missing: bool,
        mut scopes_prehook: impl FnMut(&mut Scopes) -> P,
        cb: impl FnOnce(Option<&mut Scope<K, V>>, P) -> T,
    ) -> T
    where
        K: DebugIfDevtoolsEnabled + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        let mut scopes = self.scopes_mut();
        let prehook_result = scopes_prehook(&mut scopes);
        let maybe_scope = match scopes.entry(query_scope_info.cache_key) {
            Entry::Occupied(entry) => Some(entry.into_mut()),
            Entry::Vacant(entry) => {
                if create_scope_if_missing {
                    Some(entry.insert(Box::new(Scope::<K, V>::new(
                        *self,
                        query_scope_info.clone(),
                    ))))
                } else {
                    None
                }
            }
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
        K: DebugIfDevtoolsEnabled + 'static,
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
            query_scope_info,
            true,
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

    pub async fn cached_or_fetch<K, V, T>(
        &self,
        client_options: QueryOptions,
        scope_options: Option<QueryOptions>,
        maybe_buster_if_uncached: Option<ArcRwSignal<u64>>,
        query_scope_info: &QueryScopeInfo,
        invalidation_prefix: Option<Vec<String>>,
        key: &K,
        fetcher: impl AsyncFn(K) -> MaybeLocal<V>,
        return_cb: impl Fn(CachedOrFetchCbInput<K, V>) -> CachedOrFetchCbOutput<T>,
        maybe_preheld_fetcher_mutex_guard: Option<&futures::lock::MutexGuard<'_, ()>>,
        lazy_maybe_local_key: impl FnOnce() -> MaybeLocal<K>,
        owner_chain: &OwnerChain,
    ) -> T
    where
        K: DebugIfDevtoolsEnabled + Hash + Clone + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        let key_hash = KeyHash::new(key);
        let mut cached_buster = None;
        let next_directive = self.with_cached_query::<K, V, _>(
            &key_hash,
            &query_scope_info.cache_key,
            |maybe_cached| {
                if let Some(cached) = maybe_cached {
                    cached_buster = Some(cached.buster.clone());
                    return_cb(CachedOrFetchCbInput {
                        cached,
                        variant: CachedOrFetchCbInputVariant::CachedUntouched,
                    })
                } else {
                    CachedOrFetchCbOutput::Refetch
                }
            },
        );

        match next_directive {
            CachedOrFetchCbOutput::Return(value) => value,
            CachedOrFetchCbOutput::Refetch => {
                // Will probably need to fetch, unless someone fetches whilst trying to get hold of the fetch mutex:
                let fetcher_mutex = self.fetcher_mutex::<K, V>(key_hash, query_scope_info);
                let _maybe_fetcher_mutex_guard_local = if maybe_preheld_fetcher_mutex_guard
                    .is_none()
                {
                    let _fetcher_guard = match fetcher_mutex.try_lock() {
                        Some(fetcher_guard) => fetcher_guard,
                        None => {
                            // If have to wait, should check cache again in case it was fetched while waiting.
                            let fetcher_guard = fetcher_mutex.lock().await;
                            let next_directive = self.with_cached_query::<K, V, _>(
                                &key_hash,
                                &query_scope_info.cache_key,
                                |maybe_cached| {
                                    if let Some(cached) = maybe_cached {
                                        cached_buster = Some(cached.buster.clone());
                                        return_cb(CachedOrFetchCbInput {
                                            cached,
                                            variant: CachedOrFetchCbInputVariant::CachedUntouched,
                                        })
                                    } else {
                                        CachedOrFetchCbOutput::Refetch
                                    }
                                },
                            );
                            match next_directive {
                                CachedOrFetchCbOutput::Return(value) => return value,
                                CachedOrFetchCbOutput::Refetch => fetcher_guard,
                            }
                        }
                    };
                    Some(_fetcher_guard)
                } else {
                    // Owned externally so not an issue.
                    None
                };

                #[cfg(any(
                    all(debug_assertions, feature = "devtools"),
                    feature = "devtools-always"
                ))]
                let before_time = chrono::Utc::now();

                let loading_first_time = cached_buster.is_none();

                #[cfg(any(
                    all(debug_assertions, feature = "devtools"),
                    feature = "devtools-always"
                ))]
                {
                    // Running this before fetching so it'll show up in devtools straight away:
                    if loading_first_time {
                        self.client_subscriptions_mut().notify_query_created(
                            crate::subs_client::QueryCreatedInfo {
                                cache_key: query_scope_info.cache_key,
                                scope_title: query_scope_info.title.clone(),
                                key_hash,
                                debug_key: crate::utils::DebugValue::new(key),
                                combined_options: crate::options_combine(
                                    client_options,
                                    scope_options,
                                ),
                            },
                        );
                    }
                }

                let maybe_local_key = lazy_maybe_local_key();

                enum MaybeNewValue<V> {
                    NewValue(V),
                    SsrStreamedValueOverride,
                }

                let maybe_new_value = self
                    .with_notify_fetching(
                        query_scope_info.cache_key,
                        key_hash,
                        loading_first_time,
                        // Call the fetcher, but reset and repeat if an invalidation occurs whilst in-flight:
                        async {
                            loop {
                                let query_abort_rx = self
                                    .prepare_invalidation_channel::<K, V>(
                                        query_scope_info,
                                        key_hash,
                                        &maybe_local_key,
                                    );

                                let fut = owner_chain.with(|| fetcher(key.clone()));

                                futures::select_biased! {
                                    rx_result = query_abort_rx.fuse() => {
                                        if let Ok(reason) = rx_result {
                                            match reason {
                                                QueryAbortReason::Invalidate | QueryAbortReason::Clear => {},
                                                QueryAbortReason::SsrStreamedValueOverride => {
                                                    break MaybeNewValue::SsrStreamedValueOverride;
                                                },
                                            }
                                        }
                                    },
                                    new_value = fut.fuse() => {
                                        break MaybeNewValue::NewValue(new_value);
                                    },
                                }
                            }
                        },
                    )
                    .await;

                #[cfg(any(
                    all(debug_assertions, feature = "devtools"),
                    feature = "devtools-always"
                ))]
                let elapsed_ms = chrono::Utc::now()
                    .signed_duration_since(before_time)
                    .num_milliseconds();

                let buster_if_uncached = if loading_first_time {
                    Some(
                        maybe_buster_if_uncached
                            .unwrap_or_else(|| ArcRwSignal::new(new_buster_id())),
                    )
                } else {
                    None
                };

                let next_directive = self.with_cached_scope_mut::<_, _, _, _>(
                    query_scope_info,
                    true,
                    |_| {},
                    |scope, _| {
                        let scope = scope.expect("provided a default");
                        match maybe_new_value {
                            MaybeNewValue::NewValue(new_value) => {
                                if let Some(cached) = scope.get_mut(&key_hash) {
                                    cached.set_value(
                                        new_value,
                                        true,
                                        #[cfg(any(
                                            all(debug_assertions, feature = "devtools"),
                                            feature = "devtools-always"
                                        ))]
                                        crate::events::Event::new(
                                            crate::events::EventVariant::Fetched { elapsed_ms },
                                        ),
                                        ResetInvalidated::Reset,
                                    );
                                    return_cb(CachedOrFetchCbInput {
                                        cached,
                                        variant: CachedOrFetchCbInputVariant::CachedUpdated,
                                    })
                                } else {
                                    // We already notified before the async fetch, so it would show up sooner.
                                    scope.insert_without_query_created_notif(
                                        key_hash,
                                        Query::new(
                                            client_options,
                                            *self,
                                            query_scope_info,
                                            invalidation_prefix,
                                            key_hash,
                                            maybe_local_key,
                                            new_value,
                                            buster_if_uncached.expect(
                                                "loading_first_time means this is Some(). (bug)",
                                            ),
                                            scope_options,
                                            None,
                                            #[cfg(any(
                                                all(debug_assertions, feature = "devtools"),
                                                feature = "devtools-always"
                                            ))]
                                            crate::events::Event::new(
                                                crate::events::EventVariant::Fetched { elapsed_ms },
                                            ),
                                        ),
                                    );
                                    return_cb(CachedOrFetchCbInput {
                                        cached: scope.get(&key_hash).expect("Just set. (bug)"),
                                        variant: CachedOrFetchCbInputVariant::Fresh,
                                    })
                                }
                            }
                            MaybeNewValue::SsrStreamedValueOverride => {
                                return_cb(CachedOrFetchCbInput {
                                    cached: scope
                                        .get(&key_hash)
                                        .expect("Should contain value streamed from server. (bug)"),
                                    variant: CachedOrFetchCbInputVariant::Fresh,
                                })
                            }
                        }
                    },
                );

                match next_directive {
                    CachedOrFetchCbOutput::Refetch => {
                        panic!("Unexpected refetch directive after providing fresh value. (bug)")
                    }
                    CachedOrFetchCbOutput::Return(return_value) => return_value,
                }
            }
        }
    }
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
