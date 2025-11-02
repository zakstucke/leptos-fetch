#[cfg(any(
    all(debug_assertions, feature = "devtools"),
    feature = "devtools-always"
))]
use std::{collections::HashMap, sync::LazyLock};

use crate::{
    cache::{ScopeLookup, Scopes},
    query_scope::ScopeCacheKey,
    subs_scope::ScopeSubs,
    trie::Trie,
    utils::{KeyHash, new_scope_id},
};

pub(crate) static GLOBAL_SCOPE_LOOKUPS: LazyLock<parking_lot::RwLock<HashMap<u64, Scopes>>> =
    LazyLock::new(|| parking_lot::RwLock::new(HashMap::new()));

pub(crate) static GLOBAL_SCOPE_SUBSCRIPTION_LOOKUPS: LazyLock<
    parking_lot::Mutex<HashMap<u64, ScopeSubs>>,
> = LazyLock::new(|| parking_lot::Mutex::new(HashMap::new()));

pub(crate) static GLOBAL_INVALIDATION_TRIE: LazyLock<
    parking_lot::Mutex<HashMap<u64, Trie<(ScopeCacheKey, KeyHash)>>>,
> = LazyLock::new(|| parking_lot::Mutex::new(HashMap::new()));

#[cfg(any(
    all(debug_assertions, feature = "devtools"),
    feature = "devtools-always"
))]
pub(crate) static GLOBAL_CLIENT_SUBSCRIPTION_LOOKUPS: LazyLock<
    parking_lot::Mutex<HashMap<u64, crate::subs_client::ClientSubs>>,
> = LazyLock::new(|| parking_lot::Mutex::new(HashMap::new()));

#[cfg(test)]
pub fn does_scope_id_exist(scope_id: u64) -> bool {
    GLOBAL_SCOPE_LOOKUPS.read().contains_key(&scope_id)
}

impl ScopeLookup {
    pub(crate) fn new() -> Self {
        let scope_id = new_scope_id();

        let scope_lookup = Self { scope_id };

        GLOBAL_SCOPE_LOOKUPS
            .write()
            .insert(scope_lookup.scope_id, Default::default());
        GLOBAL_SCOPE_SUBSCRIPTION_LOOKUPS
            .lock()
            .insert(scope_lookup.scope_id, ScopeSubs::new(scope_lookup));
        GLOBAL_INVALIDATION_TRIE
            .lock()
            .insert(scope_lookup.scope_id, Default::default());
        #[cfg(any(
            all(debug_assertions, feature = "devtools"),
            feature = "devtools-always"
        ))]
        GLOBAL_CLIENT_SUBSCRIPTION_LOOKUPS.lock().insert(
            scope_lookup.scope_id,
            crate::subs_client::ClientSubs::new(scope_lookup),
        );

        println!(
            "Post insert of scope_id {scope_id}, cache size: {}",
            GLOBAL_SCOPE_LOOKUPS.read().len()
        );

        // If there is an owner, which there should be, and it's cleaned up,
        // gc the globals to prevent leaks.
        // i.e. server side with the root owner this should run at the end of every request.
        leptos::prelude::on_cleanup(move || {
            GLOBAL_SCOPE_LOOKUPS.write().remove(&scope_id);
            GLOBAL_SCOPE_SUBSCRIPTION_LOOKUPS.lock().remove(&scope_id);
            GLOBAL_INVALIDATION_TRIE.lock().remove(&scope_id);
            #[cfg(any(
                all(debug_assertions, feature = "devtools"),
                feature = "devtools-always"
            ))]
            GLOBAL_CLIENT_SUBSCRIPTION_LOOKUPS.lock().remove(&scope_id);
            println!("Cleaned up scope_id {scope_id}",);
        });

        scope_lookup
    }
}
