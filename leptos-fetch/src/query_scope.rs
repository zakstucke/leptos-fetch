use paste::paste;
use std::{
    any::TypeId,
    fmt::{self, Debug, Formatter},
    future::Future,
    hash::{DefaultHasher, Hash, Hasher},
    pin::Pin,
    sync::Arc,
};

use crate::{QueryOptions, maybe_local::MaybeLocal};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScopeCacheKey(u64);

impl ScopeCacheKey {
    pub fn new(fetcher_type_id: TypeId, options: &QueryOptions) -> Self {
        let mut hasher = DefaultHasher::new();
        fetcher_type_id.hash(&mut hasher);
        options.hash(&mut hasher);
        Self(hasher.finish())
    }
}

impl Hash for ScopeCacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

#[cfg(any(
    all(debug_assertions, feature = "devtools"),
    feature = "devtools-always"
))]
#[track_caller]
fn format_title(base: &str) -> Arc<String> {
    let loc = std::panic::Location::caller();
    let filepath = loc.file();
    let file = format!(
        "{}:{}:{}",
        // Only want the final file, not the full path:
        filepath
            .split(std::path::MAIN_SEPARATOR_STR)
            .last()
            .unwrap_or(filepath),
        loc.line(),
        loc.column()
    );
    Arc::new(format!(
        "{}: {}",
        file,
        base.trim_end_matches("::{{closure}}")
    ))
}

/// A marker struct to allow query function with or without a key.
///
/// Ignore.
pub struct QueryMarkerWithKey;

/// A marker struct to allow query function with or without a key.
///
/// Ignore.
pub struct QueryMarkerNoKey;

