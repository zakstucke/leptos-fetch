use std::{
    any::{Any, TypeId},
    collections::{HashMap, hash_map::Entry},
    future::Future,
    hash::Hash,
    sync::{Arc, LazyLock},
    thread::ThreadId,
};

use leptos::prelude::ArcRwSignal;

use crate::{
    QueryOptions,
    debug_if_devtools_enabled::DebugIfDevtoolsEnabled,
    maybe_local::MaybeLocal,
    query::Query,
    query_scope::{QueryTypeInfo, ScopeCacheKey},
    subs_scope::ScopeSubs,
    utils::{KeyHash, OnDrop, new_buster_id, new_scope_id},
};

#[derive(Debug)]
pub(crate) struct Scope<K: 'static, V: 'static> {
    threadsafe_cache: HashMap<KeyHash, Query<K, V>>,
    local_caches: HashMap<ThreadId, HashMap<KeyHash, Query<K, V>>>,
    // To make sure parallel fetches for the same key aren't happening across different resources.
    fetcher_mutexes: HashMap<KeyHash, Arc<futures::lock::Mutex<()>>>,
    scope_lookup: ScopeLookup,
    query_type_info: QueryTypeInfo,
}

impl<K, V> Scope<K, V>
where
    K: DebugIfDevtoolsEnabled + 'static,
    V: 'static,
{
    fn new(scope_lookup: ScopeLookup, query_type_info: QueryTypeInfo) -> Self {
        Self {
            threadsafe_cache: HashMap::new(),
            local_caches: HashMap::new(),
            fetcher_mutexes: HashMap::new(),
            scope_lookup,
            query_type_info,
        }
    }

    // TODO should think about the wrong-thread exposure on some of these.
    fn all_queries(&self) -> impl Iterator<Item = &Query<K, V>> {
        self.threadsafe_cache.values().chain(
            self.local_caches
                .values()
                .flat_map(|local_cache| local_cache.values()),
        )
    }

    pub(crate) fn all_queries_mut(&mut self) -> impl Iterator<Item = &mut Query<K, V>> {
        self.threadsafe_cache.values_mut().chain(
            self.local_caches
                .values_mut()
                .flat_map(|local_cache| local_cache.values_mut()),
        )
    }

    fn insert_without_query_created_notif(&mut self, key_hash: KeyHash, query: Query<K, V>) {
        if query.value_maybe_stale().is_local() {
            self.local_caches
                .entry(std::thread::current().id())
                .or_default()
                .insert(key_hash, query);
        } else {
            self.threadsafe_cache.insert(key_hash, query);
        }
        self.scope_lookup
            .scope_subscriptions_mut()
            .notify_value_set_updated_or_removed::<V>(self.query_type_info.cache_key, key_hash);
    }

    pub fn insert(&mut self, key_hash: KeyHash, query: Query<K, V>) {
        #[cfg(any(
            all(debug_assertions, feature = "devtools"),
            feature = "devtools-always"
        ))]
        {
            let info = crate::subs_client::QueryCreatedInfo {
                cache_key: self.query_type_info.cache_key,
                scope_title: self.query_type_info.title.clone(),
                key_hash,
                // SAFETY: query just created, so same thread
                debug_key: crate::utils::DebugValue::new(query.key().value_may_panic()),
                v_type_id: TypeId::of::<V>(),
                combined_options: query.combined_options,
            };
            self.insert_without_query_created_notif(key_hash, query);
            self.scope_lookup
                .client_subscriptions_mut()
                .notify_query_created(info);
        }
        #[cfg(not(any(
            all(debug_assertions, feature = "devtools"),
            feature = "devtools-always"
        )))]
        {
            self.insert_without_query_created_notif(key_hash, query);
        }
    }

    pub fn get(&self, key_hash: &KeyHash) -> Option<&Query<K, V>> {
        // Threadsafe always takes priority:
        self.threadsafe_cache.get(key_hash).or_else(|| {
            self.local_caches
                .get(&std::thread::current().id())
                .and_then(|local_cache| local_cache.get(key_hash))
        })
    }

    pub fn get_mut(&mut self, key_hash: &KeyHash) -> Option<&mut Query<K, V>> {
        // Threadsafe always takes priority:
        self.threadsafe_cache.get_mut(key_hash).or_else(|| {
            self.local_caches
                .get_mut(&std::thread::current().id())
                .and_then(|local_cache| local_cache.get_mut(key_hash))
        })
    }

    /// Should use this and the threadsafe one if updating the value, have to be separated.
    pub fn get_mut_local_only(&mut self, key_hash: &KeyHash) -> Option<&mut Query<K, V>> {
        self.local_caches
            .get_mut(&std::thread::current().id())
            .and_then(|local_cache| local_cache.get_mut(key_hash))
    }

    /// Should use this and the threadsafe one if updating the value, have to be separated.
    pub fn get_mut_threadsafe_only(&mut self, key_hash: &KeyHash) -> Option<&mut Query<K, V>> {
        self.threadsafe_cache.get_mut(key_hash)
    }

    pub fn contains_key(&self, key_hash: &KeyHash) -> bool {
        if self.threadsafe_cache.contains_key(key_hash) {
            true
        } else {
            self.local_caches
                .get(&std::thread::current().id())
                .is_some_and(|local_cache| local_cache.contains_key(key_hash))
        }
    }

    pub fn remove_entry(&mut self, key_hash: &KeyHash) -> Option<(KeyHash, Query<K, V>)> {
        let result = if let Some(query) = self.threadsafe_cache.remove_entry(key_hash) {
            // Threadsafe always takes priority:
            Some(query)
        } else if let Some(local_cache) = self.local_caches.get_mut(&std::thread::current().id()) {
            local_cache.remove_entry(key_hash)
        } else {
            None
        };
        result
    }

    fn is_empty(&self) -> bool {
        self.threadsafe_cache.is_empty()
            && self.local_caches.iter().all(|(_, cache)| cache.is_empty())
    }
}

