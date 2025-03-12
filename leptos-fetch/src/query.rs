use std::{any::TypeId, hash::Hash, time::Duration};

use leptos::prelude::{ArcRwSignal, TimeoutHandle};

use crate::{options_combine, QueryClient, QueryOptions};

#[derive(Debug)]
pub(crate) struct Query<V> {
    value_maybe_stale: Option<V>, // Only None with steal_value_danger_must_replace().
    combined_options: QueryOptions,
    updated_at: chrono::DateTime<chrono::Utc>,
    gc_handle: GcHandle,
    pub buster: ArcRwSignal<u64>,
}

#[derive(Debug)]
enum GcHandle {
    None,
    #[allow(dead_code)]
    Wasm(TimeoutHandle),
    #[cfg(all(test, not(target_arch = "wasm32")))]
    #[allow(dead_code)]
    Tokio(tokio::task::JoinHandle<()>),
}

impl GcHandle {
    fn new(handler: impl FnOnce() + 'static, duration: Duration) -> Self {
        #[cfg(any(not(test), target_arch = "wasm32"))]
        {
            let handle = leptos::prelude::set_timeout_with_handle(handler, duration).expect("TODO");
            GcHandle::Wasm(handle)
        }
        #[cfg(all(test, not(target_arch = "wasm32")))]
        {
            // TODO not sure why this isn't working.
            // let handle = tokio::task::spawn_local(async move {
            //     // tokio::time::sleep(duration).await;
            //     // handler();
            // });
            // GcHandle::Tokio(handle)
            let _ = handler;
            let _ = duration;
            GcHandle::None
        }
    }

    fn cancel(&mut self) {
        match self {
            GcHandle::None => {}
            GcHandle::Wasm(handle) => handle.clear(),
            #[cfg(all(test, not(target_arch = "wasm32")))]
            GcHandle::Tokio(handle) => handle.abort(),
        }
        *self = GcHandle::None;
    }
}

/// Cancel the gc cleanup timeout if the query is dropped for any reason, e.g. invalidation or replacement with something new.
impl<V> Drop for Query<V> {
    fn drop(&mut self) {
        self.gc_handle.cancel();
    }
}

impl<V> Query<V> {
    pub fn new<K>(
        client: QueryClient,
        cache_key: TypeId,
        key: &K,
        value: V,
        buster: ArcRwSignal<u64>,
        scope_options: Option<QueryOptions>,
    ) -> Self
    where
        K: Clone + Eq + Hash + 'static,
        V: 'static,
    {
        let combined_options = options_combine(client.options(), scope_options);

        let gc_handle = if cfg!(not(feature = "ssr"))
            && combined_options.gc_time() < Duration::from_secs(60 * 60 * 24 * 365)
        {
            let key = key.clone();
            GcHandle::new(
                move || {
                    client.invalidate_queries_inner::<K, V, _>(cache_key, std::iter::once(&key));
                },
                combined_options.gc_time(),
            )
        } else {
            GcHandle::None
        };

        Self {
            value_maybe_stale: Some(value),
            combined_options,
            updated_at: chrono::Utc::now(),
            gc_handle,
            buster,
        }
    }

    pub fn value_if_not_stale(&self) -> Option<&V> {
        let stale_after = self.updated_at + self.combined_options.stale_time();
        if chrono::Utc::now() > stale_after {
            None
        } else {
            Some(
                self.value_maybe_stale
                    .as_ref()
                    .expect("Query value should never be None. (bug)"),
            )
        }
    }

    pub fn value_even_if_stale(&self) -> &V {
        self.value_maybe_stale
            .as_ref()
            .expect("Query value should never be None. (bug)")
    }

    pub fn steal_value_danger_must_replace(&mut self) -> V {
        self.value_maybe_stale
            .take()
            .expect("Query value should never be None. (bug)")
    }

    pub fn set_value(&mut self, new_value: V) {
        self.value_maybe_stale = Some(new_value);
        self.updated_at = chrono::Utc::now();
    }
}
