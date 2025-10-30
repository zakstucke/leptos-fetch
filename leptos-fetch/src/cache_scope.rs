use std::{collections::HashMap, sync::Arc, thread::ThreadId};

use crate::{
    cache::{ScopeLookup, Scopes},
    debug_if_devtools_enabled::DebugIfDevtoolsEnabled,
    maybe_local::MaybeLocal,
    query::Query,
    query_scope::QueryScopeInfo,
    utils::KeyHash,
};

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum QueryOrPending<K, V: 'static> {
    Query(Query<K, V>),
    Pending {
        query_abort_tx: Option<futures::channel::oneshot::Sender<QueryAbortReason>>,
        key: MaybeLocal<K>,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum QueryAbortReason {
    Clear,
    Invalidate,
    // Scenario: LocalResource, prefetch/fetch_query etc, starts running on client,
    // same query used in a resource on server, that streams over and is the actual value to be used.
    // need to invalidate pending query that had been triggered by the local query running,
    // before realising there was a value being streamed from the server.
    SsrStreamedValueOverride,
}

impl<K, V> QueryOrPending<K, V> {
    pub fn as_query(&self) -> Option<&Query<K, V>> {
        if let QueryOrPending::Query(query) = self {
            Some(query)
        } else {
            None
        }
    }

    pub fn as_query_mut(&mut self) -> Option<&mut Query<K, V>> {
        if let QueryOrPending::Query(query) = self {
            Some(query)
        } else {
            None
        }
    }

    pub fn into_query(self) -> Option<Query<K, V>> {
        if let QueryOrPending::Query(query) = self {
            Some(query)
        } else {
            None
        }
    }

    pub fn invalidate(
        &mut self,
        invalidation_type: QueryAbortReason,
    ) -> impl FnOnce(&mut Scopes) + 'static + use<K, V> {
        let mut inner_cb = None;
        match self {
            QueryOrPending::Pending { query_abort_tx, .. } => {
                // Invalidate any in-flight fetch if there is still one:
                if let Some(query_abort_tx) = query_abort_tx.take() {
                    let _ = query_abort_tx.send(invalidation_type);
                }
            }
            QueryOrPending::Query(query) => {
                inner_cb = Some(query.invalidate(invalidation_type));
            }
        }
        move |scopes| {
            if let Some(cb) = inner_cb {
                cb(scopes);
            }
        }
    }

    pub fn key(&self) -> &MaybeLocal<K> {
        match self {
            QueryOrPending::Query(query) => query.key(),
            QueryOrPending::Pending { key, .. } => key,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Scope<K: 'static, V: 'static> {
    threadsafe_cache: HashMap<KeyHash, QueryOrPending<K, V>>,
    local_caches: HashMap<ThreadId, HashMap<KeyHash, QueryOrPending<K, V>>>,
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
        self.threadsafe_cache
            .values()
            .filter_map(QueryOrPending::as_query)
            .chain(
                self.local_caches
                    .get(&std::thread::current().id())
                    .into_iter()
                    .flat_map(|local_cache| {
                        local_cache.values().filter_map(QueryOrPending::as_query)
                    }),
            )
    }

    pub fn all_queries_mut_include_pending(
        &mut self,
    ) -> impl Iterator<Item = &mut QueryOrPending<K, V>> {
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
                .insert(key_hash, QueryOrPending::Query(query));
        } else {
            self.threadsafe_cache
                .insert(key_hash, QueryOrPending::Query(query));
        }
        self.scope_lookup
            .scope_subscriptions_mut()
            .notify_value_set_updated_or_removed(self.query_scope_info.cache_key, key_hash);
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

    pub fn insert_pending(
        &mut self,
        key: MaybeLocal<K>,
        query_abort_tx: futures::channel::oneshot::Sender<QueryAbortReason>,
        key_hash: KeyHash,
    ) {
        let is_local = key.is_local();
        let pending = QueryOrPending::Pending {
            query_abort_tx: Some(query_abort_tx),
            key,
        };
        if is_local {
            self.local_caches
                .entry(std::thread::current().id())
                .or_default()
                .insert(key_hash, pending);
        } else {
            self.threadsafe_cache.insert(key_hash, pending);
        }
    }

    pub fn get(&self, key_hash: &KeyHash) -> Option<&Query<K, V>> {
        // Threadsafe always takes priority:
        self.threadsafe_cache
            .get(key_hash)
            .or_else(|| {
                self.local_caches
                    .get(&std::thread::current().id())
                    .and_then(|local_cache| local_cache.get(key_hash))
            })
            .and_then(QueryOrPending::as_query)
    }

    pub fn get_mut(&mut self, key_hash: &KeyHash) -> Option<&mut Query<K, V>> {
        self.get_mut_include_pending(key_hash)
            .and_then(QueryOrPending::as_query_mut)
    }

    pub fn get_mut_include_pending(
        &mut self,
        key_hash: &KeyHash,
    ) -> Option<&mut QueryOrPending<K, V>> {
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
            .and_then(QueryOrPending::as_query_mut)
    }

    /// Should use this and the threadsafe one if updating the value, have to be separated.
    pub fn get_mut_threadsafe_only(&mut self, key_hash: &KeyHash) -> Option<&mut Query<K, V>> {
        self.threadsafe_cache
            .get_mut(key_hash)
            .and_then(QueryOrPending::as_query_mut)
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
        // Threadsafe always takes priority:
        if let Some((key_hash, query)) = self.threadsafe_cache.remove_entry(key_hash) {
            query.into_query().map(|query| (key_hash, query))
        } else if let Some(local_cache) = self.local_caches.get_mut(&std::thread::current().id()) {
            local_cache
                .remove_entry(key_hash)
                .and_then(|(key_hash, query)| query.into_query().map(|query| (key_hash, query)))
        } else {
            None
        }
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
