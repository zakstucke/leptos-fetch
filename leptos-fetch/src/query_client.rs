use std::{any::TypeId, borrow::Borrow, fmt::Debug, future::Future, hash::Hash, sync::Arc};

use codee::{string::JsonSerdeCodec, Decoder, Encoder};
use leptos::{
    prelude::{provide_context, ArcRwSignal, ArcSignal, Effect, Get, Read, Set, Signal, Track},
    server::{
        ArcLocalResource, ArcResource, FromEncodedStr, IntoEncodedString, LocalResource, Resource,
    },
};
use send_wrapper::SendWrapper;

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
///    QueryClient::new().provide();
///     // ...
/// }
///
/// #[component]
/// pub fn MyComponent() -> impl IntoView {
///     let client: QueryClient = expect_context();
///      // ...
/// }
/// ```
#[derive(Debug)]
pub struct QueryClient<Codec = JsonSerdeCodec> {
    pub(crate) scope_lookup: ScopeLookup,
    options: QueryOptions,
    _ser: std::marker::PhantomData<SendWrapper<Codec>>,
}

impl<Codec: 'static> Clone for QueryClient<Codec> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Codec: 'static> Copy for QueryClient<Codec> {}

impl Default for QueryClient<JsonSerdeCodec> {
    fn default() -> Self {
        Self::new()
    }
}

impl QueryClient<JsonSerdeCodec> {
    /// Creates a new [`QueryClient`] with the default codec: [`codee::string::JsonSerdeCodec`].
    ///
    /// Call [`QueryClient::set_codec()`] to set a different codec.
    ///
    /// Call [`QueryClient::set_options()`] to set non-default options.
    #[track_caller]
    pub fn new() -> Self {
        Self {
            scope_lookup: ScopeLookup::new(),
            options: QueryOptions::default(),
            _ser: std::marker::PhantomData,
        }
    }
}

