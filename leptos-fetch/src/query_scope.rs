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
    ([$($impl_fut_generics:tt)*], [$($impl_fn_generics:tt)*], $name:ident) => {
        /// TODO
        #[derive(Clone)]
        pub struct $name<K, V> {
            query: Arc<dyn Fn(K) -> Pin<Box<dyn Future<Output = V> $($impl_fut_generics)*>> $($impl_fn_generics)*>,
            query_type_id: TypeId,
            options: QueryOptions,
        }

        impl<K, V> $name<K, V> {
            /// TODO
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
            /// TODO
            pub trait [<$name Trait>] <K, V>
            where
                K: 'static,
                V: 'static,
             {
                /// TODO
                fn options(&self) -> Option<QueryOptions> {
                    Default::default()
                }

                /// TODO
                fn cache_key(&self) -> TypeId;

                /// TODO
                fn query(&self, key: K) -> impl Future<Output = V> $($impl_fut_generics)* + '_;
            }

            impl<K, V, Fut> [<$name Trait>]<K, V> for fn(K) -> Fut
            where
                K: 'static,
                V: 'static,
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

define! { [+ Send], [+ Send + Sync], QueryScope }
define! { [], [], QueryScopeLocal }
