use std::{any::TypeId, collections::HashMap, sync::Arc};

use futures::{
    channel::mpsc::{UnboundedReceiver, UnboundedSender},
    StreamExt,
};
use parking_lot::Mutex;

use crate::{
    cache::ScopeLookup,
    utils::{new_subscription_id, DebugValue, KeyHash},
    QueryOptions,
};

// sub_id -> Sub
type Subs = Arc<Mutex<HashMap<u64, Sub>>>;

#[derive(Debug)]
pub(crate) struct ClientSubs {
    subs: Subs,
    scope_lookup: ScopeLookup,
}

#[derive(Debug, Clone)]
pub(crate) struct QueryCreatedInfo {
    pub cache_key: TypeId,
    pub scope_title: Arc<String>,
    pub key_hash: KeyHash,
    pub v_type_id: TypeId,
    pub debug_key: DebugValue,
    pub combined_options: QueryOptions,
}

pub(crate) struct QueryCreated {
    rx: UnboundedReceiver<QueryCreatedInfo>,
    _guard: SubDropGuard,
}

impl QueryCreated {
    pub async fn next(&mut self) -> Option<QueryCreatedInfo> {
        self.rx.next().await
    }
}

impl ClientSubs {
    pub fn new(scope_lookup: ScopeLookup) -> Self {
        Self {
            subs: Arc::new(Mutex::new(HashMap::new())),
            scope_lookup,
        }
    }

    pub fn add_query_created_subscription(&mut self, send_existing: bool) -> QueryCreated {
        let (tx, rx) = futures::channel::mpsc::unbounded();

        let sub_id = new_subscription_id();

        if send_existing {
            for (cache_key, scope) in self.scope_lookup.scopes().iter() {
                for query in scope.iter_dyn_queries() {
                    let _ = tx.unbounded_send(QueryCreatedInfo {
                        cache_key: *cache_key,
                        scope_title: scope.title().clone(),
                        key_hash: *query.key_hash(),
                        v_type_id: query.value_type_id(),
                        debug_key: query.debug_key().clone(),
                        combined_options: query.combined_options(),
                    });
                }
            }
        }

        self.subs
            .lock()
            .insert(sub_id, Sub::new(SubVariant::QueryCreated(tx)));

        QueryCreated {
            rx,
            // This will GC the subscriber on drop:
            _guard: SubDropGuard {
                subs: self.subs.clone(),
                sub_id,
            },
        }
    }

    pub fn notify_query_created(&mut self, info: QueryCreatedInfo) {
        for sub in self.subs.lock().values() {
            #[allow(irrefutable_let_patterns)]
            if let SubVariant::QueryCreated(tx) = &sub.variant {
                let _ = tx.unbounded_send(info.clone());
            }
        }
    }
}

#[derive(Debug)]
struct Sub {
    variant: SubVariant,
}

impl Sub {
    fn new(variant: SubVariant) -> Self {
        Self { variant }
    }
}

#[derive(Debug)]
#[non_exhaustive]
enum SubVariant {
    QueryCreated(UnboundedSender<QueryCreatedInfo>),
}

struct SubDropGuard {
    sub_id: u64,
    subs: Subs,
}

impl Drop for SubDropGuard {
    fn drop(&mut self) {
        self.subs.lock().remove(&self.sub_id);
    }
}
