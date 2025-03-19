use std::{any::TypeId, borrow::Borrow, future::Future, hash::Hash, sync::Arc};

use leptos::{
    prelude::{
        expect_context, provide_context, ArcMemo, ArcRwSignal, Effect, Get, Read, Set, Track,
    },
    server::{ArcLocalResource, ArcResource, LocalResource, Resource},
};
use send_wrapper::SendWrapper;
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    maybe_local::MaybeLocal,
    query::Query,
    resource_drop_guard::ResourceDropGuard,
    utils::{new_buster_id, new_resource_id, KeyHash},
    QueryOptions, QueryScopeLocalTrait, QueryScopeTrait,
};

use super::cache::ScopeLookup;

/// The [`QueryClient`] stores all query data, and is used to manage queries.
///
/// Should be provided via leptos context at the top of the app.
///
/// # Example
///
/// ```
/// use leptos::prelude::*;
/// use leptos_fetch::QueryClient;
///
/// #[component]
/// pub fn App() -> impl IntoView {
///    QueryClient::provide();
///     // ...
/// }
///
/// #[component]
/// pub fn MyComponent() -> impl IntoView {
///     let client = QueryClient::expect();
///      // ...
/// }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct QueryClient {
    pub(crate) scope_lookup: ScopeLookup,
    options: QueryOptions,
}

impl Default for QueryClient {
    fn default() -> Self {
        Self::new()
    }
}

impl QueryClient {
    /// Creates a new [`QueryClient`].
    pub fn new() -> Self {
        Self {
            scope_lookup: ScopeLookup::new(),
            options: QueryOptions::default(),
        }
    }

    /// Create a new [`QueryClient`] with custom options.
    ///
    /// These options will be combined with any options for a specific query type/scope.
    pub fn new_with_options(options: QueryOptions) -> Self {
        Self {
            scope_lookup: ScopeLookup::new(),
            options,
        }
    }

    /// Create a new [`QueryClient`] and provide it via leptos context.
    ///
    /// The client can then be accessed with [`QueryClient::expect()`] from any child component.
    pub fn provide() {
        provide_context(Self::new())
    }

    /// Create a new [`QueryClient`] with custom options and provide it via leptos context.
    ///
    /// The client can then be accessed with [`QueryClient::expect()`] from any child component.
    ///
    /// These options will be combined with any options for a specific query type/scope.
    pub fn provide_with_options(options: QueryOptions) {
        provide_context(Self::new_with_options(options))
    }

    /// Extract the [`QueryClient`] out of leptos context.
    ///
    /// Shorthand for `expect_context::<QueryClient>()`.
    ///
    /// # Panics
    ///
    /// Panics if the [`QueryClient`] has not been provided via leptos context by a parent component.
    #[track_caller]
    pub fn expect() -> Self {
        expect_context()
    }

    /// Read the base [`QueryOptions`] for this [`QueryClient`].
    ///
    /// These will be combined with any options for a specific query type/scope.
    pub fn options(&self) -> QueryOptions {
        self.options
    }

