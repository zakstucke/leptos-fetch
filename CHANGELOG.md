# Changelog

All notable changes to this project are documented in this file.

## [Unreleased]

### BREAKING
- Subscriptions: ([#9](https://github.com/zakstucke/leptos-fetch/pull/9))
    - The `key: &K where K: 'static` argument has been replaced with `keyer: impl Fn() -> K + Send + Sync + 'static where K: Send + Sync + static`. This makes subscribers reactive to a changing key value and matches resources.
    - Because `Send + Sync` are now required, `subscribe_is_fetching_local()`, `subscribe_is_fetching_arc_local()`, `subscribe_is_loading_local()`, `subscribe_is_loading_arc_local()` have been added that do not have these bounds, and are always `true` on the server in `ssr`.
- Renaming, standardized `arc|local` to come at the end of method names to make autocomplete easier. Resources are the exception, they inherit their naming from leptos itself: ([#9](https://github.com/zakstucke/leptos-fetch/pull/9))
    - `arc_subscribe_is_fetching` -> `subscribe_is_fetching_arc`
    - `arc_subscribe_is_loading` -> `subscribe_is_loading_arc`
    - `prefetch_local_query` -> `prefetch_query_local`
    - `fetch_local_query` -> `fetch_query_local`
    - `set_local_query` -> `set_query_local`

### Added

### Fixed

### Tests

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