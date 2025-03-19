use std::{
    any::{Any, TypeId},
    collections::{hash_map::Entry, HashMap},
    future::Future,
    hash::Hash,
    sync::{Arc, LazyLock},
    thread::ThreadId,
};

use leptos::prelude::{ArcRwSignal, Set, Track};
use send_wrapper::SendWrapper;

use crate::{
    maybe_local::MaybeLocal,
    query::Query,
    utils::{new_buster_id, new_scope_id, KeyHash},
    QueryClient, QueryOptions,
};

#[derive(Debug)]
pub(crate) struct Scope<K, V> {
    threadsafe_cache: HashMap<KeyHash, Query<V>>,
    local_caches: HashMap<ThreadId, HashMap<KeyHash, Query<V>>>,
    // To make sure parallel fetches for the same key aren't happening across different resources.
    pub fetcher_mutexes: HashMap<KeyHash, Arc<futures::lock::Mutex<()>>>,
    _k: std::marker::PhantomData<SendWrapper<K>>,
}

impl<K, V> Default for Scope<K, V> {
    fn default() -> Self {
        Self {
            threadsafe_cache: HashMap::new(),
            local_caches: HashMap::new(),
            fetcher_mutexes: HashMap::new(),
            _k: std::marker::PhantomData,
        }
    }
}

impl<K, V> Scope<K, V> {
    fn all_caches(&self) -> impl Iterator<Item = &HashMap<KeyHash, Query<V>>> {
        std::iter::once(&self.threadsafe_cache).chain(self.local_caches.values())
    }

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

    pub fn insert(&mut self, key: KeyHash, query: Query<V>)
    where
        K: Eq + Hash,
    {
        if query.value_maybe_stale.value_is_local() {
            self.local_caches
                .entry(std::thread::current().id())
                .or_default()
                .insert(key, query);
        } else {
            self.threadsafe_cache.insert(key, query);
        }
    }

    pub fn get(&self, key: &K) -> Option<&Query<V>>
    where
        K: Eq + Hash,
    {
        let key = KeyHash::new(key);

        // Threadsafe always takes priority:
        self.threadsafe_cache.get(&key).or_else(|| {
            self.local_caches
                .get(&std::thread::current().id())
                .and_then(|local_cache| local_cache.get(&key))
        })
    }

    pub fn get_mut(&mut self, key: &K) -> Option<&mut Query<V>>
    where
        K: Eq + Hash,
    {
        self.get_mut_with_key_hash(&KeyHash::new(key))
    }

    pub fn get_mut_with_key_hash(&mut self, key: &KeyHash) -> Option<&mut Query<V>>
    where
        K: Eq + Hash,
    {
        // Threadsafe always takes priority:
        self.threadsafe_cache.get_mut(key).or_else(|| {
            self.local_caches
                .get_mut(&std::thread::current().id())
                .and_then(|local_cache| local_cache.get_mut(key))
        })
    }

    /// Should use this and the threadsafe one if updating the value, have to be separated.
    pub fn get_mut_local_only(&mut self, key: &K) -> Option<&mut Query<V>>
    where
        K: Eq + Hash,
    {
        self.local_caches
            .get_mut(&std::thread::current().id())
            .and_then(|local_cache| local_cache.get_mut(&KeyHash::new(key)))
    }

    /// Should use this and the threadsafe one if updating the value, have to be separated.
    pub fn get_mut_threadsafe_only(&mut self, key: &K) -> Option<&mut Query<V>>
    where
        K: Eq + Hash,
    {
        self.threadsafe_cache.get_mut(&KeyHash::new(key))
    }

    pub fn contains_key(&self, key: &K) -> bool
    where
        K: Eq + Hash,
    {
        let key = KeyHash::new(key);
        if self.threadsafe_cache.contains_key(&key) {
            true
        } else {
            self.local_caches
                .get(&std::thread::current().id())
                .is_some_and(|local_cache| local_cache.contains_key(&key))
        }
    }

    pub fn remove_entry(&mut self, key: &K) -> Option<(KeyHash, Query<V>)>
    where
        K: Eq + Hash,
    {
        self.remove_entry_with_key_hash(&KeyHash::new(key))
    }

