#![allow(ungated_async_fn_track_caller)] // Want it to auto-turn on when stable

use std::{
    borrow::Borrow,
    collections::HashMap,
    fmt::Debug,
    hash::Hash,
    sync::{Arc, LazyLock, atomic::AtomicBool},
};

use chrono::{DateTime, Utc};
use codee::{Decoder, Encoder};
use futures::pin_mut;
use leptos::{
    prelude::{
        ArcRwSignal, ArcSignal, Effect, Get, GetUntracked, LocalStorage, Owner, Read,
        ReadUntracked, Set, Signal, Track, provide_context,
    },
    server::{
        ArcLocalResource, ArcResource, FromEncodedStr, IntoEncodedString, LocalResource, Resource,
    },
};
use send_wrapper::SendWrapper;

use crate::{
    ArcLocalSignal, QueryOptions,
    cache::{CachedOrFetchCbInputVariant, CachedOrFetchCbOutput},
    cache_scope::{QueryAbortReason, QueryOrPending},
    debug_if_devtools_enabled::DebugIfDevtoolsEnabled,
    maybe_local::MaybeLocal,
    query::Query,
    query_maybe_key::QueryMaybeKey,
    query_scope::{
        QueryScopeInfo, QueryScopeLocalTrait, QueryScopeQueryInfo, QueryScopeTrait, ScopeCacheKey,
    },
    resource_drop_guard::ResourceDropGuard,
    utils::{KeyHash, OwnerChain, ResetInvalidated, new_buster_id, new_resource_id},
};

use super::cache::ScopeLookup;

#[cfg(not(feature = "rkyv"))]
pub(crate) type DefaultCodec = codee::string::JsonSerdeCodec;

#[cfg(feature = "rkyv")]
pub(crate) type DefaultCodec = codee::binary::RkyvCodec;

task_local::task_local! {
    pub(crate) static ASYNC_TRACK_UPDATE_MARKER: Arc<AtomicBool>;
}

std::thread_local! {
    pub(crate) static SYNC_TRACK_UPDATE_MARKER: AtomicBool = const { AtomicBool::new(true) };
}

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
pub struct QueryClient<Codec = DefaultCodec> {
    pub(crate) scope_lookup: ScopeLookup,
    options: QueryOptions,
    created_at: DateTime<Utc>,
    _ser: std::marker::PhantomData<SendWrapper<Codec>>,
}

impl<Codec> Debug for QueryClient<Codec> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryClient")
            .field("scope_lookup", &self.scope_lookup)
            .field("options", &self.options)
            .field("codec", &std::any::type_name::<Codec>())
            .finish()
    }
}

impl<Codec: 'static> Clone for QueryClient<Codec> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Codec: 'static> Copy for QueryClient<Codec> {}

impl Default for QueryClient<DefaultCodec> {
    fn default() -> Self {
        Self::new()
    }
}

impl QueryClient<DefaultCodec> {
    /// Creates a new [`QueryClient`] with the default codec: [`codee::string::JsonSerdeCodec`].
    ///
    /// Call [`QueryClient::set_codec()`] to set a different codec.
    ///
    /// Call [`QueryClient::with_options()`] to set non-default options.
    #[track_caller]
    pub fn new() -> Self {
        Self {
            scope_lookup: ScopeLookup::new(),
            options: QueryOptions::default(),
            created_at: Utc::now(),
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
            created_at: self.created_at,
            _ser: std::marker::PhantomData,
        }
    }

    /// Set non-default options to apply to all queries.
    ///
    /// These options will be combined with any options for a specific query scope.
    #[track_caller]
    pub fn with_options(mut self, options: QueryOptions) -> Self {
        self.options = options;
        self
    }

    /// Read the base [`QueryOptions`] for this [`QueryClient`].
    ///
    /// These will be combined with any options for a specific query scope.
    #[track_caller]
    pub fn options(&self) -> QueryOptions {
        self.options
    }

    /// Provide a signal to globally enable/disable auto refetching of queries
    /// that have active resources and have set [`QueryOptions::with_refetch_interval`].
    ///
    /// Whilst this signal returns `false`, refetches will be skipped.
    #[track_caller]
    pub fn with_refetch_enabled_toggle(self, refetch_enabled: impl Into<ArcSignal<bool>>) -> Self {
        self.scope_lookup.scopes_mut().refetch_enabled = Some(refetch_enabled.into());
        self
    }

    /// If set, access the signal that globally enables/disables auto refetching of queries.
    #[track_caller]
    pub fn refetch_enabled(&self) -> Option<ArcSignal<bool>> {
        self.scope_lookup.scopes().refetch_enabled.clone()
    }

