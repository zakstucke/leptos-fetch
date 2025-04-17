#![allow(clippy::module_inception)]
#![allow(clippy::type_complexity)]
#![allow(clippy::too_many_arguments)]
#![warn(clippy::disallowed_types)]
#![warn(missing_docs)]
#![doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md"))]
// When docs auto created for docs.rs, will include features, given docs.rs uses nightly by default:
#![cfg_attr(all(doc, CHANNEL_NIGHTLY), feature(doc_auto_cfg))]

mod cache;
mod debug_if_devtools_enabled;
#[cfg(any(
    all(debug_assertions, feature = "devtools"),
    feature = "devtools-always"
))]
mod events;
mod maybe_local;
mod query;
mod query_client;
mod query_maybe_key;
mod query_options;
mod query_scope;
mod resource_drop_guard;
#[cfg(any(
    all(debug_assertions, feature = "devtools"),
    feature = "devtools-always"
))]
mod subs_client;
mod subs_scope;
mod utils;
mod value_with_callbacks;

#[cfg(any(feature = "devtools", feature = "devtools-always"))]
mod dev_tools;
#[cfg(any(feature = "devtools", feature = "devtools-always"))]
pub use dev_tools::QueryDevtools;

pub use query_client::*;
pub use query_options::*;
pub use query_scope::{QueryScope, QueryScopeLocal};

#[cfg(test)]
mod test {
    use std::{
        fmt::Debug,
        marker::PhantomData,
        ptr::NonNull,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };

    use hydration_context::{
        PinnedFuture, PinnedStream, SerializedDataId, SharedContext, SsrSharedContext,
    };

    use leptos::{error::ErrorId, prelude::*, task::Executor};

    use rstest::*;

    use super::*;

    pub struct MockHydrateSharedContext {
        id: AtomicUsize,
        is_hydrating: AtomicBool,
        during_hydration: AtomicBool,

        // CUSTOM_TO_MOCK:

        // errors: LazyLock<Vec<(SerializedDataId, ErrorId, Error)>>,
        // incomplete: LazyLock<Vec<SerializedDataId>>,
        resolved_resources: Vec<(SerializedDataId, String)>,
    }

    impl MockHydrateSharedContext {
        async fn new(ssr_ctx: Option<&SsrSharedContext>) -> Self {
            Self {
                id: AtomicUsize::new(0),
                is_hydrating: AtomicBool::new(true),
                during_hydration: AtomicBool::new(true),
                // errors: LazyLock::new(serialized_errors),
                // incomplete: Lazy::new(incomplete_chunks),
                resolved_resources: if let Some(ssr_ctx) = ssr_ctx {
                    ssr_ctx.consume_buffers().await
                } else {
                    vec![]
                },
            }
        }
    }

