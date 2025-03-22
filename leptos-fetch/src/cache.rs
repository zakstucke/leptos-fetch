use std::{
    any::{Any, TypeId},
    collections::{hash_map::Entry, HashMap},
    future::Future,
    hash::Hash,
    sync::{Arc, LazyLock},
    thread::ThreadId,
};

use leptos::prelude::{ArcRwSignal, Set, Track};

use crate::{
    maybe_local::MaybeLocal,
    query::Query,
    subscriptions::Subscriptions,
    utils::{new_buster_id, new_scope_id, KeyHash},
    QueryOptions,
};

#[derive(Debug)]
pub(crate) struct Scope<V> {
    threadsafe_cache: HashMap<KeyHash, Query<V>>,
    local_caches: HashMap<ThreadId, HashMap<KeyHash, Query<V>>>,
    // To make sure parallel fetches for the same key aren't happening across different resources.
    fetcher_mutexes: HashMap<KeyHash, Arc<futures::lock::Mutex<()>>>,
}

impl<V> Default for Scope<V> {
    fn default() -> Self {
        Self {
            threadsafe_cache: HashMap::new(),
            local_caches: HashMap::new(),
            fetcher_mutexes: HashMap::new(),
        }
    }
}

impl<V> Scope<V> {
    fn all_queries(&self) -> impl Iterator<Item = &Query<V>> {
        self.threadsafe_cache.values().chain(
            self.local_caches
                .values()
                .flat_map(|local_cache| local_cache.values()),
        )
    }

    fn all_queries_mut(&mut self) -> impl Iterator<Item = &mut Query<V>> {
        self.threadsafe_cache.values_mut().chain(
            self.local_caches
                .values_mut()
                .flat_map(|local_cache| local_cache.values_mut()),
        )
    }

    pub fn insert(&mut self, key_hash: KeyHash, query: Query<V>) {
        if query.value_maybe_stale.value_is_local() {
            self.local_caches
                .entry(std::thread::current().id())
                .or_default()
                .insert(key_hash, query);
        } else {
            self.threadsafe_cache.insert(key_hash, query);
        }
    }

    pub fn get(&self, key_hash: &KeyHash) -> Option<&Query<V>> {
        // Threadsafe always takes priority:
        self.threadsafe_cache.get(key_hash).or_else(|| {
            self.local_caches
                .get(&std::thread::current().id())
                .and_then(|local_cache| local_cache.get(key_hash))
        })
    }

    pub fn get_mut(&mut self, key_hash: &KeyHash) -> Option<&mut Query<V>> {
        // Threadsafe always takes priority:
        self.threadsafe_cache.get_mut(key_hash).or_else(|| {
            self.local_caches
                .get_mut(&std::thread::current().id())
                .and_then(|local_cache| local_cache.get_mut(key_hash))
        })
    }

    /// Should use this and the threadsafe one if updating the value, have to be separated.
    pub fn get_mut_local_only(&mut self, key_hash: &KeyHash) -> Option<&mut Query<V>> {
        self.local_caches
            .get_mut(&std::thread::current().id())
            .and_then(|local_cache| local_cache.get_mut(key_hash))
    }