    /// Query with [`LocalResource`]. Local resouces only load data on the client, so can be used with non-threadsafe/serializable data.
    ///
    /// If a cached value exists but is stale, the cached value will be initially used, then refreshed in the background, updating once the new value is ready.
    #[track_caller]
    pub fn local_resource<K, MaybeKey, V, M>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V, M> + 'static,
        keyer: impl Fn() -> MaybeKey + 'static,
    ) -> LocalResource<MaybeKey::MappedValue>
    where
        K: DebugIfDevtoolsEnabled + Hash + Clone + 'static,
        MaybeKey: QueryMaybeKey<K, V>,
        MaybeKey::MappedValue: DebugIfDevtoolsEnabled + Clone + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        self.arc_local_resource(query_scope, keyer).into()
    }

    /// Query with [`ArcLocalResource`]. Local resouces only load data on the client, so can be used with non-threadsafe/serializable data.
    ///
    /// If a cached value exists but is stale, the cached value will be initially used, then refreshed in the background, updating once the new value is ready.
    #[track_caller]
    pub fn arc_local_resource<K, MaybeKey, V, M>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V, M> + 'static,
        keyer: impl Fn() -> MaybeKey + 'static,
    ) -> ArcLocalResource<MaybeKey::MappedValue>
    where
        K: DebugIfDevtoolsEnabled + Hash + Clone + 'static,
        MaybeKey: QueryMaybeKey<K, V>,
        MaybeKey::MappedValue: DebugIfDevtoolsEnabled + Clone + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        let client = *self;
        let client_options = self.options();
        let scope_lookup = self.scope_lookup;
        let cache_key = query_scope.cache_key();
        let query_scope_info = QueryScopeInfo::new_local(&query_scope);
        let query_scope = Arc::new(query_scope);
        let query_options = query_scope.options();
        let resource_id = new_resource_id();

        // To call .mark_resource_dropped() when the resource is dropped:
        let drop_guard = ResourceDropGuard::<K, V>::new(self.scope_lookup, resource_id, cache_key);

        ArcLocalResource::new({
            move || {
                let query_scope = query_scope.clone();
                let query_scope_info = query_scope_info.clone();
                let maybe_key = keyer().into_maybe_key();
                let drop_guard = drop_guard.clone();
                if let Some(key) = maybe_key.as_ref() {
                    drop_guard.set_active_key(KeyHash::new(key));
                }
                // Note: cannot hoist outside of resource,
                // it prevents the resource itself dropping when the owner is held by the resource itself,
                // here each instance only lasts as long as the query.
                let owner_chain = OwnerChain::new(Owner::current());
                async move {
                    if let Some(key) = maybe_key {
                        let query_scope_query_info =
                            || QueryScopeQueryInfo::new_local(&query_scope, &key);
                        let value = scope_lookup
                            .cached_or_fetch(
                                client_options,
                                query_options,
                                None,
                                &query_scope_info,
                                query_scope_query_info,
                                &key,
                                {
                                    let query_scope = query_scope.clone();
                                    async move |key| {
                                        MaybeLocal::new_local(query_scope.query(key).await)
                                    }
                                },
                                |info| {
                                    info.cached.buster.track();
                                    info.cached.mark_resource_active(resource_id);
                                    match info.variant {
                                        CachedOrFetchCbInputVariant::CachedUntouched => {
                                            // If stale refetch in the background with the prefetch() function, which'll recognise it's stale, refetch it and invalidate busters:
                                            if cfg!(any(test, not(feature = "ssr")))
                                                && info.cached.stale()
                                            {
                                                let key = key.clone();
                                                let query_scope = query_scope.clone();
                                                let owner_chain = owner_chain.clone();
                                                // Just adding the SendWrapper and using spawn() rather than spawn_local() to fix tests:
                                                leptos::task::spawn(SendWrapper::new(async move {
                                                    client
                                                        .prefetch_inner(
                                                            QueryScopeInfo::new_local(&query_scope),
                                                            || {
                                                                QueryScopeQueryInfo::new_local(
                                                                    &query_scope,
                                                                    &key,
                                                                )
                                                            },
                                                            async |key| {
                                                                MaybeLocal::new_local(
                                                                    query_scope.query(key).await,
                                                                )
                                                            },
                                                            key.borrow(),
                                                            || {
                                                                MaybeLocal::new_local(
                                                                    key.borrow().clone(),
                                                                )
                                                            },
                                                            &owner_chain,
                                                        )
                                                        .await;
                                                }));
                                            }
                                        }
                                        CachedOrFetchCbInputVariant::Fresh => {}
                                        CachedOrFetchCbInputVariant::CachedUpdated => {
                                            panic!("Didn't direct inner to refetch here. (bug)")
                                        }
                                    }
                                    CachedOrFetchCbOutput::Return(
                                        // WONTPANIC: cached_or_fetch will only output values that are safe on this thread:
                                        info.cached.value_maybe_stale().value_may_panic().clone(),
                                    )
                                },
                                None,
                                || MaybeLocal::new_local(key.clone()),
                                &owner_chain,
                            )
                            .await;
                        MaybeKey::prepare_mapped_value(Some(value))
                    } else {
                        MaybeKey::prepare_mapped_value(None)
                    }
                }
            }
        })
    }

    /// Query with [`Resource`].
    ///
    /// Resources must be serializable to potentially load in `ssr` and stream to the client.
    ///
    /// Resources must be `Send` and `Sync` to be multithreaded in ssr.
    ///
    /// If a cached value exists but is stale, the cached value will be initially used, then refreshed in the background, updating once the new value is ready.
    #[track_caller]
    pub fn resource<K, MaybeKey, V, M>(
        &self,
        query_scope: impl QueryScopeTrait<K, V, M> + Send + Sync + 'static,
        keyer: impl Fn() -> MaybeKey + Send + Sync + 'static,
    ) -> Resource<MaybeKey::MappedValue, Codec>
    where
        K: DebugIfDevtoolsEnabled + PartialEq + Hash + Clone + Send + Sync + 'static,
        MaybeKey: QueryMaybeKey<K, V>,
        MaybeKey::MappedValue: Clone + Send + Sync + 'static,
        V: DebugIfDevtoolsEnabled + Clone + Send + Sync + 'static,
        Codec: Encoder<MaybeKey::MappedValue> + Decoder<MaybeKey::MappedValue>,
        <Codec as Encoder<MaybeKey::MappedValue>>::Error: Debug,
        <Codec as Decoder<MaybeKey::MappedValue>>::Error: Debug,
        <<Codec as Decoder<MaybeKey::MappedValue>>::Encoded as FromEncodedStr>::DecodingError:
            Debug,
        <Codec as Encoder<MaybeKey::MappedValue>>::Encoded: IntoEncodedString,
        <Codec as Decoder<MaybeKey::MappedValue>>::Encoded: FromEncodedStr,
    {
        self.arc_resource_with_options(query_scope, keyer, false)
            .into()
    }

    /// Query with a blocking [`Resource`].
    ///
    /// Resources must be serializable to potentially load in `ssr` and stream to the client.
    ///
    /// Resources must be `Send` and `Sync` to be multithreaded in ssr.
    ///
    /// If a cached value exists but is stale, the cached value will be initially used, then refreshed in the background, updating once the new value is ready.
    #[track_caller]
    pub fn resource_blocking<K, MaybeKey, V, M>(
        &self,
        query_scope: impl QueryScopeTrait<K, V, M> + Send + Sync + 'static,
        keyer: impl Fn() -> MaybeKey + Send + Sync + 'static,
    ) -> Resource<MaybeKey::MappedValue, Codec>
    where
        K: DebugIfDevtoolsEnabled + PartialEq + Hash + Clone + Send + Sync + 'static,
        MaybeKey: QueryMaybeKey<K, V>,
        MaybeKey::MappedValue: Clone + Send + Sync + 'static,
        V: DebugIfDevtoolsEnabled + Clone + Send + Sync + 'static,
        Codec: Encoder<MaybeKey::MappedValue> + Decoder<MaybeKey::MappedValue>,
        <Codec as Encoder<MaybeKey::MappedValue>>::Error: Debug,
        <Codec as Decoder<MaybeKey::MappedValue>>::Error: Debug,
        <<Codec as Decoder<MaybeKey::MappedValue>>::Encoded as FromEncodedStr>::DecodingError:
            Debug,
        <Codec as Encoder<MaybeKey::MappedValue>>::Encoded: IntoEncodedString,
        <Codec as Decoder<MaybeKey::MappedValue>>::Encoded: FromEncodedStr,
    {
        self.arc_resource_with_options(query_scope, keyer, true)
            .into()
    }

    /// Query with [`ArcResource`].
    ///
    /// Resources must be serializable to potentially load in `ssr` and stream to the client.
    ///
    /// Resources must be `Send` and `Sync` to be multithreaded in ssr.
    ///
    /// If a cached value exists but is stale, the cached value will be initially used, then refreshed in the background, updating once the new value is ready.
    #[track_caller]
    pub fn arc_resource<K, MaybeKey, V, M>(
        &self,
        query_scope: impl QueryScopeTrait<K, V, M> + Send + Sync + 'static,
        keyer: impl Fn() -> MaybeKey + Send + Sync + 'static,
    ) -> ArcResource<MaybeKey::MappedValue, Codec>
    where
        K: DebugIfDevtoolsEnabled + PartialEq + Hash + Clone + Send + Sync + 'static,
        MaybeKey: QueryMaybeKey<K, V>,
        MaybeKey::MappedValue: Clone + Send + Sync + 'static,
        V: DebugIfDevtoolsEnabled + Clone + Send + Sync + 'static,
        Codec: Encoder<MaybeKey::MappedValue> + Decoder<MaybeKey::MappedValue>,
        <Codec as Encoder<MaybeKey::MappedValue>>::Error: Debug,
        <Codec as Decoder<MaybeKey::MappedValue>>::Error: Debug,
        <<Codec as Decoder<MaybeKey::MappedValue>>::Encoded as FromEncodedStr>::DecodingError:
            Debug,
        <Codec as Encoder<MaybeKey::MappedValue>>::Encoded: IntoEncodedString,
        <Codec as Decoder<MaybeKey::MappedValue>>::Encoded: FromEncodedStr,
    {
        self.arc_resource_with_options(query_scope, keyer, false)
    }

    /// Query with a blocking [`ArcResource`].
    ///
    /// Resources must be serializable to potentially load in `ssr` and stream to the client.
    ///
    /// Resources must be `Send` and `Sync` to be multithreaded in ssr.
    ///
    /// If a cached value exists but is stale, the cached value will be initially used, then refreshed in the background, updating once the new value is ready.
    #[track_caller]
    pub fn arc_resource_blocking<K, MaybeKey, V, M>(
        &self,
        query_scope: impl QueryScopeTrait<K, V, M> + Send + Sync + 'static,
        keyer: impl Fn() -> MaybeKey + Send + Sync + 'static,
    ) -> ArcResource<MaybeKey::MappedValue, Codec>
    where
        K: DebugIfDevtoolsEnabled + PartialEq + Hash + Clone + Send + Sync + 'static,
        MaybeKey: QueryMaybeKey<K, V>,
        MaybeKey::MappedValue: Clone + Send + Sync + 'static,
        V: DebugIfDevtoolsEnabled + Clone + Send + Sync + 'static,
        Codec: Encoder<MaybeKey::MappedValue> + Decoder<MaybeKey::MappedValue>,
        <Codec as Encoder<MaybeKey::MappedValue>>::Error: Debug,
        <Codec as Decoder<MaybeKey::MappedValue>>::Error: Debug,
        <<Codec as Decoder<MaybeKey::MappedValue>>::Encoded as FromEncodedStr>::DecodingError:
            Debug,
        <Codec as Encoder<MaybeKey::MappedValue>>::Encoded: IntoEncodedString,
        <Codec as Decoder<MaybeKey::MappedValue>>::Encoded: FromEncodedStr,
    {
        self.arc_resource_with_options(query_scope, keyer, true)
    }

    #[track_caller]
    fn arc_resource_with_options<K, MaybeKey, V, M>(
        &self,
        query_scope: impl QueryScopeTrait<K, V, M> + Send + Sync + 'static,
        keyer: impl Fn() -> MaybeKey + Send + Sync + 'static,
        blocking: bool,
    ) -> ArcResource<MaybeKey::MappedValue, Codec>
    where
        K: DebugIfDevtoolsEnabled + PartialEq + Hash + Clone + Send + Sync + 'static,
        MaybeKey: QueryMaybeKey<K, V>,
        MaybeKey::MappedValue: Clone + Send + Sync + 'static,
        V: DebugIfDevtoolsEnabled + Clone + Send + Sync + 'static,
        Codec: Encoder<MaybeKey::MappedValue> + Decoder<MaybeKey::MappedValue>,
        <Codec as Encoder<MaybeKey::MappedValue>>::Error: Debug,
        <Codec as Decoder<MaybeKey::MappedValue>>::Error: Debug,
        <<Codec as Decoder<MaybeKey::MappedValue>>::Encoded as FromEncodedStr>::DecodingError:
            Debug,
        <Codec as Encoder<MaybeKey::MappedValue>>::Encoded: IntoEncodedString,
        <Codec as Decoder<MaybeKey::MappedValue>>::Encoded: FromEncodedStr,
    {
        let client = *self;
        let client_options = self.options();
        let cache_key = query_scope.cache_key();
        let query_scope_info = QueryScopeInfo::new(&query_scope);
        let query_scope = Arc::new(query_scope);
        let scope_lookup = self.scope_lookup;
        let query_options = query_scope.options();

        let buster_if_uncached = ArcRwSignal::new(new_buster_id());
        let resource_id = new_resource_id();

        // To call .mark_resource_dropped() when the resource is dropped:
        let drop_guard = ResourceDropGuard::<K, V>::new(self.scope_lookup, resource_id, cache_key);

        let keyer = Arc::new(keyer);
        let resource = ArcResource::new_with_options(
            {
                let buster_if_uncached = buster_if_uncached.clone();
                let drop_guard = drop_guard.clone();
                let keyer = keyer.clone();
                move || {
                    if let Some(key) = keyer().into_maybe_key() {
                        let key_hash = KeyHash::new(&key);
                        drop_guard.set_active_key(key_hash);
                        scope_lookup.with_cached_query::<K, V, _>(
                            &key_hash,
                            &cache_key,
                            |maybe_cached| {
                                if let Some(cached) = maybe_cached {
                                    // Buster must be returned for it to be tracked.
                                    (Some(key.clone()), cached.buster.get())
                                } else {
                                    // Buster must be returned for it to be tracked.
                                    (Some(key.clone()), buster_if_uncached.get())
                                }
                            },
                        )
                    } else {
                        (None, buster_if_uncached.get())
                    }
                }
            },
            {
                let buster_if_uncached = buster_if_uncached.clone();
                let query_scope_info = query_scope_info.clone();
                let query_scope = query_scope.clone();
                move |(maybe_key, last_used_buster)| {
                    let query_scope = query_scope.clone();
                    let query_scope_info = query_scope_info.clone();
                    let buster_if_uncached = buster_if_uncached.clone();
                    let _drop_guard = drop_guard.clone(); // Want the guard around everywhere until the resource is dropped.
                    // Note: cannot hoist outside of resource,
                    // it prevents the resource itself dropping when the owner is held by the resource itself,
                    // here each instance only lasts as long as the query.
                    let owner_chain = OwnerChain::new(Owner::current());
                    async move {
                        if let Some(key) = maybe_key {
                            let query_scope_query_info =
                                || QueryScopeQueryInfo::new(&query_scope, &key);
                            let value = scope_lookup
                                .cached_or_fetch(
                                    client_options,
                                    query_options,
                                    Some(buster_if_uncached.clone()),
                                    &query_scope_info,
                                    query_scope_query_info,
                                    &key,
                                    {
                                        let query_scope = query_scope.clone();
                                        async move |key| {
                                            MaybeLocal::new(query_scope.query(key).await)
                                        }
                                    },
                                    |info| {
                                        info.cached.mark_resource_active(resource_id);
                                        match info.variant {
                                            CachedOrFetchCbInputVariant::CachedUntouched => {
                                                // If stale refetch in the background with the prefetch() function, which'll recognise it's stale, refetch it and invalidate busters:
                                                if cfg!(any(test, not(feature = "ssr")))
                                                    && info.cached.stale()
                                                {
                                                    let key = key.clone();
                                                    let query_scope = query_scope.clone();
                                                    let owner_chain = owner_chain.clone();

                                                    leptos::task::spawn(async move {
                                                        client
                                                            .prefetch_inner(
                                                                QueryScopeInfo::new(&query_scope),
                                                                || {
                                                                    QueryScopeQueryInfo::new(
                                                                        &query_scope,
                                                                        &key,
                                                                    )
                                                                },
                                                                {
                                                                    let query_scope =
                                                                        query_scope.clone();
                                                                    async move |key| {
                                                                        MaybeLocal::new(
                                                                            query_scope
                                                                                .query(key)
                                                                                .await,
                                                                        )
                                                                    }
                                                                },
                                                                key.borrow(),
                                                                || {
                                                                    MaybeLocal::new(
                                                                        key.borrow().clone(),
                                                                    )
                                                                },
                                                                &owner_chain,
                                                            )
                                                            .await;
                                                    });
                                                }

                                                // Handle edge case where the key function saw it was uncached, so entered here,
                                                // but when the cached_or_fetch was acquiring the fetch mutex, someone else cached it,
                                                // meaning our key function won't be reactive correctly unless we invalidate it now:
                                                if last_used_buster
                                                    != info.cached.buster.get_untracked()
                                                {
                                                    buster_if_uncached
                                                        .set(info.cached.buster.get_untracked());
                                                }
                                            }
                                            CachedOrFetchCbInputVariant::Fresh => {}
                                            CachedOrFetchCbInputVariant::CachedUpdated => {
                                                panic!("Didn't direct inner to refetch here. (bug)")
                                            }
                                        }
                                        CachedOrFetchCbOutput::Return(
                                            // WONTPANIC: cached_or_fetch will only output values that are safe on this thread:
                                            info.cached
                                                .value_maybe_stale()
                                                .value_may_panic()
                                                .clone(),
                                        )
                                    },
                                    None,
                                    || MaybeLocal::new(key.clone()),
                                    &owner_chain,
                                )
                                .await;
                            MaybeKey::prepare_mapped_value(Some(value))
                        } else {
                            MaybeKey::prepare_mapped_value(None)
                        }
                    }
                }
            },
            blocking,
        );

        // On the client, want to repopulate the frontend cache, so should write resources to the cache here if they don't exist.
        // It would be better if in here we could check if the resource was started on the backend/streamed, saves doing most of this if already a frontend resource.
        let effect = {
            let resource = resource.clone();
            let buster_if_uncached = buster_if_uncached.clone();
            let client_created_at = self.created_at;
            // Converting to Arc because the tests like the client get dropped even though this persists:
            move |complete: Option<Option<()>>| {
                if let Some(Some(())) = complete {
                    return Some(());
                }
                if let Some(val) = resource.read().as_ref() {
                    if MaybeKey::mapped_value_is_some(val) {
                        scope_lookup.with_cached_scope_mut::<K, V, _, _>(
                            &mut scope_lookup.scopes_mut(),
                            &query_scope_info,
                            true,
                            |_| {},
                            |maybe_scope, _| {
                                let scope = maybe_scope.expect("provided a default");
                                if let Some(key) = keyer().into_maybe_key() {
                                    let query_scope_query_info = || QueryScopeQueryInfo::new(&query_scope, &key);
                                    let key_hash = KeyHash::new(&key);

                                    // Had a bug in tests where:
                                    // calling client.resource() multiple times
                                    // previous impl would only run the effect once per client.resource(), 
                                    // but would still run again on subsequent client.resource() with the same key value
                                    // - client.resource()
                                    // - invalidate, starts loading in background
                                    // - client.resource()
                                    // - effect runs again, sees resource has value, goes to set it, fires QueryAbortReason::SsrStreamedValueOverride which cancels the actual new query which is loading to replace the invalidated one
                                    // So have to have a client wide protector, to make sure the effect really only runs once on the client, even if resource() called multiple times.
                                    if (cfg!(test) || cfg!(not(feature = "ssr"))) && 
                                        // Preventing memory leak, issue is only for first render:
                                        (Utc::now() - client_created_at).num_seconds() < 10
                                    {
                                        use parking_lot::Mutex;

                                        static ALREADY_SEEN: LazyLock<Mutex<HashMap<(u64, ScopeCacheKey, KeyHash), ()>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

                                        let key = (scope_lookup.scope_id, query_scope_info.cache_key, key_hash);
                                        let mut guard = ALREADY_SEEN.lock();
                                        if guard.contains_key(&key) {
                                            return;
                                        }
                                        guard.insert(key, ());
                                    }

                                    let mut was_pending = false;
                                    // Protect against race condition: cancel any frontend query that started already if there was a client side 
                                    // local resource or prefetch/fetch_query etc that started on the initial client-side ticks:
                                    if let Some(QueryOrPending::Pending { query_abort_tx, .. }) = scope.get_mut_include_pending(&key_hash)
                                        && let Some(tx) = query_abort_tx.take() {
                                            tx.send(QueryAbortReason::SsrStreamedValueOverride).unwrap();
                                            was_pending = true;
                                        }

                                    if was_pending || !scope.contains_key(&key_hash) {
                                        let query = Query::new(
                                            client_options,
                                            scope_lookup,
                                            &query_scope_info,
                                            query_scope_query_info(),
                                            key_hash,
                                            MaybeLocal::new(key),
                                            MaybeLocal::new(MaybeKey::mapped_to_maybe_value(val.clone()).expect("Just checked MaybeKey::mapped_value_is_some() is true")),
                                            buster_if_uncached.clone(),
                                            query_options,
                                            None,
                                            #[cfg(any(
                                                all(debug_assertions, feature = "devtools"),
                                                feature = "devtools-always"
                                            ))]
                                            crate::events::Event::new(
                                                crate::events::EventVariant::StreamedFromServer,
                                            ),
                                        );
                                        scope.insert(key_hash, query);
                                    }
                                    scope
                                        .get(&key_hash)
                                        .expect("Just inserted")
                                        .mark_resource_active(resource_id)
                                }
                            },
                        );
                    }
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

    /// Prevent reactive updates being triggered from the current updater callback fn.
    ///
    /// Call [`QueryClient::untrack_update_query`] in the callback of [`QueryClient::update_query`]
    /// to prevent resources being updated after the callback completes.
    ///
    /// This is really useful when e.g:
    /// - you know nothing changes after reading the existing value in the callback
    /// - you don't want resources/subs to react and rerender, given nothings changed.
    ///
    /// This function is a noop outside the callbacks of:
    /// - [`QueryClient::update_query`]
    /// - [`QueryClient::update_query_async`]
    /// - [`QueryClient::update_query_async_local`]
    pub fn untrack_update_query(&self) {
        // Set markers to false, these will all be reset to true automatically where needed.
        SYNC_TRACK_UPDATE_MARKER.with(|marker| {
            marker.store(false, std::sync::atomic::Ordering::Relaxed);
        });
        // Ignore returned AccessError if didn't exist (not in async ctx):
        let _ = ASYNC_TRACK_UPDATE_MARKER.try_with(|marker| {
            marker.store(false, std::sync::atomic::Ordering::Relaxed);
        });
    }

    /// Prefetch a query and store it in the cache.
    ///
    /// - Entry doesn't exist: fetched and stored in the cache.
    /// - Entry exists but stale: fetched and updated in the cache.
    /// - Entry exists but **not** stale: not refreshed, existing cache item remains.
    ///
    /// If the cached query changes, active resources using the query will be updated.
    #[track_caller]
    pub async fn prefetch_query<K, V, M>(
        &self,
        query_scope: impl QueryScopeTrait<K, V, M>,
        key: impl Borrow<K>,
    ) where
        K: DebugIfDevtoolsEnabled + Clone + Hash + Send + Sync + 'static,
        V: DebugIfDevtoolsEnabled + Clone + Send + Sync + 'static,
    {
        self.prefetch_inner(
            QueryScopeInfo::new(&query_scope),
            || QueryScopeQueryInfo::new(&query_scope, key.borrow()),
            async |key| MaybeLocal::new(query_scope.query(key).await),
            key.borrow(),
            || MaybeLocal::new(key.borrow().clone()),
            &OwnerChain::new(Owner::current()),
        )
        .await
    }

    /// Prefetch a non-threadsafe query and store it in the cache for this thread only.
    ///
    /// - Entry doesn't exist: fetched and stored in the cache.
    /// - Entry exists but stale: fetched and updated in the cache.
    /// - Entry exists but **not** stale: not refreshed, existing cache item remains.
    ///
    /// If the cached query changes, active resources using the query will be updated.
    #[track_caller]
    pub async fn prefetch_query_local<K, V, M>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V, M>,
        key: impl Borrow<K>,
    ) where
        K: DebugIfDevtoolsEnabled + Clone + Hash + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        self.prefetch_inner(
            QueryScopeInfo::new_local(&query_scope),
            || QueryScopeQueryInfo::new_local(&query_scope, key.borrow()),
            async |key| MaybeLocal::new_local(query_scope.query(key).await),
            key.borrow(),
            || MaybeLocal::new_local(key.borrow().clone()),
            &OwnerChain::new(Owner::current()),
        )
        .await
    }

    #[track_caller]
    async fn prefetch_inner<K, V>(
        &self,
        query_scope_info: QueryScopeInfo,
        query_scope_info_for_new_query: impl Fn() -> QueryScopeQueryInfo<K>,
        fetcher: impl AsyncFn(K) -> MaybeLocal<V>,
        key: &K,
        lazy_maybe_local_key: impl FnOnce() -> MaybeLocal<K>,
        owner_chain: &OwnerChain,
    ) where
        K: DebugIfDevtoolsEnabled + Clone + Hash + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        self.scope_lookup
            .cached_or_fetch(
                self.options(),
                query_scope_info.options,
                None,
                &query_scope_info,
                query_scope_info_for_new_query,
                key,
                fetcher,
                |info| {
                    match info.variant {
                        CachedOrFetchCbInputVariant::CachedUntouched => {
                            if info.cached.stale() {
                                return CachedOrFetchCbOutput::Refetch;
                            }
                        }
                        CachedOrFetchCbInputVariant::CachedUpdated => {
                            // Update anything using it:
                            info.cached.buster.set(new_buster_id());
                        }
                        CachedOrFetchCbInputVariant::Fresh => {}
                    }
                    CachedOrFetchCbOutput::Return(())
                },
                None,
                lazy_maybe_local_key,
                owner_chain,
            )
            .await;
    }

    /// Fetch a query, store it in the cache and return it.
    ///
    /// - Entry doesn't exist: fetched and stored in the cache.
    /// - Entry exists but stale: fetched and updated in the cache.
    /// - Entry exists but **not** stale: not refreshed, existing cache item remains.
    ///
    /// If the cached query changes, active resources using the query will be updated.
    ///
    /// Returns the up-to-date cached query.
    #[track_caller]
    pub async fn fetch_query<K, V, M>(
        &self,
        query_scope: impl QueryScopeTrait<K, V, M>,
        key: impl Borrow<K>,
    ) -> V
    where
        K: DebugIfDevtoolsEnabled + Clone + Hash + Send + Sync + 'static,
        V: DebugIfDevtoolsEnabled + Clone + Send + Sync + 'static,
    {
        self.fetch_inner(
            QueryScopeInfo::new(&query_scope),
            || QueryScopeQueryInfo::new(&query_scope, key.borrow()),
            async |key| MaybeLocal::new(query_scope.query(key).await),
            key.borrow(),
            None,
            || MaybeLocal::new(key.borrow().clone()),
            &OwnerChain::new(Owner::current()),
        )
        .await
    }

    /// Fetch a non-threadsafe query, store it in the cache for this thread only and return it.
    ///
    /// - Entry doesn't exist: fetched and stored in the cache.
    /// - Entry exists but stale: fetched and updated in the cache.
    /// - Entry exists but **not** stale: not refreshed, existing cache item remains.
    ///
    /// If the cached query changes, active resources using the query will be updated.
    ///
    /// Returns the up-to-date cached query.
    #[track_caller]
    pub async fn fetch_query_local<K, V, M>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V, M>,
        key: impl Borrow<K>,
    ) -> V
    where
        K: DebugIfDevtoolsEnabled + Clone + Hash + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        self.fetch_inner(
            QueryScopeInfo::new_local(&query_scope),
            || QueryScopeQueryInfo::new_local(&query_scope, key.borrow()),
            async |key| MaybeLocal::new_local(query_scope.query(key).await),
            key.borrow(),
            None,
            || MaybeLocal::new_local(key.borrow().clone()),
            &OwnerChain::new(Owner::current()),
        )
        .await
    }

    #[track_caller]
    async fn fetch_inner<K, V>(
        &self,
        query_scope_info: QueryScopeInfo,
        query_scope_info_for_new_query: impl Fn() -> QueryScopeQueryInfo<K>,
        fetcher: impl AsyncFn(K) -> MaybeLocal<V>,
        key: &K,
        maybe_preheld_fetcher_mutex_guard: Option<&futures::lock::MutexGuard<'_, ()>>,
        lazy_maybe_local_key: impl FnOnce() -> MaybeLocal<K>,
        owner_chain: &OwnerChain,
    ) -> V
    where
        K: DebugIfDevtoolsEnabled + Clone + Hash + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        self.scope_lookup
            .cached_or_fetch(
                self.options(),
                query_scope_info.options,
                None,
                &query_scope_info,
                query_scope_info_for_new_query,
                key,
                fetcher,
                |info| {
                    match info.variant {
                        CachedOrFetchCbInputVariant::CachedUntouched => {
                            if info.cached.stale() {
                                return CachedOrFetchCbOutput::Refetch;
                            }
                        }
                        CachedOrFetchCbInputVariant::CachedUpdated => {
                            // Update anything using it:
                            info.cached.buster.set(new_buster_id());
                        }
                        CachedOrFetchCbInputVariant::Fresh => {}
                    }
                    CachedOrFetchCbOutput::Return(
                        // WONTPANIC: cached_or_fetch will only output values that are safe on this thread:
                        info.cached.value_maybe_stale().value_may_panic().clone(),
                    )
                },
                maybe_preheld_fetcher_mutex_guard,
                lazy_maybe_local_key,
                owner_chain,
            )
            .await
    }

    /// Set the value of a query in the cache.
    /// This cached value will be available from all threads and take priority over any locally cached value for this query.
    ///
    /// Active resources using the query will be updated.
    #[track_caller]
    pub fn set_query<K, V, M>(
        &self,
        query_scope: impl QueryScopeTrait<K, V, M>,
        key: impl Borrow<K>,
        new_value: V,
    ) where
        K: DebugIfDevtoolsEnabled + Clone + Hash + Send + Sync + 'static,
        V: DebugIfDevtoolsEnabled + Clone + Send + Sync + 'static,
    {
        self.set_inner(
            QueryScopeInfo::new(&query_scope),
            || QueryScopeQueryInfo::new(&query_scope, key.borrow()),
            key.borrow(),
            MaybeLocal::new(new_value),
            || MaybeLocal::new(key.borrow().clone()),
            true,
            // like update methods, this does not come from the query function itself,
            // so should not reset the invalidated status:
            ResetInvalidated::NoReset,
        )
    }

    /// Set the value of a non-threadsafe query in the cache for this thread only.
    /// This cached value will only be available from this thread, the cache will return empty, unless a nonlocal value is set, from any other thread.
    ///
    /// Active resources using the query will be updated.
    #[track_caller]
    pub fn set_query_local<K, V, M>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V, M>,
        key: impl Borrow<K>,
        new_value: V,
    ) where
        K: DebugIfDevtoolsEnabled + Clone + Hash + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        self.set_inner::<K, V>(
            QueryScopeInfo::new_local(&query_scope),
            || QueryScopeQueryInfo::new_local(&query_scope, key.borrow()),
            key.borrow(),
            MaybeLocal::new_local(new_value),
            || MaybeLocal::new_local(key.borrow().clone()),
            true,
            // like update methods, this does not come from the query function itself,
            // so should not reset the invalidated status:
            ResetInvalidated::NoReset,
        )
    }

    #[track_caller]
    fn set_inner<K, V>(
        &self,
        query_scope_info: QueryScopeInfo,
        query_scope_info_for_new_query: impl Fn() -> QueryScopeQueryInfo<K>,
        key: &K,
        new_value: MaybeLocal<V>,
        lazy_maybe_local_key: impl FnOnce() -> MaybeLocal<K>,
        track: bool,
        reset_invalidated: ResetInvalidated,
    ) where
        K: DebugIfDevtoolsEnabled + Clone + Hash + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        let key_hash = KeyHash::new(key.borrow());
        self.scope_lookup.with_cached_scope_mut::<K, V, _, _>(
            &mut self.scope_lookup.scopes_mut(),
            &query_scope_info,
            true,
            |_| {},
            |maybe_scope, _| {
                let scope = maybe_scope.expect("provided a default");

                // Make sure to look both caches if threadsafe, and prefer threadsafe:
                let maybe_cached = if !new_value.is_local() {
                    if let Some(threadsafe_existing) = scope.get_mut_threadsafe_only(&key_hash) {
                        Some(threadsafe_existing)
                    } else {
                        scope.get_mut_local_only(&key_hash)
                    }
                } else {
                    scope.get_mut_local_only(&key_hash)
                };

                if let Some(cached) = maybe_cached {
                    cached.set_value(
                        new_value,
                        track,
                        #[cfg(any(
                            all(debug_assertions, feature = "devtools"),
                            feature = "devtools-always"
                        ))]
                        crate::events::Event::new(crate::events::EventVariant::DeclarativeSet),
                        reset_invalidated,
                    );
                } else {
                    let query = Query::new(
                        self.options(),
                        self.scope_lookup,
                        &query_scope_info,
                        query_scope_info_for_new_query(),
                        key_hash,
                        lazy_maybe_local_key(),
                        new_value,
                        ArcRwSignal::new(new_buster_id()),
                        query_scope_info.options,
                        None,
                        #[cfg(any(
                            all(debug_assertions, feature = "devtools"),
                            feature = "devtools-always"
                        ))]
                        crate::events::Event::new(crate::events::EventVariant::DeclarativeSet),
                    );
                    scope.insert(key_hash, query);
                }
            },
        );
    }

    /// Synchronously update the value of a query in the cache with a callback.
    ///
    /// Active resources using the query will be updated.
    ///
    /// The callback takes `Option<&mut V>`, will be None if the value is not available in the cache.
    ///
    /// If you want async and/or always get the value, use [`QueryClient::update_query_async`]/[`QueryClient::update_query_async_local`].
    ///
    /// Returns the output of the callback.
    ///
    /// If you decide you don't want to trigger resources and subscribers, e.g. if you know nothing changed,
    /// call [`QueryClient::untrack_update_query`] in the callback to prevent reactive updates.
    #[track_caller]
    pub fn update_query<K, V, T, M>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V, M>,
        key: impl Borrow<K>,
        modifier: impl FnOnce(Option<&mut V>) -> T,
    ) -> T
    where
        K: DebugIfDevtoolsEnabled + Clone + Hash + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        self.update_query_inner::<K, V, T>(
            &QueryScopeInfo::new_local(&query_scope),
            key.borrow(),
            ResetInvalidated::NoReset,
            modifier,
        )
    }

    #[track_caller]
    fn update_query_inner<K, V, T>(
        &self,
        query_scope_info: &QueryScopeInfo,
        key: &K,
        reset_invalidated: ResetInvalidated,
        modifier: impl FnOnce(Option<&mut V>) -> T,
    ) -> T
    where
        K: DebugIfDevtoolsEnabled + Clone + Hash + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        let key_hash = KeyHash::new(key);
        let mut modifier_holder = Some(modifier);

        let maybe_return_value = self.scope_lookup.with_cached_scope_mut::<K, V, _, _>(
            &mut self.scope_lookup.scopes_mut(),
            query_scope_info,
            false,
            |_| {},
            |maybe_scope, _| {
                if let Some(scope) = maybe_scope
                    && let Some(cached) = scope.get_mut(&key_hash)
                {
                    let modifier = modifier_holder
                        .take()
                        .expect("Should never be used more than once. (bug)");
                    let return_value = cached.update_value(
                        // WONTPANIC: the internals will only supply the value if available from this thread:
                        |value| modifier(Some(value.value_mut_may_panic())),
                        #[cfg(any(
                            all(debug_assertions, feature = "devtools"),
                            feature = "devtools-always"
                        ))]
                        crate::events::Event::new(crate::events::EventVariant::DeclarativeUpdate),
                        reset_invalidated,
                    );
                    return Some(return_value);
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
                .expect("Should never be used more than once. (bug)")(None)
        }
    }

    /// Asynchronously map a threadsafe query in the cache from one value to another.
    ///
    /// Unlike [`QueryClient::update_query`], this will fetch the query first, if it doesn't exist.
    ///
    /// Active resources using the query will be updated.
    ///
    /// Returns the output of the callback.
    ///
    /// If you decide you don't want to trigger resources and subscribers, e.g. if you know nothing changed,
    /// call [`QueryClient::untrack_update_query`] in the callback to prevent reactive updates.
    #[track_caller]
    pub async fn update_query_async<'a, K, V, T, M>(
        &'a self,
        query_scope: impl QueryScopeTrait<K, V, M>,
        key: impl Borrow<K>,
        mapper: impl AsyncFnOnce(&mut V) -> T,
    ) -> T
    where
        K: DebugIfDevtoolsEnabled + Clone + Hash + Send + Sync + 'static,
        V: DebugIfDevtoolsEnabled + Clone + Send + Sync + 'static,
    {
        self.update_query_async_inner(
            QueryScopeInfo::new(&query_scope),
            || QueryScopeQueryInfo::new(&query_scope, key.borrow()),
            async |key| MaybeLocal::new(query_scope.query(key).await),
            key.borrow(),
            mapper,
            MaybeLocal::new,
            || MaybeLocal::new(key.borrow().clone()),
            &OwnerChain::new(Owner::current()),
        )
        .await
    }

    /// Asynchronously map a non-threadsafe query in the cache from one value to another.
    ///
    /// Unlike [`QueryClient::update_query`], this will fetch the query first, if it doesn't exist.
    ///
    /// Active resources using the query will be updated.
    ///
    /// Returns the output of the callback.
    ///
    /// If you decide you don't want to trigger resources and subscribers, e.g. if you know nothing changed,
    /// call [`QueryClient::untrack_update_query`] in the callback to prevent reactive updates.
    #[track_caller]
    pub async fn update_query_async_local<'a, K, V, T, M>(
        &'a self,
        query_scope: impl QueryScopeLocalTrait<K, V, M>,
        key: impl Borrow<K>,
        mapper: impl AsyncFnOnce(&mut V) -> T,
    ) -> T
    where
        K: DebugIfDevtoolsEnabled + Clone + Hash + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        self.update_query_async_inner(
            QueryScopeInfo::new_local(&query_scope),
            || QueryScopeQueryInfo::new_local(&query_scope, key.borrow()),
            async |key| MaybeLocal::new_local(query_scope.query(key).await),
            key.borrow(),
            mapper,
            MaybeLocal::new_local,
            || MaybeLocal::new_local(key.borrow().clone()),
            &OwnerChain::new(Owner::current()),
        )
        .await
    }

    #[track_caller]
    async fn update_query_async_inner<'a, K, V, T>(
        &'a self,
        query_scope_info: QueryScopeInfo,
        query_scope_info_for_new_query: impl Fn() -> QueryScopeQueryInfo<K>,
        fetcher: impl AsyncFn(K) -> MaybeLocal<V>,
        key: &K,
        mapper: impl AsyncFnOnce(&mut V) -> T,
        into_maybe_local: impl FnOnce(V) -> MaybeLocal<V>,
        lazy_maybe_local_key: impl Fn() -> MaybeLocal<K>,
        owner_chain: &OwnerChain,
    ) -> T
    where
        K: DebugIfDevtoolsEnabled + Clone + Hash + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        let key_hash = KeyHash::new(key.borrow());

        // By holding the fetcher mutex from start to finish, prevent the chance of the value being fetched between the fetch async call and the external user mapper async call and the final set().
        let fetcher_mutex = self
            .scope_lookup
            .fetcher_mutex::<K, V>(key_hash, &query_scope_info);
        let fetcher_guard = fetcher_mutex.lock().await;

        let mut new_value = self
            .fetch_inner(
                query_scope_info.clone(),
                &query_scope_info_for_new_query,
                fetcher,
                key.borrow(),
                Some(&fetcher_guard),
                &lazy_maybe_local_key,
                owner_chain,
            )
            .await;

        // The fetch will "turn on" is_fetching during it's lifetime, but we also want it during the mapper function:
        self.scope_lookup
            .with_notify_fetching(query_scope_info.cache_key, key_hash, false, async {
                let track = Arc::new(AtomicBool::new(true));

                // Will monitor for invalidations during the user async fn, which could take a long time:
                let query_abort_rx = self.scope_lookup.prepare_invalidation_channel::<K, V>(
                    &query_scope_info,
                    key_hash,
                    &lazy_maybe_local_key(),
                );

                let result_fut = ASYNC_TRACK_UPDATE_MARKER
                    .scope(track.clone(), async { mapper(&mut new_value).await });

                // Call the user async function, but also monitor for invalidations:
                // Specifically, we care about if .clear() is called.
                // If so, we shouldn't set the value, as it will have been generated with the cleared cached query value.
                // If invalidated should still set, because the value will be deemed "less stale", but won't de-invalidate it.
                // Don't care about QueryAbortReason::SsrStreamedValueOverride
                // as it shouldn't be possible to get to this point before all ssr streaming is done.
                let mut cleared_during_user_fn = false;
                let result = {
                    pin_mut!(result_fut);
                    pin_mut!(query_abort_rx);
                    match futures::future::select(result_fut, query_abort_rx).await {
                        futures::future::Either::Left((result, _query_abort_rx)) => result,
                        futures::future::Either::Right((query_abort_reason, result_fut)) => {
                            if let Ok(QueryAbortReason::Clear) = query_abort_reason {
                                // If the query was cleared, don't set the new value.
                                cleared_during_user_fn = true;
                            }
                            result_fut.await
                        }
                    }
                };

                // If the query was cleared, don't set the new value.
                if !cleared_during_user_fn {
                    // Cover a small edge case:
                    // - value in threadsafe
                    // - .update_query_async_local called
                    // - replacement value .set() called, stored in local cache because no type safety on threadsafe cache
                    // - now have 2 versions in both caches
                    // by trying to update first, means it'll stick with existing cache if it exists,
                    // otherwise doesn't matter and can just be set normally:
                    let mut new_value = Some(new_value);
                    let updated = self.update_query_inner::<K, V, _>(
                        &query_scope_info,
                        key,
                        // fetch_inner would've reset it if needed, then the update itself shouldn't change the value:
                        ResetInvalidated::NoReset,
                        |value| {
                            if let Some(value) = value {
                                // This sync update call itself needs untracking if !track:
                                if !track.load(std::sync::atomic::Ordering::Relaxed) {
                                    self.untrack_update_query();
                                }

                                *value = new_value.take().expect("Should be Some");
                                true
                            } else {
                                false
                            }
                        },
                    );
                    if !updated {
                        self.set_inner::<K, V>(
                            query_scope_info,
                            query_scope_info_for_new_query,
                            key.borrow(),
                            into_maybe_local(
                                new_value
                                    .take()
                                    .expect("Should be Some, should only be here if not updated"),
                            ),
                            lazy_maybe_local_key,
                            track.load(std::sync::atomic::Ordering::Relaxed),
                            // fetch_inner would've reset it if needed, then the update itself shouldn't change the value:
                            ResetInvalidated::NoReset,
                        );
                    }
                }

                result
            })
            .await
    }

    /// Synchronously get a query from the cache, if it exists.
    #[track_caller]
    pub fn get_cached_query<K, V, M>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V, M>,
        key: impl Borrow<K>,
    ) -> Option<V>
    where
        K: DebugIfDevtoolsEnabled + Hash + Clone + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        self.scope_lookup.with_cached_query::<K, V, _>(
            &KeyHash::new(key.borrow()),
            &query_scope.cache_key(),
            |maybe_cached| {
                // WONTPANIC: with_cached_query will only output values that are safe on this thread:
                maybe_cached.map(|cached| cached.value_maybe_stale().value_may_panic().clone())
            },
        )
    }

    /// Synchronously check if a query exists in the cache.
    ///
    /// Returns `true` if the query exists.
    #[track_caller]
    pub fn query_exists<K, V, M>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V, M>,
        key: impl Borrow<K>,
    ) -> bool
    where
        K: DebugIfDevtoolsEnabled + Hash + Clone + 'static,
        V: DebugIfDevtoolsEnabled + 'static,
    {
        let key_hash = KeyHash::new(key.borrow());
        self.scope_lookup.with_cached_query::<K, V, _>(
            &key_hash,
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
    pub fn subscribe_is_loading<K, MaybeKey, V, M>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V, M>,
        keyer: impl Fn() -> MaybeKey + Send + Sync + 'static,
    ) -> Signal<bool>
    where
        K: Hash + Send + Sync + 'static,
        MaybeKey: QueryMaybeKey<K, V>,
        MaybeKey::MappedValue: 'static,
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
    pub fn subscribe_is_loading_local<K, MaybeKey, V, M>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V, M>,
        keyer: impl Fn() -> MaybeKey + 'static,
    ) -> Signal<bool>
    where
        K: Hash + 'static,
        MaybeKey: QueryMaybeKey<K, V>,
        MaybeKey::MappedValue: 'static,
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
    pub fn subscribe_is_loading_arc_local<K, MaybeKey, V, M>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V, M>,
        keyer: impl Fn() -> MaybeKey + 'static,
    ) -> ArcSignal<bool>
    where
        K: Hash + 'static,
        MaybeKey: QueryMaybeKey<K, V>,
        MaybeKey::MappedValue: 'static,
        V: 'static,
    {
        let keyer = SendWrapper::new(keyer);
        self.scope_lookup
            .scope_subscriptions_mut()
            .add_is_loading_subscription(
                query_scope.cache_key(),
                MaybeLocal::new_local(ArcSignal::derive(move || {
                    keyer().into_maybe_key().map(|k| KeyHash::new(&k))
                })),
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
    pub fn subscribe_is_loading_arc<K, MaybeKey, V, M>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V, M>,
        keyer: impl Fn() -> MaybeKey + Send + Sync + 'static,
    ) -> ArcSignal<bool>
    where
        K: Hash + Send + Sync + 'static,
        MaybeKey: QueryMaybeKey<K, V>,
        MaybeKey::MappedValue: 'static,
        V: 'static,
    {
        self.scope_lookup
            .scope_subscriptions_mut()
            .add_is_loading_subscription(
                query_scope.cache_key(),
                MaybeLocal::new(ArcSignal::derive(move || {
                    keyer().into_maybe_key().map(|k| KeyHash::new(&k))
                })),
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
    pub fn subscribe_is_fetching<K, MaybeKey, V, M>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V, M>,
        keyer: impl Fn() -> MaybeKey + Send + Sync + 'static,
    ) -> Signal<bool>
    where
        K: Hash + Send + Sync + 'static,
        MaybeKey: QueryMaybeKey<K, V>,
        MaybeKey::MappedValue: 'static,
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
    pub fn subscribe_is_fetching_local<K, MaybeKey, V, M>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V, M>,
        keyer: impl Fn() -> MaybeKey + 'static,
    ) -> Signal<bool>
    where
        K: Hash + 'static,
        MaybeKey: QueryMaybeKey<K, V>,
        MaybeKey::MappedValue: 'static,
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
    pub fn subscribe_is_fetching_arc_local<K, MaybeKey, V, M>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V, M>,
        keyer: impl Fn() -> MaybeKey + 'static,
    ) -> ArcSignal<bool>
    where
        K: Hash + 'static,
        MaybeKey: QueryMaybeKey<K, V>,
        MaybeKey::MappedValue: 'static,
        V: 'static,
    {
        let keyer = SendWrapper::new(keyer);
        self.scope_lookup
            .scope_subscriptions_mut()
            .add_is_fetching_subscription(
                query_scope.cache_key(),
                MaybeLocal::new_local(ArcSignal::derive(move || {
                    keyer().into_maybe_key().map(|k| KeyHash::new(&k))
                })),
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
    pub fn subscribe_is_fetching_arc<K, MaybeKey, V, M>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V, M>,
        keyer: impl Fn() -> MaybeKey + Send + Sync + 'static,
    ) -> ArcSignal<bool>
    where
        K: Hash + Send + Sync + 'static,
        MaybeKey: QueryMaybeKey<K, V>,
        MaybeKey::MappedValue: 'static,
        V: 'static,
    {
        self.scope_lookup
            .scope_subscriptions_mut()
            .add_is_fetching_subscription(
                query_scope.cache_key(),
                MaybeLocal::new(ArcSignal::derive(move || {
                    keyer().into_maybe_key().map(|k| KeyHash::new(&k))
                })),
            )
    }

    /// Subscribe to the value of a query.
    /// The keyer function is reactive to changes in `K`.
    ///
    /// This will update whenever the query is created, removed, updated, refetched or set.
    ///
    /// Compared to a resource:
    /// - This will not trigger a fetch of a query, if it's not in the cache, this will be `None`.       
    #[track_caller]
    pub fn subscribe_value<K, MaybeKey, V, M>(
        &self,
        query_scope: impl QueryScopeTrait<K, V, M>,
        keyer: impl Fn() -> MaybeKey + Send + Sync + 'static,
    ) -> Signal<Option<V>>
    where
        K: DebugIfDevtoolsEnabled + Hash + Clone + Send + Sync + 'static,
        MaybeKey: QueryMaybeKey<K, V>,
        MaybeKey::MappedValue: DebugIfDevtoolsEnabled + Clone + Send + Sync + 'static,
        V: DebugIfDevtoolsEnabled + Clone + Send + Sync + 'static,
    {
        self.subscribe_value_arc(query_scope, keyer).into()
    }

    /// Subscribe to the value of a non-threadsafe query.
    /// The keyer function is reactive to changes in `K`.
    ///
    /// This will update whenever the query is created, removed, updated, refetched or set.
    ///
    /// Compared to a resource:
    /// - This will not trigger a fetch of a query, if it's not in the cache, this will be `None`.
    #[track_caller]
    pub fn subscribe_value_local<K, MaybeKey, V, M>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V, M>,
        keyer: impl Fn() -> MaybeKey + 'static,
    ) -> Signal<Option<V>, LocalStorage>
    where
        K: DebugIfDevtoolsEnabled + Hash + Clone + 'static,
        MaybeKey: QueryMaybeKey<K, V>,
        MaybeKey::MappedValue: DebugIfDevtoolsEnabled + Clone + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        self.subscribe_value_arc_local(query_scope, keyer).into()
    }

    /// Subscribe to the value of a non-threadsafe query.
    /// The keyer function is reactive to changes in `K`.
    ///
    /// This will update whenever the query is created, removed, updated, refetched or set.
    ///
    /// Compared to a resource:
    /// - This will not trigger a fetch of a query, if it's not in the cache, this will be `None`.
    #[track_caller]
    pub fn subscribe_value_arc_local<K, MaybeKey, V, M>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V, M>,
        keyer: impl Fn() -> MaybeKey + 'static,
    ) -> ArcLocalSignal<Option<V>>
    where
        K: DebugIfDevtoolsEnabled + Hash + Clone + 'static,
        MaybeKey: QueryMaybeKey<K, V>,
        MaybeKey::MappedValue: DebugIfDevtoolsEnabled + Clone + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        let cache_key = query_scope.cache_key();
        let keyer = ArcLocalSignal::derive_local(move || {
            keyer().into_maybe_key().map(|k| KeyHash::new(&k))
        });
        let dyn_signal = self
            .scope_lookup
            .scope_subscriptions_mut()
            .add_value_set_updated_or_removed_subscription(
                cache_key,
                MaybeLocal::new_local({
                    let keyer = keyer.clone();
                    move || keyer.get()
                }),
            );

        let scope_lookup = self.scope_lookup;
        ArcLocalSignal::derive_local(move || {
            dyn_signal.track();
            if let Some(key) = keyer.read_untracked().as_ref() {
                scope_lookup.with_cached_query::<K, V, _>(key, &cache_key, |maybe_cached| {
                    // WONTPANIC: with_cached_query will only output values that are safe on this thread:
                    maybe_cached.map(|cached| cached.value_maybe_stale().value_may_panic().clone())
                })
            } else {
                None
            }
        })
    }

    /// Subscribe to the value of a query.
    /// The keyer function is reactive to changes in `K`.
    ///
    /// This will update whenever the query is created, removed, updated, refetched or set.
    ///
    /// Compared to a resource:
    /// - This will not trigger a fetch of a query, if it's not in the cache, this will be `None`.
    #[track_caller]
    pub fn subscribe_value_arc<K, MaybeKey, V, M>(
        &self,
        query_scope: impl QueryScopeTrait<K, V, M>,
        keyer: impl Fn() -> MaybeKey + Send + Sync + 'static,
    ) -> ArcSignal<Option<V>>
    where
        K: DebugIfDevtoolsEnabled + Hash + Clone + Send + Sync + 'static,
        MaybeKey: QueryMaybeKey<K, V>,
        MaybeKey::MappedValue: DebugIfDevtoolsEnabled + Clone + Send + Sync + 'static,
        V: DebugIfDevtoolsEnabled + Clone + Send + Sync + 'static,
    {
        let cache_key = query_scope.cache_key();
        let keyer = ArcSignal::derive(move || keyer().into_maybe_key().map(|k| KeyHash::new(&k)));

        let dyn_signal = self
            .scope_lookup
            .scope_subscriptions_mut()
            .add_value_set_updated_or_removed_subscription(
                cache_key,
                MaybeLocal::new({
                    let keyer = keyer.clone();
                    move || keyer.get()
                }),
            );

        let scope_lookup = self.scope_lookup;
        ArcSignal::derive(move || {
            dyn_signal.track();
            if let Some(key) = keyer.read_untracked().as_ref() {
                scope_lookup.with_cached_query::<K, V, _>(key, &cache_key, |maybe_cached| {
                    // WONTPANIC: with_cached_query will only output values that are safe on this thread:
                    maybe_cached.map(|cached| cached.value_maybe_stale().value_may_panic().clone())
                })
            } else {
                None
            }
        })
    }

    /// Mark a query as stale.
    ///
    /// Any active resources will refetch in the background, replacing them when ready.
    #[track_caller]
    pub fn invalidate_query<K, V, M>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V, M>,
        key: impl Borrow<K>,
    ) -> bool
    where
        K: DebugIfDevtoolsEnabled + Hash + Clone + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        let cleared = self.invalidate_queries(query_scope, std::iter::once(key));
        !cleared.is_empty()
    }

    /// Mark multiple queries of a specific type as stale.
    ///
    /// Any active resources will refetch in the background, replacing them when ready.
    #[track_caller]
    pub fn invalidate_queries<K, V, KRef, M>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V, M>,
        keys: impl IntoIterator<Item = KRef>,
    ) -> Vec<KRef>
    where
        K: DebugIfDevtoolsEnabled + Hash + Clone + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
        KRef: Borrow<K>,
    {
        self.invalidate_queries_inner::<K, V, _>(&QueryScopeInfo::new_local(&query_scope), keys)
    }

    /// Mark one or more queries of a specific type as stale with a callback.
    ///
    /// When the callback returns `true`, the specific query will be invalidated.
    ///
    /// Any active resources subscribing to that query will refetch in the background,
    /// replacing them when ready.
    #[track_caller]
    pub fn invalidate_queries_with_predicate<K, V, M>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V, M>,
        should_invalidate: impl Fn(&K) -> bool,
    ) where
        K: DebugIfDevtoolsEnabled + Hash + Clone + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        let mut scopes = self.scope_lookup.scopes_mut();
        let mut cbs_scopes = vec![];
        self.scope_lookup.with_cached_scope_mut::<K, V, _, _>(
            &mut scopes,
            &QueryScopeInfo::new_local(&query_scope),
            false,
            |_| {},
            |maybe_scope, _| {
                if let Some(scope) = maybe_scope {
                    for query in scope.all_queries_mut_include_pending() {
                        if let Some(key) = query.key().value_if_safe()
                            && should_invalidate(key)
                        {
                            let cb_scopes = query.invalidate(QueryAbortReason::Invalidate);
                            cbs_scopes.push(cb_scopes);
                        }
                    }
                }
            },
        );
        let mut cbs_external = vec![];
        for cb in cbs_scopes {
            if let Some(cb_external) = cb(&mut scopes) {
                cbs_external.push(cb_external);
            }
        }
        drop(scopes);
        for cb in cbs_external {
            cb();
        }
    }

    #[track_caller]
    pub(crate) fn invalidate_queries_inner<K, V, KRef>(
        &self,
        query_scope_info: &QueryScopeInfo,
        keys: impl IntoIterator<Item = KRef>,
    ) -> Vec<KRef>
    where
        K: DebugIfDevtoolsEnabled + Hash + Clone + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
        KRef: Borrow<K>,
    {
        let keys = keys.into_iter().collect::<Vec<_>>();
        let key_hashes = keys
            .iter()
            .map(|key| KeyHash::new(key.borrow()))
            .collect::<Vec<_>>();

        if key_hashes.is_empty() {
            return vec![];
        }

        let mut scopes = self.scope_lookup.scopes_mut();
        let mut cbs_scopes = vec![];
        let results = self.scope_lookup.with_cached_scope_mut::<K, V, _, _>(
            &mut scopes,
            query_scope_info,
            false,
            |_| {},
            |maybe_scope, _| {
                let mut invalidated = vec![];
                if let Some(scope) = maybe_scope {
                    for (key, key_hash) in keys.into_iter().zip(key_hashes.iter()) {
                        if let Some(cached) = scope.get_mut_include_pending(key_hash) {
                            let cb_scopes = cached.invalidate(QueryAbortReason::Invalidate);
                            cbs_scopes.push(cb_scopes);
                            invalidated.push(key);
                        }
                    }
                }
                invalidated
            },
        );
        let mut cbs_external = vec![];
        for cb in cbs_scopes {
            if let Some(cb_external) = cb(&mut scopes) {
                cbs_external.push(cb_external);
            }
        }
        drop(scopes);
        for cb in cbs_external {
            cb();
        }
        results
    }

    /// Mark all queries of a specific type as stale.
    ///
    /// Any active resources will refetch in the background, replacing them when ready.
    #[track_caller]
    pub fn invalidate_query_scope<K, V, M>(&self, query_scope: impl QueryScopeLocalTrait<K, V, M>)
    where
        K: Hash + 'static,
        V: Clone + 'static,
    {
        self.invalidate_query_scope_inner(&query_scope.cache_key())
    }

    pub(crate) fn invalidate_query_scope_inner(&self, scope_cache_key: &ScopeCacheKey) {
        let mut scopes = self.scope_lookup.scopes_mut();
        let mut cbs_scopes = vec![];
        if let Some(scope) = scopes.get_mut(scope_cache_key) {
            let cb_scopes = scope.invalidate_scope(QueryAbortReason::Invalidate);
            cbs_scopes.push(cb_scopes);
            for buster in scope.busters() {
                buster.try_set(new_buster_id());
            }
        }
        let mut cbs_external = vec![];
        for cb in cbs_scopes {
            if let Some(cb_external) = cb(&mut scopes) {
                cbs_external.push(cb_external);
            }
        }
        drop(scopes);
        for cb in cbs_external {
            cb();
        }
    }

    /// Mark all queries as stale.
    ///
    /// Any active resources will refetch in the background, replacing them when ready.
    ///
    /// To have the cache instantly cleared and all listeners reset to pending, e.g. for user logout,
    /// see [`QueryClient::clear`].
    #[track_caller]
    pub fn invalidate_all_queries(&self) {
        let mut scopes = self.scope_lookup.scopes_mut();
        let mut cbs_scopes = vec![];
        for scope in scopes.values_mut() {
            let busters = scope.busters();
            let cb_scopes = scope.invalidate_scope(QueryAbortReason::Invalidate);
            cbs_scopes.push(cb_scopes);
            for buster in busters {
                buster.try_set(new_buster_id());
            }
        }
        let mut cbs_external = vec![];
        for cb in cbs_scopes {
            if let Some(cb_external) = cb(&mut scopes) {
                cbs_external.push(cb_external);
            }
        }
        drop(scopes);
        for cb in cbs_external {
            cb();
        }
    }

    /// Empty the cache, like [`QueryClient::invalidate_all_queries`] except:
    /// - the cache is instantly cleared of all queries
    /// - All active resources etc are reset to their pending state instantly
    ///   until the new query finishes refetching.
    ///
    /// Useful for e.g. user logout.
    ///
    /// [`QueryClient::invalidate_all_queries`] on the other hand, will only refetch active queries in the background, replacing them when ready.
    #[track_caller]
    pub fn clear(&self) {
        let mut scopes = self.scope_lookup.scopes_mut();
        let mut cbs_scopes = vec![];
        for scope in scopes.values_mut() {
            let busters = scope.busters();
            let cb_scopes = scope.invalidate_scope(QueryAbortReason::Clear);
            cbs_scopes.push(cb_scopes);
            scope.clear();
            for buster in busters {
                buster.try_set(new_buster_id());
            }
        }
        let mut cbs_external = vec![];
        for cb in cbs_scopes {
            if let Some(cb_external) = cb(&mut scopes) {
                cbs_external.push(cb_external);
            }
        }
        drop(scopes);
        for cb in cbs_external {
            cb();
        }
    }

    #[cfg(test)]
    /// Clear a specific query key.
    #[track_caller]
    pub fn clear_query<K, V, M>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V, M>,
        key: impl Borrow<K>,
    ) -> bool
    where
        K: DebugIfDevtoolsEnabled + Hash + Clone + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        let mut scopes = self.scope_lookup.scopes_mut();
        let mut cbs_scopes = vec![];
        let result = self.scope_lookup.with_cached_scope_mut::<K, V, _, _>(
            &mut scopes,
            &QueryScopeInfo::new_local(&query_scope),
            false,
            |_| {},
            |maybe_scope, _| {
                if let Some(scope) = maybe_scope {
                    let key_hash = KeyHash::new(key.borrow());
                    if let Some(cached) = scope.get_mut_include_pending(&key_hash) {
                        let cb_scopes = cached.invalidate(QueryAbortReason::Clear);
                        cbs_scopes.push(cb_scopes);
                    }
                    let removed = scope.remove_entry(&key_hash);
                    // Calling it again just in case because in tests might be in sync cache and non sync cache:
                    scope.remove_entry(&KeyHash::new(key.borrow()));
                    return removed.is_some();
                }
                false
            },
        );
        let mut cbs_external = vec![];
        for cb in cbs_scopes {
            if let Some(cb_external) = cb(&mut scopes) {
                cbs_external.push(cb_external);
            }
        }
        drop(scopes);
        for cb in cbs_external {
            cb();
        }
        result
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
    pub(crate) fn is_key_invalid<K, V, M>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V, M>,
        key: impl Borrow<K>,
    ) -> bool
    where
        K: DebugIfDevtoolsEnabled + Hash + Clone + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        self.scope_lookup.with_cached_scope_mut::<K, V, _, _>(
            &mut self.scope_lookup.scopes_mut(),
            &QueryScopeInfo::new_local(&query_scope),
            false,
            |_| {},
            |maybe_scope, _| {
                if let Some(scope) = maybe_scope {
                    scope
                        .get(&KeyHash::new(key.borrow()))
                        .map(|query| query.is_invalidated())
                        .unwrap_or(false)
                } else {
                    false
                }
            },
        )
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn mark_key_valid<K, V, M>(
        &self,
        query_scope: impl QueryScopeLocalTrait<K, V, M>,
        key: impl Borrow<K>,
    ) where
        K: DebugIfDevtoolsEnabled + Hash + Clone + 'static,
        V: DebugIfDevtoolsEnabled + Clone + 'static,
    {
        self.scope_lookup.with_cached_scope_mut::<K, V, _, _>(
            &mut self.scope_lookup.scopes_mut(),
            &QueryScopeInfo::new_local(&query_scope),
            false,
            |_| {},
            |maybe_scope, _| {
                if let Some(scope) = maybe_scope
                    && let Some(query) = scope.get_mut(&KeyHash::new(key.borrow()))
                {
                    query.mark_valid();
                }
            },
        );
    }

    #[cfg(test)]
    pub(crate) fn subscriber_count(&self) -> usize {
        self.scope_lookup.scope_subscriptions_mut().count()
    }
}
