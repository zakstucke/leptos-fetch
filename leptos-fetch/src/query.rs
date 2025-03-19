use std::{any::TypeId, collections::HashSet, fmt::Debug, hash::Hash, sync::Arc, time::Duration};

use leptos::prelude::{ArcRwSignal, Set};
use parking_lot::Mutex;
use send_wrapper::SendWrapper;

use crate::{
    maybe_local::MaybeLocal,
    options_combine,
    utils::{new_buster_id, KeyHash},
    value_with_callbacks::{GcHandle, GcValue, RefetchHandle},
    QueryClient, QueryOptions,
};

pub(crate) struct Query<V> {
    pub value_maybe_stale: GcValue<V>,
    combined_options: QueryOptions,
    // When None has been forcefully made stale/invalidated:
    updated_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Will always be None on the server, hence the SendWrapper is fine:
    gc_cb: Option<Arc<SendWrapper<Box<dyn Fn() -> bool>>>>,
    /// Will always be None on the server, hence the SendWrapper is fine:
    refetch_cb: Option<Arc<SendWrapper<Box<dyn Fn()>>>>,
    pub active_resources: Arc<Mutex<HashSet<u64>>>,
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
        key: &KeyHash,
        value: MaybeLocal<V>,
        buster: ArcRwSignal<u64>,
        scope_options: Option<QueryOptions>,
        active_resources: Option<Arc<Mutex<HashSet<u64>>>>,
    ) -> Self
    where
        K: Clone + Eq + Hash + 'static,
        V: 'static,
    {
        let combined_options = options_combine(client.options(), scope_options);
        let active_resources =
            active_resources.unwrap_or_else(|| Arc::new(Mutex::new(HashSet::new())));

        let gc_cb = if cfg!(any(test, not(feature = "ssr")))
            && combined_options.gc_time() < Duration::from_secs(60 * 60 * 24 * 365)
        {
            let key = *key;
            let active_resources = active_resources.clone();
            // GC is client only (non-ssr) hence can wrap in a SendWrapper:
            Some(Arc::new(SendWrapper::new(Box::new(move || {
                if active_resources.lock().is_empty() {
                    client.scope_lookup.gc_query::<K, V>(cache_key, &key);
                    true
                } else {
                    false
                }
            })
                as Box<dyn Fn() -> bool>)))
        } else {
            None
        };

        let refetch_cb = if cfg!(any(test, not(feature = "ssr")))
            && combined_options.refetch_interval().is_some()
        {
            let key = *key;
            // Refetching is client only (non-ssr) hence can wrap in a SendWrapper:
            Some(Arc::new(SendWrapper::new(Box::new(move || {
                client.scope_lookup.with_cached_scope_mut::<K, V, _>(
                    cache_key,
                    false,
                    |maybe_scope| {
                        // Invalidation will only trigger a refetch if there are active resources, hence fine to always call:
                        if let Some(scope) = maybe_scope {
                            if let Some(cached) = scope.get_mut_with_key_hash(&key) {
                                cached.invalidate();
                                cached.buster.set(new_buster_id());
                            }
                        }
                    },
                );
            }) as Box<dyn Fn()>)))
        } else {
            None
        };

        Self {
            value_maybe_stale: GcValue::new(
                value,
                GcHandle::new(gc_cb.clone(), combined_options.gc_time()),
                RefetchHandle::new(refetch_cb.clone(), combined_options.refetch_interval()),
            ),
            combined_options,
            updated_at: Some(chrono::Utc::now()),
            gc_cb,
            refetch_cb,
            active_resources,
            buster,
        }
    }

    pub fn mark_resource_active(&self, resource_id: u64) {
        self.active_resources.lock().insert(resource_id);
    }

    pub fn mark_resource_dropped(&self, resource_id: u64) {
        self.active_resources.lock().remove(&resource_id);
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

    pub fn set_value(&mut self, new_value: MaybeLocal<V>) {
        self.value_maybe_stale = GcValue::new(
            new_value,
            GcHandle::new(self.gc_cb.clone(), self.combined_options.gc_time()),
            RefetchHandle::new(
                self.refetch_cb.clone(),
                self.combined_options.refetch_interval(),
            ),
        );
        self.updated_at = Some(chrono::Utc::now());
    }
}
