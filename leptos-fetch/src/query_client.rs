use std::{any::TypeId, borrow::Borrow, future::Future, hash::Hash, sync::Arc};

use leptos::{
    prelude::{ArcMemo, ArcRwSignal, Effect, Get, Read, Set, Track, WriteValue},
    server::{ArcLocalResource, ArcResource, LocalResource, Resource},
};
use send_wrapper::SendWrapper;
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    cache::{Scope, ScopeTrait},
    query::Query,
    utils::random_u64_rolling,
    QueryOptions, QueryScopeLocalTrait, QueryScopeTrait,
};

use super::cache::ScopeLookup;

// TODO test query type separation even when K and V are the same, should fail but work once we switch to the trait method.
// TODO check a local resource can be accessed from a normal one and vice versa.
// TODO SendWrapper should never panic, a local resource accessed from a different thread should just have to fetch again
// TODO: garbage collection etc and other LQ stuff.
// TODO feature parity
// TODO minimal breaking
// TODO license
// TODO readme

/// The root query client you should add to the top of your app.
#[derive(Debug, Clone, Copy)]
pub struct QueryClient {
    scope_lookup: ScopeLookup,
    options: QueryOptions,
}

impl Default for QueryClient {
    fn default() -> Self {
        Self::new()
    }
}

impl QueryClient {
    /// Create a new QueryClient.
    pub fn new() -> Self {
        Self {
            scope_lookup: ScopeLookup::new(),
            options: QueryOptions::default(),
        }
    }

    /// Create a new QueryClient with non-default options.
    pub fn new_with_options(options: QueryOptions) -> Self {
        Self {
            scope_lookup: ScopeLookup::new(),
            options,
        }
    }

    /// TODO
    pub fn options(&self) -> QueryOptions {
        self.options
    }

