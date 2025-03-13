<!-- cargo-rdme start -->

# Leptos Fetch

[<img alt="github" src="https://img.shields.io/badge/github-zakstucke/leptos--fetch-8da0cb?style=for-the-badge&labelColor=555555&logo=github" height="20">](https://github.com/zakstucke/leptos-fetch)
[<img alt="crates.io" src="https://img.shields.io/crates/v/leptos-fetch.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/leptos-fetch)
[<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-leptos--fetch-66c2a5?style=for-the-badge&labelColor=555555&logo=docs.rs" height="20">](https://docs.rs/leptos-fetch)
<!-- [<img alt="build status" src="https://img.shields.io/github/actions/workflow/status/zakstucke/leptos-fetch/rust.yml?branch=main&style=for-the-badge" height="20">](https://github.com/zakstucke/leptos-fetch/actions?query=branch%3Amain) -->


<!-- [FAQ](https://github.com/zakstucke/leptos-fetch/blob/main/FAQ.md) | 
[Examples](https://github.com/zakstucke/leptos-fetch/tree/main/example/) |
[Live Demo](https://leptos-fetch-demo.vercel.app/) -->

## About

Leptos Fetch is a async state management library for [Leptos](https://github.com/leptos-rs/leptos).

The successor of, and heavily inspired by [Leptos Query](https://github.com/gaucho-labs/leptos-query), which has been unmaintained for ~1 year.

Queries are useful for data fetching, caching, and synchronization with server state.

This crate provides:
- Caching
- Request de-duplication
- Invalidation
- Background refetching
- ~~Refetch intervals~~
- Memory management with cache lifetimes
- ~~Cancellation~~
- ~~Debugging tools~~
- Optimistic updates
- ~~Client side cache persistance (localstorage, indexdb, custom, etc.)~~

Lines that have a strike through are features not currently brought over from [Leptos Query](https://github.com/gaucho-labs/leptos-query).

## Feature Flags
<!-- - `csr` Client-side rendering: Use queries on the client. -->
- `ssr` Server-side rendering: Initiate queries on the server.
<!-- - `hydrate` Hydration: Ensure that queries are hydrated on the client, when using server-side rendering. -->

## Version compatibility for Leptos and Leptos Fetch

The table below shows the compatible versions of `leptos-fetch` for each `leptos` version. Ensure you are using compatible versions to avoid potential issues.

| `leptos` version | `leptos-fetch` version |
|------------------|------------------------|
| 0.7.*            | `0.1.*` `0.2.* `       |


## Installation

```bash
cargo add leptos-fetch
```

If using ssr, add the relevant feature to your `Cargo.toml` when in ssr:

```toml
[features]
ssr = [
    "leptos-fetch/ssr",
    # ...
 ]
```


## Quick Start

In the root of your App, provide a query client with [QueryClient::provide()] or [QueryClient::provide_with_options()] if you want to override the default options.

```rust
use leptos::prelude::*;
use leptos_fetch::QueryClient;

#[component]
pub fn App() -> impl IntoView {
    // Provides the Query Client for the entire app via leptos context.
    QueryClient::provide();
    
    // QueryClient::provide_with_options(QueryOptions::new()..) can customize default caching behaviour.

    // Rest of App...
}
```

Any async function can be used as a query:

```rust
/// The query function.
async fn get_track(id: i32) -> String {
    todo!()
}
```

Now you can use the query in any component in your app.

```rust
use leptos::prelude::*;
use leptos_fetch::QueryClient;

#[component]
fn TrackView(id: i32) -> impl IntoView {
    // Usually at the root of the App:
    QueryClient::provide();

    // Extract the root client from leptos context,
    // this is identical to expect_context::<QueryClient>()
    let client = QueryClient::expect();
    
    // Native leptos resources are returned, 
    // there are also variants for local, blocking, arc resources. 
    let resource = client.resource(get_track, move || id.clone());

    view! {
       <div>
           // Resources can be awaited inside a Transition/Suspense components.
           // Alternative .read()/.get()/.with() etc can be used synchronously returning Option's.
           <Transition
               fallback=move || {
                   view! { <h2>"Loading..."</h2> }
               }>
                {move || Suspend::new(async move {
                    let track = resource.await;
                    view! { <h2>{track}</h2> }
                })}          
           </Transition>
       </div>
    }
}

/// The query function.
async fn get_track(id: i32) -> String {
    todo!()
}
```

You can read more about leptos resources in the [Leptos Book](https://book.leptos.dev/async/10_resources.html?highlight=resources#resources)

<!-- For a complete working example see [the example directory](/example) -->

`QueryScope` and `QueryScopeLocal` can be used instead of directly passing a function to [`QueryClient`] methods to only apply to one query type.

These [`QueryOptions`] will be combined with the global [`QueryOptions`] set on the [`crate::QueryClient`], with the local options taking precedence.

```rust
use std::time::Duration;
use leptos_fetch::{QueryScope, QueryOptions};

// this can be used just like the function directly in QueryClient methods.
fn track_query() -> QueryScope<i32, String> {
    QueryScope::new(
        get_track, 
        QueryOptions::new()
            .set_stale_time(Duration::from_secs(10))
            .set_gc_time(Duration::from_secs(60))
    )
}

/// The query function.
async fn get_track(id: i32) -> String {
    todo!()
}
```

The [`QueryClient`] contains many documented utility methods other than resources for:
- Declaratively fetching queries
- Declaratively prefetching queries
- Mutating cached queries
- Invalidating cached queries
- Accessing cached queries

<!-- ## Devtools Quickstart

To use the devtools, you need to add the devtools crate:

```bash
cargo add leptos_query_devtools
```

Then in your `cargo.toml` enable the `csr` feature.

#### Hydrate Example
- If your app is using SSR, then this should go under the "hydrate" feature.
```toml
[features]
hydrate = [
    "leptos_query_devtools/csr",
]
```

#### CSR Example
- If your app is using CSR, then this should go under the "csr" feature.
```toml
[features]
csr = [
    "leptos_query_devtools/csr",
]
```

Then in your app, render the devtools component. Make sure you also provide the query client.

Devtools will by default only show in development mode. It will not be shown, or included in binary, when you build your app in release mode. If you want to override this behaviour, you can enable the `force` feature.

```rust

use leptos_query_devtools::LeptosQueryDevtools;
use leptos_fetch::*;
use leptos::prelude::*;

#[component]
fn App() -> impl IntoView {
    provide_query_client();

    view!{
        <LeptosQueryDevtools />
        // Rest of App...
    }
}

``` -->

<!-- cargo-rdme end -->