pub(crate) trait Busters: 'static {
    fn invalidate_scope(&mut self);

    fn busters(&self) -> Vec<ArcRwSignal<u64>>;
}

impl<K, V> Busters for Scope<K, V>
where
    K: DebugIfDevtoolsEnabled + 'static,
    V: DebugIfDevtoolsEnabled + 'static,
{
    fn invalidate_scope(&mut self) {
        for query in self.all_queries_mut() {
            query.invalidate();
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
    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    fn invalidate_query(&mut self, key_hash: &KeyHash);
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
        self.threadsafe_cache.clear();
        self.local_caches.clear();
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

    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    fn invalidate_query(&mut self, key_hash: &KeyHash) {
        if let Some(query) = self.get_mut(key_hash) {
            query.invalidate();
        }
    }

    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    fn title(&self) -> &Arc<String> {
        &self.query_type_info.title
    }

    #[cfg(test)]
    fn size(&self) -> usize {
        std::iter::once(&self.threadsafe_cache)
            .chain(self.local_caches.values())
            .map(|cache| cache.len())
            .sum()
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
    scope_id: u64,
}

type Scopes = HashMap<ScopeCacheKey, Box<dyn ScopeTrait>>;

static SCOPE_LOOKUPS: LazyLock<parking_lot::RwLock<HashMap<u64, Scopes>>> =
    LazyLock::new(|| parking_lot::RwLock::new(HashMap::new()));

static SCOPE_SUBSCRIPTION_LOOKUPS: LazyLock<parking_lot::Mutex<HashMap<u64, ScopeSubs>>> =
    LazyLock::new(|| parking_lot::Mutex::new(HashMap::new()));

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

        SCOPE_LOOKUPS.write().insert(scope_id, HashMap::new());
        SCOPE_SUBSCRIPTION_LOOKUPS
            .lock()
            .insert(scope_id, ScopeSubs::new(result));

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
        query_type_info: &QueryTypeInfo,
    ) -> Arc<futures::lock::Mutex<()>>
    where
        K: DebugIfDevtoolsEnabled + Clone + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        self.scopes_mut()
            .entry(query_type_info.cache_key)
            .or_insert_with(|| Box::new(Scope::<K, V>::new(*self, query_type_info.clone())))
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

    pub fn with_cached_scope_mut<K, V, T>(
        &self,
        query_type_info: &QueryTypeInfo,
        create_scope_if_missing: bool,
        cb: impl FnOnce(Option<&mut Scope<K, V>>) -> T,
    ) -> T
    where
        K: DebugIfDevtoolsEnabled + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        let mut guard = self.scopes_mut();
        let maybe_scope = match guard.entry(query_type_info.cache_key) {
            Entry::Occupied(entry) => Some(entry.into_mut()),
            Entry::Vacant(entry) => {
                if create_scope_if_missing {
                    Some(entry.insert(Box::new(Scope::<K, V>::new(*self, query_type_info.clone()))))
                } else {
                    None
                }
            }
        };

        if let Some(scope) = maybe_scope {
            cb(Some(
                scope
                    .as_any_mut()
                    .downcast_mut::<Scope<K, V>>()
                    .expect("Cache entry type mismatch."),
            ))
        } else {
            cb(None)
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

    pub async fn cached_or_fetch<K, V, T>(
        &self,
        client_options: QueryOptions,
        scope_options: Option<QueryOptions>,
        maybe_buster_if_uncached: Option<ArcRwSignal<u64>>,
        query_type_info: &QueryTypeInfo,
        key: &K,
        fetcher: impl AsyncFnOnce(K) -> MaybeLocal<V>,
        return_cb: impl Fn(CachedOrFetchCbInput<K, V>) -> CachedOrFetchCbOutput<T>,
        maybe_preheld_fetcher_mutex_guard: Option<&futures::lock::MutexGuard<'_, ()>>,
        lazy_maybe_local_key: impl FnOnce() -> MaybeLocal<K>,
    ) -> T
    where
        K: DebugIfDevtoolsEnabled + Hash + Clone + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        let key_hash = KeyHash::new(key);
        let mut cached_buster = None;
        let next_directive = self.with_cached_query::<K, V, _>(
            &key_hash,
            &query_type_info.cache_key,
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
                let fetcher_mutex = self.fetcher_mutex::<K, V>(key_hash, query_type_info);
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
                                &query_type_info.cache_key,
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
                                cache_key: query_type_info.cache_key,
                                scope_title: query_type_info.title.clone(),
                                key_hash,
                                v_type_id: TypeId::of::<V>(),
                                debug_key: crate::utils::DebugValue::new(key),
                                combined_options: crate::options_combine(
                                    client_options,
                                    scope_options,
                                ),
                            },
                        );
                    }
                }

                let new_value = self
                    .with_notify_fetching(
                        query_type_info.cache_key,
                        key_hash,
                        loading_first_time,
                        fetcher(key.clone()),
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

                let next_directive =
                    self.with_cached_scope_mut::<_, _, _>(query_type_info, true, |scope| {
                        let scope = scope.expect("provided a default");
                        if let Some(cached) = scope.get_mut(&key_hash) {
                            cached.set_value(
                                new_value,
                                #[cfg(any(
                                    all(debug_assertions, feature = "devtools"),
                                    feature = "devtools-always"
                                ))]
                                crate::events::Event::new(crate::events::EventVariant::Fetched {
                                    elapsed_ms,
                                }),
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
                                    query_type_info,
                                    key_hash,
                                    lazy_maybe_local_key(),
                                    new_value,
                                    buster_if_uncached
                                        .expect("loading_first_time means this is Some(). (bug)"),
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
                    });

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

pub(crate) enum CachedOrFetchCbInputVariant {
    CachedUntouched,
    CachedUpdated,
    Fresh,
}
pub(crate) enum CachedOrFetchCbOutput<T> {
    Refetch,
    Return(T),
}
