# Changelog

All notable changes to this project are documented in this file.

## [Unreleased]

### Added
- Updated readme ([#8](https://github.com/zakstucke/leptos-fetch/pull/8))

### Fixed

### Tests

---

## [0.4.0-beta](https://github.com/zakstucke/leptos-fetch/releases/tag/v0.4.0-beta)

### Added
- Upgrade to `leptos-0.8.0-beta` ([#7](https://github.com/zakstucke/leptos-fetch/pull/7))

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