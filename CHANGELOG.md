# Changelog

All notable changes to this project are documented in this file.

## [Unreleased]

### BREAKING
- Changes required for the custom codec feature for correct type inferrence. ([#10](https://github.com/zakstucke/leptos-fetch/pull/10))
    - `QueryClient::new_with_options` and `QueryClient::provide_with_options` have been removed. Instead, `QueryClient::set_options(self, options) -> Self` is available as a builder method, e.g. `QueryClient::new().set_options(..)`. 
    - `QueryClient::provide()` replaced with `QueryClient::provide(self) -> Self`, e.g. `QueryClient::new().provide()`. 
    - `QueryClient::expect()` removed, `expect_context::<QueryClient>()` directly from leptos should be used instead. 
- Subscriptions: ([#9](https://github.com/zakstucke/leptos-fetch/pull/9))
    - `_subscribe_()` methods' `key: &K where K: 'static` argument replaced with `keyer: impl Fn() -> K + Send + Sync + 'static where K: Send + Sync + static`. This makes subscribers reactive to a changing key value to mirror resources.
    - Because `Send + Sync` are now required, `subscribe_is_fetching_local()`, `subscribe_is_fetching_arc_local()`, `subscribe_is_loading_local()`, `subscribe_is_loading_arc_local()` have been added that do not have these bounds.
- Renaming, standardized `arc|local` to come at the end of method names to make autocomplete easier. Resources are the exception, they inherit their naming from leptos itself: ([#9](https://github.com/zakstucke/leptos-fetch/pull/9))
    - `arc_subscribe_is_fetching` -> `subscribe_is_fetching_arc`
    - `arc_subscribe_is_loading` -> `subscribe_is_loading_arc`
    - `prefetch_local_query` -> `prefetch_query_local`
    - `fetch_local_query` -> `fetch_query_local`
    - `set_local_query` -> `set_query_local`

### Added
- For `ssr`, different codecs other than `serde`/json can be used when streaming resources from the backend. Codec choice defaults to [`codee::string::JsonSerdeCodec`](https://docs.rs/codee/latest/codee/string/struct.JsonSerdeCodec.html) like before and applies to the whole `QueryClient`. Customize with [`QueryClient::set_codec`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.set_codec). ([#10](https://github.com/zakstucke/leptos-fetch/pull/10))
- A devtools component has been added under the feature flag `devtools` to help visualize all of the inner workings of Leptos Fetch and will likely save a bunch of tedious debugging. See relevant section in the [README](https://docs.rs/leptos-fetch/latest/leptos_fetch/#devtools). ([#13](https://github.com/zakstucke/leptos-fetch/pull/13))
- Added `subscribe_value()`, `subscribe_value_local()`, `subscribe_value_arc()`, `subscribe_value_arc_local()` methods to the `QueryClient` returning `Signal<Option<V>>` for subscribing to query values without preventing their garbage collection or triggering fetching ([#13](https://github.com/zakstucke/leptos-fetch/pull/13))
- [`QueryClient::map_query`](https://docs.rs/leptos-fetch/latest/leptos_fetch/struct.QueryClient.html#method.map_query) added to make async updates to queries seamlessly, making patterns such as infinite queries simpler. Pagination and infinite query examples added to readme. ([#14](https://github.com/zakstucke/leptos-fetch/pull/14))
- Query functions can be supplied with no key parameter/argument, will default to `()`, prevents needing to add a wrapping function to external fns that don't need an argument. ([#16](https://github.com/zakstucke/leptos-fetch/pull/16))

### Fixed
- Reduced internal generic codegen bloat over `K` internally, `Eq` no longer required, just `PartialEq` on nonlocal resources ([#10](https://github.com/zakstucke/leptos-fetch/pull/10))
- Subscription bugs ([#13](https://github.com/zakstucke/leptos-fetch/pull/10))

### Tests
- Sped up CI with erase_components ([#17](https://github.com/zakstucke/leptos-fetch/pull/17))

---

## [0.3.1](https://github.com/zakstucke/leptos-fetch/releases/tag/v0.3.1)

### Added
- Added `subscribe_is_fetching()`, `arc_subscribe_is_fetching()`, `subscribe_is_loading()`, `arc_subscribe_is_loading()` methods to the `QueryClient` for monitoring query activity ([#3](https://github.com/zakstucke/leptos-fetch/pull/3))
- Improved README ([#3](https://github.com/zakstucke/leptos-fetch/pull/3))
- Sped up CI (([#6](https://github.com/zakstucke/leptos-fetch/pull/6)))

### Fixed
- Fixed bug in `invalidate_query_type()` and `invalidate_all_queries()` to correctly invalidate active resources ([#3](https://github.com/zakstucke/leptos-fetch/pull/3))
- If `stale_time` is greater than `gc_time`, `stale_time` will be set to `gc_time` ([#3](https://github.com/zakstucke/leptos-fetch/pull/3))
- Workaround leptos bug with `ArcLocalResource` until they release a new version ([#5](https://github.com/zakstucke/leptos-fetch/pull/5))

### Tests
- Refactor ([#3](https://github.com/zakstucke/leptos-fetch/pull/3)) ([#5](https://github.com/zakstucke/leptos-fetch/pull/5))
- Revert removing task localset from tests (([#6](https://github.com/zakstucke/leptos-fetch/pull/6)))
- Test building for `--target wasm32-unknown-unknown` in CI ([#3](https://github.com/zakstucke/leptos-fetch/pull/3))
- Fixed non-deterministic test failure ([#4](https://github.com/zakstucke/leptos-fetch/pull/4))

## [0.3.0](https://github.com/zakstucke/leptos-fetch/releases/tag/v0.3.0)

Start recording changes.