use std::time::Duration;

use leptos::prelude::*;
use leptos_fetch::{QueryClient, QueryDevtools, QueryOptions};

use crate::blog_list::BlogList;

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
    let client = QueryClient::new()
        .with_options(QueryOptions::default().with_stale_time(Duration::from_secs(10)))
        .provide();

    view! {
        <QueryDevtools client=client />

        <header>
            <h1>"My Tasks"</h1>
        </header>
        <main>
            <BlogList />
        </main>
    }
}
