# Changelog

All notable changes to this project are documented in this file.

## [Unreleased]

### Added
- User callbacks on query garbage collection with [`QueryScope::on_gc`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryScope.html#method.on_gc) and [`QueryScopeLocal::on_gc`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryScopeLocal.html#method.on_gc) ([#58](https://github.com/zakstucke/leptos-fetch/pull/58))

- User callbacks on query invalidation with [`QueryScope::on_invalidation`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryScope.html#method.on_invalidation) and [`QueryScopeLocal::on_invalidation`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryScopeLocal.html#method.on_invalidation) ([#57](https://github.com/zakstucke/leptos-fetch/pull/57))

### Fixed
- Replaced spawn during invalidation with internal callbacks. ([#56](https://github.com/zakstucke/leptos-fetch/pull/56))

---

## [0.4.7](https://github.com/zakstucke/leptos-fetch/releases/tag/v0.4.7)

### Fixed
- Fix doc building. ([#52](https://github.com/zakstucke/leptos-fetch/pull/52))

## [0.4.6](https://github.com/zakstucke/leptos-fetch/releases/tag/v0.4.6)

### Added
Leptos bumped to version `0.8.10` for new `Owner::parent` method, increases MSRV to `1.88`. ([#50](https://github.com/zakstucke/leptos-fetch/pull/50))

### Fixed
- Fix race condition between streamed resource queries and local queries [#48](https://github.com/zakstucke/leptos-fetch/issues/48). ([#49](https://github.com/zakstucke/leptos-fetch/pull/49))
- Preserve the chain of owners available at the start of a query for its duration, to prevent sporadic owner/context failures in query functions. ([#51](https://github.com/zakstucke/leptos-fetch/pull/51))

## [0.4.5](https://github.com/zakstucke/leptos-fetch/releases/tag/v0.4.5)

### Added
- New [`QueryClient::with_refetch_enabled_toggle`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.with_refetch_enabled_toggle) & [`QueryClient::refetch_enabled`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.refetch_enabled) to toggle refetching on and off via a signal globally. ([#46](https://github.com/zakstucke/leptos-fetch/pull/46))

### Fixed
- Fix issue where the leptos context `Owner` would sometimes not be available on active resource query reloading. ([#47](https://github.com/zakstucke/leptos-fetch/pull/47))

## [0.4.4](https://github.com/zakstucke/leptos-fetch/releases/tag/v0.4.4)

### Fixed
- Fixed a panic when opening DevTools when `stale_time=Duration::MAX` reported in #43. ([#44](https://github.com/zakstucke/leptos-fetch/pull/44))

## [0.4.3](https://github.com/zakstucke/leptos-fetch/releases/tag/v0.4.3)

### Fixed
- Fixed some edge-case invalidation semantics ([#41](https://github.com/zakstucke/leptos-fetch/pull/41)):
    - When an async query function is loading and `QueryClient::clear`/`QueryClient::invalidate_*` is called, the query is aborted and recalled internally, to prevent the stale query/data being misrepresented as fresh. Applies when resources are loading queries, `QueryClient::fetch_query_*`, `QueryClient::prefetch_query*`, the query loading portion of `QueryClient::update_query_async*` etc.
    - Whilst the user-defined mapping async function in `QueryClient::update_query_async*` etc is running, and `QueryClient::clear` is called, the updated value will not be written to the cache and propagated, as the query being operated on existed prior to the `clear` call, to prevent old data being misrepresented as fresh.
    - The `QueryClient::update_query*` methods, when updating an existing query, were incorrectly resetting the `invalidated` status of queries, they now correctly do not affect the existing `invalidated` values. Likewise the `QueryClient::set_query*` methods no longer reset the invalidated status, as the new data does not come from the source query function.
- Fix `QueryClient::set_query` behaviour when overwriting a value that had previously been set with `QueryClient::set_query_local` or some other local setter. ([#41](https://github.com/zakstucke/leptos-fetch/pull/41))

---

## [0.4.2](https://github.com/zakstucke/leptos-fetch/releases/tag/v0.4.2)

### Added
- New [`QueryClient::untrack_update_query`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.untrack_update_query) method to prevent reactive updates being triggered from the current updater callback fn context. ([#39](https://github.com/zakstucke/leptos-fetch/pull/39))

---

## [0.4.1](https://github.com/zakstucke/leptos-fetch/releases/tag/v0.4.1)

### Fixed
- Rectify mistake in docs: the default stale_time is now `never`. ([#38](https://github.com/zakstucke/leptos-fetch/pull/38))

---

## [0.4.0](https://github.com/zakstucke/leptos-fetch/releases/tag/v0.4.0)

### BREAKING
- Upgrade to `leptos-0.8.0` ([#37](https://github.com/zakstucke/leptos-fetch/pull/37))
- Changes required for the custom codec feature for correct type inference. ([#10](https://github.com/zakstucke/leptos-fetch/pull/10))
    - Replaced `QueryClient::new_with_options` + `QueryClient::provide_with_options` with `QueryClient::with_options(self, options) -> Self`, e.g. `QueryClient::new().with_options(..)`. 
    - Replaced `QueryClient::provide()` with `QueryClient::provide(self) -> Self`, e.g. `QueryClient::new().provide()`. 
    - `QueryClient::expect()` removed, `expect_context::<QueryClient>()` directly from leptos should be used instead. 
- `QueryScope::new(fetcher, options)` -> `QueryScope::new(fetcher).with_options(options)` ([#20](https://github.com/zakstucke/leptos-fetch/pull/20))
- Default `stale_time` changed from `10 seconds` to never, providing less surprising default behaviour. ([#20](https://github.com/zakstucke/leptos-fetch/pull/20)) To revert to old behaviour: 
```rust
QueryClient::new()
    .with_options(
        QueryOptions::default()
            .with_stale_time(Duration::from_secs(10)
        )
    )
    .provide()
```
- Subscriptions: ([#9](https://github.com/zakstucke/leptos-fetch/pull/9))
    - `_subscribe_()` methods' `key: &K` argument replaced with `keyer: impl Fn() -> K`. This makes subscribers reactive to a changing key value to mirror resources.
    - `Send + Sync` required on the keyer functions, so `subscribe_is_fetching_local()`, `subscribe_is_fetching_arc_local()`, `subscribe_is_loading_local()`, `subscribe_is_loading_arc_local()` have been added that do not have these bounds.
- Renaming, standardized `arc|local` to come at the end of method names to make autocomplete easier. Resources are the exception, they inherit their naming from leptos itself: ([#9](https://github.com/zakstucke/leptos-fetch/pull/9))
    - `arc_subscribe_is_fetching` -> `subscribe_is_fetching_arc`
    - `arc_subscribe_is_loading` -> `subscribe_is_loading_arc`
    - `prefetch_local_query` -> `prefetch_query_local`
    - `fetch_local_query` -> `fetch_query_local`
    - `set_local_query` -> `set_query_local`
- `QueryClient::invalidate_query_type` renamed `QueryClient::invalidate_query_scope` ([#24](https://github.com/zakstucke/leptos-fetch/pull/24))
- MSRV increased to `1.85` to migrate to edition 2024 and use async closures ([#19](https://github.com/zakstucke/leptos-fetch/pull/19))
- Chain setters returning `-> Self` standardized from `set_` to `with_` (([#34](https://github.com/zakstucke/leptos-fetch/pull/34))):
    - `QueryClient::set_options` -> `QueryClient::with_options`
    - `QueryScope::set_options` -> `QueryScope::with_options`
    - `QueryOptions::set_stale_time` -> `QueryOptions::with_stale_time`
    - `QueryOptions::set_gc_time` -> `QueryOptions::with_gc_time`
    - `QueryOptions::set_refetch_interval` -> `QueryOptions::with_refetch_interval`

### Added
- For `ssr`, different codecs other than `serde`/json can be used when streaming resources from the backend. Codec choice defaults to [`codee::string::JsonSerdeCodec`](https://docs.rs/codee/latest/codee/string/struct.JsonSerdeCodec.html) like before and applies to the whole `QueryClient`. `rkyv` feature added to change default to [`codee::binary::RkyvCodec`](https://docs.rs/codee/latest/codee/binary/struct.RkyvCodec.html) using the [rkyv](https://docs.rs/rkyv/latest/rkyv/) crate. Other custom codecs can be set with [`QueryClient::set_codec`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.set_codec). ([#10](https://github.com/zakstucke/leptos-fetch/pull/10), [#29](https://github.com/zakstucke/leptos-fetch/pull/29))
- A devtools component has been added under the feature flag `devtools` to help visualize all of the inner workings of Leptos Fetch and will likely save a bunch of tedious debugging. [README](https://docs.rs/leptos-fetch/latest/leptos_fetch/#devtools). ([#13](https://github.com/zakstucke/leptos-fetch/pull/13), [#35](https://github.com/zakstucke/leptos-fetch/pull/35), [#36](https://github.com/zakstucke/leptos-fetch/pull/36), [#37](https://github.com/zakstucke/leptos-fetch/pull/37))
- Added `subscribe_value()`, `subscribe_value_local()`, `subscribe_value_arc()`, `subscribe_value_arc_local()` methods to the `QueryClient` returning `Signal<Option<V>>` for subscribing to query values without preventing their garbage collection or triggering fetching ([#13](https://github.com/zakstucke/leptos-fetch/pull/13))
- [`QueryClient::update_query_async`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.update_query_async) added to make async updates to queries seamlessly, making patterns such as infinite queries simpler. Pagination and infinite query examples added to readme. ([#14](https://github.com/zakstucke/leptos-fetch/pull/14, [#23](https://github.com/zakstucke/leptos-fetch/pull/23)))
- Query functions can be supplied with no key parameter/argument, will default to `()`, prevents needing to add a wrapping function to external fns that don't need an argument. ([#16](https://github.com/zakstucke/leptos-fetch/pull/16))
- [`QueryClient::invalidate_queries_with_predicate`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.invalidate_queries_with_predicate) to invalidate a subset of queries in a scope without knowing the key ahead of time. ([#18](https://github.com/zakstucke/leptos-fetch/pull/18))
- [`QueryScope::with_invalidation_link`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryScope.html#method.subscribe_is_fetching::with_invalidation_link) to allow automatic hierarchies of invalidations across query types. [README](https://docs.rs/leptos-fetch/latest/leptos_fetch/#linked-invalidation). ([#33](https://github.com/zakstucke/leptos-fetch/pull/33), closes [#12](https://github.com/zakstucke/leptos-fetch/issues/12))

### Fixed
- Reduced internal generic codegen bloat over `K` internally, `Eq` no longer required, just `PartialEq` on nonlocal resources ([#10](https://github.com/zakstucke/leptos-fetch/pull/10))
- Subscription bugs ([#13](https://github.com/zakstucke/leptos-fetch/pull/10))
- Fixed edge case where 2 scopes with different options would be treated the same ([#22](https://github.com/zakstucke/leptos-fetch/pull/22))
- Improved threadsafety edgecases ([#25](https://github.com/zakstucke/leptos-fetch/pull/25))
- Remove disposed error edgecases in devtools ([#27](https://github.com/zakstucke/leptos-fetch/pull/27))
- keyer reactivity issues and an `ArcLocalSignal` type given leptos itself doesn't natively support one ([#31](https://github.com/zakstucke/leptos-fetch/pull/31))

### Tests
- Sped up CI with erase_components ([#17](https://github.com/zakstucke/leptos-fetch/pull/17))

---

## [0.4.0-beta](https://github.com/zakstucke/leptos-fetch/releases/tag/v0.4.0-beta)

### Added
- Upgrade to `leptos-0.8.0-beta` ([#7](https://github.com/zakstucke/leptos-fetch/pull/7))

## [0.3.1](https://github.com/zakstucke/leptos-fetch/releases/tag/v0.3.1)

### Added
- Added `subscribe_is_fetching()`, `arc_subscribe_is_fetching()`, `subscribe_is_loading()`, `arc_subscribe_is_loading()` methods to the `QueryClient` for monitoring query activity ([#3](https://github.com/zakstucke/leptos-fetch/pull/3))
- Improved README ([#3](https://github.com/zakstucke/leptos-fetch/pull/3))
- Sped up CI (([#6](https://github.com/zakstucke/leptos-fetch/pull/6)))
- `keyer` reactive args to resources, subscriptions etc now support `Option<K>` as output natively, making the usecase of an initially disabled resource much more seamless (([#21](https://github.com/zakstucke/leptos-fetch/pull/21)))

### Fixed
- Fixed bug in `invalidate_query_scope()` and `invalidate_all_queries()` to correctly invalidate active resources ([#3](https://github.com/zakstucke/leptos-fetch/pull/3))
- If `stale_time` is greater than `gc_time`, `stale_time` will be set to `gc_time` ([#3](https://github.com/zakstucke/leptos-fetch/pull/3))
- Workaround leptos bug with `ArcLocalResource` until they release a new version ([#5](https://github.com/zakstucke/leptos-fetch/pull/5))

### Tests
- Refactor ([#3](https://github.com/zakstucke/leptos-fetch/pull/3)) ([#5](https://github.com/zakstucke/leptos-fetch/pull/5))
- Revert removing task localset from tests (([#6](https://github.com/zakstucke/leptos-fetch/pull/6)))
- Test building for `--target wasm32-unknown-unknown` in CI ([#3](https://github.com/zakstucke/leptos-fetch/pull/3))
- Fixed non-deterministic test failure ([#4](https://github.com/zakstucke/leptos-fetch/pull/4))

## [0.3.0](https://github.com/zakstucke/leptos-fetch/releases/tag/v0.3.0)

Start recording changes.
