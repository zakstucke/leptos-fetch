use std::collections::BTreeMap;
use std::time::Duration;
use std::{hash::Hash, sync::Arc};

use leptos::prelude::*;

use crate::{
    PaginatedPageKey, QueryOptions, QueryScope, QueryScopeLocal, UntypedQueryClient,
    cache::OnScopeMissing, debug_if_devtools_enabled::DebugIfDevtoolsEnabled,
    query_scope::ScopeCacheKey,
};

macro_rules! define {
    ([$($impl_fut_generics:tt)*], [$($impl_fn_generics:tt)*], $name:ident, $fetch_fn:ident, $sname:literal) => {

        impl<Key, PageItem> $name<PaginatedPageKey<Key>, Option<(Vec<PageItem>, bool)>>
        where
            Key: DebugIfDevtoolsEnabled + Clone + Hash + PartialEq + 'static $($impl_fn_generics)*,
            PageItem: DebugIfDevtoolsEnabled + Clone + 'static $($impl_fn_generics)*,
        {
            /// TODO
            pub fn new_paginated_with_offset<Cursor, Fut>(
                getter: impl Fn(Key, usize, u64) -> Fut + 'static $($impl_fn_generics)*,
            ) -> $name<PaginatedPageKey<Key>, Option<(Vec<PageItem>, Option<u64>)>>
            where
                Cursor: DebugIfDevtoolsEnabled + Clone + 'static $($impl_fn_generics)*,
                Fut: Future<Output = (Vec<PageItem>, Option<u64>)> $($impl_fut_generics)*,
            {
                let getter = Arc::new(getter);
                let backing_cache_scope = $name::new({
                    let getter = getter.clone();
                    move |key: KeyWithUnhashedItemCountAndOffsetRequested<Key>| {
                        let getter = getter.clone();
                        async move {
                            let (items, mut maybe_total_items) = getter(key.key, key.item_count_requested, key.offset_requested).await;

                            // Protect against incorrect item count when clearly no items left:
                            if let Some(cur_total_items) = &mut maybe_total_items {
                                if *cur_total_items > key.offset_requested {
                                    *cur_total_items = key.offset_requested;
                                }
                            } else {
                                maybe_total_items = Some(key.offset_requested);
                            }

                            BackingCache {
                                inner: Arc::new(BackingCacheInner {
                                    items: SparseItems {
                                        items: parking_lot::Mutex::new(
                                            items
                                                .into_iter()
                                                .enumerate()
                                                .map(|(idx, item)| (key.offset_requested + (idx as u64), item))
                                                .collect(),
                                        ),
                                    },
                                    maybe_total_items: parking_lot::Mutex::new(maybe_total_items),
                                    update_lock: futures::lock::Mutex::new(()),
                                }),
                            }
                        }
                    }
                })
                // Shouldn't itself expire, the paginated query will control that:
                .with_options(
                    QueryOptions::default()
                        .with_stale_time(Duration::MAX)
                        .with_gc_time(Duration::MAX)
                        .with_refetch_interval(Duration::MAX)
                );

                $name::new({
                    let backing_cache_scope = backing_cache_scope.clone();
                    move |page_key: PaginatedPageKey<Key>| {
                        let backing_cache_scope = backing_cache_scope.clone();
                        let getter = getter.clone();
                        async move {
                            let untyped_client = use_context::<UntypedQueryClient>()
                                .expect(
                                    "leptos-fetch bug, UntypedQueryClient should always have been \
                                    provided to the query context internally"
                            );
                            let scope_cache_key = use_context::<ScopeCacheKey>()
                                .expect(
                                    "leptos-fetch bug, ScopeCacheKey itself should always have been \
                                    provided to the query context internally"
                                );

                            // If this query is reloading because it was stale, should invalidate the backing cache before reading it,
                            // otherwise will still get back the same stale data again:
                            if let Some(metadata) = untyped_client.query_metadata::<PaginatedPageKey<Key>, Option<(Vec<PageItem>, bool)>>(
                                scope_cache_key,
                                &page_key,
                            )
                            && metadata.stale_or_invalidated
                            && let Some(backing_metadata) = untyped_client.query_metadata::<KeyWithUnhashedItemCountAndOffsetRequested<Key>, BackingCache<PageItem>>(
                                backing_cache_scope.cache_key,
                                &KeyWithUnhashedItemCountAndOffsetRequested {
                                    key: page_key.key.clone(),
                                    item_count_requested: 0, // Doesn't matter, it's not part of the hash
                                    offset_requested: 0, // Doesn't matter, it's not part of the hash
                                },
                            )
                            && backing_metadata.updated_at <= metadata.updated_at
                            {
                                untyped_client.invalidate_query(
                                    &backing_cache_scope,
                                    KeyWithUnhashedItemCountAndOffsetRequested {
                                        key: page_key.key.clone(),
                                        item_count_requested: 0, // Doesn't matter, it's not part of the hash
                                        offset_requested: 0, // Doesn't matter, it's not part of the hash
                                    },
                                );
                            }

                            let infinite_cache = untyped_client
                                .$fetch_fn(
                                    backing_cache_scope,
                                    KeyWithUnhashedItemCountAndOffsetRequested {
                                        key: page_key.key.clone(),
                                        item_count_requested: page_key.page_size,
                                        offset_requested: (page_key.page_index as u64) * (page_key.page_size as u64)
                                    },
                                )
                                .await;

                            let target_idx_start = (page_key.page_index as u64) * (page_key.page_size as u64);
                            let mut target_idx_end_exclusive = ((page_key.page_index as u64) + 1) * (page_key.page_size as u64);
                            if let Some(maybe_total_items) = *infinite_cache.inner.maybe_total_items.lock() {
                                if target_idx_end_exclusive > maybe_total_items {
                                    target_idx_end_exclusive = maybe_total_items;
                                }
                            }

                            // Load x more if needed:
                            let should_request_range = || {
                                !infinite_cache.inner.items.check_range(target_idx_start, target_idx_end_exclusive)
                            };

                            if should_request_range() {
                                // Preventing multiple simultaneous fetches by holding an async lock,
                                // but note making sure to not hold any sync locks across await boundaries.
                                let mut _guard = infinite_cache.inner.update_lock.lock().await;
                                while should_request_range() {
                                    let (items, maybe_total_items) =
                                        getter(page_key.key.clone(), (target_idx_end_exclusive - target_idx_start) as usize, target_idx_start).await;
                                    if !items.is_empty() {
                                        infinite_cache.inner.items.extend(target_idx_start, items);
                                        *infinite_cache.inner.maybe_total_items.lock() = maybe_total_items;
                                    } else {
                                        // Protect against incorrect item count when clearly no items left:
                                        let mut guard = infinite_cache.inner.maybe_total_items.lock();
                                        if let Some(cur_total_items) = &mut* guard {
                                            if *cur_total_items > target_idx_start {
                                                *cur_total_items = target_idx_start;
                                            }
                                        } else {
                                            *guard = Some(target_idx_start);
                                        }
                                    }
                                }
                                drop(_guard);
                            }

                            infinite_cache.inner.items.get_range(target_idx_start, (target_idx_end_exclusive - target_idx_start) as usize).map(|items| {
                                (items, infinite_cache.inner.maybe_total_items.lock().clone())
                            })
                        }
                    }
                })
                // An invalidation or clear of any page should invalidate the backing cache:
                .on_invalidation({
                    let backing_cache_scope = backing_cache_scope.clone();
                    move |key| {
                        let untyped_client = use_context::<UntypedQueryClient>()
                            .expect(
                                "leptos-fetch bug, UntypedQueryClient should always have been \
                                provided to the on_invalidation context internally"
                            );
                        untyped_client.invalidate_query(
                            &backing_cache_scope,
                            KeyWithUnhashedItemCountAndOffsetRequested {
                                key: key.key.clone(),
                                item_count_requested: 0, // Doesn't matter, it's not part of the hash
                                offset_requested: 0, // Doesn't matter, it's not part of the hash
                            },
                        );
                    }
                })
                // If this was the last page existing now being gc'd, clear the backing cache too:
                .on_gc(move |key| {
                    let untyped_client = use_context::<UntypedQueryClient>()
                        .expect(
                            "leptos-fetch bug, UntypedQueryClient should always have been \
                            provided to the on_gc context internally"
                        );
                    let scope_cache_key = use_context::<ScopeCacheKey>()
                        .expect(
                            "leptos-fetch bug, ScopeCacheKey itself should always have been \
                            provided to the on_gc context internally"
                        );
                    let mut found_nb = 0;
                    untyped_client
                        .scope_lookup
                        .with_cached_scope_mut::<PaginatedPageKey<Key>, Option<(Vec<PageItem>, bool)>, _, _>(
                            &mut untyped_client.scope_lookup.scopes_mut(),
                            scope_cache_key,
                            OnScopeMissing::Skip,
                            |_| {},
                            |maybe_scope, _| {
                                if let Some(scope) = maybe_scope {
                                    for query_or_pending in scope.all_queries_mut_include_pending() {
                                        if query_or_pending.key().value_if_safe().map(|test_key| test_key.key == key.key).unwrap_or(false) {
                                            found_nb += 1;
                                        }
                                    }
                                }
                            },
                        );
                    if found_nb == 0 {
                        untyped_client.clear_query(
                            &backing_cache_scope,
                            KeyWithUnhashedItemCountAndOffsetRequested {
                                key: key.key.clone(),
                                item_count_requested: 0, // Doesn't matter, it's not part of the hash
                                offset_requested: 0, // Doesn't matter, it's not part of the hash
                            },
                        );
                    }
                })
            }
        }
    };
}

