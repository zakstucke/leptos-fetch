#[cfg(all(not(feature = "devtools"), not(feature = "devtools-always")))]
/// If the `devtools` or `devtools-always` feature is enabled, this trait requires T to implement `std::fmt::Debug`.
/// Otherwise, this trait is implemented for all types.
///
/// This enforces query keys and values to implement [`std::fmt::Debug`] when using the `devtools` or `devtools-always` feature,
/// but doesn't enforce this if the user isn't using devtools.
pub trait DebugIfDevtoolsEnabled {}

#[cfg(all(not(feature = "devtools"), not(feature = "devtools-always")))]
impl<T> DebugIfDevtoolsEnabled for T {}

#[cfg(any(feature = "devtools", feature = "devtools-always"))]
/// If the `devtools` or `devtools-always` feature is enabled, this trait requires T to implement `std::fmt::Debug`.
/// Otherwise, this trait is implemented for all types.
///
/// This enforces query keys and values to implement [`std::fmt::Debug`] when using the `devtools` or `devtools-always` feature,
/// but doesn't enforce this if the user isn't using devtools.
pub trait DebugIfDevtoolsEnabled: std::fmt::Debug {}

#[cfg(any(feature = "devtools", feature = "devtools-always"))]
impl<T> DebugIfDevtoolsEnabled for T where T: std::fmt::Debug {}
