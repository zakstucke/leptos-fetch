<!-- cargo-rdme start -->

[<img alt="github" src="https://img.shields.io/badge/github-zakstucke/leptos--fetch-8da0cb?style=for-the-badge&labelColor=555555&logo=github" height="20">](https://github.com/zakstucke/leptos-fetch)
[<img alt="crates.io" src="https://img.shields.io/crates/v/leptos-fetch.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/leptos-fetch)
[<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-leptos--fetch-66c2a5?style=for-the-badge&labelColor=555555&logo=docs.rs" height="20">](https://docs.rs/leptos-fetch)
![Crates.io MSRV](https://img.shields.io/crates/msrv/leptos-fetch)
[<img alt="build status" src="https://img.shields.io/github/actions/workflow/status/zakstucke/leptos-fetch/rust.yml?branch=main&style=for-the-badge" height="20">](https://github.com/zakstucke/leptos-fetch/actions?query=branch%3Amain)

Leptos Fetch is an async state management library for [Leptos](https://github.com/leptos-rs/leptos). LF is a refined and enhanced successor to [Leptos Query](https://github.com/gaucho-labs/leptos-fetch), following a year of inactivity.

**PR's for bugfixes, documentation, performance, features and examples are very welcome!**

LF provides:
- Caching
- Request de-duplication
- Invalidation
- Background refetching
- Refetch intervals
- Memory management with cache lifetimes
- Optimistic updates
- Debugging tools
- Declarative query interaction as a supplement to leptos resources
- In `ssr`, custom stream encoding at a global level

### How's this different from a Leptos Resource?

LF extends the functionality of [Leptos Resources](https://leptos-rs.github.io/leptos/async/10_resources.html) with features like caching, de-duplication, and invalidation, while also allowing easy access and manipulation of cached data throughout your app.

Queries are all bound to the [`QueryClient`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html) they are created in, meaning that once you have a [`QueryClient`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html) in your app, you can access the value for a query anywhere in your app, and you have a single cache for your entire app. Queries are stateful on a per-key basis, meaning you can use the same query with the same key in multiple places and only one request will be made, and they all share the same state. 

With a resource, you have to manually lift it to a higher scope if you want to preserve it, which can be cumbersome if you have many resources.

LF also allows you to interact declaratively with queries outside resources, subscribe to changes, and automatically update active resources where applicable.

## Table of Contents
- [Installation](#installation)
- [Quick Start](#quick-start)
- [Query Options](#query-options)
- [Declarative Query Interactions](#declarative-query-management)
- [Subscriptions](#subscriptions)
- [Thread Local and Threadsafe Variants](#thread-local-and-threadsafe-variants)
- [Custom Streaming Codecs (`ssr`)](#custom-streaming-codecs)

## Installation

### Feature Flags
- `ssr` Server-side rendering: Initiate queries on the server.

### Version compatibility for Leptos and LF

The table below shows the compatible versions of `leptos-fetch` for each `leptos` version. Ensure you are using compatible versions to avoid potential issues.

| `leptos` version | `leptos-fetch` version |
|------------------|------------------------|
| 0.7.*            | `0.1.*` `0.2.*` `0.3.*`|


### Installation

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

In the root of your App, create a query client with [`QueryClient::new`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.new) then call [`QueryClient::provide`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.provide) to store in leptos context.

```rust
use leptos::prelude::*;
use leptos_fetch::QueryClient;

#[component]
pub fn App() -> impl IntoView {
    // Provides the Query Client for the entire app via leptos context.
    QueryClient::new().provide();
    
    // QueryClient::set_options(QueryOptions::new()..) can customize default behaviour.
    // QueryClient::set_codec::<Codec>() can be used to change the codec for streaming in ssr.

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
    QueryClient::new().provide();

    // Extract the root client from leptos context,
    let client: QueryClient = expect_context();
    
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

### Devtools
<p align="start">
    <img src="https://raw.githubusercontent.com/zakstucke/leptos-query/main/devtools_modal.jpg" alt="Devtools Modal"/>
</p>

[`QueryDevtools`](https://docs.rs/leptos-fetch/latest/leptos_fetch/fn.QueryDevtools.html) is provided to help visualize all of the inner workings of Leptos Fetch and will likely save you hours of debugging if you find yourself in a pinch!

To enable, the `devtools` feature must be added, the component won't be shown or included in the binary when you build your app in release mode for performance.

If you need the devtools component in release mode too, you can use the `devtools-always` feature instead.

```bash
cargo add leptos-fetch --feature devtools
```

```rust
use leptos::*;
use leptos_fetch::{QueryClient, QueryDevtools};
#[component]
fn App() -> impl IntoView {
   let client = QueryClient::new().provide();
    view!{
        // This will render the devtools as a small widget in the bottom-right of the screen, 
        // this will only show in development mode.
        <QueryDevtools client=client />
        // Rest of App...
    }
}
```

In the bottom right of the screen, this widget should appear and be clickable to open the devtools:
<p align="start">
    <img src="https://raw.githubusercontent.com/zakstucke/leptos-query/main/devtools_widget.jpg" alt="Devtools widget"/>
</p>

## Query Options

The [`QueryOptions`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryOptions.html) struct can be used to configure the following:

| Option           | Default       | Description                                                                                                                                                                                                                                                  |
|------------------|---------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| stale_time       | 10 seconds    | The duration that should pass before a query is considered stale.  Once stale, after any new interaction with the query, a new resource using it, declarative interactions etc,  the query will be refetched in the background, and update active resources. |
| gc_time          | 5 minutes     | After this time, if the query isn't being used by any resources,  the query will be removed from the cache, to minimise the cache's size.  If the query is in active use, the gc will be scheduled to check again after the same time interval.              |
| refetch_interval | No refetching | If the query is being used by any resources, it will be invalidated and refetched in the background,  updating active resources according to this interval.                                                                                                     |

**NOTE: `stale_time` can never be greater than `gc_time`.**
> If `stale_time` is greater than `gc_time`, `stale_time` will be set to `gc_time`.

[`QueryOptions`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryOptions.html) can be applied to the whole [`QueryClient`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html) by calling it with [`QueryClient::set_options`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.set_options).

Options can also be applied to individual query types by wrapping query functions in either [`QueryScope`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryScope.html) or [`QueryScopeLocal`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryScopeLocal.html) and passing this scope to [`QueryClient`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html) methods.

**NOTE: query types are separated based on the unique identity of the function (or closure) provided to both query scopes, and those directly provided to a [`QueryClient`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html)..**
> If you pass different closures, even with the same arguments, they will be treated as unique query types.

Query type specific [`QueryOptions`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryOptions.html) will be combined with the global [`QueryOptions`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryOptions.html) set on the [`QueryClient`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html), with the local options taking precedence when both have a value set.

```rust
use std::time::Duration;

use leptos_fetch::{QueryClient, QueryScope, QueryOptions};
use leptos::prelude::*;

// A QueryScope/QueryScopeLocal can be used just like the function directly in QueryClient methods.
fn track_query() -> QueryScope<i32, String> {
    QueryScope::new(
        get_track, 
        QueryOptions::new()
            .set_stale_time(Duration::from_secs(10))
            .set_gc_time(Duration::from_secs(60))
            .set_refetch_interval(Duration::from_secs(10))
    )
}

/// The query function.
async fn get_track(id: i32) -> String {
    todo!()
}

fn foo() {
    let client: QueryClient = expect_context();
    let resource = client.resource(track_query(), || 2);
}
```

## Declarative Query Interactions

Resources are just one way to load and interact with queries. The [`QueryClient`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html) allows you to [prefetch](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.prefetch_query), [fetch](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.fetch_query), [set](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.set_query), [update](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.update_query), [check if exists](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.query_exists) and [invalidate queries](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.invalidate_query) declaratively, where any changes will automatically update active resources.

### Query Invalidation

Sometimes you can't wait for a query to become stale before you refetch it. [`QueryClient::invalidate_query`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.invalidate_query) and friends allow you to intelligently mark queries as stale and potentially refetch them too.

When a query is invalidated, the following happens:

- It is marked as `invalid`, which overrides any `stale_time` configuration.
- The next time the query is used, it will be refetched in the background.
- If a query is currently being used, it will be refetched immediately.

This can be particularly useful in cases where you have a highly dynamic data source, or when user actions in the application can directly modify data that other parts of your application rely on.

## Subscriptions

Subscriptions allow you to reactively respond to a query's lifecycle outside of using a leptos resource directly.
- [`QueryClient::subscribe_is_fetching`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.subscribe_is_fetching) returns a `Signal<bool>` which reactively updates to `true` whenever a query is being fetched in the background. This could be used to e.g. show a spinner next to some data visualisation, implying the data is stale and is about to be replaced.

- [`QueryClient::subscribe_is_loading`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.subscribe_is_loading) returns a `Signal<bool>` which reactively updates to `true` whenever a query is being fetched for the first time, i.e. stale data was not already in the cache. This could be used to e.g. show something before the data is ready, without having to use a fallback with the leptos `Transition` or `Suspense` components.

## Thread Local and Threadsafe Variants
If using SSR, some resources will initially load on the server, in this case multiple threads are in use. 

To prevent needing all types to be `Sync` + `Send`, `_local()` variants of many functions exist that do not require `Send` + `Sync`. `_local()` variants also will not stream from the server to the client in `ssr`, therefore do not need to implement codec traits.

This is achieved by internally utilising a threadsafe cache, alongside a local cache per thread, abstracting this away to expose a singular combined cache. 

The public API will only provide access to cache values that are either threadsafe, or created on the current thread, and this distinction should be completely invisible to a user.

## Custom Streaming Codecs

**Applies to `ssr` only**

It's possible to use non-json codecs for streaming leptos resources from the backend.
The default is [`codee::string::JsonSerdeCodec`](https://docs.rs/codee/latest/codee/string/struct.JsonSerdeCodec.html).

The current `codee` major version is `0.3` and will need to be imported in your project to customize the codec.

E.g. to use [`codee::binary::MsgpackSerdeCodec`](https://docs.rs/codee/latest/codee/binary/struct.MsgpackSerdeCodec.html): 
```toml
codee = { version = "0.3", features = ["msgpack_serde"] }
```

[`MsgpackSerdeCodec`](https://docs.rs/codee/latest/codee/binary/struct.MsgpackSerdeCodec.html) will become a generic type on the [`QueryClient`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html), so when calling [`expect_context`](https://docs.rs/leptos/latest/leptos/prelude/fn.expect_context.html),
this type must be specified when not using the default.

A useful pattern is to type alias the client with the custom codec for your whole app:

```rust,no_run
use codee::binary::MsgpackSerdeCodec;
use leptos::prelude::*;
use leptos_fetch::QueryClient;

type MyQueryClient = QueryClient<MsgpackSerdeCodec>;

// Create and provide to context to make accessible everywhere:
QueryClient::new().set_codec::<MsgpackSerdeCodec>().provide();

let client: MyQueryClient = expect_context();
```

<!-- cargo-rdme end -->