macro_rules! define {
    ([$($impl_fut_generics:tt)*], [$($impl_fn_generics:tt)*], $name:ident, $sname:literal, $sthread:literal) => {
        /// A
        #[doc = $sthread]
        /// wrapper for a query function. This can be used to add specific [`QueryOptions`] to only apply to one query scope.
        ///
        /// These [`QueryOptions`] will be combined with the global [`QueryOptions`] set on the [`crate::QueryClient`], with the local options taking precedence.
        ///
        /// If you don't need to set specific options, you can use functions with the [`crate::QueryClient`] directly.
        #[derive(Clone)]
        pub struct $name<K, V> {
            query: Arc<dyn Fn(K) -> Pin<Box<dyn Future<Output = V> $($impl_fut_generics)*>> $($impl_fn_generics)*>,
            invalidation_hierarchy_fn: Option<Arc<dyn Fn(&K) -> Vec<String> $($impl_fn_generics)*>>,
            on_invalidation: Vec<Arc<dyn Fn(&K) $($impl_fn_generics)*>>,
            fetcher_type_id: TypeId,
            cache_key: ScopeCacheKey,
            options: QueryOptions,
            #[cfg(any(
                all(debug_assertions, feature = "devtools"),
                feature = "devtools-always"
            ))]
            title: Arc<String>,
        }

        impl<K, V> Debug for $name<K, V> {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($name))
                    .field("query", &"Arc<dyn Fn(K) -> Pin<Box<dyn Future<Output = V>>")
                    .field("options", &self.options)
                    .finish()
            }
        }

        paste! {
            impl<K, V> $name<K, V> {
                /// Create a new
                #[doc = $sname]
                ///.
                /// If the query fn does not have a key argument, `K=()`
                ///
                #[track_caller]
                pub fn new<M>(
                    query_scope: impl [<$name Trait>]<K, V, M> $($impl_fn_generics)* + 'static,
                ) -> Self
                where
                    K: 'static $($impl_fn_generics)*,
                    V: 'static $($impl_fn_generics)*,
                {
                    let options = query_scope.options().unwrap_or_default();
                    let fetcher_type_id = query_scope.fetcher_type_id();
                    Self {
                        fetcher_type_id,
                        cache_key: ScopeCacheKey::new(fetcher_type_id, &options),
                        options,
                        #[cfg(any(
                            all(debug_assertions, feature = "devtools"),
                            feature = "devtools-always"
                        ))]
                        title: query_scope.title(),
                        invalidation_hierarchy_fn: None,
                        on_invalidation: vec![],
                        query: Arc::new(move |key| Box::pin(query_scope.query(key))),
                    }
                }

                /// Set specific [`QueryOptions`] to only apply to this query scope.
                ///
                /// These [`QueryOptions`] will be combined with the global [`QueryOptions`] set on the [`crate::QueryClient`], with the local options taking precedence.
                pub fn with_options(mut self, options: QueryOptions) -> Self {
                    self.options = options;
                    self
                }

                /// Different query types are sometimes linked to the same source, e.g. you may want an invalidation of `list_blogposts()` to always automatically invalidate `get_blogpost(id)`.
                ///
                /// [`QueryScope::with_invalidation_link`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryScope.html#method.subscribe_is_fetching::with_invalidation_link) can be used to this effect, given a query key `&K`, you provide a `Vec<String>` that's used as a **hierarchy key (HK)** for that query. When a query is invalidated, any query's **HK** that's prefixed by this **HK** will also be invalidated automatically. E.g. A query with **HK** `["users"]` will also auto invalidate another query with `["users", "1"]`, but not the other way around. 2 queries with an identicial **HK** of `["users"]` will auto invalidate each other.
                ///
                /// ```rust
                /// use std::time::Duration;
                ///
                /// use leptos_fetch::{QueryClient, QueryScope, QueryOptions};
                /// use leptos::prelude::*;
                ///
                /// #[derive(Debug, Clone)]
                /// struct User;
                ///
                /// fn list_users_query() -> QueryScope<(), Vec<User>> {
                ///     QueryScope::new(async || vec![])
                ///         .with_invalidation_link(
                ///             |_key| ["users"]
                ///         )
                /// }
                ///
                /// fn get_user_query() -> QueryScope<i32, User> {
                ///     QueryScope::new(async move |user_id: i32| User)
                ///         .with_invalidation_link(
                ///             |user_id| ["users".to_string(), user_id.to_string()]
                ///         )
                /// }
                ///
                /// let client = QueryClient::new();
                ///
                /// // This invalidates only user "2", because ["users", "2"] is not a prefix of ["users"],
                /// // list_users_query is NOT invalidated.
                /// client.invalidate_query(get_user_query(), &2);
                ///
                /// // This invalidates both queries, because ["users"] is a prefix of ["users", "$x"]
                /// client.invalidate_query(list_users_query(), &());
                /// ```
                pub fn with_invalidation_link<S, I>(
                    mut self,
                    invalidation_hierarchy_fn: impl Fn(&K) -> I + 'static $($impl_fn_generics)*,
                ) -> Self
                where
                    I: IntoIterator<Item = S> + 'static $($impl_fn_generics)*,
                    S: Into<String> + 'static $($impl_fn_generics)*,
                {
                    self.invalidation_hierarchy_fn = Some(Arc::new(move |key| {
                        invalidation_hierarchy_fn(key).into_iter().map(|s| s.into()).collect()
                    }));
                    self
                }

                /// Additive callbacks to be run when this query with a specific key is invalidated.
                pub fn on_invalidation(
                    mut self,
                    on_invalidation_cb: impl Fn(&K) + 'static $($impl_fn_generics)*,
                ) -> Self {
                    self.on_invalidation.push(Arc::new(on_invalidation_cb));
                    self
                }

                #[cfg(any(feature = "devtools", feature = "devtools-always"))]
                /// Set a custom query scope/type title that will show in devtools.
                #[track_caller]
                pub fn with_title(mut self, title: impl Into<String>) -> Self {
                    #[cfg(any(
                        all(debug_assertions, feature = "devtools"),
                        feature = "devtools-always"
                    ))]
                    {
                        self.title = format_title(&title.into());
                    }
                    self
                }
            }

            pub trait [<$name Trait>] <K, V, M>
            where
                K: 'static,
                V: 'static,
             {
                fn options(&self) -> Option<QueryOptions> {
                    Default::default()
                }

                fn fetcher_type_id(&self) -> TypeId;

                fn cache_key(&self) -> ScopeCacheKey;

                fn query(&self, key: K) -> impl Future<Output = V> $($impl_fut_generics)* + 'static;

                fn invalidation_prefix(&self, key: &K) -> Option<Vec<String>>;

                #[allow(unused_parens)]
                fn on_invalidation(&self) -> Option<Arc<dyn Fn(&K) $($impl_fn_generics)*>>;

                #[cfg(any(
                    all(debug_assertions, feature = "devtools"),
                    feature = "devtools-always"
                ))]
                #[track_caller]
                fn title(&self) -> Arc<String>;
            }

            impl<K, V, F, Fut> [<$name Trait>]<K, V, QueryMarkerWithKey> for F
            where
                K: 'static,
                V: 'static,
                F: Fn(K) -> Fut + 'static,
                Fut: Future<Output = V> $($impl_fut_generics)* + 'static,
             {

                fn fetcher_type_id(&self) -> TypeId {
                    TypeId::of::<Self>()
                }

                fn cache_key(&self) -> ScopeCacheKey {
                    ScopeCacheKey::new(TypeId::of::<Self>(), &Default::default())
                }

                fn query(&self, key: K) -> impl Future<Output = V> $($impl_fut_generics)* + 'static {
                    self(key)
                }

                fn invalidation_prefix(&self, _key: &K) -> Option<Vec<String>> {
                    None
                }

                fn on_invalidation(&self) -> Option<Arc<dyn Fn(&K) $($impl_fn_generics)*>> {
                    None
                }

                #[cfg(any(
                    all(debug_assertions, feature = "devtools"),
                    feature = "devtools-always"
                ))]
                #[track_caller]
                fn title(&self) -> Arc<String> {
                    format_title(std::any::type_name::<Self>())
                }
            }

            impl<V, F, Fut> [<$name Trait>]<(), V, QueryMarkerNoKey> for F
            where
                V: 'static,
                F: Fn() -> Fut + 'static,
                Fut: Future<Output = V> $($impl_fut_generics)* + 'static,
             {
                fn fetcher_type_id(&self) -> TypeId {
                    TypeId::of::<Self>()
                }

                fn cache_key(&self) -> ScopeCacheKey {
                    ScopeCacheKey::new(TypeId::of::<Self>(), &Default::default())
                }

                fn query(&self, _key: ()) -> impl Future<Output = V> $($impl_fut_generics)* + 'static {
                    self()
                }

                fn invalidation_prefix(&self, _key: &()) -> Option<Vec<String>> {
                    None
                }

                #[allow(unused_parens)]
                fn on_invalidation(&self) -> Option<Arc<dyn Fn(&()) $($impl_fn_generics)*>> {
                    None
                }

                #[cfg(any(
                    all(debug_assertions, feature = "devtools"),
                    feature = "devtools-always"
                ))]
                #[track_caller]
                fn title(&self) -> Arc<String> {
                    format_title(std::any::type_name::<Self>())
                }
            }

            impl<K, V> [<$name Trait>]<K, V, QueryMarkerWithKey> for $name<K, V>
            where
                K: 'static,
                V: 'static,
            {
                fn options(&self) -> Option<QueryOptions> {
                    Some(self.options)
                }

                fn fetcher_type_id(&self) -> TypeId {
                    self.fetcher_type_id
                }

                fn cache_key(&self) -> ScopeCacheKey {
                    self.cache_key
                }

                fn query(&self, key: K) -> impl Future<Output = V> $($impl_fut_generics)* + 'static {
                    (self.query)(key)
                }

                fn invalidation_prefix(&self, key: &K) -> Option<Vec<String>> {
                    if let Some(invalidation_hierarchy_fn) = &self.invalidation_hierarchy_fn {
                        Some(invalidation_hierarchy_fn(key))
                    } else {
                        None
                    }
                }

                #[allow(unused_parens)]
                fn on_invalidation(&self) -> Option<Arc<dyn Fn(&K) $($impl_fn_generics)*>> {
                    if self.on_invalidation.is_empty() {
                        None
                    } else {
                        let callbacks = self.on_invalidation.clone();
                        Some(Arc::new(move |key| {
                            for cb in &callbacks {
                                cb(key);
                            }
                        }))
                    }
                }

                #[cfg(any(
                    all(debug_assertions, feature = "devtools"),
                    feature = "devtools-always"
                ))]
                #[track_caller]
                fn title(&self) -> Arc<String> {
                    self.title.clone()
                }
            }

            impl<K, V> [<$name Trait>]<K, V, QueryMarkerWithKey> for &$name<K, V>
            where
                K: 'static,
                V: 'static,
            {
                fn options(&self) -> Option<QueryOptions> {
                    Some(self.options)
                }

                fn fetcher_type_id(&self) -> TypeId {
                    self.fetcher_type_id
                }

                fn cache_key(&self) -> ScopeCacheKey {
                    self.cache_key
                }

                fn query(&self, key: K) -> impl Future<Output = V> $($impl_fut_generics)* + 'static {
                    (self.query)(key)
                }

                fn invalidation_prefix(&self, key: &K) -> Option<Vec<String>> {
                    if let Some(invalidation_hierarchy_fn) = &self.invalidation_hierarchy_fn {
                        Some(invalidation_hierarchy_fn(key))
                    } else {
                        None
                    }
                }

                #[allow(unused_parens)]
                fn on_invalidation(&self) -> Option<Arc<dyn Fn(&K) $($impl_fn_generics)*>> {
                    if self.on_invalidation.is_empty() {
                        None
                    } else {
                        let callbacks = self.on_invalidation.clone();
                        Some(Arc::new(move |key| {
                            for cb in &callbacks {
                                cb(key);
                            }
                        }))
                    }
                }

                #[cfg(any(
                    all(debug_assertions, feature = "devtools"),
                    feature = "devtools-always"
                ))]
                #[track_caller]
                fn title(&self) -> Arc<String> {
                    self.title.clone()
                }
            }

            impl<K, V, T, M> [<$name Trait>]<K, V, M> for Arc<T>
            where
                K: 'static,
                V: 'static,
                T: [<$name Trait>]<K, V, M>,
            {
                fn options(&self) -> Option<QueryOptions> {
                    T::options(self)
                }

                fn fetcher_type_id(&self) -> TypeId {
                    T::fetcher_type_id(self)
                }

                fn cache_key(&self) -> ScopeCacheKey {
                    T::cache_key(self)
                }

                fn query(&self, key: K) -> impl Future<Output = V> $($impl_fut_generics)* + 'static {
                    T::query(self, key)
                }

                fn invalidation_prefix(&self, key: &K) -> Option<Vec<String>> {
                    T::invalidation_prefix(self, key)
                }

                #[allow(unused_parens)]
                fn on_invalidation(&self) -> Option<Arc<dyn Fn(&K) $($impl_fn_generics)*>> {
                    T::on_invalidation(self)
                }

                #[cfg(any(
                    all(debug_assertions, feature = "devtools"),
                    feature = "devtools-always"
                ))]
                #[track_caller]
                fn title(&self) -> Arc<String> {
                    T::title(self)
                }
            }
        }
    };
}

