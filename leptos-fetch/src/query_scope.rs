use paste::paste;
use std::{
    any::TypeId,
    fmt::{self, Debug, Formatter},
    future::Future,
    pin::Pin,
    sync::Arc,
};

use crate::QueryOptions;

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
        }

        impl<K, V> $name<K, V> {
            /// Create a new
            #[doc = $sname]
            ///  with specific [`QueryOptions`] to only apply to this query type.
            ///
            /// These [`QueryOptions`] will be combined with the global [`QueryOptions`] set on the [`crate::QueryClient`], with the local options taking precedence.
            pub fn new<F, Fut>(query: F, options: QueryOptions) -> Self
            where
                F: Fn(K) -> Fut $($impl_fn_generics)* + 'static,
                Fut: Future<Output = V> $($impl_fut_generics)* + 'static,
            {
                Self {
                    query: Arc::new(move |key| Box::pin(query(key))),
                    query_type_id: TypeId::of::<F>(),
                    options,
                }
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
}

define! { [+ Send], [+ Send + Sync], QueryScope, "QueryScope", "threadsafe" }
define! { [], [], QueryScopeLocal, "QueryScopeLocal", "non-threadsafe" }