    /// A manually created version of a local resource,
    /// the one in leptos is pretty broken,
    /// and has to be used inside suspense.
    ///
    /// To me it makes no sense it needing suspense, so making my own.
    /// Returning an RwSignal for ease, but maybe could be changed to something that supports await.
    ///
    /// Note all resource types share a cache, meaning you can use this somewhere you need local only,
    /// even though you've used a different resource type elsewhere.
    #[track_caller]
    pub fn local_resource<K: PartialEq + Eq + Hash + Clone + 'static, V: Clone + 'static>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V> + 'static,
        keyer: impl Fn() -> K + 'static,
    ) -> LocalResource<V> {
        self.arc_local_resource(query_scope, keyer).into()
    }

    /// A manually created version of a local resource,
    /// the one in leptos is pretty broken,
    /// and has to be used inside suspense.
    ///
    /// To me it makes no sense it needing suspense, so making my own.
    /// Returning an RwSignal for ease, but maybe could be changed to something that supports await.
    ///
    /// Note all resource types share a cache, meaning you can use this somewhere you need local only,
    /// even though you've used a streamed resource elsewhere.
    #[track_caller]
    pub fn arc_local_resource<K: PartialEq + Eq + Hash + Clone + 'static, V: Clone + 'static>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V> + 'static,
        keyer: impl Fn() -> K + 'static,
    ) -> ArcLocalResource<V> {
        let scope_lookup = self.scope_lookup;
        let cache_key = query_scope.cache_key();
        let query_scope = Arc::new(query_scope);
        let self_ = *self;
        let query_options = query_scope.options();
        ArcLocalResource::new({
            move || {
                let query_scope = query_scope.clone();
                let key = keyer();
                async move {
                    // First try using the cache:
                    if let Some(cached) = scope_lookup.with_cached_query::<K, V, _>(
                        &key,
                        &cache_key,
                        move |maybe_cached| {
                            if let Some(cached) = maybe_cached {
                                if let Some(value) = cached.value_if_not_stale() {
                                    cached.buster.track();
                                    Some(value.clone())
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        },
                    ) {
                        return cached;
                    }

                    scope_lookup
                        .cached_or_fetch(
                            &self_,
                            key,
                            cache_key,
                            move |key| async move { query_scope.query(key).await },
                            None,
                            true,
                            || Box::new(SendWrapper::new(Scope::<K, V>::default())),
                            query_options,
                        )
                        .await
                }
            }
        })
    }

    /// A Resource.
    ///
    /// Note all resource types share a cache, meaning you can use this somewhere you need local only,
    /// even though you've used a different resource type elsewhere.
    #[track_caller]
    pub fn resource<
        K: PartialEq + Eq + Hash + Clone + Send + Sync + 'static,
        V: Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
    >(
        &self,
        query_scope: impl QueryScopeTrait<K, V> + Send + Sync + 'static,
        keyer: impl Fn() -> K + Send + Sync + 'static,
    ) -> Resource<V> {
        self.arc_resource_with_options(query_scope, keyer, false)
            .into()
    }

    /// A blocking Resource.
    ///
    /// Note all resource types share a cache, meaning you can use this somewhere you need local only,
    /// even though you've used a different resource type elsewhere.
    #[track_caller]
    pub fn resource_blocking<
        K: PartialEq + Eq + Hash + Clone + Send + Sync + 'static,
        V: Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
    >(
        &self,
        query_scope: impl QueryScopeTrait<K, V> + Send + Sync + 'static,
        keyer: impl Fn() -> K + Send + Sync + 'static,
    ) -> Resource<V> {
        self.arc_resource_with_options(query_scope, keyer, true)
            .into()
    }

    /// An ArcResource.
    ///
    /// Note all resource types share a cache, meaning you can use this somewhere you need local only,
    /// even though you've used a different resource type elsewhere.
    #[track_caller]
    pub fn arc_resource<
        K: PartialEq + Eq + Hash + Clone + Send + Sync + 'static,
        V: Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
    >(
        &self,
        query_scope: impl QueryScopeTrait<K, V> + Send + Sync + 'static,
        keyer: impl Fn() -> K + Send + Sync + 'static,
    ) -> ArcResource<V> {
        self.arc_resource_with_options(query_scope, keyer, false)
    }

    /// A blocking ArcResource.
    ///
    /// Note all resource types share a cache, meaning you can use this somewhere you need local only,
    /// even though you've used a different resource type elsewhere.
    #[track_caller]
    pub fn arc_resource_blocking<
        K: PartialEq + Eq + Hash + Clone + Send + Sync + 'static,
        V: Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
    >(
        &self,
        query_scope: impl QueryScopeTrait<K, V> + Send + Sync + 'static,
        keyer: impl Fn() -> K + Send + Sync + 'static,
    ) -> ArcResource<V> {
        self.arc_resource_with_options(query_scope, keyer, true)
    }

    /// An ArcResource with options.
    ///
    /// Note all resource types share a cache, meaning you can use this somewhere you need local only,
    /// even though you've used a different resource type elsewhere.
    #[track_caller]
    fn arc_resource_with_options<
        K: PartialEq + Eq + Hash + Clone + Send + Sync + 'static,
        V: Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
    >(
        &self,
        query_scope: impl QueryScopeTrait<K, V> + Send + Sync + 'static,
        keyer: impl Fn() -> K + Send + Sync + 'static,
        blocking: bool,
    ) -> ArcResource<V> {
        let cache_key = query_scope.cache_key();
        let query_scope = Arc::new(query_scope);
        let scope_lookup = self.scope_lookup;
        let self_ = *self;
        let query_options = query_scope.options();

        let active_key_memo = ArcMemo::new(move |_| keyer());
        let next_buster = ArcRwSignal::new(random_u64_rolling());
        let resource = ArcResource::new_with_options(
            {
                let next_buster = next_buster.clone();
                let active_key_memo = active_key_memo.clone();
                move || {
                    let key = active_key_memo.get();
                    scope_lookup.with_cached_query::<K, V, _>(&key, &cache_key, |maybe_cached| {
                        if let Some(cached) = maybe_cached {
                            // Buster must be returned for it to be tracked.
                            (key.clone(), cached.buster.get())
                        } else {
                            // Buster must be returned for it to be tracked.
                            (key.clone(), next_buster.get())
                        }
                    })
                }
            },
            {
                move |(key, _)| {
                    let query_scope = query_scope.clone();
                    let next_buster = next_buster.clone();
                    async move {
                        if let Some(cached) = scope_lookup.with_cached_query::<K, V, _>(
                            &key,
                            &cache_key,
                            |maybe_cached| {
                                maybe_cached.and_then(|cached| cached.value_if_not_stale().cloned())
                            },
                        ) {
                            cached
                        } else {
                            scope_lookup
                                .cached_or_fetch(
                                    &self_,
                                    key,
                                    cache_key,
                                    move |key| async move { query_scope.query(key).await },
                                    Some(next_buster),
                                    false, // tracking is done via the key fn
                                    || Box::new(Scope::<K, V>::default()),
                                    query_options,
                                )
                                .await
                        }
                    }
                }
            },
            blocking,
        );

        // On the client, want to repopulate the frontend cache, so should write resources to the cache here if they don't exist.
        // TODO it would be better if in here we could check if the resource was started on the backend/streamed, saves doing most of this if already a frontend resource.
        let effect = {
            let active_key_memo = active_key_memo.clone();
            let resource = resource.clone();
            let self_ = *self;
            // Converting to Arc because the tests like the client get dropped even though this persists:
            move |complete: Option<Option<()>>| {
                if let Some(Some(())) = complete {
                    return Some(());
                }
                if let Some(val) = resource.read().as_ref() {
                    scope_lookup.with_cached_scope_mut::<K, V, _>(
                        cache_key,
                        || Some(Box::new(Scope::<K, V>::default())),
                        |maybe_scope| {
                            let scope = maybe_scope.expect("provided a default");
                            let key = active_key_memo.read();
                            if !scope.cache.contains_key(&key) {
                                scope.cache.insert(
                                    key.clone(),
                                    Query::new(
                                        self_,
                                        cache_key,
                                        &*key,
                                        val.clone(),
                                        ArcRwSignal::new(random_u64_rolling()),
                                        query_options,
                                    ),
                                );
                            }
                        },
                    );
                    Some(())
                } else {
                    None
                }
            }
        };
        // Won't run in tests if not isomorphic, but in prod Effect is wanted to not run on server:
        #[cfg(test)]
        Effect::new_isomorphic(effect);
        #[cfg(not(test))]
        Effect::new(effect);

        resource
    }

    /// TODO
    pub async fn prefetch_query<K, V>(
        &self,
        query_scope: impl QueryScopeTrait<K, V> + Send + Sync + 'static,
        key: &K,
    ) where
        K: Clone + Eq + Hash + Send + Sync + 'static,
        V: serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
    {
        let query_options = query_scope.options();
        self.prefetch_inner(
            query_scope.cache_key(),
            move |key| async move { query_scope.query(key).await },
            key,
            || Box::new(Scope::<K, V>::default()),
            query_options,
        )
        .await
    }

    /// TODO
    pub async fn prefetch_local_query<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V> + 'static,
        key: &K,
    ) where
        K: Clone + Eq + Hash + 'static,
        V: 'static,
    {
        let query_options = query_scope.options();
        self.prefetch_inner(
            query_scope.cache_key(),
            move |key| async move { query_scope.query(key).await },
            key,
            || Box::new(SendWrapper::new(Scope::<K, V>::default())),
            query_options,
        )
        .await
    }

    async fn prefetch_inner<K, V, Fut>(
        &self,
        cache_key: TypeId,
        fetcher: impl FnOnce(K) -> Fut + 'static,
        key: &K,
        default_scope_cb: impl FnOnce() -> Box<dyn ScopeTrait> + Clone,
        query_options: Option<QueryOptions>,
    ) where
        K: Clone + Eq + Hash + 'static,
        V: 'static,
        Fut: Future<Output = V> + 'static,
    {
        let needs_prefetch =
            self.scope_lookup
                .with_cached_query::<K, V, _>(key, &cache_key, |maybe_cached| {
                    maybe_cached.is_none()
                });
        if needs_prefetch {
            self.scope_lookup
                .cached_or_fetch_inner::<K, V, _, _>(
                    self,
                    key.clone(),
                    cache_key,
                    fetcher,
                    None,
                    false,
                    default_scope_cb,
                    |_v| {},
                    query_options,
                )
                .await
        }
    }

    /// TODO
    pub async fn fetch_query<K, V>(
        &self,
        query_scope: impl QueryScopeTrait<K, V> + Send + Sync + 'static,
        key: &K,
    ) -> V
    where
        K: Clone + Eq + Hash + Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        let query_options = query_scope.options();
        self.fetch_inner(
            query_scope.cache_key(),
            move |key| async move { query_scope.query(key).await },
            key,
            || Box::new(Scope::<K, V>::default()),
            query_options,
        )
        .await
    }

    /// TODO
    pub async fn fetch_local_query<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V> + 'static,
        key: &K,
    ) -> V
    where
        K: Clone + Eq + Hash + 'static,
        V: Clone + 'static,
    {
        let query_options = query_scope.options();
        self.fetch_inner(
            query_scope.cache_key(),
            move |key| async move { query_scope.query(key).await },
            key,
            || Box::new(SendWrapper::new(Scope::<K, V>::default())),
            query_options,
        )
        .await
    }

    async fn fetch_inner<K, V, Fut>(
        &self,
        cache_key: TypeId,
        fetcher: impl FnOnce(K) -> Fut + 'static,
        key: &K,
        default_scope_cb: impl FnOnce() -> Box<dyn ScopeTrait> + Clone,
        query_options: Option<QueryOptions>,
    ) -> V
    where
        K: Clone + Eq + Hash + 'static,
        V: Clone + 'static,
        Fut: Future<Output = V> + 'static,
    {
        let maybe_cached =
            self.scope_lookup
                .with_cached_query::<K, V, _>(key, &cache_key, |maybe_cached| {
                    maybe_cached.and_then(|cached| cached.value_if_not_stale().cloned())
                });
        if let Some(cached) = maybe_cached {
            cached
        } else {
            self.scope_lookup
                .cached_or_fetch_inner::<K, V, _, _>(
                    self,
                    key.clone(),
                    cache_key,
                    fetcher,
                    None,
                    false,
                    default_scope_cb,
                    |v| v.clone(),
                    query_options,
                )
                .await
        }
    }

    /// TODO
    #[track_caller]
    pub fn set_query<K, V>(
        &self,
        query_scope: impl QueryScopeTrait<K, V> + Send + Sync + 'static,
        key: &K,
        new_value: V,
    ) where
        K: Clone + Eq + Hash + Send + Sync + 'static,
        V: Send + Sync + 'static,
    {
        self.set_inner(
            query_scope.cache_key(),
            key,
            new_value,
            || Box::new(Scope::<K, V>::default()),
            query_scope.options(),
        )
    }

    /// TODO
    #[track_caller]
    pub fn set_local_query<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V> + 'static,
        key: &K,
        new_value: V,
    ) where
        K: Clone + Eq + Hash + 'static,
        V: 'static,
    {
        self.set_inner::<K, V>(
            query_scope.cache_key(),
            key,
            new_value,
            || Box::new(SendWrapper::new(Scope::<K, V>::default())),
            query_scope.options(),
        )
    }

    #[track_caller]
    fn set_inner<K, V>(
        &self,
        cache_key: TypeId,
        key: &K,
        new_value: V,
        default_scope_cb: impl FnOnce() -> Box<dyn ScopeTrait> + Clone,
        query_options: Option<QueryOptions>,
    ) where
        K: Clone + Eq + Hash + 'static,
        V: 'static,
    {
        self.scope_lookup.with_cached_scope_mut::<K, V, _>(
            cache_key,
            || Some(default_scope_cb()),
            |maybe_scope| {
                let scope = maybe_scope.expect("provided a default");
                if let Some(cached) = scope.cache.get_mut(key) {
                    cached.set_value(new_value);
                    // To update all existing resources:
                    cached.buster.set(random_u64_rolling());
                } else {
                    let query = Query::new(
                        *self,
                        cache_key,
                        key,
                        new_value,
                        ArcRwSignal::new(random_u64_rolling()),
                        query_options,
                    );
                    scope.cache.insert(key.clone(), query);
                }
            },
        );
    }

    /// If the key already exists, update it with the provided modifier callback.
    #[track_caller]
    pub fn update_query<K, V, T>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V> + 'static,
        key: &K,
        modifier: impl FnOnce(&mut Option<V>) -> T,
    ) -> T
    where
        K: Clone + Eq + Hash + 'static,
        V: 'static,
    {
        let mut modifier_holder = Some(modifier);

        let maybe_return_value = self.scope_lookup.with_cached_scope_mut::<K, V, _>(
            query_scope.cache_key(),
            || None,
            |maybe_scope| {
                if let Some(scope) = maybe_scope {
                    if let Some((key, mut cached)) = scope.cache.remove_entry(key) {
                        let old_value = cached.steal_value_danger_must_replace();
                        let mut new_value_holder = Some(old_value);
                        let return_value = modifier_holder
                            .take()
                            .expect("Should never be used more than once.")(
                            &mut new_value_holder
                        );
                        if let Some(new_value) = new_value_holder {
                            let query = Query::new(
                                *self,
                                query_scope.cache_key(),
                                &key,
                                new_value,
                                cached.buster.clone(),
                                query_scope.options(),
                            );
                            scope.cache.insert(key, query);
                        } else {
                            // Means the user wants to invalidate the query, just removed so no need to do anything.
                        }
                        // To update all existing resources:
                        cached.buster.set(random_u64_rolling());
                        return Some(return_value);
                    }
                }
                None
            },
        );
        if let Some(return_value) = maybe_return_value {
            return_value
        } else {
            let mut new_value = None;
            let return_value =
                modifier_holder
                    .take()
                    .expect("Should never be used more than once.")(&mut new_value);
            if new_value.is_some() {
                todo!()
            }
            return_value
        }
    }

    /// TODO
    pub fn get_cached_query<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V> + 'static,
        key: &K,
    ) -> Option<V>
    where
        K: Eq + Hash + 'static,
        V: Clone + 'static,
    {
        self.scope_lookup.with_cached_query::<K, V, _>(
            key,
            &query_scope.cache_key(),
            |maybe_cached| maybe_cached.map(|cached| cached.value_even_if_stale().clone()),
        )
    }

    /// Check whether the key exists in the cache or not.
    #[track_caller]
    pub fn query_exists<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V> + 'static,
        key: &K,
    ) -> bool
    where
        K: Eq + Hash + 'static,
        V: 'static,
    {
        self.scope_lookup.with_cached_query::<K, V, _>(
            key,
            &query_scope.cache_key(),
            |maybe_cached| maybe_cached.is_some(),
        )
    }

    /// Clear a specific key from the cache. Any resources actively in use will refetch.
    /// Returns true if the key existed and was removed.
    #[track_caller]
    pub fn invalidate_query<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V> + 'static,
        key: &K,
    ) -> bool
    where
        K: Eq + Hash + 'static,
        V: 'static,
    {
        let cleared = self.invalidate_queries(query_scope, std::iter::once(key));
        !cleared.is_empty()
    }

    /// TODO
    #[track_caller]
    pub fn invalidate_queries<K, V, KRef>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V> + 'static,
        keys: impl IntoIterator<Item = KRef>,
    ) -> Vec<K>
    where
        K: Eq + Hash + 'static,
        V: 'static,
        KRef: Borrow<K>,
    {
        self.invalidate_queries_inner::<K, V, _>(query_scope.cache_key(), keys)
    }

    #[track_caller]
    pub(crate) fn invalidate_queries_inner<K, V, KRef>(
        &self,
        cache_key: TypeId,
        keys: impl IntoIterator<Item = KRef>,
    ) -> Vec<K>
    where
        K: Eq + Hash + 'static,
        V: 'static,
        KRef: Borrow<K>,
    {
        // Need to remove from the cache if it exists, if it does need to ping the buster too.
        self.scope_lookup.with_cached_scope_mut::<K, V, _>(
            cache_key,
            || None,
            |maybe_scope| {
                let mut cleared = vec![];
                if let Some(scope) = maybe_scope {
                    for key in keys.into_iter() {
                        if let Some((key, cached)) = scope.cache.remove_entry(key.borrow()) {
                            cached.buster.set(random_u64_rolling());
                            cleared.push(key);
                        }
                    }
                }
                cleared
            },
        )
    }

    /// Clear a query type from the cache. Any resources actively in use will refetch.
    #[track_caller]
    pub fn invalidate_query_type<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V> + 'static,
    ) where
        K: Eq + Hash + 'static,
        V: 'static,
    {
        let maybe_scope = self
            .scope_lookup
            .scopes
            .write_value()
            .remove(&query_scope.cache_key());
        if let Some(scope) = maybe_scope {
            for buster in scope.busters() {
                buster.try_set(random_u64_rolling());
            }
        }
    }

    /// Clear all data in the cache/store, all resources etc will refetch.
    #[track_caller]
    pub fn invalidate_all_queries(&self) {
        let scopes = std::mem::take(&mut *self.scope_lookup.scopes.write_value());
        for buster in scopes
            .values()
            .flat_map(|scope_cache| scope_cache.busters())
        {
            buster.try_set(random_u64_rolling());
        }
    }
}
