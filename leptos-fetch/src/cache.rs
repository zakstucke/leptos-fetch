use std::{
    any::{Any, TypeId},
    collections::{hash_map::Entry, HashMap},
    future::Future,
    hash::Hash,
    ops::{Deref, DerefMut},
    sync::Arc,
};

use futures::lock::Mutex;
use leptos::prelude::{ArcRwSignal, ReadValue, Set, StoredValue, Track, WriteValue};
use send_wrapper::SendWrapper;

use crate::{query::Query, utils::random_u64_rolling, QueryClient, QueryOptions};

#[derive(Debug)]
pub(crate) struct Scope<K, V> {
    pub cache: HashMap<K, Query<V>>,
    // To make sure parallel fetches for the same key aren't happening across different resources.
    pub fetcher_mutexes: HashMap<K, Arc<Mutex<()>>>,
}

impl<K, V> Default for Scope<K, V> {
    fn default() -> Self {
        Self {
            cache: HashMap::new(),
            fetcher_mutexes: HashMap::new(),
        }
    }
}

pub(crate) trait Busters: 'static {
    fn invalidate_scope(&mut self);

    fn busters(&self) -> Vec<ArcRwSignal<u64>>;
}

impl<K: 'static, V: 'static> Busters for Scope<K, V> {
    fn invalidate_scope(&mut self) {
        for query in self.cache.values_mut() {
            query.invalidate();
        }
    }

    fn busters(&self) -> Vec<ArcRwSignal<u64>> {
        self.cache
            .values()
            .map(|query| query.buster.clone())
            .collect::<Vec<_>>()
    }
}

impl<K: 'static, V: 'static> Busters for SendWrapper<Scope<K, V>> {
    fn invalidate_scope(&mut self) {
        self.deref_mut().invalidate_scope();
    }

    fn busters(&self) -> Vec<ArcRwSignal<u64>> {
        self.deref().busters()
    }
}

pub(crate) trait ScopeTrait: Busters + Send + Sync + 'static {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<K, V> ScopeTrait for Scope<K, V>
where
    K: Send + Sync + 'static,
    V: Send + Sync + 'static,
{
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl<K, V> ScopeTrait for SendWrapper<Scope<K, V>>
where
    K: 'static,
    V: 'static,
{
    fn as_any(&self) -> &dyn Any {
        self.deref()
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self.deref_mut()
    }
}

/// Internalising the sharing of a cache, which needs to be sync on the backend,
/// but not on the frontend where LocalResource's are used.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ScopeLookup {
    // Happy to use a non-arc signal here to allow the client to be Copy.
    // The client is created at the root of the app, so there shouldn't be any chance of disposed errors.
    pub scopes: StoredValue<HashMap<TypeId, Box<dyn ScopeTrait>>>,
}

impl ScopeLookup {
    pub fn new() -> Self {
        Self {
            scopes: StoredValue::new(HashMap::new()),
        }
    }

    pub fn fetcher_mutex<K, V>(
        &self,
        key: K,
        cache_key: TypeId,
        default_scope_cb: impl FnOnce() -> Box<dyn ScopeTrait>,
    ) -> Arc<Mutex<()>>
    where
        K: Eq + std::hash::Hash + 'static,
        V: 'static,
    {
        self.scopes
            .write_value()
            .entry(cache_key)
            .or_insert_with(default_scope_cb)
            .as_any_mut()
            .downcast_mut::<Scope<K, V>>()
            .expect("Cache entry type mismatch.")
            .fetcher_mutexes
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
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
        let guard = self.scopes.read_value();
        let maybe_query = guard.get(cache_key).and_then(|scope_cache| {
            scope_cache
                .as_any()
                .downcast_ref::<Scope<K, V>>()
                .expect("Cache entry type mismatch.")
                .cache
                .get(key)
        });
        cb(maybe_query)
    }

