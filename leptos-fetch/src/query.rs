use std::{any::TypeId, fmt::Debug, hash::Hash, sync::Arc, time::Duration};

use leptos::prelude::ArcRwSignal;
use send_wrapper::SendWrapper;

use crate::{
    gc::{GcHandle, GcValue},
    options_combine, QueryClient, QueryOptions,
};

pub(crate) struct Query<V> {
    pub value_maybe_stale: GcValue<V>,
    combined_options: QueryOptions,
    // When None has been forcefully made stale/invalidated:
    updated_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Will always be None on the server, hence the SendWrapper is fine:
    gc_cb: Option<Arc<SendWrapper<Box<dyn Fn()>>>>,
    pub buster: ArcRwSignal<u64>,
}

impl<V> Debug for Query<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Query")
            .field("value", &std::any::type_name::<V>())
            .field("combined_options", &self.combined_options)
            .field("updated_at", &self.updated_at)
            .finish()
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

        let gc_cb = if cfg!(any(test, not(feature = "ssr")))
            && combined_options.gc_time() < Duration::from_secs(60 * 60 * 24 * 365)
        {
            let key = key.clone();
            // GC is client only (non-ssr) hence can wrap in a SendWrapper:
            Some(Arc::new(SendWrapper::new(Box::new(move || {
                client.scope_lookup.gc_query::<K, V>(cache_key, &key);
            }) as Box<dyn Fn()>)))
        } else {
            None
        };

        Self {
            value_maybe_stale: GcValue::new(
                value,
                GcHandle::new(gc_cb.clone(), combined_options.gc_time()),
            ),
            combined_options,
            updated_at: Some(chrono::Utc::now()),
            gc_cb,
            buster,
        }
    }

    pub fn invalidate(&mut self) {
        self.updated_at = None;
    }

    pub fn stale(&self) -> bool {
        if let Some(updated_at) = self.updated_at {
            let stale_after = updated_at + self.combined_options.stale_time();
            chrono::Utc::now() > stale_after
        } else {
            true
        }
    }

    pub fn set_value(&mut self, new_value: V) {
        self.value_maybe_stale = GcValue::new(
            new_value,
            GcHandle::new(self.gc_cb.clone(), self.combined_options.gc_time()),
        );
        self.updated_at = Some(chrono::Utc::now());
    }
}