impl<K, V> QueryScopeLocalTrait<K, V, QueryMarkerWithKey> for QueryScope<K, V>
where
    K: 'static,
    V: 'static,
{
    fn options(&self) -> Option<QueryOptions> {
        Some(self.options)
    }

    fn fetcher_type_id(&self) -> TypeId {
        self.fetcher_type_id
    }

    fn cache_key(&self) -> ScopeCacheKey {
        self.cache_key
    }

    fn query(&self, key: K) -> impl Future<Output = V> + 'static {
        (self.query)(key)
    }

    fn invalidation_prefix(&self, key: &K) -> Option<Vec<String>> {
        self.invalidation_hierarchy_fn
            .as_ref()
            .map(|invalidation_hierarchy_fn| invalidation_hierarchy_fn(key))
    }

    fn on_invalidation(&self) -> Option<Arc<dyn Fn(&K)>> {
        if self.on_invalidation.is_empty() {
            None
        } else {
            let callbacks = self.on_invalidation.clone();
            Some(Arc::new(move |key| {
                for cb in &callbacks {
                    cb(key);
                }
            }))
        }
    }

    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    fn title(&self) -> Arc<String> {
        self.title.clone()
    }
}

impl<K, V> QueryScopeLocalTrait<K, V, QueryMarkerWithKey> for &QueryScope<K, V>
where
    K: 'static,
    V: 'static,
{
    fn options(&self) -> Option<QueryOptions> {
        Some(self.options)
    }

    fn fetcher_type_id(&self) -> TypeId {
        self.fetcher_type_id
    }

    fn cache_key(&self) -> ScopeCacheKey {
        self.cache_key
    }

    fn query(&self, key: K) -> impl Future<Output = V> + 'static {
        (self.query)(key)
    }

    fn invalidation_prefix(&self, key: &K) -> Option<Vec<String>> {
        self.invalidation_hierarchy_fn
            .as_ref()
            .map(|invalidation_hierarchy_fn| invalidation_hierarchy_fn(key))
    }

    fn on_invalidation(&self) -> Option<Arc<dyn Fn(&K)>> {
        if self.on_invalidation.is_empty() {
            None
        } else {
            let callbacks = self.on_invalidation.clone();
            Some(Arc::new(move |key| {
                for cb in &callbacks {
                    cb(key);
                }
            }))
        }
    }

    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    fn title(&self) -> Arc<String> {
        self.title.clone()
    }
}

