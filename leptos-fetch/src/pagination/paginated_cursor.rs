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
            /// Create a cursor-based paginated query scope.
            ///
            /// Use this when your API uses continuation tokens/cursors rather than numeric offsets.
            /// Good for infinite scroll patterns or when your API doesn't support offset-based pagination.
            ///
            /// # Arguments
            ///
            /// The getter receives:
            /// - `query_key: Key` - Your custom key, same across all pages
            /// - `nb_items_requested: usize` - Target number of items to return, will call again if not enough items returned
            /// - `cursor: Option<Cursor>` - Cursor from previous fetch, `None` on first page
            ///
            /// The getter must return:
            /// - `Vec<Item>` - Items for this page
            /// - `Option<Cursor>` - Next cursor token, `None` when no more data
            ///
            /// # Example
            ///
            /// ```rust,ignore
            /// use leptos_fetch::{QueryScope, PaginatedPageKey};
            ///
            /// // Create paginated scope with cursor-based API
            /// let scope = QueryScope::new_paginated_with_cursor(
            ///     |_key: (), nb_items, cursor: Option<String>| async move {
            ///         // Call your API with cursor token
            ///         let (items, next_cursor) = fetch_from_api(cursor, nb_items).await;
            ///         (items, next_cursor)
            ///     }
            /// );
            ///
            /// // Use like any other scope - fetch pages with PaginatedPageKey
            /// let (items, has_more) = client.fetch_query(scope, PaginatedPageKey {
            ///     key: (),
            ///     page_index: 0,
            ///     page_size: 20,
            /// }).await.expect("Page exists");
            ///
            /// // has_more: bool indicates if another page is available
            /// if has_more {
            ///     let (next_items, _) = client.fetch_query(scope, PaginatedPageKey {
            ///         key: (),
            ///         page_index: 1,
            ///         page_size: 20,
            ///     }).await.expect("Next page exists");
            /// }
            /// ```
            pub fn new_paginated_with_cursor<Cursor, Fut>(
                getter: impl Fn(Key, usize, Option<Cursor>) -> Fut + 'static $($impl_fn_generics)*,
            ) -> $name<PaginatedPageKey<Key>, Option<(Vec<PageItem>, bool)>>
            where
                Cursor: DebugIfDevtoolsEnabled + Clone + 'static $($impl_fn_generics)*,
                Fut: Future<Output = (Vec<PageItem>, Option<Cursor>)> $($impl_fut_generics)*,
            {
                let getter = Arc::new(getter);
                let backing_cache_scope = $name::new({
                    let getter = getter.clone();
                    move |key: KeyWithItemCountRequestedUnhashed<Key>| {
                        let getter = getter.clone();
                        async move {
                            let (items, mut cursor) = getter(key.key, key.item_count_requested, None).await;

                            // Protect incorrect Some(cursor) when clearly no items left:
                            if items.is_empty() {
                                cursor = None;
                            }

                            BackingCache {
                                inner: Arc::new(BackingCacheInner {
                                    items: parking_lot::Mutex::new(items),
                                    cursor: parking_lot::Mutex::new(cursor),
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
                            && let Some(backing_metadata) = untyped_client.query_metadata::<KeyWithItemCountRequestedUnhashed<Key>, BackingCache<PageItem, Cursor>>(
                                backing_cache_scope.cache_key,
                                &KeyWithItemCountRequestedUnhashed {
                                    key: page_key.key.clone(),
                                    item_count_requested: 0, // Doesn't matter, it's not part of the hash
                                },
                            )
                            && backing_metadata.updated_at <= metadata.updated_at
                            {
                                untyped_client.invalidate_query(
                                    &backing_cache_scope,
                                    KeyWithItemCountRequestedUnhashed {
                                        key: page_key.key.clone(),
                                        item_count_requested: 0, // Doesn't matter, it's not part of the hash
                                    },
                                );
                            }

                            let infinite_cache = untyped_client
                                .$fetch_fn(
                                    backing_cache_scope,
                                    KeyWithItemCountRequestedUnhashed {
                                        key: page_key.key.clone(),
                                        item_count_requested: page_key.page_size,
                                    },
                                )
                                .await;

                            let target_idx_start = page_key.page_index * (page_key.page_size as usize);
                            let target_idx_end_exclusive = (page_key.page_index + 1) * (page_key.page_size as usize);

                            // Load x more if needed:
                            let should_request_x_more = || {
                                let items = infinite_cache.inner.items.lock();
                                if items.len() < target_idx_end_exclusive
                                    && infinite_cache.inner.cursor.lock().is_some() {
                                        Some(target_idx_end_exclusive - items.len())
                                    } else {
                                        None
                                    }
                            };

                            if should_request_x_more().is_some() {
                                // Preventing multiple simultaneous fetches by holding an async lock,
                                // but note making sure to not hold any sync locks across await boundaries.
                                let mut _guard = infinite_cache.inner.update_lock.lock().await;
                                while let Some(amount_needed) = should_request_x_more() {
                                    let cur_token = (&*infinite_cache.inner.cursor.lock()).clone();
                                    let (items, cursor) =
                                        getter(page_key.key.clone(), amount_needed, cur_token).await;
                                    if !items.is_empty() {
                                        infinite_cache.inner.items.lock().extend(items);
                                        *infinite_cache.inner.cursor.lock() = cursor;
                                    } else {
                                        // Protect incorrect Some(cursor) when clearly no items left:
                                        *infinite_cache.inner.cursor.lock() = None;
                                    }
                                }
                                drop(_guard);
                            }

                            let items_guard = infinite_cache.inner.items.lock();
                            if target_idx_start >= items_guard.len() {
                                None
                            } else {
                                let items = items_guard
                                    [target_idx_start..std::cmp::min(target_idx_end_exclusive, items_guard.len())]
                                    .to_vec();
                                let next_page_exists = items_guard.len() > target_idx_end_exclusive
                                    || infinite_cache.inner.cursor.lock().is_some();
                                Some((items, next_page_exists))
                            }
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
                            KeyWithItemCountRequestedUnhashed {
                                key: key.key.clone(),
                                item_count_requested: 0, // Doesn't matter, it's not part of the hash
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
                            KeyWithItemCountRequestedUnhashed {
                                key: key.key.clone(),
                                item_count_requested: 0, // Doesn't matter, it's not part of the hash
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
struct KeyWithItemCountRequestedUnhashed<Key> {
    key: Key,
    item_count_requested: usize,
}

impl<Key: Hash> Hash for KeyWithItemCountRequestedUnhashed<Key> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key.hash(state);
    }
}

#[derive(Debug, Clone)]
struct BackingCache<Item, Cursor> {
    inner: Arc<BackingCacheInner<Item, Cursor>>,
}

#[derive(Debug)]
struct BackingCacheInner<Item, Cursor> {
    items: parking_lot::Mutex<Vec<Item>>,
    cursor: parking_lot::Mutex<Option<Cursor>>,
    update_lock: futures::lock::Mutex<()>,
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
    /// Hence returning (Vec<Item>, has_more_pages: bool) instead of a custom wrapping Page<Item> type.
    /// This test is just checking compilation.
    #[tokio::test]
    async fn test_paginated_serialization_works() {
        crate::test::identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (client, _guard, _owner) = prep_vari!(true);
                let scope = QueryScope::new_paginated_with_cursor(|_: (), _, _| async {
                    (vec![()], Some(()))
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
        impl Fn(usize, Option<usize>) -> (Vec<usize>, Option<usize>) + Clone + 'static,
    ) {
        let call_count = Arc::new(AtomicUsize::new(0));

        let api_fn = {
            let call_count = call_count.clone();
            move |target_return_count: usize, offset: Option<usize>| {
                let call_count = call_count.clone();
                call_count.fetch_add(1, Ordering::Relaxed);

                let offset = offset.unwrap_or(0);
                let items = (0..num_rows)
                    .skip(offset)
                    .take(target_return_count)
                    .collect::<Vec<_>>();
                let next_offset = if offset + target_return_count < num_rows {
                    Some(offset + items.len())
                } else {
                    None
                };
                (items, next_offset)
            }
        };

        (call_count, api_fn)
    }

    #[tokio::test]
    async fn test_paginated_cursor() {
        crate::test::identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (client, _guard, _owner) = prep_vari!(true);

                let (_call_count, my_api_fn) = get_simple_api_fn(30);

                // The scope/queryer can now be used like any other QueryScope, just with PaginatedPageKey<YourKey> as the key type.
                // In resources, fetch_query, etc.
                let scope =
                    QueryScope::new_paginated_with_cursor(move |_query_key, page_size, offset| {
                        let my_api_fn = my_api_fn.clone();
                        async move {
                            let (items, maybe_next_offset) = my_api_fn(page_size, offset);
                            (items, maybe_next_offset)
                        }
                    });

                let (first_page_logs, more_pages) = client
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

                assert!(
                    more_pages,
                    "There should be more pages after the first page, \
                    ROW_COUNT=30 which is > page size of 20."
                );

                let (second_page_logs, more_pages) = client
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

                assert!(
                    !more_pages,
                    "20+20=40 which is > ROW_COUNT=30, so no more pages after the second page."
                );

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
    async fn test_paginated_cursor_fills_page_size_with_multiple_calls() {
        crate::test::identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (client, _guard, _owner) = prep_vari!(true);

                let call_count = Arc::new(AtomicUsize::new(0));
                let call_count_clone = call_count.clone();

                // API that returns fewer items than requested
                let scope = QueryScope::new_paginated_with_cursor(
                    move |_key: (), _page_size, continuation| {
                        let call_count = call_count_clone.clone();
                        async move {
                            call_count.fetch_add(1, Ordering::Relaxed);

                            // Return only 3 items per call, even if more are requested
                            match continuation {
                                None => {
                                    // First call
                                    (vec![0, 1, 2], Some(3))
                                }
                                Some(3) => {
                                    // Second call
                                    (vec![3, 4, 5], Some(6))
                                }
                                Some(6) => {
                                    // Third call
                                    (vec![6, 7, 8], Some(9))
                                }
                                Some(9) => {
                                    // Fourth call
                                    (vec![9, 10], None) // Only 2 items left
                                }
                                _ => (vec![], None),
                            }
                        }
                    },
                );

                // Request page with size 10 - should make 4 API calls to get 10 items
                let (items, has_more) = client
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
                assert!(has_more, "Should indicate more pages available");
                assert_eq!(
                    call_count.load(Ordering::Relaxed),
                    4,
                    "Should have made 4 API calls to fill page"
                );

                // Request second page - should only need 1 more API call
                let (items, has_more) = client
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
                assert!(!has_more, "No more pages");
                assert_eq!(
                    call_count.load(Ordering::Relaxed),
                    4,
                    "Should not make additional calls - data already cached"
                );
            })
            .await;

        let (_call_count, my_api_fn) = get_simple_api_fn(30);

        // Create the scope
        let scope = QueryScope::new_paginated_with_cursor(move |_query_key, page_size, offset| {
            let my_api_fn = my_api_fn.clone();
            async move {
                let (items, maybe_next_offset) = my_api_fn(page_size, offset);
                (items, maybe_next_offset)
            }
        });

        let client = QueryClient::new();

        // Fetch first page
        let (first_page, more_pages) = client
            .fetch_query(
                scope.clone(),
                PaginatedPageKey {
                    key: (),
                    page_index: 0,
                    page_size: 20,
                },
            )
            .await
            .expect("First page should exist");

        assert_eq!(first_page, (0..20).collect::<Vec<_>>());
        assert!(more_pages);

        // Fetch second page
        let (second_page, more_pages) = client
            .fetch_query(
                scope.clone(),
                PaginatedPageKey {
                    key: (),
                    page_index: 1,
                    page_size: 20,
                },
            )
            .await
            .expect("Second page should exist");

        assert_eq!(second_page, (20..30).collect::<Vec<_>>());
        assert!(!more_pages);

        // Requesting beyond available data returns None
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
    }

    /// Test that API calls are cached properly and not repeated unnecessarily
    #[tokio::test]
    async fn test_paginated_cursor_no_unnecessary_api_calls() {
        crate::test::identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (client, _guard, _owner) = prep_vari!(true);

                let call_count = Arc::new(AtomicUsize::new(0));
                let call_count_clone = call_count.clone();

                let scope = QueryScope::new_paginated_with_cursor(
                    move |_key: (), page_size, continuation| {
                        let call_count = call_count_clone.clone();
                        async move {
                            call_count.fetch_add(1, Ordering::Relaxed);

                            let offset = continuation.unwrap_or(0);
                            let items: Vec<usize> = (offset..offset + page_size).collect();
                            let next = if offset + page_size < 100 {
                                Some(offset + page_size)
                            } else {
                                None
                            };
                            (items, next)
                        }
                    },
                );

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
    async fn test_paginated_cursor_linked_invalidation_and_clear(
        #[values(true, false)] clear: bool,
    ) {
        crate::test::identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (client, _guard, _owner) = prep_vari!(true);

                let version = Arc::new(AtomicUsize::new(0));
                let version_clone = version.clone();

                let scope = QueryScope::new_paginated_with_cursor(
                    move |key: String, page_size, continuation| {
                        let v = version_clone.clone();
                        async move {
                            let current_version = v.load(Ordering::Relaxed);
                            let offset = continuation.unwrap_or(0);

                            // Return different data based on version
                            let items: Vec<String> = (offset..offset + page_size)
                                .map(|i| format!("{}_v{}_{}", key, current_version, i))
                                .collect();

                            let next = if offset + page_size < 30 {
                                Some(offset + page_size)
                            } else {
                                None
                            };
                            (items, next)
                        }
                    },
                );

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
                    "Should have new version after invalidation/clear"
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
    async fn test_paginated_cursor_empty_response_handling() {
        crate::test::identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (client, _guard, _owner) = prep_vari!(true);

                let scope = QueryScope::new_paginated_with_cursor(
                    |_key: (), _page_size, continuation| async move {
                        match continuation {
                            None => (vec![1, 2, 3], Some(3)),
                            Some(3) => (vec![], Some(6)), // Empty response but with cursor
                            _ => (vec![], None),
                        }
                    },
                );

                let (items, has_more) = client
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

                // Should only have 3 items since second call returned empty
                assert_eq!(items.len(), 3);
                assert!(
                    !has_more,
                    "Should not have more pages when empty response received"
                );
            })
            .await;
    }

    /// Test concurrent page fetches don't cause duplicate API calls
    #[tokio::test]
    async fn test_paginated_cursor_concurrent_fetches() {
        crate::test::identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (client, _guard, _owner) = prep_vari!(true);

                let call_count = Arc::new(AtomicUsize::new(0));
                let call_count_clone = call_count.clone();

                let scope = QueryScope::new_paginated_with_cursor(
                    move |_key: (), page_size, continuation| {
                        let call_count = call_count_clone.clone();
                        async move {
                            call_count.fetch_add(1, Ordering::Relaxed);

                            // Add a small delay to simulate network latency
                            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

                            let offset = continuation.unwrap_or(0);
                            let items: Vec<usize> = (offset..offset + page_size).collect();
                            let next = if offset + page_size < 100 {
                                Some(offset + page_size)
                            } else {
                                None
                            };
                            (items, next)
                        }
                    },
                );

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
    async fn test_paginated_cursor_different_page_sizes() {
        crate::test::identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (client, _guard, _owner) = prep_vari!(true);

                let call_count = Arc::new(AtomicUsize::new(0));
                let call_count_clone = call_count.clone();

                let scope = QueryScope::new_paginated_with_cursor(
                    move |_key: (), page_size, continuation| {
                        let call_count = call_count_clone.clone();
                        async move {
                            call_count.fetch_add(1, Ordering::Relaxed);

                            let offset = continuation.unwrap_or(0);
                            let items: Vec<usize> =
                                (offset..std::cmp::min(offset + page_size, 50)).collect();
                            let next = if offset + page_size < 50 {
                                Some(offset + page_size)
                            } else {
                                None
                            };
                            (items, next)
                        }
                    },
                );

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
    async fn test_paginated_cursor_backing_cache_lifecycle(#[case] mode: TestMode) {
        crate::test::identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (client, _guard, _owner) = prep_vari!(true);

                let call_count = Arc::new(AtomicUsize::new(0));
                let call_count_clone = call_count.clone();

                let scope = QueryScope::new_paginated_with_cursor(
                    move |key: String, page_size, continuation| {
                        let call_count = call_count_clone.clone();
                        async move {
                            call_count.fetch_add(1, Ordering::Relaxed);

                            let offset = continuation.unwrap_or(0);
                            let items: Vec<String> = (offset..offset + page_size)
                                .map(|i| format!("{}_{}", key, i))
                                .collect();
                            let next = if offset + page_size < 30 {
                                Some(offset + page_size)
                            } else {
                                None
                            };
                            (items, next)
                        }
                    },
                )
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

    #[derive(Debug, Clone, Copy)]
    enum TestMode {
        /// Test GC behavior: backing cache should only be cleared when all pages are GC'd
        GcTime,
        /// Test stale time behavior: backing cache should be invalidated when any page goes stale
        StaleTime,
    }
}
