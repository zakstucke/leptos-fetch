use leptos::{either::Either, prelude::*};
use leptos_fetch::QueryClient;

use crate::blog_api::{delete_blogpost, get_blogpost_full, list_blogposts, AddBlogpost};

#[component]
pub fn BlogList() -> impl IntoView {
    let add_blogposts = ServerMultiAction::<AddBlogpost>::new();

    let client = expect_context::<QueryClient>();

    let bloglist = client.resource(list_blogposts, move || ());

    let active_blogpost_id = RwSignal::new(None);
    let active_blogpost = client.resource(get_blogpost_full, move || active_blogpost_id.get());

    let existing_blogposts = move || {
        Suspend::new(async move {
            bloglist.await.map(|blogposts| {
                if blogposts.is_empty() {
                    Either::Left(view! { <p>"No tasks were found."</p> })
                } else {
                    Either::Right(
                        blogposts
                            .iter()
                            .map(move |blogpost| {
                                let id = blogpost.id;
                                view! {
                                    <li style:padding-bottom="0.5em">
                                        {format!("{}: {}", blogpost.id, blogpost.title.clone())}
                                        <button
                                            type="button"
                                            style:margin-left="0.5em"
                                            on:click=move |_| {
                                                active_blogpost_id.try_set(Some(id));
                                            }
                                        >
                                            "Select"
                                        </button>
                                        <button
                                            type="button"
                                            style:margin-left="0.5em"
                                            on:click=move |_| {
                                                leptos::task::spawn_local(async move {
                                                    delete_blogpost(id).await.expect("delete blogposts");
                                                    if active_blogpost_id.try_get_untracked() == Some(Some(id))
                                                    {
                                                        active_blogpost_id.try_set(None);
                                                    }
                                                    client.invalidate_query_scope(list_blogposts);
                                                })
                                            }
                                        >
                                            "Delete (update via client.invalidate_query_scope())"
                                        </button>
                                        <button
                                            type="button"
                                            style:margin-left="0.5em"
                                            on:click=move |_| {
                                                leptos::task::spawn_local(async move {
                                                    delete_blogpost(id).await.expect("delete blogposts");
                                                    if active_blogpost_id.try_get_untracked() == Some(Some(id))
                                                    {
                                                        active_blogpost_id.try_set(None);
                                                    }
                                                    client
                                                        .update_query(
                                                            list_blogposts,
                                                            (),
                                                            |cached| {
                                                                if let Some(Ok(blogposts)) = cached {
                                                                    blogposts.retain(|blogpost| blogpost.id != id);
                                                                }
                                                            },
                                                        );
                                                })
                                            }
                                        >
                                            "Delete (update via client.update_query())"
                                        </button>
                                    </li>
                                }
                            })
                            .collect::<Vec<_>>(),
                    )
                }
            })
        })
    };

    let selected_blogpost = move || {
        Suspend::new(async move {
            active_blogpost.await.map(|blogpost| {
                if let Ok(Some(blogpost)) = blogpost {
                    Either::Left(view! {
                        <div style:padding-bottom="0.5em">
                            {format!(
                                "{}: {}",
                                blogpost.blogpost.id,
                                blogpost.blogpost.title.clone(),
                            )} <p>{blogpost.body.clone()}</p>
                        </div>
                    })
                } else {
                    Either::Right(view! { <p>"No selected blogpost."</p> })
                }
            })
        })
    };

    view! {
        <MultiActionForm action=add_blogposts>
            <label>"Add a blogpost" <input type="text" name="title" /></label>
            <input type="submit" value="Add" />
        </MultiActionForm>
        <div>
            <h3>"All blogposts in <Suspense/>"</h3>
            <Suspense fallback=move || view! { <p>"Loading..."</p> }>
                <ul>{existing_blogposts}</ul>
            </Suspense>
            <h3>"All blogposts in <Transition/>"</h3>
            <Transition fallback=move || view! { <p>"Loading..."</p> }>
                <ul>{existing_blogposts}</ul>
            </Transition>
        </div>
        <div>
            <h3>"Selected blogpost in <Suspense/>"</h3>
            <Suspense fallback=move || view! { <p>"Loading..."</p> }>{selected_blogpost}</Suspense>
            <h3>"Selected blogpost in <Transition/>"</h3>
            <Transition fallback=move || {
                view! { <p>"Loading..."</p> }
            }>{selected_blogpost}</Transition>
        </div>
    }
}