impl<Codec: 'static> QueryClient<Codec> {
    /// Provide the client to leptos context.
    /// ```no_run
    /// use leptos_fetch::QueryClient;
    /// use leptos::prelude::*;
    ///
    /// QueryClient::new().provide();
    ///
    /// let client: QueryClient = expect_context();
    /// ```
    #[track_caller]
    pub fn provide(self) -> Self {
        provide_context(self);
        self
    }

    /// **Applies to `ssr` only**
    ///
    /// It's possible to use non-json codecs for streaming leptos resources from the backend.
    /// The default is [`codee::string::JsonSerdeCodec`].
    ///
    /// The current `codee` major version is `0.3` and will need to be imported in your project to customize the codec.
    ///
    /// E.g. to use [`codee::binary::MsgpackSerdeCodec`](https://docs.rs/codee/latest/codee/binary/struct.MsgpackSerdeCodec.html):
    /// ```toml
    /// codee = { version = "0.3", features = ["msgpack_serde"] }
    /// ```
    ///
    /// This is a generic type on the [`QueryClient`], so when calling [`leptos::prelude::expect_context`],
    /// this type must be specified when not using the default.
    ///
    /// A useful pattern is to type alias the client with the custom codec for your whole app:
    ///
    /// ```no_run
    /// use codee::binary::MsgpackSerdeCodec;
    /// use leptos::prelude::*;
    /// use leptos_fetch::QueryClient;
    ///
    /// type MyQueryClient = QueryClient<MsgpackSerdeCodec>;
    ///
    /// // Create and provide to context to make accessible everywhere:
    /// QueryClient::new().set_codec::<MsgpackSerdeCodec>().provide();
    ///
    /// let client: MyQueryClient = expect_context();
    /// ```
    #[track_caller]
    pub fn set_codec<NewCodec>(self) -> QueryClient<NewCodec> {
        QueryClient {
            scope_lookup: self.scope_lookup,
            options: self.options,
            _ser: std::marker::PhantomData,
        }
    }

    /// Set non-default options to apply to all queries.
    ///
    /// These options will be combined with any options for a specific query type.
    #[track_caller]
    pub fn set_options(mut self, options: QueryOptions) -> Self {
        self.options = options;
        self
    }

    /// Read the base [`QueryOptions`] for this [`QueryClient`].
    ///
    /// These will be combined with any options for a specific query type/scope.
    #[track_caller]
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
        K: Hash + Clone + 'static,
        V: Clone + 'static,
    {
        let client = *self;
        let client_options = self.options();
        let scope_lookup = self.scope_lookup;
        let cache_key = query_scope.cache_key();
        let query_scope = Arc::new(query_scope);
        let query_options = query_scope.options();
        let resource_id = new_resource_id();

        // To call .mark_resource_dropped() when the resource is dropped:
        let drop_guard = ResourceDropGuard::<V>::new(self.scope_lookup, resource_id, cache_key);

        LocalResource::new({
            move || {
                let query_scope = query_scope.clone();
                let key = keyer();
                let key_hash = KeyHash::new(&key);
                let drop_guard = drop_guard.clone();
                drop_guard.set_active_key(key_hash);
                async move {
                    // First try using the cache:
                    if let Some(cached) = scope_lookup.with_cached_query::<V, _>(
                        &key_hash,
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
                                        client.prefetch_query_local(query_scope, &key).await;
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
                            client_options,
                            scope_lookup,
                            &key,
                            cache_key,
                            move |key| async move {
                                MaybeLocal::new_local(query_scope.query(key).await)
                            },
                            None,
                            true,
                            query_options,
                            Some(resource_id),
                            true,
                        )
                        .await
                }
            }
        })
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
        K: Hash + Clone + 'static,
        V: Clone + 'static,
    {
        // TODO on next 0.7 + 0.8 release, switch back to the arc as the base not the signal one:
        // https://github.com/leptos-rs/leptos/pull/3740
        // https://github.com/leptos-rs/leptos/pull/3741
        self.local_resource(query_scope, keyer).into()
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
    ) -> Resource<V, Codec>
    where
        K: PartialEq + Hash + Clone + Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
        Codec: Encoder<V> + Decoder<V>,
        <Codec as Encoder<V>>::Error: Debug,
        <Codec as Decoder<V>>::Error: Debug,
        <<Codec as Decoder<V>>::Encoded as FromEncodedStr>::DecodingError: Debug,
        <Codec as Encoder<V>>::Encoded: IntoEncodedString,
        <Codec as Decoder<V>>::Encoded: FromEncodedStr,
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
    ) -> Resource<V, Codec>
    where
        K: PartialEq + Hash + Clone + Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
        Codec: Encoder<V> + Decoder<V>,
        <Codec as Encoder<V>>::Error: Debug,
        <Codec as Decoder<V>>::Error: Debug,
        <<Codec as Decoder<V>>::Encoded as FromEncodedStr>::DecodingError: Debug,
        <Codec as Encoder<V>>::Encoded: IntoEncodedString,
        <Codec as Decoder<V>>::Encoded: FromEncodedStr,
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
    ) -> ArcResource<V, Codec>
    where
        K: PartialEq + Hash + Clone + Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
        Codec: Encoder<V> + Decoder<V>,
        <Codec as Encoder<V>>::Error: Debug,
        <Codec as Decoder<V>>::Error: Debug,
        <<Codec as Decoder<V>>::Encoded as FromEncodedStr>::DecodingError: Debug,
        <Codec as Encoder<V>>::Encoded: IntoEncodedString,
        <Codec as Decoder<V>>::Encoded: FromEncodedStr,
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
    ) -> ArcResource<V, Codec>
    where
        K: PartialEq + Hash + Clone + Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
        Codec: Encoder<V> + Decoder<V>,
        <Codec as Encoder<V>>::Error: Debug,
        <Codec as Decoder<V>>::Error: Debug,
        <<Codec as Decoder<V>>::Encoded as FromEncodedStr>::DecodingError: Debug,
        <Codec as Encoder<V>>::Encoded: IntoEncodedString,
        <Codec as Decoder<V>>::Encoded: FromEncodedStr,
    {
        self.arc_resource_with_options(query_scope, keyer, true)
    }

    #[track_caller]
    fn arc_resource_with_options<K, V>(
        &self,
        query_scope: impl QueryScopeTrait<K, V> + Send + Sync + 'static,
        keyer: impl Fn() -> K + Send + Sync + 'static,
        blocking: bool,
    ) -> ArcResource<V, Codec>
    where
        K: PartialEq + Hash + Clone + Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
        Codec: Encoder<V> + Decoder<V>,
        <Codec as Encoder<V>>::Error: Debug,
        <Codec as Decoder<V>>::Error: Debug,
        <<Codec as Decoder<V>>::Encoded as FromEncodedStr>::DecodingError: Debug,
        <Codec as Encoder<V>>::Encoded: IntoEncodedString,
        <Codec as Decoder<V>>::Encoded: FromEncodedStr,
    {
        let client = *self;
        let client_options = self.options();
        let cache_key = query_scope.cache_key();
        let query_scope = Arc::new(query_scope);
        let scope_lookup = self.scope_lookup;
        let query_options = query_scope.options();

        let next_buster = ArcRwSignal::new(new_buster_id());
        let resource_id = new_resource_id();

        // To call .mark_resource_dropped() when the resource is dropped:
        let drop_guard = ResourceDropGuard::<V>::new(self.scope_lookup, resource_id, cache_key);

        let keyer = Arc::new(keyer);
        let resource = ArcResource::new_with_options(
            {
                let next_buster = next_buster.clone();
                let drop_guard = drop_guard.clone();
                let keyer = keyer.clone();
                move || {
                    let key = keyer();
                    let key_hash = KeyHash::new(&key);
                    drop_guard.set_active_key(key_hash);
                    scope_lookup.with_cached_query::<V, _>(&key_hash, &cache_key, |maybe_cached| {
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
                    let key_hash = KeyHash::new(&key);
                    let query_scope = query_scope.clone();
                    let next_buster = next_buster.clone();
                    let _drop_guard = drop_guard.clone(); // Want the guard around everywhere until the resource is dropped.
                    async move {
                        if let Some(cached) = scope_lookup.with_cached_query::<V, _>(
                            &key_hash,
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
                                    client_options,
                                    scope_lookup,
                                    &key,
                                    cache_key,
                                    move |key| async move {
                                        MaybeLocal::new(query_scope.query(key).await)
                                    },
                                    Some(next_buster),
                                    false, // tracking is done via the key fn
                                    query_options,
                                    Some(resource_id),
                                    true,
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
            let resource = resource.clone();
            // Converting to Arc because the tests like the client get dropped even though this persists:
            move |complete: Option<Option<()>>| {
                if let Some(Some(())) = complete {
                    return Some(());
                }
                if let Some(val) = resource.read().as_ref() {
                    scope_lookup.with_cached_scope_mut::<V, _>(cache_key, true, |maybe_scope| {
                        let scope = maybe_scope.expect("provided a default");
                        let key = keyer();
                        let key_hash = KeyHash::new(&key);
                        if !scope.contains_key(&key_hash) {
                            let query = Query::new(
                                client_options,
                                scope_lookup,
                                cache_key,
                                &key_hash,
                                MaybeLocal::new(val.clone()),
                                ArcRwSignal::new(new_buster_id()),
                                query_options,
                                None,
                            );
                            scope.insert(key_hash, query);
                        }
                        scope
                            .get(&key_hash)
                            .unwrap()
                            .mark_resource_active(resource_id)
                    });
                    Some(())
                } else {
                    None
                }
            }
        };
        // Won't run in tests if not isomorphic, but in prod Effect is wanted to not run on server:
        if cfg!(test) {
            // Wants Send + Sync on the Codec despite using SendWrapper in the PhantomData.
            // Can put a SendWrapper around it as this is test only and they're single threaded:
            let effect = SendWrapper::new(effect);
            #[allow(clippy::redundant_closure)]
            Effect::new_isomorphic(move |v| effect(v));
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
        K: Clone + Hash + 'static,
        V: Send + Sync + 'static,
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
    pub async fn prefetch_query_local<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V>,
        key: impl Borrow<K>,
    ) where
        K: Clone + Hash + 'static,
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
        K: Clone + Hash + 'static,
        V: 'static,
        Fut: Future<Output = MaybeLocal<V>>,
    {
        let key_hash = KeyHash::new(key);
        let (needs_prefetch, custom_next_buster, loading_first_time) = self
            .scope_lookup
            .with_cached_query::<V, _>(&key_hash, &cache_key, |maybe_cached| {
                if let Some(cached) = maybe_cached {
                    (cached.stale(), Some(cached.buster.clone()), false)
                } else {
                    (true, None, true)
                }
            });
        if needs_prefetch {
            self.scope_lookup
                .cached_or_fetch_inner::<K, V, _, _>(
                    self.options(),
                    self.scope_lookup,
                    key,
                    cache_key,
                    fetcher,
                    custom_next_buster,
                    false,
                    |_v| {},
                    query_options,
                    None,
                    loading_first_time,
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
        K: Clone + Hash + 'static,
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
    pub async fn fetch_query_local<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V>,
        key: impl Borrow<K>,
    ) -> V
    where
        K: Clone + Hash + 'static,
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
        K: Clone + Hash + 'static,
        V: Clone + 'static,
        Fut: Future<Output = MaybeLocal<V>>,
    {
        let key_hash = KeyHash::new(key);
        let (maybe_cached, loading_first_time) = self
            .scope_lookup
            .with_cached_query::<V, _>(&key_hash, &cache_key, |maybe_cached| {
                maybe_cached.map(|cached| {
                    if cached.stale() {
                        (None, false)
                    } else {
                        (Some(cached.value_maybe_stale.value().clone()), false)
                    }
                })
            })
            .unwrap_or((None, true));
        if let Some(cached) = maybe_cached {
            cached
        } else {
            self.scope_lookup
                .cached_or_fetch_inner::<K, V, _, _>(
                    self.options(),
                    self.scope_lookup,
                    key,
                    cache_key,
                    fetcher,
                    None,
                    false,
                    |v| v.clone(),
                    query_options,
                    None,
                    loading_first_time,
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
        K: Clone + Hash + 'static,
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
    pub fn set_query_local<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V>,
        key: impl Borrow<K>,
        new_value: V,
    ) where
        K: Clone + Hash + 'static,
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
        K: Clone + Hash + 'static,
        V: 'static,
    {
        let key_hash = KeyHash::new(key);
        self.scope_lookup
            .with_cached_scope_mut::<V, _>(cache_key, true, |maybe_scope| {
                let scope = maybe_scope.expect("provided a default");

                // If we're updating an existing, make sure to update
                let maybe_cached = if new_value.is_local() {
                    scope.get_mut_local_only(&key_hash)
                } else {
                    scope.get_mut_threadsafe_only(&key_hash)
                };
                if let Some(cached) = maybe_cached {
                    cached.set_value(new_value);
                    // To update all existing resources:
                    cached.buster.set(new_buster_id());
                } else {
                    let query = Query::new(
                        self.options(),
                        self.scope_lookup,
                        cache_key,
                        &key_hash,
                        new_value,
                        ArcRwSignal::new(new_buster_id()),
                        query_options,
                        None,
                    );
                    scope.insert(key_hash, query);
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
        K: Clone + Hash + 'static,
        V: 'static,
    {
        let key = key.borrow();
        let key_hash = KeyHash::new(key);

        let mut modifier_holder = Some(modifier);

        let maybe_return_value = self.scope_lookup.with_cached_scope_mut::<V, _>(
            query_scope.cache_key(),
            false,
            |maybe_scope| {
                if let Some(scope) = maybe_scope {
                    if let Some((key, cached)) = scope.remove_entry(&key_hash) {
                        let mut value = cached.value_maybe_stale.into_value_maybe_local();
                        let return_value = modifier_holder
                            .take()
                            .expect("Should never be used more than once.")(
                            Some(value.value_mut()),
                        );
                        let query = Query::new(
                            self.options(),
                            self.scope_lookup,
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
    #[track_caller]
    pub fn get_cached_query<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V>,
        key: impl Borrow<K>,
    ) -> Option<V>
    where
        K: Hash + 'static,
        V: Clone + 'static,
    {
        self.scope_lookup.with_cached_query::<V, _>(
            &KeyHash::new(key.borrow()),
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
        K: Hash + 'static,
        V: 'static,
    {
        self.scope_lookup.with_cached_query::<V, _>(
            &KeyHash::new(key.borrow()),
            &query_scope.cache_key(),
            |maybe_cached| maybe_cached.is_some(),
        )
    }

    /// Subscribe to the `is_loading` status of a query.
    /// The keyer function is reactive to changes in `K`.
    ///
    /// This is `true` when the query is in the process of fetching data for the first time.
    /// This is in contrast to `is_fetching`, that is `true` whenever the query is fetching data, including when it's refetching.
    ///
    /// From a resource perspective:
    /// - `is_loading=true`, the resource will be in a pending state until ready and implies `is_fetching=true`
    /// - `is_fetching=true` + `is_loading=false` means the resource is showing previous data, and will update once new data finishes refetching
    /// - `is_fetching=false` means the resource is showing the latest data and implies `is_loading=false`
    #[track_caller]
    pub fn subscribe_is_loading<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V>,
        keyer: impl Fn() -> K + Send + Sync + 'static,
    ) -> Signal<bool>
    where
        K: Hash + Send + Sync + 'static,
        V: 'static,
    {
        self.subscribe_is_loading_arc(query_scope, keyer).into()
    }

    /// Subscribe to the `is_loading` status of a query with a non-threadsafe key.
    /// The keyer function is reactive to changes in `K`.
    ///
    /// This is `true` when the query is in the process of fetching data for the first time.
    /// This is in contrast to `is_fetching`, that is `true` whenever the query is fetching data, including when it's refetching.
    ///
    /// From a resource perspective:
    /// - `is_loading=true`, the resource will be in a pending state until ready and implies `is_fetching=true`
    /// - `is_fetching=true` + `is_loading=false` means the resource is showing previous data, and will update once new data finishes refetching
    /// - `is_fetching=false` means the resource is showing the latest data and implies `is_loading=false`
    #[track_caller]
    pub fn subscribe_is_loading_local<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V>,
        keyer: impl Fn() -> K + 'static,
    ) -> Signal<bool>
    where
        K: Hash + 'static,
        V: 'static,
    {
        self.subscribe_is_loading_arc_local(query_scope, keyer)
            .into()
    }

    /// Subscribe to the `is_loading` status of a query with a non-threadsafe key.
    /// The keyer function is reactive to changes in `K`.
    ///
    /// This is `true` when the query is in the process of fetching data for the first time.
    /// This is in contrast to `is_fetching`, that is `true` whenever the query is fetching data, including when it's refetching.
    ///
    /// From a resource perspective:
    /// - `is_loading=true`, the resource will be in a pending state until ready and implies `is_fetching=true`
    /// - `is_fetching=true` + `is_loading=false` means the resource is showing previous data, and will update once new data finishes refetching
    /// - `is_fetching=false` means the resource is showing the latest data and implies `is_loading=false`
    #[track_caller]
    pub fn subscribe_is_loading_arc_local<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V>,
        keyer: impl Fn() -> K + 'static,
    ) -> ArcSignal<bool>
    where
        K: Hash + 'static,
        V: 'static,
    {
        if cfg!(feature = "ssr") {
            ArcSignal::derive(|| true)
        } else {
            let keyer = SendWrapper::new(keyer);
            self.scope_lookup
                .subscriptions_mut()
                .add_is_loading_subscription(
                    query_scope.cache_key(),
                    ArcSignal::derive(move || KeyHash::new(&keyer())),
                )
        }
    }

    /// Subscribe to the `is_loading` status of a query.
    /// The keyer function is reactive to changes in `K`.
    ///
    /// This is `true` when the query is in the process of fetching data for the first time.
    /// This is in contrast to `is_fetching`, that is `true` whenever the query is fetching data, including when it's refetching.
    ///
    /// From a resource perspective:
    /// - `is_loading=true`, the resource will be in a pending state until ready and implies `is_fetching=true`
    /// - `is_fetching=true` + `is_loading=false` means the resource is showing previous data, and will update once new data finishes refetching
    /// - `is_fetching=false` means the resource is showing the latest data and implies `is_loading=false`
    #[track_caller]
    pub fn subscribe_is_loading_arc<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V>,
        keyer: impl Fn() -> K + Send + Sync + 'static,
    ) -> ArcSignal<bool>
    where
        K: Hash + Send + Sync + 'static,
        V: 'static,
    {
        self.scope_lookup
            .subscriptions_mut()
            .add_is_loading_subscription(
                query_scope.cache_key(),
                ArcSignal::derive(move || KeyHash::new(&keyer())),
            )
    }

    /// Subscribe to the `is_fetching` status of a query.
    /// The keyer function is reactive to changes in `K`.
    ///
    /// This is `true` is `true` whenever the query is fetching data, including when it's refetching.
    /// This is in contrast to `is_loading`, that is `true` when the query is in the process of fetching data for the first time only.
    ///
    /// From a resource perspective:
    /// - `is_loading=true`, the resource will be in a pending state until ready and implies `is_fetching=true`
    /// - `is_fetching=true` + `is_loading=false` means the resource is showing previous data, and will update once new data finishes refetching
    /// - `is_fetching=false` means the resource is showing the latest data and implies `is_loading=false`
    #[track_caller]
    pub fn subscribe_is_fetching<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V>,
        keyer: impl Fn() -> K + Send + Sync + 'static,
    ) -> Signal<bool>
    where
        K: Hash + Send + Sync + 'static,
        V: 'static,
    {
        self.subscribe_is_fetching_arc(query_scope, keyer).into()
    }

    /// Subscribe to the `is_fetching` status of a query with a non-threadsafe key.
    /// The keyer function is reactive to changes in `K`.
    ///
    /// This is `true` is `true` whenever the query is fetching data, including when it's refetching.
    /// This is in contrast to `is_loading`, that is `true` when the query is in the process of fetching data for the first time only.
    ///
    /// From a resource perspective:
    /// - `is_loading=true`, the resource will be in a pending state until ready and implies `is_fetching=true`
    /// - `is_fetching=true` + `is_loading=false` means the resource is showing previous data, and will update once new data finishes refetching
    /// - `is_fetching=false` means the resource is showing the latest data and implies `is_loading=false`
    #[track_caller]
    pub fn subscribe_is_fetching_local<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V>,
        keyer: impl Fn() -> K + 'static,
    ) -> Signal<bool>
    where
        K: Hash + 'static,
        V: 'static,
    {
        self.subscribe_is_fetching_arc_local(query_scope, keyer)
            .into()
    }

    /// Subscribe to the `is_fetching` status of a query with a non-threadsafe key.
    /// The keyer function is reactive to changes in `K`.
    ///
    /// This is `true` is `true` whenever the query is fetching data, including when it's refetching.
    /// This is in contrast to `is_loading`, that is `true` when the query is in the process of fetching data for the first time only.
    ///
    /// From a resource perspective:
    /// - `is_loading=true`, the resource will be in a pending state until ready and implies `is_fetching=true`
    /// - `is_fetching=true` + `is_loading=false` means the resource is showing previous data, and will update once new data finishes refetching
    /// - `is_fetching=false` means the resource is showing the latest data and implies `is_loading=false`
    #[track_caller]
    pub fn subscribe_is_fetching_arc_local<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V>,
        keyer: impl Fn() -> K + 'static,
    ) -> ArcSignal<bool>
    where
        K: Hash + 'static,
        V: 'static,
    {
        if cfg!(feature = "ssr") {
            ArcSignal::derive(|| true)
        } else {
            let keyer = SendWrapper::new(keyer);
            self.scope_lookup
                .subscriptions_mut()
                .add_is_fetching_subscription(
                    query_scope.cache_key(),
                    ArcSignal::derive(move || KeyHash::new(&keyer())),
                )
        }
    }

    /// Subscribe to the `is_fetching` status of a query.
    /// The keyer function is reactive to changes in `K`.
    ///
    /// This is `true` is `true` whenever the query is fetching data, including when it's refetching.
    /// This is in contrast to `is_loading`, that is `true` when the query is in the process of fetching data for the first time only.
    ///
    /// From a resource perspective:
    /// - `is_loading=true`, the resource will be in a pending state until ready and implies `is_fetching=true`
    /// - `is_fetching=true` + `is_loading=false` means the resource is showing previous data, and will update once new data finishes refetching
    /// - `is_fetching=false` means the resource is showing the latest data and implies `is_loading=false`
    #[track_caller]
    pub fn subscribe_is_fetching_arc<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V>,
        keyer: impl Fn() -> K + Send + Sync + 'static,
    ) -> ArcSignal<bool>
    where
        K: Hash + Send + Sync + 'static,
        V: 'static,
    {
        let mut guard = self.scope_lookup.subscriptions_mut();
        guard.add_is_fetching_subscription(
            query_scope.cache_key(),
            ArcSignal::derive(move || KeyHash::new(&keyer())),
        )
    }

    /// Mark a query as stale.
    ///
    /// Any active resources will refetch in the background, replacing them when ready.
    #[track_caller]
    pub fn invalidate_query<K, V>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V>,
        key: impl Borrow<K>,
    ) -> bool
    where
        K: Hash + 'static,
        V: 'static,
    {
        let cleared = self.invalidate_queries(query_scope, std::iter::once(key));
        !cleared.is_empty()
    }

    /// Mark multiple queries of a specific type as stale.
    ///
    /// Any active resources will refetch in the background, replacing them when ready.
    #[track_caller]
    pub fn invalidate_queries<K, V, KRef>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V>,
        keys: impl IntoIterator<Item = KRef>,
    ) -> Vec<KRef>
    where
        K: Hash + 'static,
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
        K: Hash + 'static,
        V: 'static,
        KRef: Borrow<K>,
    {
        self.scope_lookup
            .with_cached_scope_mut::<V, _>(cache_key, false, |maybe_scope| {
                let mut invalidated = vec![];
                if let Some(scope) = maybe_scope {
                    for key in keys.into_iter() {
                        if let Some(cached) = scope.get_mut(&KeyHash::new(key.borrow())) {
                            cached.invalidate();
                            invalidated.push(key);
                        }
                    }
                }
                invalidated
            })
    }

    /// Mark all queries of a specific type as stale.
    ///
    /// Any active resources will refetch in the background, replacing them when ready.
    #[track_caller]
    pub fn invalidate_query_type<K, V>(&self, query_scope: impl QueryScopeLocalTrait<K, V>)
    where
        K: Hash + 'static,
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

    /// Mark all queries as stale.
    ///
    /// Any active resources will refetch in the background, replacing them when ready.
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
    ///
    /// All active resources will instantly revert to pending until the new query finishes refetching.
    ///
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

    #[cfg(test)]
    pub(crate) fn size(&self) -> usize {
        self.scope_lookup
            .scopes()
            .values()
            .map(|scope| scope.size())
            .sum()
    }

    #[cfg(test)]
    pub(crate) fn subscriber_count(&self) -> usize {
        self.scope_lookup.subscriptions_mut().count()
    }
}
