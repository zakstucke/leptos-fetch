/// [`QueryDevtools`] is provided to help visualize all of the inner workings of Leptos Fetch and will likely save a bunch of tedious debugging!
///
/// To enable, the `devtools` feature must be added, the component won't be shown or included in the binary when you build your app in release mode for performance.
///
/// If you need the devtools component in release mode, you can use the `devtools-always` feature instead.
///
/// ```bash
/// cargo add leptos-fetch --feature devtools
/// ```
///
/// ```rust,skip
/// use leptos::*;
/// use leptos_fetch::{QueryClient, QueryDevtools};
/// #[component]
/// fn App() -> impl IntoView {
///    let client = QueryClient::new().provide();
///     view!{
///         // This will render the devtools as a small widget in the bottom-right of the screen,
///         // this will only show in development mode.
///         <QueryDevtools client=client />
///         // Rest of App...
///     }
/// }
/// ```
#[leptos::component]
pub fn QueryDevtools(
    /// The client to monitor.
    client: crate::QueryClient,
) -> impl leptos::IntoView {
    #[cfg(any(
        all(debug_assertions, feature = "devtools"),
        feature = "devtools-always"
    ))]
    {
        use inner::dev_tools::DevtoolsRoot;
        use leptos::prelude::*;
        view! { <DevtoolsRoot client=client /> }
    }
}

#[cfg(any(
    all(debug_assertions, feature = "devtools"),
    feature = "devtools-always"
))]
mod inner;