define! { [+ Send], [+ Send + Sync], QueryScope, fetch_query, "QueryScope" }
define! { [], [], QueryScopeLocal, fetch_query_local, "QueryScopeLocal" }

#[derive(Debug, Clone)]
struct KeyWithUnhashedItemCountAndOffsetRequested<Key> {
    key: Key,
    item_count_requested: usize,
    offset_requested: u64,
}

impl<Key: Hash> Hash for KeyWithUnhashedItemCountAndOffsetRequested<Key> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key.hash(state);
    }
}

#[derive(Debug, Clone)]
struct BackingCache<Item> {
    inner: Arc<BackingCacheInner<Item>>,
}

#[derive(Debug)]
struct BackingCacheInner<Item> {
    items: SparseItems<Item>,
    maybe_total_items: parking_lot::Mutex<Option<u64>>,
    update_lock: futures::lock::Mutex<()>,
}

#[derive(Debug)]
struct SparseItems<Item> {
    items: parking_lot::Mutex<BTreeMap<u64, Item>>,
}

impl<Item> SparseItems<Item> {
    fn extend(&self, offset: u64, new_items: Vec<Item>) {
        let mut items = self.items.lock();
        for (idx, item) in new_items.into_iter().enumerate() {
            items.insert(offset + (idx as u64), item);
        }
    }

    /// Return the range requested, or None if any items are missing.
    fn get_range(&self, start_offset: u64, count: usize) -> Option<Vec<Item>>
    where
        Item: Clone,
    {
        let items = self.items.lock();

        // Check if we have all items in the range
        let mut result = Vec::with_capacity(count);
        for offset in start_offset..(start_offset + (count as u64)) {
            match items.get(&offset) {
                Some(item) => result.push(item),
                None => return None, // Missing item, return None
            }
        }

        Some(result.into_iter().cloned().collect())
    }

    fn check_range(&self, start_offset: u64, end_index_exclusive: u64) -> bool
    where
        Item: Clone,
    {
        let items = self.items.lock();
        for offset in start_offset..end_index_exclusive {
            if !items.contains_key(&offset) {
                return false;
            }
        }
        true
    }
}
