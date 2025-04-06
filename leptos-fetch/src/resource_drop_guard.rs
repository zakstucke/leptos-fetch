use std::{any::TypeId, sync::Arc};

use parking_lot::Mutex;

use crate::{cache::ScopeLookup, utils::KeyHash};

#[derive(Clone)]
pub struct ResourceDropGuard<V>(Arc<Mutex<ResourceDropGuardInner<V>>>)
where
    V: 'static;

impl<V> ResourceDropGuard<V>
where
    V: 'static,
{
    pub fn new(scope_lookup: ScopeLookup, resource_id: u64, cache_key: TypeId) -> Self {
        ResourceDropGuard(Arc::new(Mutex::new(ResourceDropGuardInner::<V> {
            scope_lookup,
            resource_id,
            active_key_hash: None,
            cache_key,
            _phantom: std::marker::PhantomData,
        })))
    }

    pub fn set_active_key(&self, active_key_hash: KeyHash) {
        let mut guard = self.0.lock();
        // If key changing, should mark the previous as dropped:
        if let Some(active_key_hash) = guard.active_key_hash {
            guard.scope_lookup.mark_resource_dropped::<V>(
                &active_key_hash,
                &guard.cache_key,
                guard.resource_id,
            );
        }
        guard.active_key_hash = Some(active_key_hash);
    }
}

struct ResourceDropGuardInner<V>
where
    V: 'static,
{
    scope_lookup: ScopeLookup,
    resource_id: u64,
    active_key_hash: Option<KeyHash>,
    cache_key: TypeId,
    _phantom: std::marker::PhantomData<V>,
}

impl<V> Drop for ResourceDropGuardInner<V>
where
    V: 'static,
{
    fn drop(&mut self) {
        if let Some(active_key_hash) = self.active_key_hash.take() {
            self.scope_lookup.mark_resource_dropped::<V>(
                &active_key_hash,
                &self.cache_key,
                self.resource_id,
            );
        }
    }
}
