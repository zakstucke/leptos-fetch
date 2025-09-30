use std::{
    collections::HashMap,
    sync::{Arc, atomic::AtomicBool},
};

use chrono::Utc;
use futures::FutureExt;
use leptos::{prelude::*, task::spawn_local};

use crate::{
    QueryClient, QueryOptions,
    events::Event,
    maybe_local::MaybeLocal,
    query_scope::ScopeCacheKey,
    subs_client::QueryCreatedInfo,
    utils::{DebugValue, KeyHash, new_buster_id, safe_set_timeout},
};

use super::{components::ColorOption, sort::SortConfig};

#[derive(Clone, Debug, Default)]
pub(crate) struct CacheRep {
    pub all_queries: ArcRwSignal<HashMap<ScopeCacheKey, QueriesRep>>,
}

impl CacheRep {
    pub fn remove_query(&self, cache_key: ScopeCacheKey, key_hash: KeyHash) {
        let mut guard = self.all_queries.write();
        let remove_type = if let Some(queries) = guard.get_mut(&cache_key) {
            let mut query_guard = queries.scoped_queries.write();
            query_guard.remove(&key_hash);
            query_guard.is_empty()
        } else {
            false
        };
        if remove_type {
            guard.remove(&cache_key);
        }
    }
}

#[derive(Clone, Debug)]
pub struct QueriesRep {
    pub title: Arc<String>,
    pub sort_config: ArcRwSignal<SortConfig>,
    pub filter: ArcRwSignal<String>,
    pub scoped_queries: ArcRwSignal<HashMap<KeyHash, QueryRep>>,
}

#[derive(Clone, Debug)]
pub struct ValueDerivs {
    pub debug_value: DebugValue,
    pub updated_at: chrono::DateTime<Utc>,
    pub is_gced: bool,
}

#[derive(Clone, Debug)]
pub enum QueryState {
    Loading,
    Fetching,
    Stale,
    Invalid,
    Fresh,
}

impl QueryState {
    pub fn color(&self) -> ColorOption {
        match self {
            QueryState::Loading | QueryState::Fetching => ColorOption::Blue,
            QueryState::Stale => ColorOption::Yellow,
            QueryState::Fresh => ColorOption::Green,
            QueryState::Invalid => ColorOption::Red,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            QueryState::Loading => "Loading",
            QueryState::Fetching => "Fetching",
            QueryState::Fresh => "Fresh",
            QueryState::Invalid => "Invalid",
            QueryState::Stale => "Stale",
        }
    }
}

#[derive(Clone, Debug)]
pub struct QueryRep {
    pub cache_key: ScopeCacheKey,
    pub key_hash: KeyHash,
    pub debug_key: DebugValue,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub combined_options: QueryOptions,

    pub value_derivs: ArcSignal<ValueDerivs>,

    pub state: ArcSignal<QueryState>,

    pub active_resources: ArcSignal<usize>,
    pub events: ArcSignal<Vec<Event>>,
}