    /// Should use this and the threadsafe one if updating the value, have to be separated.
    pub fn get_mut_threadsafe_only(&mut self, key_hash: &KeyHash) -> Option<&mut Query<V>> {
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

    pub fn remove_entry(&mut self, key_hash: &KeyHash) -> Option<(KeyHash, Query<V>)> {
        if let Some(query) = self.threadsafe_cache.remove_entry(key_hash) {
            // Threadsafe always takes priority:
            Some(query)
        } else if let Some(local_cache) = self.local_caches.get_mut(&std::thread::current().id()) {
            local_cache.remove_entry(key_hash)
        } else {
            None
        }
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

impl<V: 'static> Busters for Scope<V> {
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
    #[cfg(test)]
    fn size(&self) -> usize;
}

impl<V: 'static> ScopeTrait for Scope<V> {
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

type Scopes = HashMap<TypeId, Box<dyn ScopeTrait>>;

static SCOPE_LOOKUPS: LazyLock<parking_lot::RwLock<HashMap<u64, Scopes>>> =
    LazyLock::new(|| parking_lot::RwLock::new(HashMap::new()));

static SUBSCRIPTION_LOOKUPS: LazyLock<parking_lot::Mutex<HashMap<u64, Subscriptions>>> =
    LazyLock::new(|| parking_lot::Mutex::new(HashMap::new()));

impl ScopeLookup {
    pub fn new() -> Self {
        let scope_id = new_scope_id();
        SCOPE_LOOKUPS.write().insert(scope_id, HashMap::new());
        SUBSCRIPTION_LOOKUPS
            .lock()
            .insert(scope_id, Subscriptions::default());
        Self { scope_id }
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

    pub fn subscriptions_mut(&self) -> parking_lot::MappedMutexGuard<'_, Subscriptions> {
        parking_lot::MutexGuard::map(
            SUBSCRIPTION_LOOKUPS.lock(),
            |sub_lookups: &mut HashMap<u64, Subscriptions>| {
                sub_lookups
                    .get_mut(&self.scope_id)
                    .expect("Scope not found (bug)")
            },
        )
    }

    pub fn mark_resource_dropped<V>(&self, key_hash: &KeyHash, cache_key: &TypeId, resource_id: u64)
    where
        V: 'static,
    {
        if let Some(query) = self.scopes().get(cache_key).and_then(|scope_cache| {
            scope_cache
                .as_any()
                .downcast_ref::<Scope<V>>()
                .expect("Cache entry type mismatch.")
                .get(key_hash)
        }) {
            query.mark_resource_dropped(resource_id);
        }
    }

    pub fn fetcher_mutex<V>(
        &self,
        key_hash: KeyHash,
        cache_key: TypeId,
    ) -> Arc<futures::lock::Mutex<()>>
    where
        V: 'static,
    {
        self.scopes_mut()
            .entry(cache_key)
            .or_insert_with(|| Box::new(Scope::<V>::default()))
            .as_any_mut()
            .downcast_mut::<Scope<V>>()
            .expect("Cache entry type mismatch.")
            .fetcher_mutexes
            .entry(key_hash)
            .or_insert_with(|| Arc::new(futures::lock::Mutex::new(())))
            .clone()
    }

    pub fn with_cached_query<V, T>(
        &self,
        key_hash: &KeyHash,
        cache_key: &TypeId,
        cb: impl FnOnce(Option<&Query<V>>) -> T,
    ) -> T
    where
        V: 'static,
    {
        let guard = self.scopes();
        let maybe_query = guard.get(cache_key).and_then(|scope_cache| {
            scope_cache
                .as_any()
                .downcast_ref::<Scope<V>>()
                .expect("Cache entry type mismatch.")
                .get(key_hash)
        });
        cb(maybe_query)
    }

    pub fn with_cached_scope_mut<V, T>(
        &self,
        cache_key: TypeId,
        create_scope_if_missing: bool,
        cb: impl FnOnce(Option<&mut Scope<V>>) -> T,
    ) -> T
    where
        V: 'static,
    {
        let mut guard = self.scopes_mut();
        let maybe_scope = match guard.entry(cache_key) {
            Entry::Occupied(entry) => Some(entry.into_mut()),
            Entry::Vacant(entry) => {
                if create_scope_if_missing {
                    Some(entry.insert(Box::new(Scope::<V>::default())))
                } else {
                    None
                }
            }
        };

        if let Some(scope) = maybe_scope {
            cb(Some(
                scope
                    .as_any_mut()
                    .downcast_mut::<Scope<V>>()
                    .expect("Cache entry type mismatch."),
            ))
        } else {
            cb(None)
        }
    }

    pub fn gc_query<V>(&self, cache_key: TypeId, key_hash: &KeyHash)
    where
        V: 'static,
    {
        let mut guard = self.scopes_mut();
        let remove_scope = if let Some(scope) = guard.get_mut(&cache_key) {
            let scope = scope
                .as_any_mut()
                .downcast_mut::<Scope<V>>()
                .expect("Cache entry type mismatch.");
            scope.remove_entry(key_hash);
            scope.is_empty()
        } else {
            false
        };
        if remove_scope {
            guard.remove(&cache_key);
        }
    }

    pub async fn cached_or_fetch<K, V, Fut>(
        &self,
        client_options: QueryOptions,
        scope_lookup: ScopeLookup,
        key: &K,
        cache_key: TypeId,
        fetcher: impl FnOnce(K) -> Fut,
        custom_next_buster: Option<ArcRwSignal<u64>>,
        track: bool,
        scope_options: Option<QueryOptions>,
        resource_id: Option<u64>,
        loading_first_time: bool,
    ) -> V
    where
        K: Hash + Clone + 'static,
        V: Clone + 'static,
        Fut: Future<Output = MaybeLocal<V>>,
    {
        self.cached_or_fetch_inner(
            client_options,
            scope_lookup,
            key,
            cache_key,
            fetcher,
            custom_next_buster,
            track,
            |v| v.clone(),
            scope_options,
            resource_id,
            loading_first_time,
        )
        .await
    }

    pub async fn cached_or_fetch_inner<K, V, Fut, T>(
        &self,
        client_options: QueryOptions,
        scope_lookup: ScopeLookup,
        key: &K,
        cache_key: TypeId,
        fetcher: impl FnOnce(K) -> Fut,
        mut custom_next_buster: Option<ArcRwSignal<u64>>,
        track: bool,
        return_cb: impl FnOnce(&V) -> T + Clone,
        scope_options: Option<QueryOptions>,
        resource_id: Option<u64>,
        loading_first_time: bool,
    ) -> T
    where
        K: Hash + Clone + 'static,
        V: 'static,
        Fut: Future<Output = MaybeLocal<V>>,
    {
        let mut using_stale_buster = false;
        let key_hash = KeyHash::new(key);

        // Otherwise fetch and cache:
        let fetcher_mutex = self.fetcher_mutex::<V>(key_hash, cache_key);
        let _fetcher_guard = match fetcher_mutex.try_lock() {
            Some(fetcher_guard) => fetcher_guard,
            None => {
                // If have to wait, should check cache again in case it was fetched while waiting.
                let fetcher_guard = fetcher_mutex.lock().await;
                if let Some(cached) =
                    self.with_cached_query::<V, _>(&key_hash, &cache_key, |maybe_cached| {
                        if let Some(cached) = maybe_cached {
                            if track {
                                cached.buster.track();
                            }
                            if let Some(resource_id) = resource_id {
                                cached.mark_resource_active(resource_id);
                            }

                            // If stale, we won't use the cache, but we'll still need to invalidate those previously using it,
                            // so use the old buster as the custom_next_buster,
                            // and set using_stale_buster=true so we can invalidate it after the update:
                            if cached.stale() {
                                custom_next_buster = Some(cached.buster.clone());
                                using_stale_buster = true;
                                return None;
                            }

                            Some((return_cb.clone())(cached.value_maybe_stale.value()))
                        } else {
                            None
                        }
                    })
                {
                    return cached;
                } else {
                    fetcher_guard
                }
            }
        };

        self.subscriptions_mut()
            .notify_fetching_start(cache_key, key_hash, loading_first_time);
        let new_value = fetcher(key.clone()).await;
        self.subscriptions_mut()
            .notify_fetching_finish(cache_key, key_hash, loading_first_time);

        let next_buster = custom_next_buster.unwrap_or_else(|| ArcRwSignal::new(new_buster_id()));

        if track {
            next_buster.track();
        }

        let return_value = return_cb(new_value.value());

        self.with_cached_scope_mut::<_, _>(cache_key, true, |scope| {
            let query = Query::new(
                client_options,
                scope_lookup,
                cache_key,
                &key_hash,
                new_value,
                next_buster.clone(),
                scope_options,
                None,
            );
            if let Some(resource_id) = resource_id {
                query.mark_resource_active(resource_id);
            }
            scope.expect("provided a default").insert(key_hash, query);
        });

        // If we're replacing an existing item in the cache, need to invalidate anything using it:
        if using_stale_buster {
            next_buster.set(new_buster_id());
        }

        return_value
    }
}
