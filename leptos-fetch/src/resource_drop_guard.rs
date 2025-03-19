use std::{any::TypeId, hash::Hash, sync::Arc};

use parking_lot::Mutex;

use crate::cache::ScopeLookup;

#[derive(Clone)]
pub struct ResourceDropGuard<K, V>(Arc<Mutex<ResourceDropGuardInner<K, V>>>)
where
    K: Eq + Hash + 'static,
    V: 'static;

impl<K, V> ResourceDropGuard<K, V>
where
    K: Eq + Hash + 'static,
    V: 'static,
{
    pub fn new(scope_lookup: ScopeLookup, resource_id: u64, cache_key: TypeId) -> Self {
        ResourceDropGuard(Arc::new(Mutex::new(ResourceDropGuardInner::<K, V> {
            scope_lookup,
            resource_id,
            active_key: None,
            cache_key,
            _phantom: std::marker::PhantomData,
        })))
    }

    pub fn set_active_key(&self, active_key: K) {
        self.0.lock().active_key = Some(active_key);
    }
}

struct ResourceDropGuardInner<K, V>
where
    K: Eq + Hash + 'static,
    V: 'static,
{
    scope_lookup: ScopeLookup,
    resource_id: u64,
    active_key: Option<K>,
    cache_key: TypeId,
    _phantom: std::marker::PhantomData<V>,
}

impl<K, V> Drop for ResourceDropGuardInner<K, V>
where
    K: Eq + Hash + 'static,
    V: 'static,
{
    fn drop(&mut self) {
        if let Some(active_key) = self.active_key.take() {
            self.scope_lookup.mark_resource_dropped::<K, V>(
                active_key,
                &self.cache_key,
                self.resource_id,
            );
        }
    }
}
