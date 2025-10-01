pub mod app;
pub mod blog_api;
pub mod blog_list;
pub mod testcases;
pub mod utils;

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    use crate::app::App;
    tracing_wasm::set_as_global_default();
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(App);
}
