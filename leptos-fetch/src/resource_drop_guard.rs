use std::sync::Arc;

use parking_lot::Mutex;

use crate::{
    cache::ScopeLookup, debug_if_devtools_enabled::DebugIfDevtoolsEnabled,
    query_scope::ScopeCacheKey, utils::KeyHash,
};

#[derive(Clone)]
pub struct ResourceDropGuard<K, V>(Arc<Mutex<ResourceDropGuardInner<K, V>>>)
where
    K: DebugIfDevtoolsEnabled + Clone + 'static,
    V: 'static;

impl<K, V> ResourceDropGuard<K, V>
where
    K: DebugIfDevtoolsEnabled + Clone + 'static,
    V: 'static,
{
    pub fn new(scope_lookup: ScopeLookup, resource_id: u64, cache_key: ScopeCacheKey) -> Self {
        ResourceDropGuard(Arc::new(Mutex::new(ResourceDropGuardInner::<K, V> {
            scope_lookup,
            resource_id,
            active_key_hash: None,
            cache_key,
            _k: std::marker::PhantomData,
            _v: std::marker::PhantomData,
        })))
    }

    pub fn set_active_key(&self, active_key_hash: KeyHash) {
        let mut guard = self.0.lock();
        // If key changing, should mark the previous as dropped:
        if let Some(old_active_key_hash) = guard.active_key_hash
            && old_active_key_hash != active_key_hash
        {
            guard.scope_lookup.mark_resource_dropped::<K, V>(
                &old_active_key_hash,
                &guard.cache_key,
                guard.resource_id,
            );
        }
        guard.active_key_hash = Some(active_key_hash);
    }
}

struct ResourceDropGuardInner<K, V>
where
    K: DebugIfDevtoolsEnabled + Clone + 'static,
    V: 'static,
{
    scope_lookup: ScopeLookup,
    resource_id: u64,
    active_key_hash: Option<KeyHash>,
    cache_key: ScopeCacheKey,
    _k: std::marker::PhantomData<K>,
    _v: std::marker::PhantomData<V>,
}

impl<K, V> Drop for ResourceDropGuardInner<K, V>
where
    K: DebugIfDevtoolsEnabled + Clone + 'static,
    V: 'static,
{
    fn drop(&mut self) {
        if let Some(active_key_hash) = self.active_key_hash.take() {
            self.scope_lookup.mark_resource_dropped::<K, V>(
                &active_key_hash,
                &self.cache_key,
                self.resource_id,
            );
        }
    }
}