    fn remove_entry_with_key_hash(&mut self, key: &KeyHash) -> Option<(KeyHash, Query<V>)>
    where
        K: Eq + Hash,
    {
        if let Some(query) = self.threadsafe_cache.remove_entry(key) {
            // Threadsafe always takes priority:
            Some(query)
        } else if let Some(local_cache) = self.local_caches.get_mut(&std::thread::current().id()) {
            local_cache.remove_entry(key)
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

impl<K: 'static, V: 'static> Busters for Scope<K, V> {
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
    fn size(&self) -> usize;
}

impl<K, V> ScopeTrait for Scope<K, V>
where
    K: 'static,
    V: 'static,
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

    fn size(&self) -> usize {
        self.all_caches().map(|cache| cache.len()).sum()
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

impl ScopeLookup {
    pub fn new() -> Self {
        let scope_id = new_scope_id();
        SCOPE_LOOKUPS.write().insert(scope_id, HashMap::new());
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

    pub fn mark_resource_dropped<K, V>(&self, key: K, cache_key: &TypeId, resource_id: u64)
    where
        K: Eq + std::hash::Hash + 'static,
        V: 'static,
    {
        if let Some(query) = self.scopes().get(cache_key).and_then(|scope_cache| {
            scope_cache
                .as_any()
                .downcast_ref::<Scope<K, V>>()
                .expect("Cache entry type mismatch.")
                .get(&key)
        }) {
            query.mark_resource_dropped(resource_id);
        }
    }

    pub fn fetcher_mutex<K, V>(&self, key: &K, cache_key: TypeId) -> Arc<futures::lock::Mutex<()>>
    where
        K: Eq + std::hash::Hash + 'static,
        V: 'static,
    {
        self.scopes_mut()
            .entry(cache_key)
            .or_insert_with(|| Box::new(Scope::<K, V>::default()))
            .as_any_mut()
            .downcast_mut::<Scope<K, V>>()
            .expect("Cache entry type mismatch.")
            .fetcher_mutexes
            .entry(KeyHash::new(key))
            .or_insert_with(|| Arc::new(futures::lock::Mutex::new(())))
            .clone()
    }

    pub fn with_cached_query<K, V, T>(
        &self,
        key: &K,
        cache_key: &TypeId,
        cb: impl FnOnce(Option<&Query<V>>) -> T,
    ) -> T
    where
        K: Eq + std::hash::Hash + 'static,
        V: 'static,
    {
        let guard = self.scopes();
        let maybe_query = guard.get(cache_key).and_then(|scope_cache| {
            scope_cache
                .as_any()
                .downcast_ref::<Scope<K, V>>()
                .expect("Cache entry type mismatch.")
                .get(key)
        });
        cb(maybe_query)
    }

    pub fn with_cached_scope_mut<K, V, T>(
        &self,
        cache_key: TypeId,
        create_scope_if_missing: bool,
        cb: impl FnOnce(Option<&mut Scope<K, V>>) -> T,
    ) -> T
    where
        K: Eq + std::hash::Hash + 'static,
        V: 'static,
    {
        let mut guard = self.scopes_mut();
        let maybe_scope = match guard.entry(cache_key) {
            Entry::Occupied(entry) => Some(entry.into_mut()),
            Entry::Vacant(entry) => {
                if create_scope_if_missing {
                    Some(entry.insert(Box::new(Scope::<K, V>::default())))
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

    pub fn gc_query<K, V>(&self, cache_key: TypeId, key: &KeyHash)
    where
        K: Eq + Hash + 'static,
        V: 'static,
    {
        let mut guard = self.scopes_mut();
        let remove_scope = if let Some(scope) = guard.get_mut(&cache_key) {
            let scope = scope
                .as_any_mut()
                .downcast_mut::<Scope<K, V>>()
                .expect("Cache entry type mismatch.");
            scope.remove_entry_with_key_hash(key);
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
        client: &QueryClient,
        key: &K,
        cache_key: TypeId,
        fetcher: impl FnOnce(K) -> Fut,
        custom_next_buster: Option<ArcRwSignal<u64>>,
        track: bool,
        scope_options: Option<QueryOptions>,
        resource_id: Option<u64>,
    ) -> V
    where
        K: Eq + Hash + Clone + 'static,
        V: Clone + 'static,
        Fut: Future<Output = MaybeLocal<V>>,
    {
        self.cached_or_fetch_inner(
            client,
            key,
            cache_key,
            fetcher,
            custom_next_buster,
            track,
            |v| v.clone(),
            scope_options,
            resource_id,
        )
        .await
    }

    pub async fn cached_or_fetch_inner<K, V, Fut, T>(
        &self,
        client: &QueryClient,
        key: &K,
        cache_key: TypeId,
        fetcher: impl FnOnce(K) -> Fut,
        mut custom_next_buster: Option<ArcRwSignal<u64>>,
        track: bool,
        return_cb: impl FnOnce(&V) -> T + Clone,
        scope_options: Option<QueryOptions>,
        resource_id: Option<u64>,
    ) -> T
    where
        K: Eq + Hash + Clone + 'static,
        V: 'static,
        Fut: Future<Output = MaybeLocal<V>>,
    {
        let mut using_stale_buster = false;

        // Otherwise fetch and cache:
        let fetcher_mutex = self.fetcher_mutex::<K, V>(key, cache_key);
        let _fetcher_guard = match fetcher_mutex.try_lock() {
            Some(fetcher_guard) => fetcher_guard,
            None => {
                // If have to wait, should check cache again in case it was fetched while waiting.
                let fetcher_guard = fetcher_mutex.lock().await;
                if let Some(cached) =
                    self.with_cached_query::<K, V, _>(key, &cache_key, |maybe_cached| {
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

        let new_value = fetcher(key.clone()).await;

        let next_buster = custom_next_buster.unwrap_or_else(|| ArcRwSignal::new(new_buster_id()));

        if track {
            next_buster.track();
        }

        let return_value = return_cb(new_value.value());

        self.with_cached_scope_mut::<K, _, _>(cache_key, true, |scope| {
            let key = KeyHash::new(key);

            let query = Query::new::<K>(
                *client,
                cache_key,
                &key,
                new_value,
                next_buster.clone(),
                scope_options,
                None,
            );
            if let Some(resource_id) = resource_id {
                query.mark_resource_active(resource_id);
            }
            scope.expect("provided a default").insert(key, query);
        });

        // If we're replacing an existing item in the cache, need to invalidate anything using it:
        if using_stale_buster {
            next_buster.set(new_buster_id());
        }

        return_value
    }
}