    /// Query with [`LocalResource`]. Local resouces only load data on the client, so can be used with non-threadsafe/serializable data.
    ///
    /// If a cached value exists but is stale, the cached value will be initially used, then refreshed in the background, updating once the new value is ready.
    #[track_caller]
    pub fn local_resource<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V> + 'static,
        keyer: impl Fn() -> K + 'static,
    ) -> LocalResource<V>
    where
        K: Eq + Hash + Clone + 'static,
        V: Clone + 'static,
    {
        self.arc_local_resource(query_scope, keyer).into()
    }

    /// Query with [`ArcLocalResource`]. Local resouces only load data on the client, so can be used with non-threadsafe/serializable data.
    ///
    /// If a cached value exists but is stale, the cached value will be initially used, then refreshed in the background, updating once the new value is ready.
    #[track_caller]
    pub fn arc_local_resource<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V> + 'static,
        keyer: impl Fn() -> K + 'static,
    ) -> ArcLocalResource<V>
    where
        K: Eq + Hash + Clone + 'static,
        V: Clone + 'static,
    {
        let client = *self;
        let scope_lookup = self.scope_lookup;
        let cache_key = query_scope.cache_key();
        let query_scope = Arc::new(query_scope);
        let self_ = *self;
        let query_options = query_scope.options();
        let resource_id = new_resource_id();

        // To call .mark_resource_dropped() when the resource is dropped:
        let drop_guard = ResourceDropGuard::<K, V>::new(self.scope_lookup, resource_id, cache_key);

        let resource = ArcLocalResource::new({
            move || {
                let query_scope = query_scope.clone();
                let key = keyer();
                let drop_guard = drop_guard.clone();
                drop_guard.set_active_key(key.clone());
                async move {
                    let _ = drop_guard; // Want the guard around everywhere until the resource is dropped.

                    // First try using the cache:
                    if let Some(cached) = scope_lookup.with_cached_query::<K, V, _>(
                        &key,
                        &cache_key,
                        |maybe_cached| {
                            if let Some(cached) = maybe_cached {
                                cached.buster.track();

                                cached.mark_resource_active(resource_id);

                                // If stale refetch in the background with the prefetch() function, which'll recognise it's stale, refetch it and invalidate busters:
                                if cfg!(any(test, not(feature = "ssr"))) && cached.stale() {
                                    let key = key.clone();
                                    let query_scope = query_scope.clone();
                                    // Just adding the SendWrapper and using spawn() rather than spawn_local() to fix tests:
                                    leptos::task::spawn(SendWrapper::new(async move {
                                        client.prefetch_local_query(query_scope, &key).await;
                                    }));
                                }

                                Some(cached.value_maybe_stale.value().clone())
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
                            &key,
                            cache_key,
                            move |key| async move {
                                MaybeLocal::new_local(query_scope.query(key).await)
                            },
                            None,
                            true,
                            query_options,
                            Some(resource_id),
                        )
                        .await
                }
            }
        });

        resource
    }

    /// Query with [`Resource`].
    ///
    /// Resources must be serializable to potentially loade in `ssr` and stream to the client.
    ///
    /// Resources must be `Send` and `Sync` to be multithreaded in ssr.
    ///
    /// If a cached value exists but is stale, the cached value will be initially used, then refreshed in the background, updating once the new value is ready.
    #[track_caller]
    pub fn resource<K, V>(
        &self,
        query_scope: impl QueryScopeTrait<K, V> + Send + Sync + 'static,
        keyer: impl Fn() -> K + Send + Sync + 'static,
    ) -> Resource<V>
    where
        K: Eq + Hash + Clone + Send + Sync + 'static,
        V: Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
    {
        self.arc_resource_with_options(query_scope, keyer, false)
            .into()
    }

    /// Query with a blocking [`Resource`].
    ///
    /// Resources must be serializable to potentially loade in `ssr` and stream to the client.
    ///
    /// Resources must be `Send` and `Sync` to be multithreaded in ssr.
    ///
    /// If a cached value exists but is stale, the cached value will be initially used, then refreshed in the background, updating once the new value is ready.
    #[track_caller]
    pub fn resource_blocking<K, V>(
        &self,
        query_scope: impl QueryScopeTrait<K, V> + Send + Sync + 'static,
        keyer: impl Fn() -> K + Send + Sync + 'static,
    ) -> Resource<V>
    where
        K: Eq + Hash + Clone + Send + Sync + 'static,
        V: Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
    {
        self.arc_resource_with_options(query_scope, keyer, true)
            .into()
    }

    /// Query with [`ArcResource`].
    ///
    /// Resources must be serializable to potentially loade in `ssr` and stream to the client.
    ///
    /// Resources must be `Send` and `Sync` to be multithreaded in ssr.
    ///
    /// If a cached value exists but is stale, the cached value will be initially used, then refreshed in the background, updating once the new value is ready.
    #[track_caller]
    pub fn arc_resource<K, V>(
        &self,
        query_scope: impl QueryScopeTrait<K, V> + Send + Sync + 'static,
        keyer: impl Fn() -> K + Send + Sync + 'static,
    ) -> ArcResource<V>
    where
        K: Eq + Hash + Clone + Send + Sync + 'static,
        V: Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
    {
        self.arc_resource_with_options(query_scope, keyer, false)
    }

    /// Query with a blocking [`ArcResource`].
    ///
    /// Resources must be serializable to potentially loade in `ssr` and stream to the client.
    ///
    /// Resources must be `Send` and `Sync` to be multithreaded in ssr.
    ///
    /// If a cached value exists but is stale, the cached value will be initially used, then refreshed in the background, updating once the new value is ready.
    #[track_caller]
    pub fn arc_resource_blocking<K, V>(
        &self,
        query_scope: impl QueryScopeTrait<K, V> + Send + Sync + 'static,
        keyer: impl Fn() -> K + Send + Sync + 'static,
    ) -> ArcResource<V>
    where
        K: Eq + Hash + Clone + Send + Sync + 'static,
        V: Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
    {
        self.arc_resource_with_options(query_scope, keyer, true)
    }

    #[track_caller]
    fn arc_resource_with_options<K, V>(
        &self,
        query_scope: impl QueryScopeTrait<K, V> + Send + Sync + 'static,
        keyer: impl Fn() -> K + Send + Sync + 'static,
        blocking: bool,
    ) -> ArcResource<V>
    where
        K: Eq + Hash + Clone + Send + Sync + 'static,
        V: Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
    {
        let client = *self;
        let cache_key = query_scope.cache_key();
        let query_scope = Arc::new(query_scope);
        let scope_lookup = self.scope_lookup;
        let self_ = *self;
        let query_options = query_scope.options();

        let active_key_memo = ArcMemo::new(move |_| keyer());
        let next_buster = ArcRwSignal::new(new_buster_id());
        let resource_id = new_resource_id();

        // To call .mark_resource_dropped() when the resource is dropped:
        let drop_guard = ResourceDropGuard::<K, V>::new(self.scope_lookup, resource_id, cache_key);

        let resource = ArcResource::new_with_options(
            {
                let next_buster = next_buster.clone();
                let active_key_memo = active_key_memo.clone();
                let drop_guard = drop_guard.clone();
                move || {
                    let key = active_key_memo.get();
                    drop_guard.set_active_key(key.clone());
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
                    let drop_guard = drop_guard.clone();
                    async move {
                        let _ = drop_guard; // Want the guard around everywhere until the resource is dropped.

                        if let Some(cached) = scope_lookup.with_cached_query::<K, V, _>(
                            &key,
                            &cache_key,
                            |maybe_cached| {
                                maybe_cached.map(|cached| {
                                    cached.mark_resource_active(resource_id);

                                    // If stale refetch in the background with the prefetch() function, which'll recognise it's stale, refetch it and invalidate busters:
                                    if cfg!(any(test, not(feature = "ssr"))) && cached.stale() {
                                        let key = key.clone();
                                        let query_scope = query_scope.clone();
                                        leptos::task::spawn(async move {
                                            client.prefetch_query(query_scope, &key).await;
                                        });
                                    }
                                    cached.value_maybe_stale.value().clone()
                                })
                            },
                        ) {
                            cached
                        } else {
                            scope_lookup
                                .cached_or_fetch(
                                    &self_,
                                    &key,
                                    cache_key,
                                    move |key| async move {
                                        MaybeLocal::new(query_scope.query(key).await)
                                    },
                                    Some(next_buster),
                                    false, // tracking is done via the key fn
                                    query_options,
                                    Some(resource_id),
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
                    scope_lookup.with_cached_scope_mut::<K, V, _>(cache_key, true, |maybe_scope| {
                        let scope = maybe_scope.expect("provided a default");
                        let key = active_key_memo.read();
                        if !scope.contains_key(&key) {
                            let key = KeyHash::new(&*key);
                            let query = Query::new::<K>(
                                self_,
                                cache_key,
                                &key,
                                MaybeLocal::new(val.clone()),
                                ArcRwSignal::new(new_buster_id()),
                                query_options,
                                None,
                            );
                            scope.insert(key, query);
                        }
                        scope.get(&key).unwrap().mark_resource_active(resource_id)
                    });
                    Some(())
                } else {
                    None
                }
            }
        };
        // Won't run in tests if not isomorphic, but in prod Effect is wanted to not run on server:
        if cfg!(test) {
            Effect::new_isomorphic(effect);
        } else {
            Effect::new(effect);
        }

        resource
    }

    /// Prefetch a query and store it in the cache.
    ///
    /// - Entry doesn't exist: fetched and stored in the cache.
    /// - Entry exists but **not** stale: fetched and updated in the cache.
    /// - Entry exists but stale: not refreshed, existing cache item remains.
    ///
    /// If the cached query changes, active resources using the query will be updated.
    pub async fn prefetch_query<K, V>(
        &self,
        query_scope: impl QueryScopeTrait<K, V>,
        key: impl Borrow<K>,
    ) where
        K: Clone + Eq + Hash + 'static,
        V: serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
    {
        let query_options = query_scope.options();
        self.prefetch_inner(
            query_scope.cache_key(),
            move |key| async move { MaybeLocal::new(query_scope.query(key).await) },
            key.borrow(),
            query_options,
        )
        .await
    }

    /// Prefetch a non-threadsafe query and store it in the cache for this thread only.
    ///
    /// - Entry doesn't exist: fetched and stored in the cache.
    /// - Entry exists but **not** stale: fetched and updated in the cache.
    /// - Entry exists but stale: not refreshed, existing cache item remains.
    ///
    /// If the cached query changes, active resources using the query will be updated.
    pub async fn prefetch_local_query<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V>,
        key: impl Borrow<K>,
    ) where
        K: Clone + Eq + Hash + 'static,
        V: 'static,
    {
        let query_options = query_scope.options();
        self.prefetch_inner(
            query_scope.cache_key(),
            move |key| async move { MaybeLocal::new_local(query_scope.query(key).await) },
            key.borrow(),
            query_options,
        )
        .await
    }

    async fn prefetch_inner<K, V, Fut>(
        &self,
        cache_key: TypeId,
        fetcher: impl FnOnce(K) -> Fut,
        key: &K,
        query_options: Option<QueryOptions>,
    ) where
        K: Clone + Eq + Hash + 'static,
        V: 'static,
        Fut: Future<Output = MaybeLocal<V>>,
    {
        let (needs_prefetch, custom_next_buster) =
            self.scope_lookup
                .with_cached_query::<K, V, _>(key, &cache_key, |maybe_cached| {
                    if let Some(cached) = maybe_cached {
                        (cached.stale(), Some(cached.buster.clone()))
                    } else {
                        (true, None)
                    }
                });
        if needs_prefetch {
            self.scope_lookup
                .cached_or_fetch_inner::<K, V, _, _>(
                    self,
                    key,
                    cache_key,
                    fetcher,
                    custom_next_buster,
                    false,
                    |_v| {},
                    query_options,
                    None,
                )
                .await
        }
    }

    /// Fetch a query, store it in the cache and return it.
    ///
    /// - Entry doesn't exist: fetched and stored in the cache.
    /// - Entry exists but **not** stale: fetched and updated in the cache.
    /// - Entry exists but stale: not refreshed, existing cache item remains.
    ///
    /// If the cached query changes, active resources using the query will be updated.
    ///
    /// Returns the up-to-date cached query.
    pub async fn fetch_query<K, V>(
        &self,
        query_scope: impl QueryScopeTrait<K, V>,
        key: impl Borrow<K>,
    ) -> V
    where
        K: Clone + Eq + Hash + 'static,
        V: Clone + Send + Sync + 'static,
    {
        let query_options = query_scope.options();
        self.fetch_inner(
            query_scope.cache_key(),
            move |key| async move { MaybeLocal::new(query_scope.query(key).await) },
            key.borrow(),
            query_options,
        )
        .await
    }

    /// Fetch a non-threadsafe query, store it in the cache for this thread only and return it.
    ///
    /// - Entry doesn't exist: fetched and stored in the cache.
    /// - Entry exists but **not** stale: fetched and updated in the cache.
    /// - Entry exists but stale: not refreshed, existing cache item remains.
    ///
    /// If the cached query changes, active resources using the query will be updated.
    ///
    /// Returns the up-to-date cached query.
    pub async fn fetch_local_query<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V>,
        key: impl Borrow<K>,
    ) -> V
    where
        K: Clone + Eq + Hash + 'static,
        V: Clone + 'static,
    {
        let query_options = query_scope.options();
        self.fetch_inner(
            query_scope.cache_key(),
            move |key| async move { MaybeLocal::new_local(query_scope.query(key).await) },
            key.borrow(),
            query_options,
        )
        .await
    }

    async fn fetch_inner<K, V, Fut>(
        &self,
        cache_key: TypeId,
        fetcher: impl FnOnce(K) -> Fut,
        key: &K,
        query_options: Option<QueryOptions>,
    ) -> V
    where
        K: Clone + Eq + Hash + 'static,
        V: Clone + 'static,
        Fut: Future<Output = MaybeLocal<V>>,
    {
        let maybe_cached = self
            .scope_lookup
            .with_cached_query::<K, V, _>(key, &cache_key, |maybe_cached| {
                maybe_cached.map(|cached| {
                    if cached.stale() {
                        None
                    } else {
                        Some(cached.value_maybe_stale.value().clone())
                    }
                })
            })
            .flatten();
        if let Some(cached) = maybe_cached {
            cached
        } else {
            self.scope_lookup
                .cached_or_fetch_inner::<K, V, _, _>(
                    self,
                    key,
                    cache_key,
                    fetcher,
                    None,
                    false,
                    |v| v.clone(),
                    query_options,
                    None,
                )
                .await
        }
    }

    /// Set the value of a query in the cache.
    /// This cached value will be available from all threads and take priority over any locally cached value for this query.
    ///
    /// Active resources using the query will be updated.
    #[track_caller]
    pub fn set_query<K, V>(
        &self,
        query_scope: impl QueryScopeTrait<K, V>,
        key: impl Borrow<K>,
        new_value: V,
    ) where
        K: Clone + Eq + Hash + 'static,
        V: Send + Sync + 'static,
    {
        self.set_inner(
            query_scope.cache_key(),
            key.borrow(),
            MaybeLocal::new(new_value),
            query_scope.options(),
        )
    }

    /// Set the value of a non-threadsafe query in the cache for this thread only.
    /// This cached value will only be available from this thread, the cache will return empty, unless a nonlocal value is set, from any other thread.
    ///
    /// Active resources using the query will be updated.
    #[track_caller]
    pub fn set_local_query<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V>,
        key: impl Borrow<K>,
        new_value: V,
    ) where
        K: Clone + Eq + Hash + 'static,
        V: 'static,
    {
        self.set_inner::<K, V>(
            query_scope.cache_key(),
            key.borrow(),
            MaybeLocal::new_local(new_value),
            query_scope.options(),
        )
    }

    #[track_caller]
    fn set_inner<K, V>(
        &self,
        cache_key: TypeId,
        key: &K,
        new_value: MaybeLocal<V>,
        query_options: Option<QueryOptions>,
    ) where
        K: Clone + Eq + Hash + 'static,
        V: 'static,
    {
        self.scope_lookup
            .with_cached_scope_mut::<K, V, _>(cache_key, true, |maybe_scope| {
                let scope = maybe_scope.expect("provided a default");

                // If we're updating an existing, make sure to update
                let maybe_cached = if new_value.is_local() {
                    scope.get_mut_local_only(key)
                } else {
                    scope.get_mut_threadsafe_only(key)
                };
                if let Some(cached) = maybe_cached {
                    cached.set_value(new_value);
                    // To update all existing resources:
                    cached.buster.set(new_buster_id());
                } else {
                    let key = KeyHash::new(key);
                    let query = Query::new::<K>(
                        *self,
                        cache_key,
                        &key,
                        new_value,
                        ArcRwSignal::new(new_buster_id()),
                        query_options,
                        None,
                    );
                    scope.insert(key, query);
                }
            });
    }

    /// Update the value of a query in the cache with a callback.
    ///
    /// Active resources using the query will be updated.
    ///
    /// The callback takes `Option<&mut V>`, will be None if the value is not available in the cache.
    ///
    /// Returns the output of the callback.
    #[track_caller]
    pub fn update_query<K, V, T>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V>,
        key: impl Borrow<K>,
        modifier: impl FnOnce(Option<&mut V>) -> T,
    ) -> T
    where
        K: Clone + Eq + Hash + 'static,
        V: 'static,
    {
        let key = key.borrow();

        let mut modifier_holder = Some(modifier);

        let maybe_return_value = self.scope_lookup.with_cached_scope_mut::<K, V, _>(
            query_scope.cache_key(),
            false,
            |maybe_scope| {
                if let Some(scope) = maybe_scope {
                    if let Some((key, cached)) = scope.remove_entry(key) {
                        let mut value = cached.value_maybe_stale.into_value_maybe_local();
                        let return_value = modifier_holder
                            .take()
                            .expect("Should never be used more than once.")(
                            Some(value.value_mut()),
                        );
                        let query = Query::new::<K>(
                            *self,
                            query_scope.cache_key(),
                            &key,
                            value,
                            cached.buster.clone(),
                            query_scope.options(),
                            Some(cached.active_resources),
                        );
                        scope.insert(key, query);
                        // To update all existing resources:
                        cached.buster.set(new_buster_id());
                        return Some(return_value);
                    }
                }
                None
            },
        );
        if let Some(return_value) = maybe_return_value {
            return_value
        } else {
            // Didn't exist:
            modifier_holder
                .take()
                .expect("Should never be used more than once.")(None)
        }
    }

    /// Synchronously get a query from the cache, if it exists.
    pub fn get_cached_query<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V>,
        key: impl Borrow<K>,
    ) -> Option<V>
    where
        K: Eq + Hash + 'static,
        V: Clone + 'static,
    {
        self.scope_lookup.with_cached_query::<K, V, _>(
            key.borrow(),
            &query_scope.cache_key(),
            |maybe_cached| maybe_cached.map(|cached| cached.value_maybe_stale.value().clone()),
        )
    }

    /// Synchronously check if a query exists in the cache.
    ///
    /// Returns `true` if the query exists.
    #[track_caller]
    pub fn query_exists<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V>,
        key: impl Borrow<K>,
    ) -> bool
    where
        K: Eq + Hash + 'static,
        V: 'static,
    {
        self.scope_lookup.with_cached_query::<K, V, _>(
            key.borrow(),
            &query_scope.cache_key(),
            |maybe_cached| maybe_cached.is_some(),
        )
    }

    /// Mark a query as stale. The next time it's accessed it'll be refetched.
    ///
    /// Resources actively using the query will be updated.
    #[track_caller]
    pub fn invalidate_query<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V>,
        key: impl Borrow<K>,
    ) -> bool
    where
        K: Eq + Hash + 'static,
        V: 'static,
    {
        let cleared = self.invalidate_queries(query_scope, std::iter::once(key));
        !cleared.is_empty()
    }

    /// Mark multiple queries of a specific type as stale. The next time each query is accessed it'll be refetched.
    ///
    /// Active resources using a query will be updated.
    #[track_caller]
    pub fn invalidate_queries<K, V, KRef>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V>,
        keys: impl IntoIterator<Item = KRef>,
    ) -> Vec<KRef>
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
    ) -> Vec<KRef>
    where
        K: Eq + Hash + 'static,
        V: 'static,
        KRef: Borrow<K>,
    {
        self.scope_lookup
            .with_cached_scope_mut::<K, V, _>(cache_key, false, |maybe_scope| {
                let mut invalidated = vec![];
                if let Some(scope) = maybe_scope {
                    for key in keys.into_iter() {
                        if let Some(cached) = scope.get_mut(key.borrow()) {
                            cached.invalidate();
                            cached.buster.set(new_buster_id());
                            invalidated.push(key);
                        }
                    }
                }
                invalidated
            })
    }

    /// Mark all queries of a specific type as stale. The next time each query is accessed it'll be refetched.
    ///
    /// Active resources using a query will be updated.
    #[track_caller]
    pub fn invalidate_query_type<K, V>(&self, query_scope: impl QueryScopeLocalTrait<K, V>)
    where
        K: Eq + Hash + 'static,
        V: 'static,
    {
        let mut guard = self.scope_lookup.scopes_mut();
        if let Some(scope) = guard.get_mut(&query_scope.cache_key()) {
            scope.invalidate_scope();
            for buster in scope.busters() {
                buster.try_set(new_buster_id());
            }
        }
    }

    /// Mark all queries as stale. The next time any query is accessed it'll be refetched.
    ///
    /// Active resources using a query will be updated.
    #[track_caller]
    pub fn invalidate_all_queries(&self) {
        let mut guard = self.scope_lookup.scopes_mut();
        for scope in guard.values_mut() {
            scope.invalidate_scope();
        }
        for buster in guard.values().flat_map(|scope_cache| scope_cache.busters()) {
            buster.try_set(new_buster_id());
        }
    }

    /// Empty the cache, note [`QueryClient::invalidate_all_queries`] is preferred in most cases.
    /// All active resources will instantly revert to pending until a new query is available.
    /// [`QueryClient::invalidate_all_queries`] on the other hand, will only refetch active queries in the background, replacing them when ready.
    #[track_caller]
    pub fn clear(&self) {
        let mut guard = self.scope_lookup.scopes_mut();
        for scope in guard.values_mut() {
            scope.clear();
        }
        for buster in guard.values().flat_map(|scope_cache| scope_cache.busters()) {
            buster.try_set(new_buster_id());
        }
    }

    #[allow(dead_code)]
    pub(crate) fn size(&self) -> usize {
        self.scope_lookup
            .scopes()
            .values()
            .map(|scope| scope.size())
            .sum()
    }
}