pub fn prepare<Codec>(client: QueryClient<Codec>) -> CacheRep {
    let cache_rep = CacheRep::default();
    if cfg!(not(feature = "ssr")) {
        let add_query = {
            let cache_rep = cache_rep.clone();
            move |query_info: QueryCreatedInfo| {
                let mut scope_subscriptions_mut = client.scope_lookup.scope_subscriptions_mut();

                let value_set_updated_or_removed_signal = scope_subscriptions_mut
                    .add_value_set_updated_or_removed_subscription(
                        query_info.cache_key,
                        MaybeLocal::new(move || Some(query_info.key_hash)),
                    );

                let value_derivs = {
                    let value_set_updated_or_removed_signal =
                        value_set_updated_or_removed_signal.clone();
                    let value_existed_at_one_point = AtomicBool::new(false);
                    ArcSignal::derive(move || {
                        // Will be None when the query has been gc'd:
                        if value_set_updated_or_removed_signal.get().is_some()
                            && let Some(scope) =
                                client.scope_lookup.scopes().get(&query_info.cache_key)
                            && let Some(dyn_query) = scope.get_dyn_query(&query_info.key_hash)
                        {
                            // So is_gced isn't true during the initial period where the query hasn't finished fetching yet:
                            value_existed_at_one_point
                                .store(true, std::sync::atomic::Ordering::Relaxed);
                            return ValueDerivs {
                                // WONTPANIC: always on single-threaded client
                                debug_value: dyn_query.debug_value_may_panic(),
                                updated_at: dyn_query.updated_at(),
                                is_gced: false,
                            };
                        };
                        ValueDerivs {
                            debug_value: DebugValue::new(&"Fetching..."),
                            updated_at: Utc::now(),
                            // So is_gced isn't true during the initial period where the query hasn't finished fetching yet:
                            is_gced: value_existed_at_one_point
                                .load(std::sync::atomic::Ordering::Relaxed),
                        }
                    })
                };

                let events = scope_subscriptions_mut.add_events_subscription(
                    query_info.cache_key,
                    MaybeLocal::new(ArcSignal::derive(move || Some(query_info.key_hash))),
                );

                let is_invalidated = {
                    let events = events.clone();
                    let value_set_updated_or_removed_signal =
                        value_set_updated_or_removed_signal.clone();
                    ArcSignal::derive(move || {
                        // Events are the only subscription that updates on invalidated:
                        events.track();
                        if value_set_updated_or_removed_signal.get().is_some()
                            && let Some(scope) =
                                client.scope_lookup.scopes().get(&query_info.cache_key)
                            && let Some(dyn_query) = scope.get_dyn_query(&query_info.key_hash)
                        {
                            return dyn_query.is_invalidated();
                        }
                        false
                    })
                };

                let is_invalidated = is_invalidated.clone();
                let is_fetching = {
                    scope_subscriptions_mut.add_is_fetching_subscription(
                        query_info.cache_key,
                        MaybeLocal::new(ArcSignal::derive(move || Some(query_info.key_hash))),
                    )
                };
                let is_loading = {
                    scope_subscriptions_mut.add_is_loading_subscription(
                        query_info.cache_key,
                        MaybeLocal::new(ArcSignal::derive(move || Some(query_info.key_hash))),
                    )
                };
                let is_stale = {
                    let is_stale_recheck_handle = ArcRwSignal::<Option<TimeoutHandle>>::new(None);
                    let is_stale_buster = ArcRwSignal::new(new_buster_id());
                    ArcSignal::derive(move || {
                        is_stale_buster.get();

                        // Cancel any previous stale recheck:
                        if let Some(handle) = is_stale_recheck_handle.read_untracked().as_ref() {
                            handle.clear();
                        }

                        // Will be None when the query has been gc'd:
                        if value_set_updated_or_removed_signal.get().is_some()
                            && let Some(scope) =
                                client.scope_lookup.scopes().get(&query_info.cache_key)
                            && let Some(dyn_query) = scope.get_dyn_query(&query_info.key_hash)
                        {
                            if let Some(till_stale) = dyn_query.till_stale() {
                                let handle = safe_set_timeout(
                                    {
                                        let is_stale_buster = is_stale_buster.clone();
                                        move || {
                                            is_stale_buster.set(new_buster_id());
                                        }
                                    },
                                    till_stale,
                                );
                                is_stale_recheck_handle.set(Some(handle));
                            } else {
                                return true;
                            }
                        };
                        false
                    })
                };

                let state = ArcSignal::derive(move || {
                    if is_loading.get() {
                        QueryState::Loading
                    } else if is_fetching.get() {
                        QueryState::Fetching
                    } else if is_invalidated.get() {
                        QueryState::Invalid
                    } else if is_stale.get() {
                        QueryState::Stale
                    } else {
                        QueryState::Fresh
                    }
                });

                let query_rep = QueryRep {
                    cache_key: query_info.cache_key,
                    key_hash: query_info.key_hash,
                    debug_key: query_info.debug_key,
                    combined_options: query_info.combined_options,
                    state,
                    created_at: chrono::Utc::now(),
                    value_derivs: value_derivs.clone(),
                    active_resources: scope_subscriptions_mut.add_active_resources_subscription(
                        query_info.cache_key,
                        MaybeLocal::new(ArcSignal::derive(move || Some(query_info.key_hash))),
                    ),
                    events,
                };

                cache_rep
                    .all_queries
                    .write()
                    .entry(query_info.cache_key)
                    .or_insert_with(|| QueriesRep {
                        title: query_info.scope_title,
                        sort_config: ArcRwSignal::new(SortConfig::new_default_updated_at()),
                        filter: ArcRwSignal::new("".to_string()),
                        scoped_queries: ArcRwSignal::new(HashMap::new()),
                    })
                    .scoped_queries
                    .write()
                    .insert(query_info.key_hash, query_rep);
            }
        };

        let mut sub_query_created = client
            .scope_lookup
            .client_subscriptions_mut()
            .add_query_created_subscription(true);

        let (cleaned_up_tx, cleaned_up_rx) = futures::channel::oneshot::channel::<()>();

        on_cleanup(move || {
            let _ = cleaned_up_tx.send(());
        });

        spawn_local({
            async move {
                futures::select!(
                    _ = cleaned_up_rx.fuse() => {},
                    _ = async move {
                        while let Some(query_info) = sub_query_created.next().await {
                            add_query(query_info);
                        }
                    }.fuse() => {}
                );
            }
        });
    }
    cache_rep
}
