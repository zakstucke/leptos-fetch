#[cfg(feature = "ssr")]
use axum::Router;

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    use example_blog::app::{shell, App};
    use leptos_axum::{generate_route_list, LeptosRoutes};

    // Setting this to None means we'll be using cargo-leptos and its env vars
    let conf = leptos::prelude::get_configuration(None).unwrap();
    let leptos_options = conf.leptos_options;
    let addr = leptos_options.site_addr;
    let routes = generate_route_list(App);

    // build our application with a route
    let app = Router::new()
        .leptos_routes(&leptos_options, routes, {
            let leptos_options = leptos_options.clone();
            move || shell(leptos_options.clone())
        })
        .fallback(leptos_axum::file_and_error_handler(shell))
        .with_state(leptos_options);

    // run our app with hyper
    // `axum::Server` is a re-export of `hyper::Server`
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    println!("listening on http://{}", &addr);
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}

#[cfg(not(feature = "ssr"))]
pub fn main() {
    use leptos::mount::mount_to_body;

    tracing_wasm::set_as_global_default();
    console_error_panic_hook::set_once();

    mount_to_body(example_blog::app::App);
}
