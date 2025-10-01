use std::time::Duration;

use leptos::{prelude::*, task::spawn_local};
use leptos_fetch::QueryClient;

use crate::utils::sleep_compat;

#[component]
pub fn TestCases() -> impl IntoView {
    let show_testcases = RwSignal::new(false);
    provide_context(TestCaseOwnerCleanupCtx);
    view! {
        <button on:click=move |_| {
            show_testcases.update(|b| *b = !*b);
        }>"Show testcases"</button>
        <Show when=move || show_testcases.get()>
            <TestCaseOwnerCleanup />
        </Show>
    }
}

/// Scenario: previously, the owner parent chain might clean up whilst a query is in process,
/// leading to panics when the query function tried to access context.
/// Fixed in https://github.com/zakstucke/leptos-fetch/pull/51
/// owner should now be preserved for the whole duration of a query function call.
#[component]
pub fn TestCaseOwnerCleanup() -> impl IntoView {
    let enable_owner_cleanup_test_component = RwSignal::new(false);
    provide_context(TestCaseOwnerCleanupCtx);
    view! {
        <button on:click=move |_| {
            enable_owner_cleanup_test_component.update(|b| *b = !*b);
            spawn_local(async move {
                sleep_compat(Duration::from_millis(500)).await;
                enable_owner_cleanup_test_component.set(false);
            });
        }>"Testcase: owner available during query function, panics on fail"</button>
        <Show when=move || enable_owner_cleanup_test_component.get()>
            <OwnerCleanupTestComponentChild />
        </Show>
    }
}


#[derive(Debug, Clone, Copy)]
struct TestCaseOwnerCleanupCtx;


#[component]
pub fn OwnerCleanupTestComponentChild() -> impl IntoView {
    let client: QueryClient = expect_context();

    let owner = Owner::current().unwrap().child();
    owner.with(|| {
        client.resource(
            {
                let _owner = owner.clone();
                move || {
                    async move {
                        expect_context::<TestCaseOwnerCleanupCtx>();
                        sleep_compat(Duration::from_millis(1000)).await;
                        expect_context::<TestCaseOwnerCleanupCtx>();
                    }
                }
            },
            || (),
        );
    });

    view! { <div>"child"</div> }
}