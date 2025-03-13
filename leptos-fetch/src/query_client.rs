use std::{any::TypeId, borrow::Borrow, future::Future, hash::Hash, sync::Arc};

use leptos::{
    prelude::{
        expect_context, provide_context, ArcMemo, ArcRwSignal, Effect, Get, Read, Set, Track,
        WriteValue,
    },
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

// TODO: gc must not gc if resources in use, they have to reset the gc timer.
// TODO test query type separation even when K and V are the same, should fail but work once we switch to the trait method.
// TODO check a local resource can be accessed from a normal one and vice versa.
// TODO: garbage collection etc and other LQ stuff + check size on gc etc to make sure no memory leaks.
// TODO SendWrapper should never panic, a local resource/query method accessed from a different thread should just have to fetch again
// TODO readme
// TODO type docs
// TODO feature parity

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
    pub fn local_resource<K: PartialEq + Eq + Hash + Clone + 'static, V: Clone + 'static>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V> + 'static,
        keyer: impl Fn() -> K + 'static,
    ) -> LocalResource<V> {
        self.arc_local_resource(query_scope, keyer).into()
    }

    /// Query with [`ArcLocalResource`]. Local resouces only load data on the client, so can be used with non-threadsafe/serializable data.
    /// 
    /// If a cached value exists but is stale, the cached value will be initially used, then refreshed in the background, updating once the new value is ready.
    #[track_caller]
    pub fn arc_local_resource<K: PartialEq + Eq + Hash + Clone + 'static, V: Clone + 'static>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V> + 'static,
        keyer: impl Fn() -> K + 'static,
    ) -> ArcLocalResource<V> {
        let client = *self;
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
                        |maybe_cached| {
                            if let Some(cached) = maybe_cached {
                                cached.buster.track();

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

    /// Query with [`Resource`]. 
    /// 
    /// Resources must be serializable to potentially loade in `ssr` and stream to the client. 
    /// 
    /// Resources must be `Send` and `Sync` to be multithreaded in ssr.
    /// 
    /// If a cached value exists but is stale, the cached value will be initially used, then refreshed in the background, updating once the new value is ready.
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

    /// Query with a blocking [`Resource`].
    /// 
    /// Resources must be serializable to potentially loade in `ssr` and stream to the client. 
    /// 
    /// Resources must be `Send` and `Sync` to be multithreaded in ssr.
    /// 
    /// If a cached value exists but is stale, the cached value will be initially used, then refreshed in the background, updating once the new value is ready.
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

    /// Query with [`ArcResource`]. 
    /// 
    /// Resources must be serializable to potentially loade in `ssr` and stream to the client. 
    /// 
    /// Resources must be `Send` and `Sync` to be multithreaded in ssr.
    /// 
    /// If a cached value exists but is stale, the cached value will be initially used, then refreshed in the background, updating once the new value is ready.
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

    /// Query with a blocking [`ArcResource`].
    /// 
    /// Resources must be serializable to potentially loade in `ssr` and stream to the client. 
    /// 
    /// Resources must be `Send` and `Sync` to be multithreaded in ssr.
    /// 
    /// If a cached value exists but is stale, the cached value will be initially used, then refreshed in the background, updating once the new value is ready.
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
        let client = *self;
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
                                maybe_cached.map(|cached| {
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

    /// Prefetch a non-threadsafe query and store it in the cache. 
    /// 
    /// - Entry doesn't exist: fetched and stored in the cache.
    /// - Entry exists but **not** stale: fetched and updated in the cache.
    /// - Entry exists but stale: not refreshed, existing cache item remains.
    /// 
    /// If the cached query changes, active resources using the query will be updated.
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
                    if let Some(cached) = maybe_cached {
                        cached.stale()
                    } else {
                        true
                    }
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

    /// Fetch a non-threadsafe query, store it in the cache and return it. 
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

    /// Set the value of a query in the cache. 
    /// 
    /// Active resources using the query will be updated.
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

    /// Set the value of a non-threadsafe query in the cache.
    /// 
    /// Active resources using the query will be updated.
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

    /// Update the value of a query in the cache with a callback.
    ///
    /// Active resources using the query will be updated.
    /// 
    /// The callback takes `&mut Option<V>`, where `Some` is the current value if it exists so can be used for setting and removing queries as well as updating cached values.
    /// 
    /// The return value of the callback is returned.
    #[track_caller]
    pub fn update_query<K, V, T>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V> + 'static,
        key: &K,
        modifier: impl FnOnce(&mut Option<V>) -> T,
    ) -> T
    where
        K: Clone + Eq + Hash + Send + Sync + 'static,
        V: Send + Sync + 'static,
    {
        self.update_query_inner(query_scope, key, modifier, || {
            Box::new(Scope::<K, V>::default())
        })
    }    

    /// Update the value of a non-threadsafe query in the cache with a callback.
    ///
    /// Active resources using the query will be updated.
    /// 
    /// The callback takes `&mut Option<V>`, where `Some` is the current value if it exists so can be used for setting and removing queries as well as updating cached values.
    /// 
    /// The return value of the callback is returned.
    #[track_caller]
    pub fn update_query_local<K, V, T>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V> + 'static,
        key: &K,
        modifier: impl FnOnce(&mut Option<V>) -> T,
    ) -> T
    where
        K: Clone + Eq + Hash + 'static,
        V: 'static,
    {
        self.update_query_inner(query_scope, key, modifier, || {
            Box::new(SendWrapper::new(Scope::<K, V>::default()))
        })
    }

    #[track_caller]
    fn update_query_inner<K, V, T>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V> + 'static,
        key: &K,
        modifier: impl FnOnce(&mut Option<V>) -> T,
        default_scope_cb: impl FnOnce() -> Box<dyn ScopeTrait> + Clone,
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
                    if let Some((key, cached)) = scope.cache.remove_entry(key) {
                        let old_value = cached.value_maybe_stale.into_value();
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
            // Didn't exist, callback might create one:

            let mut new_value = None;
            let return_value =
                modifier_holder
                    .take()
                    .expect("Should never be used more than once.")(&mut new_value);

            // Set and cache if something's been created:
            if let Some(new_value) = new_value {
                self.set_inner(
                    query_scope.cache_key(),
                    key,
                    new_value,
                    default_scope_cb,
                    query_scope.options(),
                );
            }

            return_value
        }
    }

    /// Synchronously get a query from the cache, if it exists.
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
            |maybe_cached| maybe_cached.map(|cached| cached.value_maybe_stale.value().clone()),
        )
    }

    /// Synchronously check if a query exists in the cache.
    /// 
    /// Returns `true` if the query exists.
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

    /// Mark a query as stale. The next time it's accessed it'll be refetched.
    /// 
    /// Resources actively using the query will be updated.
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

    /// Mark multiple queries of a specific type as stale. The next time each query is accessed it'll be refetched.
    /// 
    /// Active resources using a query will be updated.
    #[track_caller]
    pub fn invalidate_queries<K, V, KRef>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V> + 'static,
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
        self.scope_lookup.with_cached_scope_mut::<K, V, _>(
            cache_key,
            || None,
            |maybe_scope| {
                let mut invalidated = vec![];
                if let Some(scope) = maybe_scope {
                    for key in keys.into_iter() {
                        if let Some(cached) = scope.cache.get_mut(key.borrow()) {
                            cached.invalidate();
                            cached.buster.set(random_u64_rolling());
                            invalidated.push(key);
                        }
                    }
                }
                invalidated
            },
        )
    }

    /// Mark all queries of a specific type as stale. The next time each query is accessed it'll be refetched.
    /// 
    /// Active resources using a query will be updated.
    #[track_caller]
    pub fn invalidate_query_type<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V> + 'static,
    ) where
        K: Eq + Hash + 'static,
        V: 'static,
    {
        let mut guard = self.scope_lookup.scopes.write_value();
        if let Some(scope) = guard.get_mut(&query_scope.cache_key()) {
            scope.invalidate_scope();
            for buster in scope.busters() {
                buster.try_set(random_u64_rolling());
            }
        }
    }

    /// Mark all queries as stale. The next time any query is accessed it'll be refetched.
    /// 
    /// Active resources using a query will be updated.
    #[track_caller]
    pub fn invalidate_all_queries(&self) {
        let mut guard = self.scope_lookup.scopes.write_value();
        for scope in guard.values_mut() {
            scope.invalidate_scope();
        }
        for buster in guard.values().flat_map(|scope_cache| scope_cache.busters()) {
            buster.try_set(random_u64_rolling());
        }
    }
}
