#![allow(clippy::module_inception)]
#![allow(clippy::type_complexity)]
#![allow(clippy::too_many_arguments)]
#![warn(clippy::disallowed_types)]
#![warn(missing_docs)]

#![doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md"))]

// When docs auto created for docs.rs, will include features, given docs.rs uses nightly by default:
#![cfg_attr(all(doc, CHANNEL_NIGHTLY), feature(doc_auto_cfg))]

mod cache;
mod query;
mod query_client;
mod query_options;
mod query_scope;
mod utils;

pub use query_client::*;
pub use query_options::*;
pub use query_scope::*;

#[cfg(test)]
mod test {
    use std::{
        fmt::Debug,
        marker::PhantomData,
        ptr::NonNull,
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc,
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
            (client, ssr_ctx)
        }};
    }

    macro_rules! prep_client {
        () => {{
            let owner = Owner::new_root(Some(Arc::new(MockHydrateSharedContext::new(None).await)));
            owner.set();
            let client = QueryClient::new();
            client
        }};
        ($ssr_ctx:expr) => {{
            let owner = Owner::new_root(Some(Arc::new(
                MockHydrateSharedContext::new(Some(&$ssr_ctx)).await,
            )));
            owner.set();
            let client = QueryClient::new();
            client
        }};
    }

    macro_rules! prep_vari {
        ($server:expr) => {
            if $server {
                let (client, ssr_ctx) = prep_server!();
                (client, Some(ssr_ctx))
            } else {
                (prep_client!(), None)
            }
        };
    }

    macro_rules! tick {
        () => {
            // Executor::poll_local();
            // futures::executor::block_on(Executor::tick());
            Executor::tick().await;
        };
    }

    #[derive(Debug, Clone, Copy)]
    enum ResourceType {
        Local,
        Normal,
        Blocking,
    }

    macro_rules! vari_new_resource_with_cb {
        ($cb:ident, $client:expr, $fetcher:expr, $keyer:expr, $resource_type:expr, $arc:expr) => {
            match ($resource_type, $arc) {
                (ResourceType::Local, true) => {
                    $cb!($client.arc_local_resource($fetcher, $keyer))
                }
                (ResourceType::Local, false) => {
                    $cb!($client.local_resource($fetcher, $keyer))
                }
                (ResourceType::Normal, true) => {
                    $cb!($client.arc_resource($fetcher, $keyer))
                }
                (ResourceType::Normal, false) => {
                    $cb!($client.resource($fetcher, $keyer))
                }
                (ResourceType::Blocking, true) => {
                    $cb!($client.arc_resource_blocking($fetcher, $keyer))
                }
                (ResourceType::Blocking, false) => {
                    $cb!($client.resource_blocking($fetcher, $keyer))
                }
            }
        };
    }

    /// Make sure !Send and !Sync values work with local resources.
    /// TODO whole public api other than resource creation.
    #[rstest]
    #[tokio::test]
    async fn test_unsync(#[values(false, true)] arc: bool) {
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
                        async move { UnsyncValue::new(key * 2) }
                    }
                };
                let fetcher = QueryScopeLocal::new(fetcher, Default::default());

                let (client, _guard) = prep_vari!(false);

                macro_rules! check {
                    ($resource:expr) => {{
                        assert_eq!($resource.await, UnsyncValue::new(4));
                        assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);

                        tick!();

                        assert_eq!($resource.await, UnsyncValue::new(4));
                        assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);
                    }};
                }

                match arc {
                    true => {
                        check!(client.arc_local_resource(fetcher.clone(), || 2))
                    }
                    false => {
                        check!(client.local_resource(fetcher.clone(), || 2))
                    }
                }
            })
            .await;
    }

    /// Make sure resources reload when queries invalidated correctly.
    #[rstest]
    #[tokio::test]
    async fn test_manual_invalidation(
        #[values(ResourceType::Local, ResourceType::Blocking, ResourceType::Normal)] resource_type: ResourceType,
        #[values(false, true)] arc: bool,
        #[values(false, true)] server_ctx: bool,
        #[values(false, true)] individual_invalidation: bool,
    ) {
        tokio::task::LocalSet::new()
            .run_until(async move {
                let fetch_calls = Arc::new(AtomicUsize::new(0));
                let fetcher = {
                    let fetch_calls = fetch_calls.clone();
                    move |key: u64| {
                        fetch_calls.fetch_add(1, Ordering::Relaxed);
                        async move { key * 2 }
                    }
                };
                let fetcher = QueryScope::new(fetcher, Default::default());

                let (client, _guard) = prep_vari!(server_ctx);

                macro_rules! check {
                    ($resource:expr) => {{
                        assert_eq!($resource.await, 4);
                        assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);

                        tick!();

                        // Shouldn't change despite ticking:
                        assert_eq!($resource.await, 4);
                        assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);

                        if individual_invalidation {
                            client.invalidate_query(fetcher.clone(), &2);
                        } else {
                            client.invalidate_all_queries();
                        }

                        tick!();

                        assert_eq!($resource.await, 4);
                        assert_eq!(fetch_calls.load(Ordering::Relaxed), 2);
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
        tokio::task::LocalSet::new()
            .run_until(async move {
                let fetch_calls = Arc::new(AtomicUsize::new(0));
                let fetcher = {
                    let fetch_calls = fetch_calls.clone();
                    move |key: u64| {
                        fetch_calls.fetch_add(1, Ordering::Relaxed);
                        async move { key * 2 }
                    }
                };
                let fetcher = QueryScope::new(fetcher, Default::default());

                let (client, _guard) = prep_vari!(server_ctx);

                let add_size = RwSignal::new(1);

                macro_rules! check {
                    ($resource:expr) => {{
                        assert_eq!($resource.await, 2);
                        assert_eq!($resource.await, 2);
                        assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);

                        add_size.set(2);

                        tick!();

                        assert_eq!($resource.await, 4);
                        assert_eq!(fetch_calls.load(Ordering::Relaxed), 2);
                        assert_eq!($resource.await, 4);
                        assert_eq!(fetch_calls.load(Ordering::Relaxed), 2);
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
        tokio::task::LocalSet::new()
            .run_until(async move {
                let fetch_calls = Arc::new(AtomicUsize::new(0));
                let fetcher = {
                    let fetch_calls = fetch_calls.clone();
                    move |key: u64| {
                        fetch_calls.fetch_add(1, Ordering::Relaxed);
                        async move { key * 2 }
                    }
                };
                let fetcher = QueryScope::new(fetcher, Default::default());

                let (client, _guard) = prep_vari!(server_ctx);

                let keys = [1, 2, 3, 4, 5];
                let results = futures::future::join_all(keys.iter().cloned().map(|key| {
                    let fetcher = fetcher.clone();
                    async move {
                        macro_rules! cb {
                            ($resource:expr) => {
                                $resource.await
                            };
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
                            ($resource:expr) => {
                                $resource.await
                            };
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
        tokio::task::LocalSet::new()
            .run_until(async move {
                let fetch_calls = Arc::new(AtomicUsize::new(0));
                let fetcher = {
                    let fetch_calls = fetch_calls.clone();
                    move |key: u64| {
                        fetch_calls.fetch_add(1, Ordering::Relaxed);
                        async move {
                            tokio::time::sleep(tokio::time::Duration::from_millis(30)).await;
                            key * 2
                        }
                    }
                };
                let fetcher = QueryScope::new(fetcher, Default::default());

                let (client, _guard) = prep_vari!(server_ctx);

                let keyer = || 1;
                let results = futures::future::join_all((0..10).map(|_| {
                    let fetcher = fetcher.clone();
                    async move {
                        macro_rules! cb {
                            ($resource:expr) => {
                                $resource.await
                            };
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
        tokio::task::LocalSet::new()
            .run_until(async move {
                for maybe_sleep_ms in &[None, Some(10), Some(30)] {
                    let (client, ssr_ctx) = prep_server!();

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
                    let fetcher = QueryScope::new(fetcher, Default::default());

                    let keyer = || 1;

                    // First call should require a fetch.
                    assert_eq!(client.arc_resource(fetcher.clone(), keyer).await, 2);
                    assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);

                    // Second should be cached by the query client because same key:
                    assert_eq!(client.arc_resource(fetcher.clone(), keyer).await, 2);
                    assert_eq!(fetch_calls.load(Ordering::Relaxed), 1);

                    // Should make it over to the frontend too:
                    let client = prep_client!(ssr_ctx);

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
                    let (ssr_client, ssr_ctx) = prep_server!();
                    fetch_calls.store(0, Ordering::Relaxed);

                    // Don't await:
                    let ssr_resource_1 = ssr_client.arc_resource(fetcher.clone(), keyer);
                    let ssr_resource_2 = ssr_client.arc_resource(fetcher.clone(), keyer);

                    let hydrate_client = prep_client!(ssr_ctx);

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
