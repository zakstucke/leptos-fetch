# Changelog

All notable changes to this project are documented in this file.

## [Unreleased]

### Added

### Fixed

### Tests

---

## [0.3.1](https://github.com/zakstucke/leptos-fetch/releases/tag/v0.3.1)

### Added
- Added `subscribe_is_fetching()`, `arc_subscribe_is_fetching()`, `subscribe_is_loading()`, `arc_subscribe_is_loading()` methods to the `QueryClient` for monitoring query activity.
- Improved README

### Fixed
- Fixed bug in `invalidate_query_type()` and `invalidate_all_queries()` to correctly invalidate active resources.
- If `stale_time` is greater than `gc_time`, `stale_time` will be set to `gc_time`.

### Tests
- Refactor
- Test building for `--target wasm32-unknown-unknown` in CI

## [0.3.0](https://github.com/zakstucke/leptos-fetch/releases/tag/v0.3.0)

Start recording changes.