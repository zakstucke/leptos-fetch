use std::{
    sync::{atomic::AtomicUsize, Arc},
    time::Duration,
};

use leptos::{prelude::*};
use leptos_fetch::{QueryClient, QueryDevtools, QueryOptions, QueryScope};

use crate::{blog_list::BlogList, testcases::TestCases};

pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <AutoReload options=options.clone() />
                <HydrationScripts options />
                <link rel="stylesheet" id="leptos" href="/pkg/example_blog.css" />
                <link rel="shortcut icon" type="image/ico" href="/favicon.ico" />
            </head>
            <body>
                <App />
            </body>
        </html>
    }
}

#[component]
pub fn App() -> impl IntoView {
    let refetch_enabled = RwSignal::new(true);
    let client = QueryClient::new()
        .with_options(QueryOptions::default().with_stale_time(Duration::from_secs(10)))
        .with_refetch_enabled_toggle(refetch_enabled)
        .provide();

    view! {
        <QueryDevtools client=client />

        <header>
            <h1>"My Tasks"</h1>
        </header>
        <main>
            <BlogList />
            <RefetchExample refetch_enabled=refetch_enabled />
        </main>
    }
}

#[component]
pub fn RefetchExample(refetch_enabled: RwSignal<bool>) -> impl IntoView {
    let client: QueryClient = expect_context();

    let call_count = Arc::new(AtomicUsize::new(0));
    let call_count_resource = client.resource(
        QueryScope::new(move |()| {
            let call_count = call_count.clone();
            async move { call_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) }
        })
        .with_options(QueryOptions::default().with_refetch_interval(Duration::from_millis(100))),
        move || (),
    );

    view! {
        <div style="border: 1px solid black; padding: 10px; margin-bottom: 10px;">
            <p>"Refetch example"</p>
            <button on:click=move |_| {
                refetch_enabled.update(|enabled| *enabled = !*enabled);
            }>
                {move || if refetch_enabled.get() { "Disable" } else { "Enable" }}
                " Auto-refetching"
            </button>
            <Suspense fallback=move || {
                view! { <p>"Loading..."</p> }
            }>
                <p>
                    "Random number resource value (updates every 100ms if refetching is enabled): "
                    <strong>{move || call_count_resource.get()}</strong>
                </p>
            </Suspense>
        </div>
        <TestCases />
    }
}