    impl Debug for MockHydrateSharedContext {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("MockHydrateSharedContext").finish()
        }
    }

    impl SharedContext for MockHydrateSharedContext {
        fn is_browser(&self) -> bool {
            true
        }

        fn next_id(&self) -> SerializedDataId {
            let id = self.id.fetch_add(1, Ordering::Relaxed);
            SerializedDataId::new(id)
        }

        fn write_async(&self, _id: SerializedDataId, _fut: PinnedFuture<String>) {}

        fn read_data(&self, id: &SerializedDataId) -> Option<String> {
            self.resolved_resources
                .get(id.clone().into_inner())
                .map(|(_, data)| data.to_string())
        }

        fn await_data(&self, _id: &SerializedDataId) -> Option<String> {
            todo!()
        }

        fn pending_data(&self) -> Option<PinnedStream<String>> {
            None
        }

        fn during_hydration(&self) -> bool {
            self.during_hydration.load(Ordering::Relaxed)
        }

        fn hydration_complete(&self) {
            self.during_hydration.store(false, Ordering::Relaxed)
        }

        fn get_is_hydrating(&self) -> bool {
            self.is_hydrating.load(Ordering::Relaxed)
        }

        fn set_is_hydrating(&self, is_hydrating: bool) {
            self.is_hydrating.store(is_hydrating, Ordering::Relaxed)
        }

        fn errors(&self, _boundary_id: &SerializedDataId) -> Vec<(ErrorId, leptos::error::Error)> {
            vec![]
            // self.errors
            //     .iter()
            //     .filter_map(|(boundary, id, error)| {
            //         if boundary == boundary_id {
            //             Some((id.clone(), error.clone()))
            //         } else {
            //             None
            //         }
            //     })
            //     .collect()
        }

        #[inline(always)]
        fn register_error(
            &self,
            _error_boundary: SerializedDataId,
            _error_id: ErrorId,
            _error: leptos::error::Error,
        ) {
        }

        #[inline(always)]
        fn seal_errors(&self, _boundary_id: &SerializedDataId) {}

        fn take_errors(&self) -> Vec<(SerializedDataId, ErrorId, leptos::error::Error)> {
            // self.errors.clone()
            vec![]
        }

        #[inline(always)]
        fn defer_stream(&self, _wait_for: PinnedFuture<()>) {}

        #[inline(always)]
        fn await_deferred(&self) -> Option<PinnedFuture<()>> {
            None
        }

        #[inline(always)]
        fn set_incomplete_chunk(&self, _id: SerializedDataId) {}

        fn get_incomplete_chunk(&self, _id: &SerializedDataId) -> bool {
            // self.incomplete.iter().any(|entry| entry == id)
            false
        }
    }

    macro_rules! prep_server {
        () => {{
            _ = Executor::init_tokio();
            let ssr_ctx = Arc::new(SsrSharedContext::new());
            let owner = Owner::new_root(Some(ssr_ctx.clone()));
            owner.set();
            let client = QueryClient::new();
            (client, ssr_ctx, owner)
        }};
    }

    macro_rules! prep_client {
        () => {{
            _ = Executor::init_tokio();
            let owner = Owner::new_root(Some(Arc::new(MockHydrateSharedContext::new(None).await)));
            owner.set();
            let client = QueryClient::new();
            (client, owner)
        }};
        ($ssr_ctx:expr) => {{
            _ = Executor::init_tokio();
            let owner = Owner::new_root(Some(Arc::new(
                MockHydrateSharedContext::new(Some(&$ssr_ctx)).await,
            )));
            owner.set();
            let client = QueryClient::new();
            (client, owner)
        }};
    }

    macro_rules! prep_vari {
        ($server:expr) => {
            if $server {
                let (client, ssr_ctx, owner) = prep_server!();
                (client, Some(ssr_ctx), owner)
            } else {
                let (client, owner) = prep_client!();
                (client, None, owner)
            }
        };
    }

    macro_rules! tick {
        () => {
            // Executor::poll_local();
            // futures::executor::block_on(Executor::tick());
            Executor::tick().await;
            tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
        };
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ResourceType {
        Local,
        Normal,
        Blocking,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum InvalidationType {
        Query,
        Scope,
        Predicate,
        All,
    }

    macro_rules! vari_new_resource_with_cb {
        ($cb:ident, $client:expr, $fetcher:expr, $keyer:expr, $resource_type:expr, $arc:expr) => {
            match ($resource_type, $arc) {
                (ResourceType::Local, true) => {
                    $cb!(|| $client.arc_local_resource($fetcher, $keyer))
                }
                (ResourceType::Local, false) => {
                    $cb!(|| $client.local_resource($fetcher, $keyer))
                }
                (ResourceType::Normal, true) => {
                    $cb!(|| $client.arc_resource($fetcher, $keyer))
                }
                (ResourceType::Normal, false) => {
                    $cb!(|| $client.resource($fetcher, $keyer))
                }
                (ResourceType::Blocking, true) => {
                    $cb!(|| $client.arc_resource_blocking($fetcher, $keyer))
                }
                (ResourceType::Blocking, false) => {
                    $cb!(|| $client.resource_blocking($fetcher, $keyer))
                }
            }
        };
    }

    const DEFAULT_FETCHER_MS: u64 = 30;
    fn default_fetcher() -> (QueryScope<u64, u64>, Arc<AtomicUsize>) {
        let fetch_calls = Arc::new(AtomicUsize::new(0));
        let fetcher_src = {
            let fetch_calls = fetch_calls.clone();
            move |key: u64| {
                let fetch_calls = fetch_calls.clone();
                async move {
                    tokio::time::sleep(tokio::time::Duration::from_millis(DEFAULT_FETCHER_MS))
                        .await;
                    fetch_calls.fetch_add(1, Ordering::Relaxed);
                    key * 2
                }
            }
        };
        (QueryScope::new(fetcher_src), fetch_calls)
    }

    fn identify_parking_lot_deadlocks() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    let deadlocks = parking_lot::deadlock::check_deadlock();
                    if deadlocks.is_empty() {
                        continue;
                    }

                    println!("{} deadlocks detected", deadlocks.len());
                    for (i, threads) in deadlocks.iter().enumerate() {
                        println!("Deadlock #{}", i);
                        for t in threads {
                            println!("Thread Id {:#?}", t.thread_id());
                            println!("{:#?}", t.backtrace());
                        }
                    }
                }
            });
        });
    }

    #[rstest]
    #[tokio::test]
    async fn test_codecs(
        #[values(ResourceType::Local, ResourceType::Blocking, ResourceType::Normal)] resource_type: ResourceType,
        #[values(false, true)] arc: bool,
        #[values(false, true)] server_ctx: bool,
    ) {
        identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (fetcher, _fetch_calls) = default_fetcher();

                let (client_default, _guard, _owner) = prep_vari!(server_ctx);
                let client_custom =
                    QueryClient::new().set_codec::<codee::binary::FromToBytesCodec>();
                use_context::<QueryClient>();
                use_context::<QueryClient<codee::binary::FromToBytesCodec>>();

                macro_rules! check {
                    ($get_resource:expr) => {{
                        let resource = $get_resource();
                        // On the server cannot actually run local resources:
                        if cfg!(not(feature = "ssr")) || resource_type != ResourceType::Local {
                            assert_eq!(resource.await, 4);
                        }
                    }};
                }

                vari_new_resource_with_cb!(
                    check,
                    client_default,
                    fetcher.clone(),
                    move || 2,
                    resource_type,
                    arc
                );
                vari_new_resource_with_cb!(
                    check,
                    client_custom,
                    fetcher.clone(),
                    move || 2,
                    resource_type,
                    arc
                );
            })
            .await;
    }

    #[rstest]
    #[tokio::test]
    async fn test_no_query_args() {
        identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (client, _guard, _owner) = prep_vari!(false);

                async fn fn_no_arg() -> &'static str {
                    "no_arg"
                }

                async fn fn_with_arg(arg: &'static str) -> &'static str {
                    arg
                }

                assert_eq!(client.fetch_query(fn_no_arg, ()).await, "no_arg");
                assert_eq!(
                    client.fetch_query(fn_with_arg, "with_arg").await,
                    "with_arg"
                );

                assert_eq!(
                    client.fetch_query(QueryScope::new(fn_no_arg), ()).await,
                    "no_arg"
                );
                assert_eq!(
                    client
                        .fetch_query(QueryScope::new(fn_with_arg), "with_arg")
                        .await,
                    "with_arg"
                );

                assert_eq!(
                    client
                        .fetch_query_local(QueryScopeLocal::new(fn_no_arg), ())
                        .await,
                    "no_arg"
                );
                assert_eq!(
                    client
                        .fetch_query_local(QueryScopeLocal::new(fn_with_arg), "with_arg")
                        .await,
                    "with_arg"
                );
            })
            .await;
    }

    /// Local and non-local values should externally be seen as the same cache.
    /// On the same thread they should both use the cached value.
    /// On a different thread, locally cached values shouldn't panic, should just be treated like they don't exist.
    #[rstest]
    #[tokio::test]
    async fn test_shared_cache() {
        identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (fetcher, _fetch_calls) = default_fetcher();
                let (client, _guard, _owner) = prep_vari!(false);

                // Locally set value to 1:
                client.set_query_local(&fetcher, 2, 1);
                assert_eq!(client.get_cached_query(&fetcher, 2), Some(1));

                // Try and get from a different thread, shouldn't try and touch the local cache, should say uncached:
                std::thread::spawn({
                    let fetcher = fetcher.clone();
                    move || {
                        tokio::runtime::Builder::new_current_thread()
                            .build()
                            .unwrap()
                            .block_on(async move {
                                // Should be seen as uncached:
                                assert_eq!(client.get_cached_query(&fetcher, 2), None);

                                // Set nonlocally to 3, set nonlocally to 2:
                                client.set_query(&fetcher, 2, 3);
                                client.set_query_local(&fetcher, 2, 2);
                            });
                    }
                })
                .join()
                .unwrap();
                // Should ignore the local value from the different thread, and get the nonlocal value of 3:
                assert_eq!(client.get_cached_query(&fetcher, 2), Some(3));

                // A clone of the fetcher should still be seen as the same cache:
                let fetcher = fetcher.clone();
                assert_eq!(client.get_cached_query(&fetcher, 2), Some(3));

                // Likewise with the same closure passed into a new scope:
                let (fetcher_2, _fetcher_2_calls) = default_fetcher();
                assert_eq!(client.get_cached_query(&fetcher_2, 2), Some(3));

                // But a new closure should be seen as a new cache:
                let fetcher = QueryScope::new(move |key| {
                    let fetcher = fetcher.clone();
                    async move { query_scope::QueryScopeTrait::query(&fetcher, key).await }
                });
                assert_eq!(client.get_cached_query(&fetcher, 2), None);
            })
            .await;
    }

    // Good example test, nothing new in here though:
    #[rstest]
    #[tokio::test]
    async fn test_infinite() {
        identify_parking_lot_deadlocks();

        // ssr won't load local_resources, which is what we're testing with to avoid needing serde impls.
        #[cfg(not(feature = "ssr"))]
        tokio::task::LocalSet::new()
            .run_until(async move {
                #[derive(Clone, Debug, PartialEq, Eq)]
                struct InfiniteItem(usize);

                #[derive(Clone, Debug, PartialEq, Eq)]
                struct InfiniteList {
                    items: Vec<InfiniteItem>,
                    offset: usize,
                    more_available: bool,
                }

                async fn get_list_items(offset: usize) -> Vec<InfiniteItem> {
                    (offset..offset + 10).map(InfiniteItem).collect()
                }

                async fn get_list_query(_key: ()) -> InfiniteList {
                    let items = get_list_items(0).await;
                    InfiniteList {
                        offset: items.len(),
                        more_available: !items.is_empty(),
                        items,
                    }
                }

                let (client, _guard, _owner) = prep_vari!(false);

                // Initialise the query with the first load.
                // we're not using a reactive key here for extending the list, but declarative updates instead.
                let resource = client.local_resource(get_list_query, || ());
                assert_eq!(
                    resource.await,
                    InfiniteList {
                        items: (0..10).map(InfiniteItem).collect::<Vec<_>>(),
                        offset: 10,
                        more_available: true
                    }
                );

                // When wanting to load more items, update_query_async can be called declaratively to update the cached item:
                client
                    .update_query_async(get_list_query, (), async |last| {
                        if last.more_available {
                            let next_items = get_list_items(last.offset).await;
                            last.offset += next_items.len();
                            last.more_available = !next_items.is_empty();
                            last.items.extend(next_items);
                        }
                    })
                    .await;

                // Should've been updated in place:
                assert_eq!(
                    client.get_cached_query(get_list_query, ()),
                    Some(InfiniteList {
                        items: (0..20).map(InfiniteItem).collect::<Vec<_>>(),
                        offset: 20,
                        more_available: true
                    })
                );
            })
            .await;
    }

    /// prefetch_query
    /// prefetch_query_local
    /// fetch_query
    /// fetch_query_local
    /// update_query
    /// query_exists
    /// update_query_async
    #[rstest]
    #[tokio::test]
    async fn test_declaratives() {
        identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (fetcher, _fetch_calls) = default_fetcher();
                let (client, _guard, _owner) = prep_vari!(false);

                let key = 1;
                assert!(!client.query_exists(&fetcher, key));
                client.set_query_local(&fetcher, key, 1);
                assert_eq!(client.get_cached_query(&fetcher, key), Some(1));
                assert!(client.update_query(&fetcher, key, |value| {
                    value
                        .map(|v| {
                            *v = 2;
                            true
                        })
                        .unwrap_or(false)
                }));
                assert_eq!(client.get_cached_query(&fetcher, key), Some(2));
                client.set_query(&fetcher, key, 3);
                assert_eq!(client.get_cached_query(&fetcher, key), Some(3));
                assert!(client.update_query(&fetcher, key, |value| {
                    value
                        .map(|v| {
                            *v *= 2;
                            true
                        })
                        .unwrap_or(false)
                }));
                assert_eq!(client.get_cached_query(&fetcher, key), Some(6));
                assert!(client.query_exists(&fetcher, key));

                let key = 2;
                assert!(!client.query_exists(&fetcher, key));
                client.prefetch_query_local(&fetcher, key).await;
                assert_eq!(client.get_cached_query(&fetcher, key), Some(4));
                client.clear();
                assert_eq!(client.size(), 0);
                client.prefetch_query(&fetcher, key).await;
                assert_eq!(client.get_cached_query(&fetcher, key), Some(4));

                let key = 3;
                assert!(!client.query_exists(&fetcher, key));
                assert_eq!(client.fetch_query_local(&fetcher, key).await, 6);
                assert!(client.query_exists(&fetcher, key));
                client.clear();
                assert_eq!(client.size(), 0);
                assert_eq!(client.fetch_query(&fetcher, key).await, 6);

                // update_query_async/update_query_async_local:
                assert_eq!(
                    client
                        .update_query_async(&fetcher, key, async |value| {
                            *value += 1;
                            *value
                        })
                        .await,
                    7
                );
                assert_eq!(client.get_cached_query(&fetcher, key), Some(7));
                assert_eq!(
                    client
                        .update_query_async(&fetcher, key, async |value| {
                            *value += 1;
                            *value
                        })
                        .await,
                    8
                );
                assert_eq!(client.get_cached_query(&fetcher, key), Some(8));
                // is_fetching should be true throughout the whole lifetime of update_query_async, even the external async section:
                let is_fetching = client.subscribe_is_fetching_arc(fetcher.clone(), move || key);
                assert!(!is_fetching.get_untracked());
                tokio::join!(
                    async {
                        assert_eq!(
                            client
                                .update_query_async(&fetcher, key, async |value| {
                                    tokio::time::sleep(tokio::time::Duration::from_millis(30))
                                        .await;
                                    *value += 1;
                                    *value
                                })
                                .await,
                            9
                        );
                    },
                    async {
                        let elapsed = std::time::Instant::now();
                        tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
                        tick!();
                        while elapsed.elapsed().as_millis() < 25 {
                            assert!(is_fetching.get_untracked());
                            tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
                        }
                    }
                );
                // To make sure finished
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                assert!(!is_fetching.get_untracked());
            })
            .await;
    }

    /// Make sure refetching works at the expected time, and only does so once there are active resources using it.
    #[rstest]
    #[tokio::test]
    async fn test_refetch(
        #[values(ResourceType::Local, ResourceType::Blocking, ResourceType::Normal)] resource_type: ResourceType,
        #[values(false, true)] arc: bool,
    ) {
        identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                const REFETCH_TIME_MS: u64 = 100;
                const FETCH_TIME_MS: u64 = 10;

                let fetch_calls = Arc::new(AtomicUsize::new(0));
                let fetcher = {
                    let fetch_calls = fetch_calls.clone();
                    move |key: u64| {
                        fetch_calls.fetch_add(1, Ordering::Relaxed);
                        async move {
                            tokio::time::sleep(tokio::time::Duration::from_millis(FETCH_TIME_MS)).await;
                            key * 2
                        }
                    }
                };
                let fetcher = QueryScope::new(
                    fetcher
                ).set_options(QueryOptions::new().set_refetch_interval(std::time::Duration::from_millis(REFETCH_TIME_MS)));

                let (client, _guard, owner) = prep_vari!(false);

                macro_rules! with_tmp_owner {
                    ($body:block) => {{
                        let tmp_owner = owner.child();
                        tmp_owner.set();
                        $body
                        owner.set();
                    }};
                }

                macro_rules! check {
                    ($get_resource:expr) => {{

                        // On the server cannot actually run local resources:
                        if cfg!(not(feature = "ssr")) || resource_type != ResourceType::Local {

                            // Initial caching:
                            with_tmp_owner! {{
                                assert_eq!($get_resource().await, 4);
                                assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);
                                assert_eq!(client.size(), 1);

                                // less than refetch_time shouldn't have recalled:
                                assert_eq!($get_resource().await, 4);
                                assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);
                                assert_eq!(client.size(), 1);
                            }}

                            // hit refetch time with no active resources shouldn't have refetched:
                            tokio::time::sleep(tokio::time::Duration::from_millis(REFETCH_TIME_MS + FETCH_TIME_MS)).await;
                            tick!();
                            assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);

                            // hit refetch_time when active resource should refetch:
                            with_tmp_owner! {{
                                // Because the refetch call would've still invalidated the value, the new resource should trigger the refetch automatically:
                                let _resource = $get_resource();
                                tokio::time::sleep(tokio::time::Duration::from_millis(FETCH_TIME_MS)).await;
                                tick!();
                                assert_eq!(fetch_calls.load(Ordering::Relaxed), 2);

                                // There's an active resource now, so this should trigger again without needing to touch the resources:
                                tokio::time::sleep(tokio::time::Duration::from_millis(REFETCH_TIME_MS + FETCH_TIME_MS)).await;
                                tick!();

                                assert_eq!(fetch_calls.load(Ordering::Relaxed), 3);
                                assert_eq!($get_resource().await, 4);
                                assert_eq!(fetch_calls.load(Ordering::Relaxed), 3);

                                // Run again to make sure:
                                tokio::time::sleep(tokio::time::Duration::from_millis(REFETCH_TIME_MS + FETCH_TIME_MS)).await;
                                tick!();

                                assert_eq!(fetch_calls.load(Ordering::Relaxed), 4);
                                assert_eq!($get_resource().await, 4);
                                assert_eq!(fetch_calls.load(Ordering::Relaxed), 4);
                            }}

                            // Should stop refetching once all resources are dropped:
                            tokio::time::sleep(tokio::time::Duration::from_millis(REFETCH_TIME_MS + FETCH_TIME_MS)).await;
                            tick!();
                            assert_eq!(fetch_calls.load(Ordering::Relaxed), 4);
                        }
                    }};
                }

                vari_new_resource_with_cb!(
                    check,
                    client,
                    fetcher.clone(),
                    || 2,
                    resource_type,
                    arc
                );
            })
            .await;
    }

    /// Make sure the cache is cleaned up at the expected time, and only do so once no resources are using it.
    #[rstest]
    #[tokio::test]
    async fn test_gc(
        #[values(ResourceType::Local, ResourceType::Blocking, ResourceType::Normal)] resource_type: ResourceType,
        #[values(false, true)] arc: bool,
    ) {
        identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
        .run_until(async move {
                const GC_TIME_MS: u64 = 30;

                let fetch_calls = Arc::new(AtomicUsize::new(0));
                let fetcher = {
                    let fetch_calls = fetch_calls.clone();
                    move |key: u64| {
                        fetch_calls.fetch_add(1, Ordering::Relaxed);
                        async move {
                            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                            key * 2
                        }
                    }
                };
                let fetcher = QueryScope::new(
                    fetcher
                ).set_options(QueryOptions::new().set_gc_time(std::time::Duration::from_millis(GC_TIME_MS)));

                let (client, _guard, owner) = prep_vari!(false);

                macro_rules! with_tmp_owner {
                    ($body:block) => {{
                        let tmp_owner = owner.child();
                        tmp_owner.set();
                        $body
                        owner.set();
                    }};
                }

                macro_rules! check {
                    ($get_resource:expr) => {{
                        let subscribed = client.subscribe_value(fetcher.clone(), move || 2);

                        // On the server cannot actually run local resources:
                        if cfg!(not(feature = "ssr")) || resource_type != ResourceType::Local {

                            // Initial caching:
                            with_tmp_owner! {{
                                assert_eq!($get_resource().await, 4);
                                assert_eq!(subscribed.get_untracked(), Some(4));
                                assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);
                                assert_eq!(client.size(), 1);

                                // < gc_time shouldn't have cleaned up:
                                tick!();
                                assert_eq!($get_resource().await, 4);
                                assert_eq!(subscribed.get_untracked(), Some(4));
                                assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);
                                assert_eq!(client.size(), 1);
                            }}

                            // all resources dropped when <gc_time shouldn't have cleaned up:
                            with_tmp_owner! {{
                                tick!();
                                assert_eq!($get_resource().await, 4);
                                assert_eq!(subscribed.get_untracked(), Some(4));
                                assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);
                                assert_eq!(client.size(), 1);
                            }}

                            // >gc_time when active resource shouldn't have cleaned up:
                            with_tmp_owner! {{
                                let _resource = $get_resource();

                                tokio::time::sleep(tokio::time::Duration::from_millis(GC_TIME_MS)).await;
                                tick!();

                                // >gc_time shouldn't cleanup because there's an active resource:
                                assert_eq!($get_resource().await, 4);
                                assert_eq!(subscribed.get_untracked(), Some(4));
                                assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);
                                assert_eq!(client.size(), 1);
                            }}

                            // >gc_time and no resources should now have been cleaned up, causing a new fetch:
                            with_tmp_owner! {{
                                assert_eq!(client.size(), 1);

                                assert_eq!(subscribed.get_untracked(), Some(4));
                                tokio::time::sleep(tokio::time::Duration::from_millis(GC_TIME_MS)).await;
                                tick!();
                                assert_eq!(subscribed.get_untracked(), None);

                                assert_eq!($get_resource().await, 4);
                                assert_eq!(fetch_calls.load(Ordering::Relaxed), 2);
                            }}

                            // Final cleanup:
                            tokio::time::sleep(tokio::time::Duration::from_millis(GC_TIME_MS)).await;
                            tick!();
                            assert_eq!(client.size(), 0);
                            assert_eq!(subscribed.get_untracked(), None);
                        }
                    }};
                }

                vari_new_resource_with_cb!(
                    check,
                    client,
                    fetcher.clone(),
                    || 2,
                    resource_type,
                    arc
                );
            })
            .await;
    }

    /// Make sure !Send and !Sync values work with local resources.
    #[rstest]
    #[tokio::test]
    async fn test_unsync(#[values(false, true)] arc: bool) {
        identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                #[derive(Debug)]
                struct UnsyncValue(u64, PhantomData<NonNull<()>>);
                impl PartialEq for UnsyncValue {
                    fn eq(&self, other: &Self) -> bool {
                        self.0 == other.0
                    }
                }
                impl Eq for UnsyncValue {}
                impl Clone for UnsyncValue {
                    fn clone(&self) -> Self {
                        Self(self.0, PhantomData)
                    }
                }
                impl UnsyncValue {
                    fn new(value: u64) -> Self {
                        Self(value, PhantomData)
                    }
                }

                let fetch_calls = Arc::new(AtomicUsize::new(0));
                let fetcher = {
                    let fetch_calls = fetch_calls.clone();
                    move |key: u64| {
                        fetch_calls.fetch_add(1, Ordering::Relaxed);
                        async move {
                            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                            UnsyncValue::new(key * 2)
                        }
                    }
                };
                let fetcher = QueryScopeLocal::new(fetcher);

                let (client, _guard, _owner) = prep_vari!(false);

                macro_rules! check {
                    ($get_resource:expr) => {{
                        let resource = $get_resource();
                        let subscribed = client.subscribe_value_local(fetcher.clone(), move || 2);

                        // Should be None initially with the sync methods:
                        assert!(resource.get_untracked().is_none());
                        assert!(resource.try_get_untracked().unwrap().is_none());
                        assert!(resource.get().is_none());
                        assert!(resource.try_get().unwrap().is_none());
                        assert!(resource.read().is_none());
                        assert!(resource.try_read().as_deref().unwrap().is_none());
                        assert!(subscribed.get_untracked().is_none());

                        // On the server cannot actually run local resources:
                        if cfg!(not(feature = "ssr")) {
                            assert_eq!(resource.await, UnsyncValue::new(4));
                            assert_eq!(subscribed.get_untracked(), Some(UnsyncValue::new(4)));
                            assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);

                            tick!();

                            assert_eq!($get_resource().await, UnsyncValue::new(4));
                            assert_eq!(subscribed.get_untracked(), Some(UnsyncValue::new(4)));
                            assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);
                        }
                    }};
                }

                match arc {
                    true => {
                        check!(|| client.arc_local_resource(fetcher.clone(), || 2))
                    }
                    false => {
                        check!(|| client.local_resource(fetcher.clone(), || 2))
                    }
                }
            })
            .await;
    }

    #[rstest]
    #[tokio::test]
    async fn test_subscribe_is_fetching_and_loading(
        #[values(ResourceType::Local, ResourceType::Blocking, ResourceType::Normal)] resource_type: ResourceType,
        #[values(false, true)] arc: bool,
        #[values(false, true)] server_ctx: bool,
    ) {
        identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (fetcher, fetch_calls) = default_fetcher();
                let (client, _guard, _owner) = prep_vari!(server_ctx);

                macro_rules! check {
                    ($get_resource:expr) => {{
                        // On the server cannot actually run local resources:
                        if cfg!(not(feature = "ssr")) || resource_type != ResourceType::Local {
                            assert_eq!(client.subscriber_count(), 0);
                            let is_fetching = client.subscribe_is_fetching_arc(fetcher.clone(), || 2);
                            assert_eq!(is_fetching.get_untracked(), false);
                            assert_eq!(client.subscriber_count(), 1);
                            let is_fetching_other = client.subscribe_is_fetching_arc(fetcher.clone(), || 3);
                            assert_eq!(is_fetching_other.get_untracked(), false);
                            assert_eq!(client.subscriber_count(), 2);
                            let is_loading = client.subscribe_is_loading_arc(fetcher.clone(), || 2);
                            assert_eq!(is_loading.get_untracked(), false);
                            assert_eq!(client.subscriber_count(), 3);
                            let is_loading_other = client.subscribe_is_loading_arc(fetcher.clone(), || 3);
                            assert_eq!(is_loading_other.get_untracked(), false);
                            assert_eq!(client.subscriber_count(), 4);


                            macro_rules! check_all {
                                ($expected:expr) => {{
                                    assert_eq!(is_fetching.get_untracked(), $expected);
                                    assert_eq!(is_fetching_other.get_untracked(), $expected);
                                    assert_eq!(is_loading.get_untracked(), $expected);
                                    assert_eq!(is_loading_other.get_untracked(), $expected);
                                }};
                            }

                            check_all!(false);

                            tokio::join!(
                                async {
                                    assert_eq!($get_resource().await, 4);
                                },
                                async {
                                    let elapsed = std::time::Instant::now();
                                    tick!();
                                    while elapsed.elapsed().as_millis() < DEFAULT_FETCHER_MS.into() {
                                        assert_eq!(is_fetching.get_untracked(), true);
                                        assert_eq!(is_fetching_other.get_untracked(), false);
                                        assert_eq!(is_loading.get_untracked(), true);
                                        assert_eq!(is_loading_other.get_untracked(), false);
                                        tick!();
                                    }
                                }
                            );
                            assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);

                            check_all!(false);

                            assert_eq!($get_resource().await, 4);
                            assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);

                            check_all!(false);

                            // Already in cache now, all should be false:
                            tokio::join!(
                                async {
                                    assert_eq!($get_resource().await, 4);
                                },
                                async {
                                    let elapsed = std::time::Instant::now();
                                    tick!();
                                    while elapsed.elapsed().as_millis() < DEFAULT_FETCHER_MS.into() {
                                        check_all!(false);
                                        tick!();
                                    }
                                }
                            );
                            assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);

                            client.invalidate_query(fetcher.clone(), &2);

                            tokio::join!(
                                async {
                                    assert_eq!($get_resource().await, 4);
                                    // This should have returned the old value straight away, but the refetch will have been initiated in the background:
                                    assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);
                                    tokio::time::sleep(std::time::Duration::from_millis(DEFAULT_FETCHER_MS + 10)).await;
                                },
                                async {
                                    let elapsed = std::time::Instant::now();
                                    tick!();
                                    while elapsed.elapsed().as_millis() < DEFAULT_FETCHER_MS.into() {
                                        assert_eq!(is_fetching.get_untracked(), true);
                                        assert_eq!(is_fetching_other.get_untracked(), false);
                                        // Loading should all be false as this is just a refetch now, 
                                        // the get_resource().await will actually return straight away, but it'll trigger the refetch.
                                        assert_eq!(is_loading.get_untracked(), false);
                                        assert_eq!(is_loading_other.get_untracked(), false);
                                        tick!();
                                    }
                                }
                            );
                            assert_eq!(fetch_calls.load(Ordering::Relaxed), 2);

                            drop(is_fetching);
                            assert_eq!(client.subscriber_count(), 3);
                            drop(is_loading);
                            assert_eq!(client.subscriber_count(), 2);
                            drop(is_fetching_other);
                            assert_eq!(client.subscriber_count(), 1);
                            drop(is_loading_other);
                            assert_eq!(client.subscriber_count(), 0);

                            client.clear();
                            assert_eq!(client.size(), 0);

                            // Make sure subscriptions start in true state if in the middle of loading:
                            tokio::join!(
                                async {
                                    assert_eq!($get_resource().await, 4);
                                },
                                async {
                                    tick!();
                                    let is_fetching = client.subscribe_is_fetching_arc(fetcher.clone(), || 2);
                                    let is_loading = client.subscribe_is_loading_arc(fetcher.clone(), || 2);
                                    assert_eq!(is_fetching.get_untracked(), true);
                                    assert_eq!(is_loading.get_untracked(), true);
                                    assert_eq!(client.subscriber_count(), 2);
                                    tokio::time::sleep(std::time::Duration::from_millis(DEFAULT_FETCHER_MS + 10)).await;
                                    assert_eq!(is_fetching.get_untracked(), false);
                                    assert_eq!(is_loading.get_untracked(), false);
                                }
                            );
                            assert_eq!(client.subscriber_count(), 0);

                            // Make sure refetch only too:
                            client.invalidate_query(fetcher.clone(), &2);
                            tokio::join!(
                                async {
                                    assert_eq!($get_resource().await, 4);
                                    tokio::time::sleep(std::time::Duration::from_millis(DEFAULT_FETCHER_MS + 10)).await;
                                },
                                async {
                                    tick!();
                                    let is_fetching = client.subscribe_is_fetching_arc(fetcher.clone(), || 2);
                                    let is_loading = client.subscribe_is_loading_arc(fetcher.clone(), || 2);
                                    assert_eq!(is_fetching.get_untracked(), true);
                                    assert_eq!(is_loading.get_untracked(), false);
                                    assert_eq!(client.subscriber_count(), 2);
                                    tokio::time::sleep(std::time::Duration::from_millis(DEFAULT_FETCHER_MS + 10)).await;
                                    assert_eq!(is_fetching.get_untracked(), false);
                                    assert_eq!(is_loading.get_untracked(), false);
                                }
                            );
                            assert_eq!(client.subscriber_count(), 0);
                            client.clear();

                            // Now confirm the subscribers keyer is reactive correctly, and subscriptions don't accidentally say true for the wrong key:
                            let sub_key_signal = RwSignal::new(2);
                            let resource_key_signal = RwSignal::new(2);
                            let is_fetching = client.subscribe_is_fetching(fetcher.clone(), move || sub_key_signal.get());
                            let is_loading = client.subscribe_is_loading(fetcher.clone(), move || sub_key_signal.get());
                            assert_eq!(is_fetching.get_untracked(), false);
                            assert_eq!(is_loading.get_untracked(), false);

                            let _resource = client.resource(fetcher.clone(), move || resource_key_signal.get());

                            // The creation of the resource should've triggered the initial fetch:
                            assert_eq!(is_fetching.get_untracked(), true);
                            assert_eq!(is_loading.get_untracked(), true);
                            tokio::time::sleep(std::time::Duration::from_millis(DEFAULT_FETCHER_MS + 10)).await;
                            assert_eq!(is_fetching.get_untracked(), false);
                            assert_eq!(is_loading.get_untracked(), false);

                            // Sanity check confirming it won't refetch if the key hasn't actually changed:
                            sub_key_signal.set(2);
                            resource_key_signal.set(2);
                            tick!();
                            assert_eq!(is_fetching.get_untracked(), false);
                            assert_eq!(is_loading.get_untracked(), false);

                            // New value should cause a fresh query, subscriber should match:
                            resource_key_signal.set(3);
                            sub_key_signal.set(3);
                            tick!();
                            assert_eq!(is_fetching.get_untracked(), true);
                            assert_eq!(is_loading.get_untracked(), true);
                            tokio::time::sleep(std::time::Duration::from_millis(DEFAULT_FETCHER_MS + 10)).await;
                            assert_eq!(is_fetching.get_untracked(), false);
                            assert_eq!(is_loading.get_untracked(), false);

                            // Stale should still mean only is_fetching is true:
                            client.invalidate_query(fetcher.clone(), &3);
                            tick!();
                            assert_eq!(is_fetching.get_untracked(), true);
                            assert_eq!(is_loading.get_untracked(), false);
                            tokio::time::sleep(std::time::Duration::from_millis(DEFAULT_FETCHER_MS + 10)).await;
                            assert_eq!(is_fetching.get_untracked(), false);
                            assert_eq!(is_loading.get_untracked(), false);

                            // If the resource diverges, subscriber shouldn't notice:
                            resource_key_signal.set(4);
                            tick!();
                            assert_eq!(is_fetching.get_untracked(), false);
                            assert_eq!(is_loading.get_untracked(), false);
                        }
                    }};
                }

                vari_new_resource_with_cb!(
                    check,
                    client,
                    fetcher.clone(),
                    || 2,
                    resource_type,
                    arc
                );
            })
            .await;
    }

    /// Make sure resources reload when queries invalidated correctly.
    #[rstest]
    #[tokio::test]
    async fn test_optional_key(
        #[values(ResourceType::Local, ResourceType::Blocking, ResourceType::Normal)] resource_type: ResourceType,
        #[values(false, true)] arc: bool,
        #[values(false, true)] server_ctx: bool,
    ) {
        identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (fetcher, fetch_calls) = default_fetcher();
                let (client, _guard, _owner) = prep_vari!(server_ctx);

                let key_value = RwSignal::new(None);
                let keyer = move || key_value.get();

                macro_rules! check {
                    ($get_resource:expr) => {{
                        // // TODO enable these on 0.8 now SendWrapper sorted:
                        // let resource = $get_resource();
                        // assert_eq!(resource.get_untracked(), Some(None));
                        // assert_eq!(resource.try_get_untracked(), Some(Some(None)));

                        // On the server cannot actually run local resources:
                        if cfg!(not(feature = "ssr")) || resource_type != ResourceType::Local {
                            assert_eq!($get_resource().await, None);

                            let sub_is_loading =
                                client.subscribe_is_loading(fetcher.clone(), keyer);
                            let sub_is_fetching =
                                client.subscribe_is_fetching(fetcher.clone(), keyer);
                            let sub_value = client.subscribe_value(fetcher.clone(), keyer);

                            assert_eq!(sub_is_loading.get_untracked(), false);
                            assert_eq!(sub_is_fetching.get_untracked(), false);
                            assert_eq!(sub_value.get_untracked(), None);

                            key_value.set(Some(2));
                            tick!();

                            assert_eq!(sub_is_loading.get_untracked(), true);
                            assert_eq!(sub_is_fetching.get_untracked(), true);

                            assert_eq!($get_resource().await, Some(4));
                            assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);
                            assert_eq!(sub_value.get_untracked(), Some(4));
                            assert_eq!(sub_is_loading.get_untracked(), false);
                            assert_eq!(sub_is_fetching.get_untracked(), false);
                        }
                    }};
                }

                vari_new_resource_with_cb!(
                    check,
                    client,
                    fetcher.clone(),
                    keyer,
                    resource_type,
                    arc
                );
            })
            .await;
    }

    /// Make sure resources reload when queries invalidated correctly.
    #[rstest]
    #[tokio::test]
    async fn test_invalidation(
        #[values(ResourceType::Local, ResourceType::Blocking, ResourceType::Normal)] resource_type: ResourceType,
        #[values(false, true)] arc: bool,
        #[values(false, true)] server_ctx: bool,
        #[values(
            InvalidationType::Query,
            InvalidationType::Scope,
            InvalidationType::Predicate,
            InvalidationType::All
        )]
        invalidation_type: InvalidationType,
    ) {
        identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (fetcher, fetch_calls) = default_fetcher();
                let (client, _guard, _owner) = prep_vari!(server_ctx);

                macro_rules! check {
                    ($get_resource:expr) => {{
                        let resource = $get_resource();

                        // Should be None initially with the sync methods:
                        assert!(resource.get_untracked().is_none());
                        assert!(resource.try_get_untracked().unwrap().is_none());
                        assert!(resource.get().is_none());
                        assert!(resource.try_get().unwrap().is_none());
                        assert!(resource.read().is_none());
                        assert!(resource.try_read().as_deref().unwrap().is_none());

                        // On the server cannot actually run local resources:
                        if cfg!(not(feature = "ssr")) || resource_type != ResourceType::Local {
                            assert_eq!(resource.await, 4);
                            assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);

                            tick!();

                            // Shouldn't change despite ticking:
                            assert_eq!($get_resource().await, 4);
                            assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);

                            match invalidation_type {
                                InvalidationType::Query => {
                                    client.invalidate_query(fetcher.clone(), &2);
                                }
                                InvalidationType::Scope => {
                                    client.invalidate_query_scope(fetcher.clone());
                                }
                                InvalidationType::Predicate => {
                                    client.invalidate_queries_with_predicate(fetcher.clone(), |key| key == &2);
                                }
                                InvalidationType::All => {
                                    client.invalidate_all_queries();
                                }
                            }

                            // Because it should now be stale, not gc'd,
                            // sync fns on a new resource instance should still return the new value, it just means a background refresh has been triggered:
                            // TODO update in 0.8 once can test output of LocalResource without the SendWrapper:
                            let resource2 = client.resource(fetcher.clone(), || 2);
                            assert_eq!(resource2.get_untracked(), Some(4));
                            assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);
                            // macro_rules! check2 {
                            //     (resource2:expr) => {{
                            //         assert_eq!(*&resource2.get_untracked(), Some(4));
                            //         assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);
                            //     }};
                            // }
                            // vari_new_resource_with_cb!(check2, client, fetcher.clone(), || 2, resource_type, arc);

                            // Because the resource should've been auto invalidated, a tick should cause it to auto refetch:
                            tick!();
                            tokio::time::sleep(std::time::Duration::from_millis(DEFAULT_FETCHER_MS + 10)).await;
                            assert_eq!(fetch_calls.load(Ordering::Relaxed), 2);
                            assert_eq!($get_resource().await, 4);
                            assert_eq!(fetch_calls.load(Ordering::Relaxed), 2);
                        }
                    }};
                }

                vari_new_resource_with_cb!(
                    check,
                    client,
                    fetcher.clone(),
                    || 2,
                    resource_type,
                    arc
                );
            })
            .await;
    }

    #[rstest]
    #[tokio::test]
    async fn test_key_tracked_autoreload(
        #[values(ResourceType::Local, ResourceType::Blocking, ResourceType::Normal)] resource_type: ResourceType,
        #[values(false, true)] arc: bool,
        #[values(false, true)] server_ctx: bool,
    ) {
        identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (fetcher, fetch_calls) = default_fetcher();

                let (client, _guard, _owner) = prep_vari!(server_ctx);

                let add_size = RwSignal::new(1);

                macro_rules! check {
                    ($get_resource:expr) => {{
                        let resource = $get_resource();
                        let subscribed = client.subscribe_value(fetcher.clone(), move || add_size.get());

                        // Should be None initially with the sync methods:
                        assert!(resource.get_untracked().is_none());
                        assert!(resource.try_get_untracked().unwrap().is_none());
                        assert!(resource.get().is_none());
                        assert!(resource.try_get().unwrap().is_none());
                        assert!(resource.read().is_none());
                        assert!(resource.try_read().as_deref().unwrap().is_none());
                        assert_eq!(subscribed.get_untracked(), None);

                        // On the server cannot actually run local resources:
                        if cfg!(not(feature = "ssr")) || resource_type != ResourceType::Local {
                            assert_eq!(resource.await, 2);
                            assert_eq!($get_resource().await, 2);
                            assert_eq!(subscribed.get_untracked(), Some(2));
                            assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);

                            // Update the resource key:
                            add_size.set(2);

                            // Until the new fetch has completed, the old should still be returned:
                            tick!();
                            // TODO remove the local resource once 0.8 if check here and switch these checks to .get() from .await, as we're really trying to test lifecycle.
                            // seems there's a different future/.await handling between local and normal resources that breaks the test but not actually important.
                            if resource_type != ResourceType::Local {
                                assert_eq!($get_resource().await, 2);
                                assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);
                                assert_eq!(subscribed.get_untracked(), Some(2));
                            }

                            // Wait for the new to complete:
                            tokio::time::sleep(std::time::Duration::from_millis(DEFAULT_FETCHER_MS + 10)).await;
                            tick!();

                            // Should have updated to the new value:
                            assert_eq!($get_resource().await, 4);
                            assert_eq!(fetch_calls.load(Ordering::Relaxed), 2);
                            assert_eq!($get_resource().await, 4);
                            assert_eq!(subscribed.get_untracked(), Some(4));
                            assert_eq!(fetch_calls.load(Ordering::Relaxed), 2);
                        }
                    }};
                }

                vari_new_resource_with_cb!(
                    check,
                    client,
                    fetcher.clone(),
                    move || add_size.get(),
                    resource_type,
                    arc
                );
            })
            .await;
    }

    /// Make sure values on first receival and cached all stick to their specific key.
    #[rstest]
    #[tokio::test]
    async fn test_key_integrity(
        #[values(ResourceType::Local, ResourceType::Blocking, ResourceType::Normal)] resource_type: ResourceType,
        #[values(false, true)] arc: bool,
        #[values(false, true)] server_ctx: bool,
    ) {
        identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                // On the server cannot actually run local resources:
                if cfg!(feature = "ssr") && resource_type == ResourceType::Local {
                    return;
                }

                let (fetcher, fetch_calls) = default_fetcher();
                let (client, _guard, _owner) = prep_vari!(server_ctx);

                let keys = [1, 2, 3, 4, 5];
                let results = futures::future::join_all(keys.iter().cloned().map(|key| {
                    let fetcher = fetcher.clone();
                    async move {
                        macro_rules! cb {
                            ($get_resource:expr) => {{
                                let resource = $get_resource();
                                resource.await
                            }};
                        }
                        vari_new_resource_with_cb!(
                            cb,
                            client,
                            fetcher,
                            move || key,
                            resource_type,
                            arc
                        )
                    }
                }))
                .await;
                assert_eq!(results, vec![2, 4, 6, 8, 10]);
                assert_eq!(fetch_calls.load(Ordering::Relaxed), 5);

                // Call again, each should still be accurate, but each should be cached so fetch call doesn't increase:
                let results = futures::future::join_all(keys.iter().cloned().map(|key| {
                    let fetcher = fetcher.clone();
                    async move {
                        macro_rules! cb {
                            ($get_resource:expr) => {{
                                let resource = $get_resource();
                                resource.await
                            }};
                        }
                        vari_new_resource_with_cb!(
                            cb,
                            client,
                            fetcher,
                            move || key,
                            resource_type,
                            arc
                        )
                    }
                }))
                .await;
                assert_eq!(results, vec![2, 4, 6, 8, 10]);
                assert_eq!(fetch_calls.load(Ordering::Relaxed), 5);
            })
            .await;
    }

    /// Make sure resources that are loaded together only run once but share the value.
    #[rstest]
    #[tokio::test]
    async fn test_resource_race(
        #[values(ResourceType::Local, ResourceType::Blocking, ResourceType::Normal)] resource_type: ResourceType,
        #[values(false, true)] arc: bool,
        #[values(false, true)] server_ctx: bool,
    ) {
        identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                // On the server cannot actually run local resources:
                if cfg!(feature = "ssr") && resource_type == ResourceType::Local {
                    return;
                }

                let (fetcher, fetch_calls) = default_fetcher();
                let (client, _guard, _owner) = prep_vari!(server_ctx);

                let keyer = || 1;
                let results = futures::future::join_all((0..10).map(|_| {
                    let fetcher = fetcher.clone();
                    async move {
                        macro_rules! cb {
                            ($get_resource:expr) => {{
                                let resource = $get_resource();
                                resource.await
                            }};
                        }
                        vari_new_resource_with_cb!(cb, client, fetcher, keyer, resource_type, arc)
                    }
                }))
                .await
                .into_iter()
                .collect::<Vec<_>>();
                assert_eq!(results, vec![2; 10]);
                assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);
            })
            .await;
    }

    #[cfg(feature = "ssr")]
    #[tokio::test]
    async fn test_resource_cross_stream_caching() {
        identify_parking_lot_deadlocks();
        tokio::task::LocalSet::new()
            .run_until(async move {
                for maybe_sleep_ms in &[None, Some(10), Some(30)] {
                    let (client, ssr_ctx, _owner) = prep_server!();

                    let fetch_calls = Arc::new(AtomicUsize::new(0));
                    let fetcher = {
                        let fetch_calls = fetch_calls.clone();
                        move |key: u64| {
                            fetch_calls.fetch_add(1, Ordering::Relaxed);
                            async move {
                                if let Some(sleep_ms) = maybe_sleep_ms {
                                    tokio::time::sleep(tokio::time::Duration::from_millis(
                                        *sleep_ms as u64,
                                    ))
                                    .await;
                                }
                                key * 2
                            }
                        }
                    };
                    let fetcher = QueryScope::new(fetcher);

                    let keyer = || 1;

                    // First call should require a fetch.
                    assert_eq!(client.arc_resource(fetcher.clone(), keyer).await, 2);
                    assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);

                    // Second should be cached by the query client because same key:
                    assert_eq!(client.arc_resource(fetcher.clone(), keyer).await, 2);
                    assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);

                    // Should make it over to the frontend too:
                    let (client, _owner) = prep_client!(ssr_ctx);

                    // This will stream from the first ssr resource:
                    assert_eq!(client.arc_resource(fetcher.clone(), keyer).await, 2);
                    assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);

                    // This will stream from the second ssr resource:
                    assert_eq!(client.arc_resource(fetcher.clone(), keyer).await, 2);
                    assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);

                    // This drives the effect that will put the resource into the frontend cache:
                    tick!();

                    // This didn't happen in ssr so nothing to stream,
                    // but the other 2 resources shoud've still put themselves into the frontend cache,
                    // so this should get picked up by that.
                    assert_eq!(client.arc_resource(fetcher.clone(), keyer).await, 2);
                    assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);

                    // Reset and confirm works for non blocking too:
                    let (ssr_client, ssr_ctx, _owner) = prep_server!();
                    fetch_calls.store(0, Ordering::Relaxed);

                    // Don't await:
                    let ssr_resource_1 = ssr_client.arc_resource(fetcher.clone(), keyer);
                    let ssr_resource_2 = ssr_client.arc_resource(fetcher.clone(), keyer);

                    let (hydrate_client, _owner) = prep_client!(ssr_ctx);

                    // Matching 2 resources on hydrate, these should stream:
                    let hydrate_resource_1 = hydrate_client.arc_resource(fetcher.clone(), keyer);
                    let hydrate_resource_2 = hydrate_client.arc_resource(fetcher.clone(), keyer);

                    // Wait for all 4 together, should still only have had 1 fetch.
                    let results = futures::future::join_all(
                        vec![
                            hydrate_resource_2,
                            ssr_resource_1,
                            ssr_resource_2,
                            hydrate_resource_1,
                        ]
                        .into_iter()
                        .map(|resource| async move { resource.await }),
                    )
                    .await
                    .into_iter()
                    .collect::<Vec<_>>();

                    assert_eq!(results, vec![2, 2, 2, 2]);
                    assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);

                    tick!();

                    // This didn't have a matching backend one so should be using the populated cache and still not fetch:
                    assert_eq!(hydrate_client.arc_resource(fetcher.clone(), keyer).await, 2);
                    assert_eq!(
                        fetch_calls.load(Ordering::Relaxed),
                        1,
                        "{:?}ms",
                        maybe_sleep_ms
                    );
                }
            })
            .await;
    }
}
