# Contributing to Leptos Fetch

Thanks for your interesting in contributing to Leptos Fetch!

## Before Submitting a PR

Any new features should have some sort of tests added, these all currently sit in the `lib.rs` file.

To pass CI, you can test your changes locally with:
- `cargo fmt`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test --all`
- `cargo test --all --features ssr`