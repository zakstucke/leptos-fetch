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

        impl<Key, PageItem> $name<PaginatedPageKey<Key>, Option<(Vec<PageItem>, Option<u64>)>>
        where
            Key: DebugIfDevtoolsEnabled + Clone + Hash + PartialEq + 'static $($impl_fn_generics)*,
            PageItem: DebugIfDevtoolsEnabled + Clone + 'static $($impl_fn_generics)*,
        {
            /// ## Paginated Query Scopes with Offset
            ///
            /// Similar to cursor-based pagination, but uses numeric offsets allowing direct page jumps.
            /// Returns total item count when known, enabling page count calculations.
            ///
            /// The getter function receives:
            /// - `query_key` - Your custom key
            /// - `nb_items_requested` - Number of items to return
            /// - `offset` - Starting position (u64)
            ///
            /// The getter must return:
            /// - `Vec<Item>` - Items for this fetch
            /// - `Option<u64>` - Total items if known
            pub fn new_paginated_with_offset<Fut>(
                getter: impl Fn(Key, usize, u64) -> Fut + 'static $($impl_fn_generics)*,
            ) -> $name<PaginatedPageKey<Key>, Option<(Vec<PageItem>, Option<u64>)>>
            where
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
                            if items.is_empty() {
                                if let Some(cur_total_items) = &mut maybe_total_items {
                                    if *cur_total_items > key.offset_requested {
                                        *cur_total_items = key.offset_requested;
                                    }
                                } else {
                                    maybe_total_items = Some(key.offset_requested);
                                }
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
                            if let Some(metadata) = untyped_client.query_metadata::<PaginatedPageKey<Key>, Option<(Vec<PageItem>, Option<u64>)>>(
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
                                // If page starts beyond available data, return None
                                if target_idx_start >= maybe_total_items {
                                    return None;
                                }
                                if target_idx_end_exclusive > maybe_total_items {
                                    target_idx_end_exclusive = maybe_total_items;
                                }
                            }

                            // Load x more if needed:
                            if !infinite_cache.inner.items.check_range(target_idx_start, target_idx_end_exclusive) {
                                // Preventing multiple simultaneous fetches by holding an async lock,
                                // but note making sure to not hold any sync locks across await boundaries.
                                let mut _guard = infinite_cache.inner.update_lock.lock().await;
                                loop {
                                    // Find the next offset we need to fetch
                                    let next_offset = {
                                        let items_lock = infinite_cache.inner.items.items.lock();
                                        // Find first missing offset in our target range
                                        let mut offset = target_idx_start;
                                        while offset < target_idx_end_exclusive {
                                            if !items_lock.contains_key(&offset) {
                                                break;
                                            }
                                            offset += 1;
                                        }
                                        offset
                                    };

                                    if next_offset >= target_idx_end_exclusive {
                                        // All items in range are present
                                        break;
                                    }

                                    let items_needed = (target_idx_end_exclusive - next_offset) as usize;
                                    let (items, maybe_total_items) =
                                        getter(page_key.key.clone(), items_needed, next_offset).await;
                                    if !items.is_empty() {
                                        infinite_cache.inner.items.extend(next_offset, items);
                                        *infinite_cache.inner.maybe_total_items.lock() = maybe_total_items;
                                    } else {
                                        // Protect against incorrect item count when clearly no items left:
                                        let mut guard = infinite_cache.inner.maybe_total_items.lock();
                                        if let Some(cur_total_items) = &mut* guard {
                                            if *cur_total_items > next_offset {
                                                *cur_total_items = next_offset;
                                            }
                                        } else {
                                            *guard = Some(next_offset);
                                        }
                                        // Can't get more items, break out
                                        break;
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
                        .with_cached_scope_mut::<PaginatedPageKey<Key>, Option<(Vec<PageItem>, Option<u64>)>, _, _>(
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

#[cfg(test)]
mod tests {
    use any_spawner::Executor;
    use hydration_context::SsrSharedContext;
    use leptos::prelude::*;
    use rstest::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::test::prep_vari;
    use crate::{PaginatedPageKey, QueryClient, QueryScope};

    /// We don't know the serialization mechanism the user is using, so cannot return wrapping types from the query function.
    /// Hence returning (Vec<Item>, Option<total_items>) instead of a custom wrapping Page<Item> type.
    /// This test is just checking compilation.
    #[tokio::test]
    async fn test_paginated_serialization_works() {
        crate::test::identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (client, _guard, _owner) = prep_vari!(true);
                let scope = QueryScope::new_paginated_with_offset(|_: (), _, _| async {
                    (vec![()], Some(10))
                });
                client.resource(scope, || PaginatedPageKey {
                    key: (),
                    page_index: 0,
                    page_size: 10,
                });
            })
            .await;
    }

    fn get_simple_api_fn(
        num_rows: usize,
    ) -> (
        Arc<AtomicUsize>,
        impl Fn(usize, u64) -> (Vec<usize>, Option<u64>) + Clone + 'static,
    ) {
        let call_count = Arc::new(AtomicUsize::new(0));

        let api_fn = {
            let call_count = call_count.clone();
            move |target_return_count: usize, offset: u64| {
                let call_count = call_count.clone();
                call_count.fetch_add(1, Ordering::Relaxed);

                let offset_usize = offset as usize;
                let items = (0..num_rows)
                    .skip(offset_usize)
                    .take(target_return_count)
                    .collect::<Vec<_>>();
                let total_items = num_rows as u64;
                (items, Some(total_items))
            }
        };

        (call_count, api_fn)
    }

    #[tokio::test]
    async fn test_paginated_offset() {
        crate::test::identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (client, _guard, _owner) = prep_vari!(true);

                let (_call_count, my_api_fn) = get_simple_api_fn(30);

                let scope =
                    QueryScope::new_paginated_with_offset(move |_query_key, page_size, offset| {
                        let my_api_fn = my_api_fn.clone();
                        async move {
                            let (items, total_items) = my_api_fn(page_size, offset);
                            (items, total_items)
                        }
                    });

                let (first_page_logs, maybe_total) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: (),
                            page_index: 0,
                            page_size: 20,
                        },
                    )
                    .await
                    .expect(
                        "This page should exist (Some()), \
                        None when a page is requested beyond the end of the data.",
                    );

                assert_eq!(first_page_logs, (0..20).collect::<Vec<_>>());
                assert_eq!(maybe_total, Some(30));

                let (second_page_logs, maybe_total) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: (),
                            page_index: 1,
                            page_size: 20,
                        },
                    )
                    .await
                    .expect(
                        "This page should exist (Some()), \
                        None when a page is requested beyond the end of the data.",
                    );

                assert_eq!(second_page_logs, (20..30).collect::<Vec<_>>());
                assert_eq!(maybe_total, Some(30));

                assert!(
                    client
                        .fetch_query(
                            scope.clone(),
                            PaginatedPageKey {
                                key: (),
                                page_index: 2,
                                page_size: 20,
                            },
                        )
                        .await
                        .is_none()
                );
            })
            .await;
    }

    /// Test that the pagination logic keeps calling the API until page_size items are available
    #[tokio::test]
    async fn test_paginated_offset_fills_page_size_with_multiple_calls() {
        crate::test::identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (client, _guard, _owner) = prep_vari!(true);

                let call_count = Arc::new(AtomicUsize::new(0));
                let call_count_clone = call_count.clone();

                // API that returns fewer items than requested
                let scope =
                    QueryScope::new_paginated_with_offset(move |_key: (), _page_size, offset| {
                        let call_count = call_count_clone.clone();
                        async move {
                            call_count.fetch_add(1, Ordering::Relaxed);

                            // Return only 3 items per call, even if more are requested
                            let items = match offset {
                                0 => vec![0, 1, 2],
                                3 => vec![3, 4, 5],
                                6 => vec![6, 7, 8],
                                9 => vec![9, 10],
                                _ => vec![],
                            };
                            (items, Some(11))
                        }
                    });

                // Request page with size 10 - should make 4 API calls to get 10 items
                let (items, total) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: (),
                            page_index: 0,
                            page_size: 10,
                        },
                    )
                    .await
                    .expect("Page should exist");

                assert_eq!(items.len(), 10, "Should have filled to page_size");
                assert_eq!(items, vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
                assert_eq!(total, Some(11));
                assert_eq!(
                    call_count.load(Ordering::Relaxed),
                    4,
                    "Should have made 4 API calls to fill page"
                );

                // Request second page - item 10 was already fetched during first page
                let (items, total) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: (),
                            page_index: 1,
                            page_size: 10,
                        },
                    )
                    .await
                    .expect("Page should exist");

                assert_eq!(items.len(), 1, "Only 1 item left");
                assert_eq!(items, vec![10]);
                assert_eq!(total, Some(11));
                assert_eq!(
                    call_count.load(Ordering::Relaxed),
                    4,
                    "Should not make additional calls - item 10 was already cached"
                );
            })
            .await;
    }

    /// Test that API calls are cached properly and not repeated unnecessarily
    #[tokio::test]
    async fn test_paginated_offset_no_unnecessary_api_calls() {
        crate::test::identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (client, _guard, _owner) = prep_vari!(true);

                let call_count = Arc::new(AtomicUsize::new(0));
                let call_count_clone = call_count.clone();

                let scope =
                    QueryScope::new_paginated_with_offset(move |_key: (), page_size, offset| {
                        let call_count = call_count_clone.clone();
                        async move {
                            call_count.fetch_add(1, Ordering::Relaxed);

                            let offset_usize = offset as usize;
                            let items: Vec<usize> =
                                (offset_usize..offset_usize + page_size).collect();
                            (items, Some(100))
                        }
                    });

                // First fetch
                let _ = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: (),
                            page_index: 0,
                            page_size: 10,
                        },
                    )
                    .await;
                assert_eq!(
                    call_count.load(Ordering::Relaxed),
                    1,
                    "First fetch should make 1 API call"
                );

                // Fetch same page again - should use cache
                let _ = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: (),
                            page_index: 0,
                            page_size: 10,
                        },
                    )
                    .await;
                assert_eq!(
                    call_count.load(Ordering::Relaxed),
                    1,
                    "Should still be 1 call - cached"
                );

                // Fetch next page
                let _ = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: (),
                            page_index: 1,
                            page_size: 10,
                        },
                    )
                    .await;
                assert_eq!(
                    call_count.load(Ordering::Relaxed),
                    2,
                    "Second page needs new API call"
                );

                // Fetch first page again - should still be cached
                let _ = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: (),
                            page_index: 0,
                            page_size: 10,
                        },
                    )
                    .await;
                assert_eq!(
                    call_count.load(Ordering::Relaxed),
                    2,
                    "Should still be 2 calls - first page cached"
                );
            })
            .await;
    }

    /// Test linked invalidation and clear between pages with same key
    #[rstest]
    #[tokio::test]
    async fn test_paginated_offset_linked_invalidation_and_clear(
        #[values(true, false)] clear: bool,
    ) {
        crate::test::identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (client, _guard, _owner) = prep_vari!(true);

                let version = Arc::new(AtomicUsize::new(0));
                let version_clone = version.clone();

                let scope =
                    QueryScope::new_paginated_with_offset(move |key: String, page_size, offset| {
                        let v = version_clone.clone();
                        async move {
                            let current_version = v.load(Ordering::Relaxed);
                            let offset_usize = offset as usize;

                            // Return different data based on version
                            let items: Vec<String> = (offset_usize..offset_usize + page_size)
                                .map(|i| format!("{}_v{}_{}", key, current_version, i))
                                .collect();

                            (items, Some(30))
                        }
                    });

                // Fetch first page
                let (items1, _) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: "test".to_string(),
                            page_index: 0,
                            page_size: 10,
                        },
                    )
                    .await
                    .expect("Page should exist");

                assert_eq!(items1[0], "test_v0_0");

                // Fetch second page
                let (items2, _) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: "test".to_string(),
                            page_index: 1,
                            page_size: 10,
                        },
                    )
                    .await
                    .expect("Page should exist");

                assert_eq!(items2[0], "test_v0_10");

                // Increment version to simulate data change
                version.store(1, Ordering::Relaxed);

                // Fetch first page again - should still be old data
                let (items1_new, _) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: "test".to_string(),
                            page_index: 0,
                            page_size: 10,
                        },
                    )
                    .await
                    .expect("Page should exist");

                assert_eq!(
                    items1_new[0], "test_v0_0",
                    "Should still have old version before invalidation/clear"
                );

                // Invalidation/clear should lead to new data:
                if clear {
                    // Currently not public, but still want to check as used internally for some things:
                    client.untyped_client.clear_query_scope(scope.clone());
                } else {
                    client.invalidate_query_scope(scope.clone());
                }

                // Fetch first page again - should get new data
                let (items1_new, _) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: "test".to_string(),
                            page_index: 0,
                            page_size: 10,
                        },
                    )
                    .await
                    .expect("Page should exist");

                assert_eq!(
                    items1_new[0], "test_v1_0",
                    "Should have new version after invalidation/clear"
                );

                // Second page should also be invalidated/cleared
                let (items2_new, _) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: "test".to_string(),
                            page_index: 1,
                            page_size: 10,
                        },
                    )
                    .await
                    .expect("Page should exist");

                assert_eq!(
                    items2_new[0], "test_v1_10",
                    "Second page should also have new version"
                );
            })
            .await;
    }

    /// Test that empty responses are handled correctly
    #[tokio::test]
    async fn test_paginated_offset_empty_response_handling() {
        crate::test::identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (client, _guard, _owner) = prep_vari!(true);

                let scope = QueryScope::new_paginated_with_offset(
                    |_key: (), _page_size, offset| async move {
                        match offset {
                            0 => (vec![1, 2, 3], Some(3)),
                            _ => (vec![], Some(3)),
                        }
                    },
                );

                let (items, total) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: (),
                            page_index: 0,
                            page_size: 10,
                        },
                    )
                    .await
                    .expect("Page should exist");

                // Should only have 3 items since total is 3
                assert_eq!(items.len(), 3);
                assert_eq!(total, Some(3));
            })
            .await;
    }

    /// Test concurrent page fetches don't cause duplicate API calls
    #[tokio::test]
    async fn test_paginated_offset_concurrent_fetches() {
        crate::test::identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (client, _guard, _owner) = prep_vari!(true);

                let call_count = Arc::new(AtomicUsize::new(0));
                let call_count_clone = call_count.clone();

                let scope =
                    QueryScope::new_paginated_with_offset(move |_key: (), page_size, offset| {
                        let call_count = call_count_clone.clone();
                        async move {
                            call_count.fetch_add(1, Ordering::Relaxed);

                            // Add a small delay to simulate network latency
                            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

                            let offset_usize = offset as usize;
                            let items: Vec<usize> =
                                (offset_usize..offset_usize + page_size).collect();
                            (items, Some(100))
                        }
                    });

                // Launch multiple concurrent fetches for the same page
                let futures = (0..5).map(|_| {
                    client.fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: (),
                            page_index: 0,
                            page_size: 10,
                        },
                    )
                });

                let results = futures::future::join_all(futures).await;

                // All should succeed
                for result in results {
                    assert!(result.is_some());
                }

                // Should only make 1 API call despite 5 concurrent requests
                assert_eq!(
                    call_count.load(Ordering::Relaxed),
                    1,
                    "Concurrent fetches should share single API call"
                );
            })
            .await;
    }

    /// Test different page sizes work correctly with shared cache
    #[tokio::test]
    async fn test_paginated_offset_different_page_sizes() {
        crate::test::identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (client, _guard, _owner) = prep_vari!(true);

                let call_count = Arc::new(AtomicUsize::new(0));
                let call_count_clone = call_count.clone();

                let scope =
                    QueryScope::new_paginated_with_offset(move |_key: (), page_size, offset| {
                        let call_count = call_count_clone.clone();
                        async move {
                            call_count.fetch_add(1, Ordering::Relaxed);

                            let offset_usize = offset as usize;
                            let items: Vec<usize> = (offset_usize
                                ..std::cmp::min(offset_usize + page_size, 50))
                                .collect();
                            (items, Some(50))
                        }
                    });

                // Fetch with page size 5
                let (items1, _) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: (),
                            page_index: 0,
                            page_size: 5,
                        },
                    )
                    .await
                    .expect("Page should exist");
                assert_eq!(items1.len(), 5);
                assert_eq!(call_count.load(Ordering::Relaxed), 1);

                // Fetch with page size 15 - should reuse cached data and fetch more
                let (items2, _) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: (),
                            page_index: 0,
                            page_size: 15,
                        },
                    )
                    .await
                    .expect("Page should exist");
                assert_eq!(items2.len(), 15);
                assert_eq!(
                    call_count.load(Ordering::Relaxed),
                    2,
                    "Should fetch more data for larger page"
                );

                // Fetch with page size 10 - should use cached data
                let (items3, _) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: (),
                            page_index: 0,
                            page_size: 10,
                        },
                    )
                    .await
                    .expect("Page should exist");
                assert_eq!(items3.len(), 10);
                assert_eq!(
                    call_count.load(Ordering::Relaxed),
                    2,
                    "Should use cached data, no new fetch"
                );
            })
            .await;
    }

    /// Test that backing cache is properly managed when paginated pages expire
    /// Tests both GC (garbage collection) and stale time scenarios
    #[rstest]
    #[case::gc_time(TestMode::GcTime)]
    #[case::stale_time(TestMode::StaleTime)]
    #[tokio::test]
    async fn test_paginated_offset_backing_cache_lifecycle(#[case] mode: TestMode) {
        crate::test::identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (client, _guard, _owner) = prep_vari!(true);

                let call_count = Arc::new(AtomicUsize::new(0));
                let call_count_clone = call_count.clone();

                let scope =
                    QueryScope::new_paginated_with_offset(move |key: String, page_size, offset| {
                        let call_count = call_count_clone.clone();
                        async move {
                            call_count.fetch_add(1, Ordering::Relaxed);

                            let offset_usize = offset as usize;
                            let items: Vec<String> = (offset_usize..offset_usize + page_size)
                                .map(|i| format!("{}_{}", key, i))
                                .collect();
                            (items, Some(30))
                        }
                    })
                    .with_options(match mode {
                        TestMode::GcTime => crate::QueryOptions::default()
                            .with_gc_time(std::time::Duration::from_millis(100)),
                        TestMode::StaleTime => crate::QueryOptions::default()
                            .with_stale_time(std::time::Duration::from_millis(100)),
                    });

                // Fetch first page
                let (items, _) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: "test".to_string(),
                            page_index: 0,
                            page_size: 10,
                        },
                    )
                    .await
                    .expect("Page should exist");
                assert_eq!(items[0], "test_0");
                assert_eq!(call_count.load(Ordering::Relaxed), 1);

                // Wait for 50ms, 50ms left till gc/stale:
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

                // Fetch second page
                let (items, _) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: "test".to_string(),
                            page_index: 1,
                            page_size: 10,
                        },
                    )
                    .await
                    .expect("Page should exist");
                assert_eq!(items[0], "test_10");
                assert_eq!(call_count.load(Ordering::Relaxed), 2);

                // Wait another 50ms, so the first query is stale/gc'd, but the second is valid even when gc'd because 50ms left:
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

                // Fetch first page again
                let (items, _) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: "test".to_string(),
                            page_index: 0,
                            page_size: 10,
                        },
                    )
                    .await
                    .expect("Page should exist");
                assert_eq!(items[0], "test_0");

                // Fetch second page again
                let (items, _) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: "test".to_string(),
                            page_index: 1,
                            page_size: 10,
                        },
                    )
                    .await
                    .expect("Page should exist");
                assert_eq!(items[0], "test_10");

                let expected_calls = match mode {
                    TestMode::GcTime => {
                        // GC cleared page 0, but page 1 kept backing cache alive
                        // So page 0 refetch reuses backing cache, plus page 1 still alive so also reused
                        2
                    }
                    TestMode::StaleTime => {
                        // Stale page triggers invalidation which clears backing cache
                        // So page 0 refetch needs new API call, but page 1 is still valid so reused
                        3
                    }
                };
                assert_eq!(call_count.load(Ordering::Relaxed), expected_calls);

                // If gc, should clear the backing cache only when all pages are gc'd:
                if matches!(mode, TestMode::GcTime) {
                    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;

                    // Now fetch again - backing cache should be gone
                    let (items, _) = client
                        .fetch_query(
                            scope.clone(),
                            PaginatedPageKey {
                                key: "test".to_string(),
                                page_index: 0,
                                page_size: 10,
                            },
                        )
                        .await
                        .expect("Page should exist");
                    assert_eq!(items[0], "test_0");
                    assert_eq!(
                        call_count.load(Ordering::Relaxed),
                        3,
                        "Backing cache should be cleared when all pages are GC'd"
                    );
                }
            })
            .await;
    }

    /// Test jumping to distant pages without loading intermediate pages (offset-specific feature)
    #[tokio::test]
    async fn test_paginated_offset_jump_to_distant_page() {
        crate::test::identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (client, _guard, _owner) = prep_vari!(true);

                let call_count = Arc::new(AtomicUsize::new(0));
                let call_count_clone = call_count.clone();

                let scope =
                    QueryScope::new_paginated_with_offset(move |_key: (), page_size, offset| {
                        let call_count = call_count_clone.clone();
                        async move {
                            call_count.fetch_add(1, Ordering::Relaxed);

                            let offset_usize = offset as usize;
                            let items: Vec<usize> =
                                (offset_usize..offset_usize + page_size).collect();
                            (items, Some(1000))
                        }
                    });

                // Jump directly to page 10 without loading pages 0-9
                let (items, total) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: (),
                            page_index: 10,
                            page_size: 10,
                        },
                    )
                    .await
                    .expect("Page should exist");

                assert_eq!(items, (100..110).collect::<Vec<_>>());
                assert_eq!(total, Some(1000));
                assert_eq!(
                    call_count.load(Ordering::Relaxed),
                    1,
                    "Should only make 1 API call to jump to distant page"
                );

                // Now jump back to page 0
                let (items, total) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: (),
                            page_index: 0,
                            page_size: 10,
                        },
                    )
                    .await
                    .expect("Page should exist");

                assert_eq!(items, (0..10).collect::<Vec<_>>());
                assert_eq!(total, Some(1000));
                assert_eq!(
                    call_count.load(Ordering::Relaxed),
                    2,
                    "Should make 1 more API call to jump back"
                );

                // Jump to page 50
                let (items, total) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: (),
                            page_index: 50,
                            page_size: 10,
                        },
                    )
                    .await
                    .expect("Page should exist");

                assert_eq!(items, (500..510).collect::<Vec<_>>());
                assert_eq!(total, Some(1000));
                assert_eq!(
                    call_count.load(Ordering::Relaxed),
                    3,
                    "Should make 1 more API call for distant page 50"
                );
            })
            .await;
    }

    /// Test total items calculation and page count (offset-specific feature)
    #[tokio::test]
    async fn test_paginated_offset_total_items_and_page_count() {
        crate::test::identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (client, _guard, _owner) = prep_vari!(true);

                let scope = QueryScope::new_paginated_with_offset(
                    move |_key: (), page_size, offset| async move {
                        let offset_usize = offset as usize;
                        let total_items = 47u64;
                        let items: Vec<usize> = (offset_usize
                            ..std::cmp::min(offset_usize + page_size, total_items as usize))
                            .collect();
                        (items, Some(total_items))
                    },
                );

                // Fetch first page
                let (items, total) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: (),
                            page_index: 0,
                            page_size: 10,
                        },
                    )
                    .await
                    .expect("Page should exist");

                assert_eq!(items.len(), 10);
                assert_eq!(total, Some(47));

                // Calculate total pages: ceil(47 / 10) = 5
                let total_pages = total.map(|t| (t as f64 / 10.0).ceil() as usize);
                assert_eq!(total_pages, Some(5));

                // Test page 4 (last page with partial data)
                let (items, total) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: (),
                            page_index: 4,
                            page_size: 10,
                        },
                    )
                    .await
                    .expect("Page should exist");

                assert_eq!(items.len(), 7, "Last page should have 7 items (47 % 10)");
                assert_eq!(items, (40..47).collect::<Vec<_>>());
                assert_eq!(total, Some(47));

                // Test beyond last page
                let result = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: (),
                            page_index: 5,
                            page_size: 10,
                        },
                    )
                    .await;

                assert!(result.is_none(), "Should return None for page beyond total");
            })
            .await;
    }

    /// Test total items update when it changes
    #[tokio::test]
    async fn test_paginated_offset_total_items_update() {
        crate::test::identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (client, _guard, _owner) = prep_vari!(true);

                let total = Arc::new(AtomicUsize::new(100));
                let total_clone = total.clone();

                let scope =
                    QueryScope::new_paginated_with_offset(move |_key: (), page_size, offset| {
                        let total = total_clone.clone();
                        async move {
                            let current_total = total.load(Ordering::Relaxed) as u64;
                            let offset_usize = offset as usize;
                            let items: Vec<usize> = (offset_usize
                                ..std::cmp::min(offset_usize + page_size, current_total as usize))
                                .collect();
                            (items, Some(current_total))
                        }
                    });

                // Fetch with initial total of 100
                let (items, returned_total) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: (),
                            page_index: 0,
                            page_size: 10,
                        },
                    )
                    .await
                    .expect("Page should exist");

                assert_eq!(items, (0..10).collect::<Vec<_>>());
                assert_eq!(returned_total, Some(100));

                // Update total to 50
                total.store(50, Ordering::Relaxed);

                // Invalidate to get fresh data
                client.invalidate_query_scope(scope.clone());

                // Fetch again - should get updated total
                let (items, returned_total) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: (),
                            page_index: 0,
                            page_size: 10,
                        },
                    )
                    .await
                    .expect("Page should exist");

                assert_eq!(items, (0..10).collect::<Vec<_>>());
                assert_eq!(returned_total, Some(50));
            })
            .await;
    }

    /// Test sparse item storage - items at non-contiguous offsets
    #[tokio::test]
    async fn test_paginated_offset_sparse_item_storage() {
        crate::test::identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (client, _guard, _owner) = prep_vari!(true);

                let scope = QueryScope::new_paginated_with_offset(
                    move |_key: (), page_size, offset| async move {
                        let offset_usize = offset as usize;
                        let items: Vec<usize> = (offset_usize..offset_usize + page_size).collect();
                        (items, Some(1000))
                    },
                );

                // Fetch page 0
                let _ = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: (),
                            page_index: 0,
                            page_size: 10,
                        },
                    )
                    .await;

                // Fetch page 10 (offset 100) - creates sparse storage
                let (items, _) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: (),
                            page_index: 10,
                            page_size: 10,
                        },
                    )
                    .await
                    .expect("Page should exist");

                assert_eq!(items, (100..110).collect::<Vec<_>>());

                // Fetch page 5 (offset 50) - fills in the middle
                let (items, _) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: (),
                            page_index: 5,
                            page_size: 10,
                        },
                    )
                    .await
                    .expect("Page should exist");

                assert_eq!(items, (50..60).collect::<Vec<_>>());

                // All three pages should still be accessible
                let (items, _) = client
                    .fetch_query(
                        scope.clone(),
                        PaginatedPageKey {
                            key: (),
                            page_index: 0,
                            page_size: 10,
                        },
                    )
                    .await
                    .expect("Page should exist");
                assert_eq!(items, (0..10).collect::<Vec<_>>());
            })
            .await;
    }

    #[derive(Debug, Clone, Copy)]
    enum TestMode {
        /// Test GC behavior: backing cache should only be cleared when all pages are GC'd
        GcTime,
        /// Test stale time behavior: backing cache should be invalidated when any page goes stale
        StaleTime,
    }
}