    pub fn with_cached_scope_mut<K, V, T>(
        &self,
        cache_key: TypeId,
        maybe_default_cb: impl FnOnce() -> Option<Box<dyn ScopeTrait>>,
        cb: impl FnOnce(Option<&mut Scope<K, V>>) -> T,
    ) -> T
    where
        K: Eq + std::hash::Hash + 'static,
        V: 'static,
    {
        let mut guard = self.scopes.write_value();
        let maybe_scope = match guard.entry(cache_key) {
            Entry::Occupied(entry) => Some(entry.into_mut()),
            Entry::Vacant(entry) => maybe_default_cb().map(|default| entry.insert(default)),
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

    pub fn gc_query<K, V>(&self, cache_key: TypeId, key: &K)
    where
        K: Eq + Hash + 'static,
        V: 'static,
    {
        let mut guard = self.scopes.write_value();
        let remove_scope = if let Some(scope) = guard.get_mut(&cache_key) {
            let scope = scope
                .as_any_mut()
                .downcast_mut::<Scope<K, V>>()
                .expect("Cache entry type mismatch.");
            scope.cache.remove(key);
            scope.cache.is_empty()
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
        key: K,
        cache_key: TypeId,
        fetcher: impl FnOnce(K) -> Fut + 'static,
        custom_next_buster: Option<ArcRwSignal<u64>>,
        track: bool,
        default_scope_cb: impl FnOnce() -> Box<dyn ScopeTrait> + Clone,
        scope_options: Option<QueryOptions>,
    ) -> V
    where
        K: Eq + Hash + Clone + 'static,
        V: Clone + 'static,
        Fut: Future<Output = V> + 'static,
    {
        self.cached_or_fetch_inner(
            client,
            key,
            cache_key,
            fetcher,
            custom_next_buster,
            track,
            default_scope_cb,
            |v| v.clone(),
            scope_options,
        )
        .await
    }

    pub async fn cached_or_fetch_inner<K, V, Fut, T>(
        &self,
        client: &QueryClient,
        key: K,
        cache_key: TypeId,
        fetcher: impl FnOnce(K) -> Fut + 'static,
        mut custom_next_buster: Option<ArcRwSignal<u64>>,
        track: bool,
        default_scope_cb: impl FnOnce() -> Box<dyn ScopeTrait> + Clone,
        return_cb: impl FnOnce(&V) -> T + Clone,
        scope_options: Option<QueryOptions>,
    ) -> T
    where
        K: Eq + Hash + Clone + 'static,
        V: 'static,
        Fut: Future<Output = V> + 'static,
    {
        let mut using_stale_buster = false;

        // Otherwise fetch and cache:
        let fetcher_mutex =
            self.fetcher_mutex::<K, V>(key.clone(), cache_key, default_scope_cb.clone());
        let _fetcher_guard = match fetcher_mutex.try_lock() {
            Some(fetcher_guard) => fetcher_guard,
            None => {
                // If have to wait, should check cache again in case it was fetched while waiting.
                let fetcher_guard = fetcher_mutex.lock().await;
                if let Some(cached) =
                    self.with_cached_query::<K, V, _>(&key, &cache_key, |maybe_cached| {
                        if let Some(cached) = maybe_cached {
                            if track {
                                cached.buster.track();
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

        let next_buster =
            custom_next_buster.unwrap_or_else(|| ArcRwSignal::new(random_u64_rolling()));

        if track {
            next_buster.track();
        }

        let return_value = return_cb(&new_value);

        self.with_cached_scope_mut(
            cache_key,
            || Some(default_scope_cb()),
            |scope| {
                let query = Query::new(
                    *client,
                    cache_key,
                    &key,
                    new_value,
                    next_buster.clone(),
                    scope_options,
                );
                scope.expect("provided a default").cache.insert(key, query);
            },
        );

        // If we're replacing an existing item in the cache, need to invalidate anything using it:
        if using_stale_buster {
            next_buster.set(random_u64_rolling());
        }

        return_value
    }
}