define! { [+ Send], [+ Send + Sync], QueryScope, "QueryScope", "threadsafe" }
define! { [], [], QueryScopeLocal, "QueryScopeLocal", "non-threadsafe" }

#[derive(Debug, Clone)]
pub(crate) struct QueryScopeInfo {
    pub options: Option<QueryOptions>,
    pub cache_key: ScopeCacheKey,
    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    pub title: Arc<String>,
}

impl QueryScopeInfo {
    #[track_caller]
    pub fn new<K, V, M>(query_scope: &impl QueryScopeTrait<K, V, M>) -> Self
    where
        K: 'static,
        V: 'static,
    {
        Self {
            options: query_scope.options(),
            cache_key: query_scope.cache_key(),
            #[cfg(any(
                all(debug_assertions, feature = "devtools"),
                feature = "devtools-always"
            ))]
            title: query_scope.title(),
        }
    }

    #[track_caller]
    pub fn new_local<K, V, M>(query_scope: &impl QueryScopeLocalTrait<K, V, M>) -> Self
    where
        K: 'static,
        V: 'static,
    {
        Self {
            options: query_scope.options(),
            cache_key: query_scope.cache_key(),
            #[cfg(any(
                all(debug_assertions, feature = "devtools"),
                feature = "devtools-always"
            ))]
            title: query_scope.title(),
        }
    }
}

pub(crate) struct QueryScopeQueryInfo<K> {
    pub on_invalidation: Option<MaybeLocal<Arc<dyn Fn(&K)>>>,
    pub invalidation_prefix: Option<Vec<String>>,
    _key_marker: std::marker::PhantomData<K>,
}

impl<K> QueryScopeQueryInfo<K>
where
    K: 'static,
{
    #[track_caller]
    pub fn new<V, M>(query_scope: &impl QueryScopeTrait<K, V, M>, key: &K) -> Self
    where
        K: 'static,
        V: 'static,
    {
        Self {
            on_invalidation: query_scope
                .on_invalidation()
                .map(MaybeLocal::new_invalidation_cb_special),
            invalidation_prefix: query_scope.invalidation_prefix(key),
            _key_marker: std::marker::PhantomData,
        }
    }

    #[track_caller]
    pub fn new_local<V, M>(query_scope: &impl QueryScopeLocalTrait<K, V, M>, key: &K) -> Self
    where
        V: 'static,
    {
        Self {
            on_invalidation: query_scope.on_invalidation().map(MaybeLocal::new_local),
            invalidation_prefix: query_scope.invalidation_prefix(key),
            _key_marker: std::marker::PhantomData,
        }
    }
}
