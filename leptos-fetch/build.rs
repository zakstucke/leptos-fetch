use rustc_version::{Channel, version_meta};

// This build script sets the `cfg` flag `CHANNEL_STABLE`, `CHANNEL_BETA`, `CHANNEL_NIGHTLY` or `CHANNEL_DEV` depending on the release channel of the compiler being used.
// https://stackoverflow.com/questions/61417452/how-to-get-a-feature-requirement-tag-in-the-documentation-generated-by-cargo-do
fn main() {
    // Set cfg flags depending on release channel

    // Declaring the flags so clippy/checks won't error:
    println!("cargo::rustc-check-cfg=cfg(CHANNEL_STABLE, values(none()))");
    println!("cargo::rustc-check-cfg=cfg(CHANNEL_BETA, values(none()))");
    println!("cargo::rustc-check-cfg=cfg(CHANNEL_NIGHTLY, values(none()))");
    println!("cargo::rustc-check-cfg=cfg(CHANNEL_DEV, values(none()))");
    // Working out the channel:
    let channel = match version_meta().unwrap().channel {
        Channel::Stable => "CHANNEL_STABLE",
        Channel::Beta => "CHANNEL_BETA",
        Channel::Nightly => "CHANNEL_NIGHTLY",
        Channel::Dev => "CHANNEL_DEV",
    };
    // Setting the correct flag:
    println!("cargo:rustc-cfg={channel}")
}
