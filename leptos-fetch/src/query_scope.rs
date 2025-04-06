use paste::paste;
use std::{
    any::TypeId,
    fmt::{self, Debug, Formatter},
    future::Future,
    pin::Pin,
    sync::Arc,
};

use crate::QueryOptions;

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

macro_rules! define {
    ([$($impl_fut_generics:tt)*], [$($impl_fn_generics:tt)*], $name:ident, $sname:literal, $sthread:literal) => {
        /// A
        #[doc = $sthread]
        /// wrapper for a query function. This can be used to add specific [`QueryOptions`] to only apply to one query type.
        ///
        /// These [`QueryOptions`] will be combined with the global [`QueryOptions`] set on the [`crate::QueryClient`], with the local options taking precedence.
        ///
        /// If you don't need to set specific options, you can use functions with the [`crate::QueryClient`] directly.
        #[derive(Clone)]
        pub struct $name<K, V> {
            query: Arc<dyn Fn(K) -> Pin<Box<dyn Future<Output = V> $($impl_fut_generics)*>> $($impl_fn_generics)*>,
            query_type_id: TypeId,
            options: QueryOptions,
            #[cfg(any(
                all(debug_assertions, feature = "devtools"),
                feature = "devtools-always"
            ))]
            title: Arc<String>,
        }

        impl<K, V> $name<K, V> {
            /// Create a new
            #[doc = $sname]
            ///  with specific [`QueryOptions`] to only apply to this query type.
            ///
            /// These [`QueryOptions`] will be combined with the global [`QueryOptions`] set on the [`crate::QueryClient`], with the local options taking precedence.
            #[track_caller]
            pub fn new<F, Fut>(query: F, options: QueryOptions) -> Self
            where
                F: Fn(K) -> Fut $($impl_fn_generics)* + 'static,
                Fut: Future<Output = V> $($impl_fut_generics)* + 'static,
            {
                Self {
                    query: Arc::new(move |key| Box::pin(query(key))),
                    query_type_id: TypeId::of::<F>(),
                    options,
                    #[cfg(any(
                        all(debug_assertions, feature = "devtools"),
                        feature = "devtools-always"
                    ))]
                    title: format_title(std::any::type_name::<F>()),
                }
            }

            #[cfg(any(feature = "devtools", feature = "devtools-always"))]
            /// Set a custom query scope/type title that will show in devtools.
            #[track_caller]
            pub fn set_title(mut self, title: impl Into<String>) -> Self {
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

        impl<K, V> Debug for $name<K, V> {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($name))
                    .field("query", &"Arc<dyn Fn(K) -> Pin<Box<dyn Future<Output = V>>")
                    .field("options", &self.options)
                    .finish()
            }
        }

        paste! {
            /// Coercer trait, ignore.
            pub trait [<$name Trait>] <K, V>
            where
                K: 'static,
                V: 'static,
             {
                /// Coercer trait, ignore.
                fn options(&self) -> Option<QueryOptions> {
                    Default::default()
                }

                /// Coercer trait, ignore.
                fn cache_key(&self) -> TypeId;

                /// Coercer trait, ignore.
                fn query(&self, key: K) -> impl Future<Output = V> $($impl_fut_generics)* + '_;

                #[cfg(any(
                    all(debug_assertions, feature = "devtools"),
                    feature = "devtools-always"
                ))]
                #[track_caller]
                fn title(&self) -> Arc<String>;
            }

            impl<K, V, F, Fut> [<$name Trait>]<K, V> for F
            where
                K: 'static,
                V: 'static,
                F: Fn(K) -> Fut + 'static,
                Fut: Future<Output = V> $($impl_fut_generics)* + 'static,
             {

                fn cache_key(&self) -> TypeId {
                    TypeId::of::<Self>()
                }

                fn query(&self, key: K) -> impl Future<Output = V> $($impl_fut_generics)* + '_ {
                    self(key)
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

            impl<K, V> [<$name Trait>]<K, V> for $name<K, V>
            where
                K: 'static,
                V: 'static,
            {
                fn options(&self) -> Option<QueryOptions> {
                    Some(self.options)
                }

                fn cache_key(&self) -> TypeId {
                    self.query_type_id
                }

                fn query(&self, key: K) -> impl Future<Output = V> $($impl_fut_generics)* + '_ {
                    (self.query)(key)
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

            impl<K, V> [<$name Trait>]<K, V> for &$name<K, V>
            where
                K: 'static,
                V: 'static,
            {
                fn options(&self) -> Option<QueryOptions> {
                    Some(self.options)
                }

                fn cache_key(&self) -> TypeId {
                    self.query_type_id
                }

                fn query(&self, key: K) -> impl Future<Output = V> $($impl_fut_generics)* + '_ {
                    (self.query)(key)
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

            impl<K, V, T> [<$name Trait>]<K, V> for Arc<T>
            where
                K: 'static,
                V: 'static,
                T: [<$name Trait>]<K, V>,
            {
                fn options(&self) -> Option<QueryOptions> {
                    T::options(self)
                }

                fn cache_key(&self) -> TypeId {
                    T::cache_key(self)
                }

                fn query(&self, key: K) -> impl Future<Output = V> $($impl_fut_generics)* + '_ {
                    T::query(self, key)
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

impl<K, V> QueryScopeLocalTrait<K, V> for QueryScope<K, V>
where
    K: 'static,
    V: 'static,
{
    fn options(&self) -> Option<QueryOptions> {
        Some(self.options)
    }

    fn cache_key(&self) -> TypeId {
        self.query_type_id
    }

    fn query(&self, key: K) -> impl Future<Output = V> + '_ {
        (self.query)(key)
    }

    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    fn title(&self) -> Arc<String> {
        self.title.clone()
    }
}

impl<K, V> QueryScopeLocalTrait<K, V> for &QueryScope<K, V>
where
    K: 'static,
    V: 'static,
{
    fn options(&self) -> Option<QueryOptions> {
        Some(self.options)
    }

    fn cache_key(&self) -> TypeId {
        self.query_type_id
    }

    fn query(&self, key: K) -> impl Future<Output = V> + '_ {
        (self.query)(key)
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
pub(crate) struct QueryTypeInfo {
    pub options: Option<QueryOptions>,
    pub cache_key: TypeId,
    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    pub title: Arc<String>,
}

impl QueryTypeInfo {
    #[track_caller]
    pub fn new<K, V>(query_scope: &impl QueryScopeTrait<K, V>) -> Self
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

    pub fn new_local<K, V>(query_scope: &impl QueryScopeLocalTrait<K, V>) -> Self
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
