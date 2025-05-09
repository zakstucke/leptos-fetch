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
- Optional resources
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
- [Devtools](#devtools)
- [Query Options](#query-options)
- [Declarative Query Interactions](#declarative-query-management)
- [Linked Invalidation](#linked-invalidation)
- [Subscriptions](#subscriptions)
- [Thread Local & Threadsafe Variants](#thread-local-and-threadsafe-variants)
- [Custom Streaming Codecs (`ssr`)](#custom-streaming-codecs)
- [Pagination & Infinite Queries](#pagination-and-infinite-queries)

## Installation

### Feature Flags
- `ssr` Server-side rendering: Initiate queries on the server.

### Version compatibility for Leptos and LF

The table below shows the compatible versions of `leptos-fetch` for each `leptos` version. Ensure you are using compatible versions to avoid potential issues.

| `leptos` version | `leptos-fetch` version |
|------------------|------------------------|
| 0.7.*            | `0.1.*` `0.2.*` `0.3.*`|
| 0.8.*            | `0.4.*`                |


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
    
    // client.with_options(QueryOptions::new()..) can customize default behaviour.
    // client.set_codec::<Codec>() can be used to change the codec for streaming in ssr.

    // Rest of App...
}
```

Any async function can be used as a query:

```rust
/// The query function
async fn get_track(id: i32) -> String {
    todo!()
}

/// If no key argument, treated as ()
async fn get_theme() -> String {
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

The reactive keyer argument supports returning `Option<K>`, which prevents the need to wrap `get_track` in an outer function that takes `Option<i32>` as an id. This is a really powerful pattern for "optional resources":
```rust,no_run
use leptos::prelude::*;
use leptos_fetch::QueryClient;

async fn get_track(id: i32) -> String {
    todo!()
}

let value = RwSignal::new(None);
QueryClient::new().resource(get_track, move || value.get());
value.set(Some(1));
```

## Devtools
<p align="start">
    <img src="https://raw.githubusercontent.com/zakstucke/leptos-fetch/main/devtools_modal.jpg" alt="Devtools Modal"/>
</p>

[`QueryDevtools`](https://docs.rs/leptos-fetch/latest/leptos_fetch/fn.QueryDevtools.html) is provided to help visualize all of the inner workings of Leptos Fetch and will likely save a bunch of tedious debugging!

To enable, the `devtools` feature must be added, the component won't be shown or included in the binary when you build your app in release mode for performance.

If you need the devtools component in release mode too, you can use the `devtools-always` feature instead.

```bash
cargo add leptos-fetch --feature devtools
```

```rust,no_run
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
    <img src="https://raw.githubusercontent.com/zakstucke/leptos-fetch/main/devtools_widget.jpg" alt="Devtools widget"/>
</p>

## Query Options

The [`QueryOptions`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryOptions.html) struct can be used to configure the following:

| Option           | Default       | Description                                                                                                                                                                                                                                                  |
|------------------|---------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| stale_time       | never    | The duration that should pass before a query is considered stale.  Once stale, after any new interaction with the query, a new resource using it, declarative interactions etc,  the query will be refetched in the background, and update active resources. |
| gc_time          | 5 minutes     | After this time, if the query isn't being used by any resources,  the query will be removed from the cache, to minimise the cache's size.  If the query is in active use, the gc will be scheduled to check again after the same time interval.              |
| refetch_interval | No refetching | If the query is being used by any resources, it will be invalidated and refetched in the background,  updating active resources according to this interval.                                                                                                     |

**NOTE: `stale_time` can never be greater than `gc_time`.**
> If `stale_time` is greater than `gc_time`, `stale_time` will be set to `gc_time`.

[`QueryOptions`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryOptions.html) can be applied to the whole [`QueryClient`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html) by calling it with [`QueryClient::with_options`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.with_options).

Options can also be applied to individual query scopes by wrapping query functions in either [`QueryScope`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryScope.html) or [`QueryScopeLocal`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryScopeLocal.html) and passing this scope to [`QueryClient`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html) methods.

**NOTE: query scopes are separated based on the unique identity of the function (or closure) provided to both query scopes, and those directly provided to a [`QueryClient`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html)..**
> If you pass different closures, even with the same arguments, they will be treated as unique query scopes.

Query scope specific [`QueryOptions`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryOptions.html) will be combined with the global [`QueryOptions`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryOptions.html) set on the [`QueryClient`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html), with the local options taking precedence when both have a value set.

```rust
use std::time::Duration;

use leptos_fetch::{QueryClient, QueryScope, QueryOptions};
use leptos::prelude::*;

// A QueryScope/QueryScopeLocal can be used just like the function directly in QueryClient methods.
fn track_query() -> QueryScope<i32, String> {
    QueryScope::new(get_track)
        .with_options(
            QueryOptions::new()
                .with_stale_time(Duration::from_secs(10))
                .with_gc_time(Duration::from_secs(60))
                .with_refetch_interval(Duration::from_secs(10))
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

Resources are just one way to load and interact with queries. The [`QueryClient`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html) allows you to [prefetch](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.prefetch_query), [fetch](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.fetch_query), [set](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.set_query), [update](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.update_query), [update async](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.update_query_async), [check if exists](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.query_exists) and [invalidate queries](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.invalidate_query) declaratively, where any changes will automatically update active resources.

### Query Invalidation

Sometimes you can't wait for a query to become stale before you refetch it. [`QueryClient::invalidate_query`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.invalidate_query) and friends allow you to intelligently mark queries as stale and potentially refetch them too.

When a query is invalidated, the following happens:

- It is marked as `invalid`, which overrides any `stale_time` configuration.
- The next time the query is used, it will be refetched in the background.
- If a query is currently being used, it will be refetched immediately.

This can be particularly useful in cases where you have a highly dynamic data source, or when user actions in the application can directly modify data that other parts of your application rely on.

## Linked Invalidation

Different query types are sometimes linked to the same source, e.g. you may want an invalidation of `list_blogposts()` to always automatically invalidate `get_blogpost(id)`.

[`QueryScope::with_invalidation_link`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryScope.html#method.subscribe_is_fetching::with_invalidation_link) can be used to this effect, given a query key `&K`, you provide a `Vec<String>` that's used as a **hierarchy key (HK)** for that query. When a query is invalidated, any query's **HK** that's prefixed by this **HK** will also be invalidated automatically. E.g. A query with **HK** `["users"]` will also auto invalidate another query with `["users", "1"]`, but not the other way around. 2 queries with an identicial **HK** of `["users"]` will auto invalidate each other.

```rust
use std::time::Duration;

use leptos_fetch::{QueryClient, QueryScope, QueryOptions};
use leptos::prelude::*;

#[derive(Debug, Clone)]
struct User;

fn list_users_query() -> QueryScope<(), Vec<User>> {
    QueryScope::new(async || vec![])
        .with_invalidation_link(
            |_key| ["users"]
        )
}

fn get_user_query() -> QueryScope<i32, User> {
    QueryScope::new(async move |user_id: i32| User)
        .with_invalidation_link(
            |user_id| ["users".to_string(), user_id.to_string()]
        )
}

let client = QueryClient::new();

// This invalidates only user "2", because ["users", "2"] is not a prefix of ["users"], 
// list_users_query is NOT invalidated.
client.invalidate_query(get_user_query(), &2);

// This invalidates both queries, because ["users"] is a prefix of ["users", "$x"]
client.invalidate_query(list_users_query(), &());
```

## Subscriptions

Subscriptions allow you to reactively respond to a query's lifecycle outside of using a leptos resource directly.
- [`QueryClient::subscribe_is_fetching`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.subscribe_is_fetching) returns a `Signal<bool>` which reactively updates to `true` whenever a query is being fetched in the background. This could be used to e.g. show a spinner next to some data visualisation, implying the data is stale and is about to be replaced.

- [`QueryClient::subscribe_is_loading`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.subscribe_is_loading) returns a `Signal<bool>` which reactively updates to `true` whenever a query is being fetched for the first time, i.e. stale data was not already in the cache. This could be used to e.g. show something before the data is ready, without having to use a fallback with the leptos `Transition` or `Suspense` components.

- [`QueryClient::subscribe_value`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.subscribe_value) returns a `Signal<Option<V>>` that subscribes to the value of a query. This can be useful to react to a query value in the cache, but not prevent it being garbage collected, or trigger fetching when missing.

## Thread Local and Threadsafe Variants
If using SSR, some resources will initially load on the server, in this case multiple threads are in use. 

To prevent needing all types to be `Sync` + `Send`, `_local()` variants of many functions exist that do not require `Send` + `Sync`. `_local()` variants also will not stream from the server to the client in `ssr`, therefore do not need to implement codec traits.

This is achieved by internally utilising a threadsafe cache, alongside a local cache per thread, abstracting this away to expose a singular combined cache. 

The public API will only provide access to cache values that are either threadsafe, or created on the current thread, and this distinction should be completely invisible to a user.

## Custom Streaming Codecs

**Applies to `ssr` only**

It's possible to use non-json codecs for streaming leptos resources from the backend.
The default is [`codee::string::JsonSerdeCodec`](https://docs.rs/codee/latest/codee/string/struct.JsonSerdeCodec.html).

If the `rkyv` feature of `leptos-fetch` is enabled, the default becomes [`codee::binary::RkyvCodec`](https://docs.rs/codee/latest/codee/binary/struct.RkyvCodec.html) using the [rkyv](https://docs.rs/rkyv/latest/rkyv/) crate, a more compact & efficient binary codec than serde/json.

To use other codecs, or completely custom ones, the [codee](https://docs.rs/codee/latest/codee/) crate is used as the interface, current major version is `0.3`.

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

## Pagination and Infinite Queries

Pagination can be achieved simply with basic primitives:
```rust,no_run
use leptos::prelude::*;

#[derive(Clone, Debug)]
struct Page;

async fn get_page(page_index: usize) -> Page {
    Page
}

let client = leptos_fetch::QueryClient::new();

// Initial page is 0:
let active_page_index = RwSignal::new(0);

// The resource is reactive over the active_page_index signal:
let resource = client.local_resource(get_page, move || active_page_index.get());

// Update the page to 1:
active_page_index.set(1);
```

Likewise with infinite queries, the [`QueryClient::update_query_async`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.update_query_async) makes it easy with a single cache key:

```rust,no_run
use leptos::prelude::*;

#[derive(Clone, Debug)]
struct InfiniteItem(usize);

#[derive(Clone, Debug)]
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

let client = leptos_fetch::QueryClient::new();

// Initialise the query with the first load.
// we're not using a reactive key here for extending the list, but declarative updates instead.
let resource = client.local_resource(get_list_query, || ());

async {
    // When wanting to load more items, update_query_async can be called declaratively to update the cached item and resource:
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
};
```

<!-- cargo-rdme end -->
