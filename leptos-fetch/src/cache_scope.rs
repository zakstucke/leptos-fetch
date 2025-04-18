use std::{collections::HashMap, sync::Arc, thread::ThreadId};

use crate::{
    cache::ScopeLookup, debug_if_devtools_enabled::DebugIfDevtoolsEnabled, query::Query,
    query_scope::QueryScopeInfo, utils::KeyHash,
};

#[derive(Debug)]
pub(crate) struct Scope<K: 'static, V: 'static> {
    threadsafe_cache: HashMap<KeyHash, Query<K, V>>,
    local_caches: HashMap<ThreadId, HashMap<KeyHash, Query<K, V>>>,
    // To make sure parallel fetches for the same key aren't happening across different resources.
    pub fetcher_mutexes: HashMap<KeyHash, Arc<futures::lock::Mutex<()>>>,
    scope_lookup: ScopeLookup,
    pub query_scope_info: QueryScopeInfo,
}

impl<K, V> Scope<K, V>
where
    K: DebugIfDevtoolsEnabled + 'static,
    V: 'static,
{
    pub fn new(scope_lookup: ScopeLookup, query_scope_info: QueryScopeInfo) -> Self {
        Self {
            threadsafe_cache: HashMap::new(),
            local_caches: HashMap::new(),
            fetcher_mutexes: HashMap::new(),
            scope_lookup,
            query_scope_info,
        }
    }

    pub fn all_queries(&self) -> impl Iterator<Item = &Query<K, V>> {
        self.threadsafe_cache.values().chain(
            self.local_caches
                .get(&std::thread::current().id())
                .into_iter()
                .flat_map(|local_cache| local_cache.values()),
        )
    }

    pub fn all_queries_mut(&mut self) -> impl Iterator<Item = &mut Query<K, V>> {
        self.threadsafe_cache.values_mut().chain(
            self.local_caches
                .get_mut(&std::thread::current().id())
                .into_iter()
                .flat_map(|local_cache| local_cache.values_mut()),
        )
    }

    pub fn insert_without_query_created_notif(&mut self, key_hash: KeyHash, query: Query<K, V>) {
        if query.value_maybe_stale().is_local() {
            self.local_caches
                .entry(std::thread::current().id())
                .or_default()
                .insert(key_hash, query);
        } else {
            self.threadsafe_cache.insert(key_hash, query);
        }
        self.scope_lookup
            .scope_subscriptions_mut()
            .notify_value_set_updated_or_removed::<V>(self.query_scope_info.cache_key, key_hash);
    }

    pub fn insert(&mut self, key_hash: KeyHash, query: Query<K, V>) {
        #[cfg(any(
            all(debug_assertions, feature = "devtools"),
            feature = "devtools-always"
        ))]
        {
            let info = crate::subs_client::QueryCreatedInfo {
                cache_key: self.query_scope_info.cache_key,
                scope_title: self.query_scope_info.title.clone(),
                key_hash,
                // SAFETY: query just created, so same thread
                debug_key: crate::utils::DebugValue::new(query.key().value_may_panic()),
                v_type_id: std::any::TypeId::of::<V>(),
                combined_options: query.combined_options,
            };
            self.insert_without_query_created_notif(key_hash, query);
            self.scope_lookup
                .client_subscriptions_mut()
                .notify_query_created(info);
        }
        #[cfg(not(any(
            all(debug_assertions, feature = "devtools"),
            feature = "devtools-always"
        )))]
        {
            self.insert_without_query_created_notif(key_hash, query);
        }
    }

    pub fn get(&self, key_hash: &KeyHash) -> Option<&Query<K, V>> {
        // Threadsafe always takes priority:
        self.threadsafe_cache.get(key_hash).or_else(|| {
            self.local_caches
                .get(&std::thread::current().id())
                .and_then(|local_cache| local_cache.get(key_hash))
        })
    }

    pub fn get_mut(&mut self, key_hash: &KeyHash) -> Option<&mut Query<K, V>> {
        // Threadsafe always takes priority:
        self.threadsafe_cache.get_mut(key_hash).or_else(|| {
            self.local_caches
                .get_mut(&std::thread::current().id())
                .and_then(|local_cache| local_cache.get_mut(key_hash))
        })
    }

    /// Should use this and the threadsafe one if updating the value, have to be separated.
    pub fn get_mut_local_only(&mut self, key_hash: &KeyHash) -> Option<&mut Query<K, V>> {
        self.local_caches
            .get_mut(&std::thread::current().id())
            .and_then(|local_cache| local_cache.get_mut(key_hash))
    }

    /// Should use this and the threadsafe one if updating the value, have to be separated.
    pub fn get_mut_threadsafe_only(&mut self, key_hash: &KeyHash) -> Option<&mut Query<K, V>> {
        self.threadsafe_cache.get_mut(key_hash)
    }

    pub fn contains_key(&self, key_hash: &KeyHash) -> bool {
        if self.threadsafe_cache.contains_key(key_hash) {
            true
        } else {
            self.local_caches
                .get(&std::thread::current().id())
                .is_some_and(|local_cache| local_cache.contains_key(key_hash))
        }
    }

    pub fn remove_entry(&mut self, key_hash: &KeyHash) -> Option<(KeyHash, Query<K, V>)> {
        let result = if let Some(query) = self.threadsafe_cache.remove_entry(key_hash) {
            // Threadsafe always takes priority:
            Some(query)
        } else if let Some(local_cache) = self.local_caches.get_mut(&std::thread::current().id()) {
            local_cache.remove_entry(key_hash)
        } else {
            None
        };
        result
    }

    pub fn is_empty(&self) -> bool {
        self.threadsafe_cache.is_empty()
            && self
                .local_caches
                .get(&std::thread::current().id())
                .is_some_and(|local_cache| local_cache.is_empty())
    }

    pub fn clear_cache(&mut self) {
        self.threadsafe_cache.clear();
        if let Some(local_cache) = self.local_caches.get_mut(&std::thread::current().id()) {
            local_cache.clear();
        }
    }

    #[cfg(test)]
    pub fn cache_size(&self) -> usize {
        std::iter::once(&self.threadsafe_cache)
            .chain(self.local_caches.get(&std::thread::current().id()))
            .map(|cache| cache.len())
            .sum()
    }
}
