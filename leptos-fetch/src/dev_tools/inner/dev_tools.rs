use chrono::{TimeDelta, Utc};

use leptos::{portal::Portal, prelude::*};

use super::{
    cache_representation::{CacheRep, QueryRep, QueryState, prepare},
    components::*,
    sort::SortConfig,
    sort::{SortOption, filter_s},
};

use crate::{
    QueryClient,
    query_scope::ScopeCacheKey,
    safe_dt_dur_add,
    utils::{KeyHash, new_buster_id},
};

const TIME_FORMAT: &str = "%H:%M:%S.%3f"; // %3f will give millisecond accuracy

#[derive(Clone)]
pub(crate) struct DevtoolsContext {
    pub cache_rep: CacheRep,
    pub open: ArcRwSignal<bool>,
    pub filter: ArcRwSignal<String>,
    pub scope_sort_config: ArcRwSignal<SortConfig>,
    pub selected_query: ArcRwSignal<Option<QueryRep>>,
}

#[component]
pub(crate) fn DevtoolsRoot(client: QueryClient) -> impl IntoView {
    let open = RwSignal::new(false);
    view! {
        <Portal>
            <style>{include_str!("./styles.css")}</style>
            <div class="leptos-fetch-devtools lq-font-mono">
                <Show
                    when=move || open.get()
                    fallback=move || {
                        view! { <FallbackLogo open=open /> }
                    }
                >
                    {
                        provide_context(DevtoolsContext {
                            cache_rep: prepare(client),
                            open: open.into(),
                            filter: ArcRwSignal::new("".to_string()),
                            scope_sort_config: ArcRwSignal::new(SortConfig::new_default_ascii()),
                            selected_query: ArcRwSignal::new(None),
                        });
                        view! { <DevtoolsInner client=client /> }
                    }
                </Show>
            </div>
        </Portal>
    }
}

#[component]
pub(crate) fn DevtoolsInner(client: QueryClient) -> impl IntoView {
    let DevtoolsContext {
        cache_rep,
        selected_query,
        filter,
        scope_sort_config,
        ..
    } = expect_context();

    let filtered_and_sorted_queries =
        Signal::derive({
            let filter = filter.clone();
            let scope_sort_config = scope_sort_config.clone();
            move || {
                let filter = filter.get();

                let mut queries = cache_rep
                    .all_queries
                    .read()
                    .iter()
                    .map(|(cache_key, scoped_queries)| {
                        (
                            *cache_key,
                            scoped_queries.title.clone(),
                            scoped_queries.sort_config.clone(),
                            scoped_queries.filter.clone(),
                            filter_s(
                                &filter,
                                &[&scoped_queries.title],
                                |(_key_hash, query)| query.debug_key.compact().as_str().into(),
                                scoped_queries.scoped_queries.read().iter(),
                            )
                            .into_iter()
                            .map(|(key_hash, query)| (*key_hash, query.clone()))
                            .collect::<Vec<_>>(),
                        )
                    })
                    .filter(
                        |(_cache_key, _scope_title, _sort_config, _filter, queries)| {
                            // Filter out empty queries
                            !queries.is_empty()
                        },
                    )
                    .collect::<Vec<_>>();

                scope_sort_config.read().sort(
                    &mut queries,
                    |(_cache_key, scope_title, _sort_config, _filter, queries)| {
                        // Including the query keys in the search, making it more of a global search
                        std::iter::once(scope_title.as_str().into())
                            .chain(queries.iter().map(|(_key_hash, query)| {
                                query.debug_key.compact().as_str().into()
                            }))
                            .collect::<Vec<_>>()
                    },
                    |(_cache_key, _scope_title, _sort_config, _filter, queries)| {
                        queries
                            .iter()
                            .map(|(_key_hash, query)| query.value_derivs.read().updated_at)
                            .max()
                            .unwrap_or(chrono::DateTime::<Utc>::MIN_UTC)
                    },
                    |(_cache_key, _scope_title, _sort_config, _filter, queries)| {
                        queries
                            .iter()
                            .map(|(_key_hash, query)| query.created_at)
                            .max()
                            .unwrap_or(chrono::DateTime::<Utc>::MIN_UTC)
                    },
                );

                queries
            }
        });

    let container_ref = leptos::prelude::NodeRef::<leptos::html::Div>::new();

    let height_signal = RwSignal::new(500);

    // Drag start handler
    let handle_drag_start = move |event: web_sys::MouseEvent| {
        use leptos::wasm_bindgen::JsCast;
        use leptos::wasm_bindgen::closure::Closure;

        let bounding = container_ref
            .get()
            .expect("container to be mounted")
            .get_bounding_client_rect();

        let height = bounding.height();

        let start_y = event.client_y() as f64;

        let move_closure = Closure::wrap(Box::new(move |move_event: web_sys::MouseEvent| {
            move_event.prevent_default();

            let val_to_add = start_y - move_event.client_y() as f64;

            let new_height = (height + val_to_add).max(200.0);

            height_signal.set(new_height as i32);
        }) as Box<dyn FnMut(_)>)
        .into_js_value();

        // Register the move event listener
        if let Some(window) = web_sys::window() {
            let end = std::rc::Rc::new(std::cell::Cell::new(None::<Closure<dyn FnMut()>>));
            let end_closure = Closure::wrap({
                let window = window.clone();
                let move_closure = move_closure.clone();
                Box::new(move || {
                    window
                        .remove_event_listener_with_callback(
                            "mousemove",
                            move_closure.as_ref().unchecked_ref(),
                        )
                        .unwrap();

                    if let Some(end) = end.take() {
                        let _ = window.remove_event_listener_with_callback(
                            "mouseup",
                            end.as_ref().unchecked_ref(),
                        );
                    }
                }) as Box<dyn FnMut()>
            })
            .into_js_value();

            window
                .add_event_listener_with_callback(
                    "mousemove",
                    move_closure.as_ref().clone().unchecked_ref(),
                )
                .unwrap();

            window
                .add_event_listener_with_callback("mouseup", end_closure.as_ref().unchecked_ref())
                .unwrap();
        }
    };

    view! {
        <div
            class="lq-bg-lq-background lq-text-lq-foreground lq-px-0 lq-fixed lq-bottom-0 lq-left-0 lq-right-0 lq-z-[1000]"
            style:height=move || format!("{}px", height_signal.get())
            node_ref=container_ref
        >
            <div
                class="lq-w-full lq-py-1 lq-bg-lq-background lq-cursor-ns-resize lq-transition-colors hover:lq-bg-lq-border"
                on:mousedown=handle_drag_start
            ></div>
            <div class="lq-h-full lq-flex lq-flex-col lq-relative">
                <div class="lq-flex-1 lq-overflow-hidden lq-flex">
                    <div class="lq-flex lq-flex-col lq-flex-1  lq-overflow-x-hidden">
                        <div class="lq-flex-none">
                            <Header />
                            <div class="lq-py-1 lq-px-2 lq-border-lq-border lq-border-b lq-flex lq-items-center lq-w-full lq-justify-between lq-max-w-full lq-overflow-x-auto lq-gap-2 lq-no-scrollbar">
                                <div class="lq-flex lq-items-center lq-gap-2 lq-flex-wrap">
                                    <SearchInput filter=filter.clone() maybe_cache_key=None />
                                    <SetSort sort_config=scope_sort_config.clone() />
                                    <SetSortOrder sort_config=scope_sort_config.clone() />
                                    <ClearCache client=client maybe_cache_key=None />
                                </div>
                            </div>
                        </div>

                        <ul class="lq-flex lq-flex-col lq-gap-1 lq-overflow-y-auto">
                            <For
                                each=move || { filtered_and_sorted_queries.get() }
                                key=move |(cache_key, _title, _sort_config, _filter, queries)| {
                                    (
                                        *cache_key,
                                        queries
                                            .iter()
                                            .map(|(key_hash, _query)| *key_hash)
                                            .collect::<Vec<_>>(),
                                    )
                                }
                                children=move |(cache_key, title, sort_config, filter, queries)| {
                                    let filtered_and_sorted_queries = Signal::derive({
                                        let sort_config = sort_config.clone();
                                        let filter = filter.clone();
                                        move || {
                                            let filter = filter.get();
                                            let mut queries = filter_s(
                                                    &filter,
                                                    &[],
                                                    |(_key_hash, query)| {
                                                        query.debug_key.compact().as_str().into()
                                                    },
                                                    queries.iter(),
                                                )
                                                .into_iter()
                                                .map(|(key_hash, query)| (*key_hash, query.clone()))
                                                .collect::<Vec<_>>();
                                            sort_config
                                                .read()
                                                .sort(
                                                    &mut queries,
                                                    |(_key_hash, query)| {
                                                        vec![query.debug_key.compact().as_str().into()]
                                                    },
                                                    |(_key_hash, query)| {
                                                        query.value_derivs.read().updated_at
                                                    },
                                                    |(_key_hash, query)| { query.created_at },
                                                );
                                            queries
                                        }
                                    });
                                    let open = RwSignal::new(true);
                                    view! {
                                        <div>
                                            <div class="lq-py-1 lq-px-2 lq-border-lq-border lq-border-b lq-flex lq-items-center lq-w-full lq-justify-between lq-max-w-full lq-overflow-x-auto lq-flex-wrap lq-gap-2 lq-no-scrollbar">
                                                <div class="lq-flex lq-flex-row lq-items-center lq-gap-2">
                                                    <CaretSwitch open=open />
                                                    <pre class="lq-font-bold lq-text-sm lq-whitespace-pre-wrap">
                                                        {title.to_string()}
                                                    </pre>
                                                </div>
                                                <div class="lq-flex lq-items-center lq-gap-2 lq-flex-wrap">
                                                    <SearchInput
                                                        filter=filter.clone()
                                                        maybe_cache_key=Some(cache_key)
                                                    />
                                                    <SetSort sort_config=sort_config.clone() />
                                                    <SetSortOrder sort_config=sort_config.clone() />
                                                    <ClearCache client=client maybe_cache_key=Some(cache_key) />
                                                </div>
                                            </div>
                                            <Show when=move || { open.get() }>
                                                <div class="lq-pl-5">
                                                    <For
                                                        each=move || { filtered_and_sorted_queries.get() }
                                                        key=move |(key_hash, _query)| (cache_key, *key_hash)
                                                        children=move |(key_hash, query)| {
                                                            view! {
                                                                <QueryRow
                                                                    cache_key=cache_key
                                                                    key_hash=key_hash
                                                                    query=query
                                                                />
                                                            }
                                                        }
                                                    />
                                                </div>
                                            </Show>
                                        </div>
                                    }
                                }
                            />
                        </ul>
                    </div>
                    <Show when={
                        let selected_query = selected_query.clone();
                        move || { selected_query.get().is_some() }
                    }>
                        {{
                            let selected_query = selected_query.clone();
                            move || {
                                selected_query
                                    .get()
                                    .map(|q| view! { <SelectedQuery client=client query=q /> })
                            }
                        }}

                    </Show>
                </div>
                <div class="lq-absolute -lq-top-6 lq-right-2">
                    <CloseButton />
                </div>
            </div>
        </div>
    }
}

#[component]
fn FallbackLogo(open: RwSignal<bool>) -> impl IntoView {
    let toggle_left = RwSignal::new(false);
    view! {
        <div class=move || {
            format!(
                "lq-fixed lq-bottom-3 {}",
                if toggle_left.get() { "lq-left-3" } else { "lq-right-3" },
            )
        }>
            <div class=move || {
                format!(
                    "lq-flex lq-justify-center lq-items-center lq-gap-x-1 {}",
                    if toggle_left.get() { "lq-flex-row" } else { "lq-flex-row-reverse" },
                )
            }>
                <button
                    on:click=move |_| open.set(true)
                    class="lq-bg-zinc-200 text-lq-foreground lq-rounded-full lq-w-12 lq-h-12 hover:-lq-translate-y-1 hover:lq-bg-zinc-300 lq-transition-all lq-duration-200"
                    inner_html=include_str!("logo.svg")
                ></button>
                <button
                    on:click=move |_| toggle_left.update(|v| *v = !*v)
                    class="lq-bg-lq-background lq-text-lq-foreground lq-rounded-sm lq-w-4 lq-h-4 lq-transition-colors lq-duration-200 hover:lq-bg-lq-border"
                >
                    <svg
                        width="15"
                        height="15"
                        viewBox="0 0 15 15"
                        fill="none"
                        xmlns="http://www.w3.org/2000/svg"
                        class=move || {
                            if toggle_left.get() { "lq-rotate-90" } else { "-lq-rotate-90" }
                        }
                    >
                        <path
                            d="M7.14645 2.14645C7.34171 1.95118 7.65829 1.95118 7.85355 2.14645L11.8536 6.14645C12.0488 6.34171 12.0488 6.65829 11.8536 6.85355C11.6583 7.04882 11.3417 7.04882 11.1464 6.85355L8 3.70711L8 12.5C8 12.7761 7.77614 13 7.5 13C7.22386 13 7 12.7761 7 12.5L7 3.70711L3.85355 6.85355C3.65829 7.04882 3.34171 7.04882 3.14645 6.85355C2.95118 6.65829 2.95118 6.34171 3.14645 6.14645L7.14645 2.14645Z"
                            fill="currentColor"
                            fill-rule="evenodd"
                            clip-rule="evenodd"
                        ></path>
                    </svg>
                </button>
            </div>
        </div>
    }
}

#[component]
fn CloseButton() -> impl IntoView {
    let DevtoolsContext { open, .. } = expect_context();

    view! {
        <button
            on:click=move |_| open.set(false)
            class="lq-bg-lq-background lq-text-lq-foreground lq-rounded-t-sm lq-w-6 lq-h-6 lq-p-1 lq-transition-colors lq-hover:bg-lq-accent"
        >
            <svg
                width="100%"
                height="100%"
                viewBox="0 0 15 15"
                fill="none"
                xmlns="http://www.w3.org/2000/svg"
            >
                <path
                    d="M12.8536 2.85355C13.0488 2.65829 13.0488 2.34171 12.8536 2.14645C12.6583 1.95118 12.3417 1.95118 12.1464 2.14645L7.5 6.79289L2.85355 2.14645C2.65829 1.95118 2.34171 1.95118 2.14645 2.14645C1.95118 2.34171 1.95118 2.65829 2.14645 2.85355L6.79289 7.5L2.14645 12.1464C1.95118 12.3417 1.95118 12.6583 2.14645 12.8536C2.34171 13.0488 2.65829 13.0488 2.85355 12.8536L7.5 8.20711L12.1464 12.8536C12.3417 13.0488 12.6583 13.0488 12.8536 12.8536C13.0488 12.6583 13.0488 12.3417 12.8536 12.1464L8.20711 7.5L12.8536 2.85355Z"
                    fill="currentColor"
                    fill-rule="evenodd"
                    clip-rule="evenodd"
                ></path>
            </svg>
        </button>
    }
}

#[component]
fn Header() -> impl IntoView {
    let DevtoolsContext { cache_rep, .. } = expect_context();

    let num_fresh = Signal::derive({
        let all_queries = cache_rep.all_queries.clone();
        move || {
            all_queries
                .read()
                .values()
                .map(|scoped_queries| {
                    scoped_queries
                        .scoped_queries
                        .read()
                        .values()
                        .filter(|query| matches!(query.state.get(), QueryState::Fresh))
                        .count()
                })
                .sum::<usize>()
        }
    });

    let num_fetching = Signal::derive({
        let all_queries = cache_rep.all_queries.clone();
        move || {
            all_queries
                .read()
                .values()
                .map(|scoped_queries| {
                    scoped_queries
                        .scoped_queries
                        .read()
                        .values()
                        .filter(|query| matches!(query.state.get(), QueryState::Fetching))
                        .count()
                })
                .sum::<usize>()
        }
    });

    let stale = Signal::derive({
        let all_queries = cache_rep.all_queries.clone();
        move || {
            all_queries
                .read()
                .values()
                .map(|scoped_queries| {
                    scoped_queries
                        .scoped_queries
                        .read()
                        .values()
                        .filter(|query| matches!(query.state.get(), QueryState::Stale))
                        .count()
                })
                .sum::<usize>()
        }
    });

    let invalid = Signal::derive({
        let all_queries = cache_rep.all_queries.clone();
        move || {
            all_queries
                .read()
                .values()
                .map(|scoped_queries| {
                    scoped_queries
                        .scoped_queries
                        .read()
                        .values()
                        .filter(|query| matches!(query.state.get(), QueryState::Invalid))
                        .count()
                })
                .sum::<usize>()
        }
    });

    let total = Signal::derive({
        let all_queries = cache_rep.all_queries.clone();
        move || {
            all_queries
                .read()
                .values()
                .map(|scoped_queries| scoped_queries.scoped_queries.read().len())
                .sum::<usize>()
        }
    });

    let label_class = "lq-hidden lg:lq-inline-block";
    view! {
        <div class="lq-flex-none lq-flex lq-justify-between lq-w-full lq-overflow-y-hidden lq-items-center lq-border-b lq-border-lq-border lq-pb-2 lq-px-1">
            <h3 class="lq-pl-2 lq-tracking-tighter lq-text-lg lq-italic lq-text-transparent lq-bg-clip-text lq-font-bold lq-bg-gradient-to-r lq-from-red-800 lq-to-orange-400">
                Leptos Fetch
            </h3>

            <div class="lq-flex lq-gap-2 lq-px-2">
                <DotBadge color=ColorOption::Blue>
                    <span class=label_class>Fetching</span>
                    <span>{num_fetching}</span>
                </DotBadge>

                <DotBadge color=ColorOption::Green>
                    <span class=label_class>Fresh</span>
                    <span>{num_fresh}</span>
                </DotBadge>

                <DotBadge color=ColorOption::Yellow>
                    <span class=label_class>Stale</span>
                    <span>{stale}</span>
                </DotBadge>

                <DotBadge color=ColorOption::Red>
                    <span class=label_class>Invalid</span>
                    <span>{invalid}</span>
                </DotBadge>

                <DotBadge color=ColorOption::Gray>
                    <span class=label_class>Total</span>
                    <span>{total}</span>
                </DotBadge>
            </div>
        </div>
    }
}

#[component]
fn SearchInput(
    filter: ArcRwSignal<String>,
    maybe_cache_key: Option<ScopeCacheKey>,
) -> impl IntoView {
    view! {
        <div class="lq-relative lq-w-64">
            <div class="lq-pointer-events-none lq-absolute lq-inset-y-0 lq-left-0 lq-flex lq-items-center lq-pl-3 lq-text-zinc-400">
                <svg
                    class="lq-h-4 lq-w-4"
                    viewBox="0 0 20 20"
                    fill="currentColor"
                    aria-hidden="true"
                >
                    <path
                        fill-rule="evenodd"
                        d="M9 3.5a5.5 5.5 0 100 11 5.5 5.5 0 000-11zM2 9a7 7 0 1112.452 4.391l3.328 3.329a.75.75 0 11-1.06 1.06l-3.329-3.328A7 7 0 012 9z"
                        clip-rule="evenodd"
                    ></path>
                </svg>
            </div>
            <input
                id=if let Some(cache_key) = maybe_cache_key {
                    format!("search-{:?}", cache_key)
                } else {
                    "search".to_string()
                }
                class="lq-form-input lq-block lq-w-full lq-rounded-md lq-bg-lq-input lq-py-0 lq-pl-10 lq-pr-3 lq-text-lq-input-foreground lq-text-xs lq-leading-6 lq-placeholder-lq-input-foreground lq-border lq-border-lq-border"
                placeholder=if maybe_cache_key.is_some() { "Search Scope" } else { "Search All" }
                name="search"
                autocomplete="off"
                type="search"
                on:input={
                    let filter = filter.clone();
                    move |ev| {
                        let value = event_target_value(&ev);
                        filter.set(value);
                    }
                }

                prop:value=filter
            />
        </div>
    }
}

#[component]
fn SetSort(sort_config: ArcRwSignal<SortConfig>) -> impl IntoView {
    view! {
        <select
            id="countries"
            class="lq-form-select lq-border-lq-border lq-border lq-text-xs lq-rounded-md lq-block lq-w-48 lq-py-1 lq-px-2 lq-bg-lq-input lq-text-lq-input-foreground lq-line-clamp-1"
            prop:value={
                let sort_config = sort_config.clone();
                move || sort_config.get().sort.as_str().to_string()
            }
            on:change={
                let sort_config = sort_config.clone();
                move |ev| {
                    let new_value = event_target_value(&ev);
                    sort_config.write().sort = SortOption::from_string(&new_value);
                }
            }
        >
            <option
                value=SortOption::UpdatedAt.as_str()
                selected=sort_config.get_untracked().sort.as_str() == SortOption::UpdatedAt.as_str()
            >
                Sort by last updated
            </option>
            <option
                value=SortOption::CreatedAt.as_str()
                selected=sort_config.get_untracked().sort.as_str() == SortOption::CreatedAt.as_str()
            >
                Sort by creation time
            </option>
            <option
                value=SortOption::Ascii.as_str()
                selected=sort_config.get_untracked().sort.as_str() == SortOption::Ascii.as_str()
            >
                Sort by query key
            </option>
        </select>
    }
}

#[component]
fn SetSortOrder(sort_config: ArcRwSignal<SortConfig>) -> impl IntoView {
    view! {
        <button
            class="lq-bg-lq-input lq-text-lq-input-foreground lq-rounded-md lq-px-2 lq-py-1 lq-text-xs lq-inline-flex lq-items-center lq-gap-1 lq-border lq-border-lq-border"
            on:click={
                let sort_config = sort_config.clone();
                move |_| {
                    let mut guard = sort_config.write();
                    guard.order_asc = !guard.order_asc;
                }
            }
        >

            <span class="w-8">
                {
                    let sort_config = sort_config.clone();
                    move || { if sort_config.read().order_asc { "Asc " } else { "Desc" } }
                }
            </span>
            <svg
                width="15"
                height="15"
                viewBox="0 0 15 15"
                fill="none"
                xmlns="http://www.w3.org/2000/svg"
                class={
                    let sort_config = sort_config.clone();
                    move || { if sort_config.read().order_asc { "" } else { "lq-rotate-180" } }
                }
            >
                <path
                    d="M7.14645 2.14645C7.34171 1.95118 7.65829 1.95118 7.85355 2.14645L11.8536 6.14645C12.0488 6.34171 12.0488 6.65829 11.8536 6.85355C11.6583 7.04882 11.3417 7.04882 11.1464 6.85355L8 3.70711L8 12.5C8 12.7761 7.77614 13 7.5 13C7.22386 13 7 12.7761 7 12.5L7 3.70711L3.85355 6.85355C3.65829 7.04882 3.34171 7.04882 3.14645 6.85355C2.95118 6.65829 2.95118 6.34171 3.14645 6.14645L7.14645 2.14645Z"
                    fill="currentColor"
                    fill-rule="evenodd"
                    clip-rule="evenodd"
                ></path>
            </svg>
        </button>
    }
}

#[component]
fn CaretSwitch(open: RwSignal<bool>) -> impl IntoView {
    view! {
        <button
            class="lq-text-lq-foreground lq-bg-lq-input lq-p-1 lq-rounded-md lq-transition-colors lq-duration-200 hover:lq-bg-lq-border"
            on:click=move |_| {
                let mut guard = open.write();
                *guard = !*guard;
            }
        >
            <svg
                class=move || {
                    format!("lq-h-4 lq-w-4 {}", if open.get() { "lq-rotate-180" } else { "" })
                }
                viewBox="0 0 20 20"
                fill="currentColor"
                aria-hidden="true"
            >
                <path
                    fill-rule="evenodd"
                    d="M5.293 7.293a1 1 0 011.414 0L10 10.586l3.293-3.293a1 1 0 111.414 1.414l-4 4a1 1 0 01-1.414 0l-4-4a1 1 0 010-1.414z"
                    clip-rule="evenodd"
                ></path>
            </svg>
        </button>
    }
}

#[component]
fn ClearCache(client: QueryClient, maybe_cache_key: Option<ScopeCacheKey>) -> impl IntoView {
    view! {
        <button
            class="lq-bg-lq-input lq-text-lq-input-foreground lq-rounded-md lq-px-2 lq-py-1 lq-text-xs lq-inline-flex lq-items-center lq-gap-1 lq-border lq-border-lq-border"
            on:click=move |_| {
                client.clear();
            }
        >
            {if maybe_cache_key.is_some() { "Clear" } else { "Clear All" }}
            <svg
                width="15"
                height="15"
                viewBox="0 0 15 15"
                fill="none"
                xmlns="http://www.w3.org/2000/svg"
            >
                <path
                    d="M5.5 1C5.22386 1 5 1.22386 5 1.5C5 1.77614 5.22386 2 5.5 2H9.5C9.77614 2 10 1.77614 10 1.5C10 1.22386 9.77614 1 9.5 1H5.5ZM3 3.5C3 3.22386 3.22386 3 3.5 3H5H10H11.5C11.7761 3 12 3.22386 12 3.5C12 3.77614 11.7761 4 11.5 4H11V12C11 12.5523 10.5523 13 10 13H5C4.44772 13 4 12.5523 4 12V4L3.5 4C3.22386 4 3 3.77614 3 3.5ZM5 4H10V12H5V4Z"
                    fill="currentColor"
                    fill-rule="evenodd"
                    clip-rule="evenodd"
                ></path>
            </svg>
        </button>
        <button
            class="lq-bg-lq-input lq-text-lq-input-foreground lq-rounded-md lq-px-2 lq-py-1 lq-text-xs lq-inline-flex lq-items-center lq-gap-1 lq-border lq-border-lq-border"
            on:click=move |_| {
                if let Some(cache_key) = maybe_cache_key.as_ref() {
                    client.invalidate_query_scope_inner(cache_key);
                } else {
                    client.invalidate_all_queries();
                }
            }
        >
            {if maybe_cache_key.is_some() { "Invalidate" } else { "Invalidate All" }}
            <svg
                width="15"
                height="15"
                viewBox="0 0 15 15"
                fill="none"
                xmlns="http://www.w3.org/2000/svg"
            >
                <path
                    d="M5.5 1C5.22386 1 5 1.22386 5 1.5C5 1.77614 5.22386 2 5.5 2H9.5C9.77614 2 10 1.77614 10 1.5C10 1.22386 9.77614 1 9.5 1H5.5ZM3 3.5C3 3.22386 3.22386 3 3.5 3H5H10H11.5C11.7761 3 12 3.22386 12 3.5C12 3.77614 11.7761 4 11.5 4H11V12C11 12.5523 10.5523 13 10 13H5C4.44772 13 4 12.5523 4 12V4L3.5 4C3.22386 4 3 3.77614 3 3.5ZM5 4H10V12H5V4Z"
                    fill="currentColor"
                    fill-rule="evenodd"
                    clip-rule="evenodd"
                ></path>
            </svg>
        </button>
    }
}

#[component]
fn QueryRow(cache_key: ScopeCacheKey, key_hash: KeyHash, query: QueryRep) -> impl IntoView {
    let DevtoolsContext {
        selected_query,
        cache_rep,
        ..
    } = expect_context();

    let query = StoredValue::new(query);

    // Remove the query from the CacheRep when it's been gc'd:
    Effect::new({
        let selected_query = selected_query.clone();
        move || {
            if query.read_value().value_derivs.read().is_gced {
                cache_rep.remove_query(cache_key, key_hash);
                if selected_query
                    .get()
                    .as_ref()
                    .is_some_and(|q| q.cache_key == cache_key && q.key_hash == key_hash)
                {
                    selected_query.set(None);
                }
            }
        }
    });

    let active_resources = move || {
        let count = query.read_value().active_resources.get();
        view! {
            <span class=format!(
                "lq-inline-flex lq-items-center lq-gap-x-1.5 lq-rounded-md lq-px-2 lq-py-1 lq-text-xs lq-font-medium {}",
                if count == 0 {
                    "lq-bg-gray-100 lq-text-gray-700"
                } else {
                    "lq-bg-blue-100 lq-text-blue-700"
                },
            )>
                {if count == 0 {
                    "0".to_string()
                } else {
                    format!("{} active resource{}", count, if count > 1 { "s" } else { "" })
                }}
            </span>
        }
    };

    let is_selected = Signal::derive({
        let selected_query = selected_query.clone();
        move || {
            selected_query
                .read()
                .as_ref()
                .is_some_and(|q| q.cache_key == cache_key && q.key_hash == key_hash)
        }
    });

    view! {
        <li
            class=move || {
                format!(
                    "hover:lq-bg-lq-accent lq-transition-colors lq-flex lq-w-full lq-gap-4 lq-items-center lq-border-lq-border lq-border-b lq-p-1 {}",
                    if is_selected.get() { "lq-bg-lq-accent" } else { "" },
                )
            }
            on:click={
                let selected_query = selected_query.clone();
                move |_| {
                    if is_selected.get_untracked() {
                        selected_query.set(None);
                    } else {
                        selected_query.set(Some(query.get_value()))
                    }
                }
            }
        >

            {active_resources}
            <span class="lq-w-[4.5rem]">
                <RowStateLabel query=query.get_value() />
            </span>
            <span class="lq-text-sm">{query.read_value().debug_key.compact().to_string()}</span>
        </li>
    }
}

#[component]
fn RowStateLabel(query: QueryRep) -> impl IntoView {
    move || {
        let state = query.state.read();
        let color = state.color();
        let label = state.as_str();
        view! {
            <DotBadge color=color dot=false>
                {label}
            </DotBadge>
        }
    }
}

#[component]
fn SelectedQuery(client: QueryClient, query: QueryRep) -> impl IntoView {
    let created_at = StoredValue::new(
        query
            .created_at
            .with_timezone(&chrono::Local)
            .format(TIME_FORMAT)
            .to_string(),
    );

    let query = StoredValue::new(query);

    let last_update = Signal::derive(move || {
        query
            .read_value()
            .value_derivs
            .read()
            .updated_at
            .with_timezone(&chrono::Local)
            .format(TIME_FORMAT)
            .to_string()
    });

    let section_class = "lq-px-2 lq-py-1 lq-flex lq-flex-col lq-items-center lq-gap-1 lq-w-full";
    let entry_class =
        "lq-flex lq-items-center lq-justify-start lq-text-xs lq-font-medium lq-w-full";

    fn format_duration(dur: Option<std::time::Duration>) -> String {
        if let Some(dur) = dur {
            chrono_humanize::HumanTime::from(
                chrono::Duration::from_std(dur).expect("chrono duration"),
            )
            .to_text_en(
                chrono_humanize::Accuracy::Precise,
                chrono_humanize::Tense::Present,
            )
            .to_string()
        } else {
            "n/a".to_string()
        }
    }

    let stale_in_str = RwSignal::new("".to_string());
    let gc_in_str = RwSignal::new("".to_string());
    let refetch_in_str = RwSignal::new("".to_string());
    let interval_buster = RwSignal::new(new_buster_id());
    Effect::new(move |maybe_handle: Option<TimeoutHandle>| {
        interval_buster.get();

        // Cancel last interval, might be stale:
        if let Some(handle) = maybe_handle {
            handle.clear();
        }

        let query = query.read_value();
        let opts = query.combined_options;
        let updated_at = query.value_derivs.read().updated_at;

        let format_td = |td: TimeDelta| {
            chrono_humanize::HumanTime::from(
                // Removing millisecond accuracy:
                TimeDelta::milliseconds((td.num_milliseconds() / 1000) * 1000),
            )
            .to_text_en(
                chrono_humanize::Accuracy::Precise,
                chrono_humanize::Tense::Future,
            )
        };

        let stale_in = safe_dt_dur_add(updated_at, opts.stale_time()) - chrono::Utc::now();
        if stale_in > TimeDelta::zero() && stale_in < TimeDelta::days(1) {
            stale_in_str.set(format!(" ({})", format_td(stale_in)));
        } else {
            stale_in_str.set("".to_string());
        }

        if query.active_resources.get() == 0 {
            let gc_in = safe_dt_dur_add(updated_at, opts.gc_time()) - chrono::Utc::now();
            if gc_in > TimeDelta::zero() && gc_in < TimeDelta::days(1) {
                gc_in_str.set(format!(" ({})", format_td(gc_in)));
            } else {
                gc_in_str.set("".to_string());
            }
        } else {
            gc_in_str.set("".to_string());
        }

        if let Some(refetch_interval) = opts.refetch_interval() {
            let refetch_in = safe_dt_dur_add(updated_at, refetch_interval) - chrono::Utc::now();
            if refetch_in > TimeDelta::zero() && refetch_in < TimeDelta::days(1) {
                refetch_in_str.set(format!(" ({})", format_td(refetch_in)));
            } else {
                refetch_in_str.set("".to_string());
            }
        }

        // Basic interval updating:
        set_timeout_with_handle(
            move || interval_buster.set(new_buster_id()),
            std::time::Duration::from_millis(33),
        )
        .expect("set_timeout")
    });

    let details_open = RwSignal::new(false);
    let events_open = RwSignal::new(false);
    view! {
        <div class="lq-w-1/2 lq-overflow-y-scroll lq-max-h-full lq-border-black lq-border-l-4">
            <div class="lq-flex lq-flex-col lq-w-full lq-h-full lq-items-center">
                <div class="lq-w-full">
                    <div class="lq-flex lq-items-center lq-gap-2 lq-p-1">
                        <Button
                            color=ColorOption::Red
                            on:click=move |_| {
                                let query = query.read_value();
                                let cache_key = query.cache_key;
                                let key_hash = query.key_hash;
                                if let Some(scope) = client
                                    .scope_lookup
                                    .scopes_mut()
                                    .get_mut(&cache_key)
                                {
                                    scope.invalidate_query(&key_hash);
                                }
                            }
                        >
                            Invalidate
                        </Button>
                    </div>
                    <div class="lq-text-sm lq-text-lq-foreground lq-p-1 lq-bg-lq-accent">
                        <div class="lq-flex lq-flex-row lq-items-center lq-gap-2">
                            <CaretSwitch open=details_open />
                            Details
                        </div>
                    </div>
                    <dl class=section_class>
                        <Show when=move || details_open.get()>
                            <div class=entry_class>
                                <dt class="lq-text-zinc-100 lq-font-bold lq-mr-2">"Status: "</dt>
                                <dd class="lq-text-zinc-200">
                                    <RowStateLabel query=query.get_value() />
                                </dd>
                            </div>
                            <div class=entry_class>
                                <dt class="lq-text-zinc-100 lq-font-bold lq-mr-2">
                                    "Created At: "
                                </dt>
                                <dd class="lq-text-zinc-200">{created_at.get_value()}</dd>
                            </div>
                            <div class=entry_class>
                                <dt class="lq-text-zinc-100 lq-font-bold lq-mr-2">
                                    "Last Update: "
                                </dt>
                                <dd class="lq-text-zinc-200">{last_update}</dd>
                            </div>
                            <div class=entry_class>
                                <dt class="lq-text-zinc-100 lq-font-bold lq-mr-2">
                                    "Active Resources: "
                                </dt>
                                <dd class="lq-text-zinc-200">
                                    {move || query.read_value().active_resources.get()}
                                </dd>
                            </div>

                            <div class=entry_class>
                                <dt class="lq-text-zinc-100 lq-font-bold lq-mr-2">
                                    "Stale Time: "
                                </dt>
                                <dd class="text-zinc-200">
                                    {move || {
                                        format!(
                                            "{}{}",
                                            format_duration(
                                                Some(query.read_value().combined_options.stale_time()),
                                            ),
                                            stale_in_str.get(),
                                        )
                                    }}
                                </dd>
                            </div>
                            <div class=entry_class>
                                <dt class="lq-text-zinc-100 lq-font-bold lq-mr-2">"GC Time: "</dt>
                                <dd class="lq-text-zinc-200">
                                    {move || {
                                        format!(
                                            "{}{}",
                                            format_duration(
                                                Some(query.read_value().combined_options.gc_time()),
                                            ),
                                            gc_in_str.get(),
                                        )
                                    }}
                                </dd>
                            </div>
                            <div class=entry_class>
                                <dt class="lq-text-zinc-100 lq-font-bold lq-mr-2">
                                    "Refetch Interval: "
                                </dt>
                                <dd class="lq-text-zinc-200">
                                    {move || {
                                        format!(
                                            "{}{}",
                                            format_duration(
                                                query.read_value().combined_options.refetch_interval(),
                                            ),
                                            refetch_in_str.get(),
                                        )
                                    }}
                                </dd>
                            </div>
                        </Show>
                    </dl>
                </div>
                <div class="lq-text-sm lq-text-lq-foreground lq-p-1 lq-bg-lq-accent lq-w-full">
                    <div class="lq-flex lq-flex-row lq-items-center lq-gap-2">
                        <CaretSwitch open=events_open />
                        Events
                    </div>
                </div>
                <div class="lq-pt-2 lq-w-full">
                    <Show when=move || events_open.get()>
                        <div class="lq-flex-1 lq-flex lq-w-full">
                            <div class="lq-mb-2 lq-flex-1 lq-p-4 lq-rounded-md lq-bg-zinc-800 lq-shadow-md lq-w-11/12 lq-text-xs lq-overflow-hidden">
                                <For
                                    each=move || query.read_value().events.get()
                                    key=|event| *event
                                    children=move |event| {
                                        view! {
                                            <p>
                                                <span class="lq-text-xs lq-font-semibold">
                                                    {format!("{}: ", event.recorded_at.format(TIME_FORMAT))}
                                                </span>
                                                {event.variant.to_string()}
                                            </p>
                                        }
                                    }
                                />
                            </div>
                        </div>
                    </Show>
                </div>

                <div class="lq-text-sm lq-text-lq-foreground lq-p-1 lq-bg-lq-accent lq-w-full">
                    Key
                </div>
                <div class="lq-flex-1 lq-flex lq-p-2 lq-w-full">
                    <div class="lq-flex-1 lq-p-4 lq-rounded-md lq-bg-zinc-800 lq-shadow-md lq-w-11/12 lq-text-xs lq-overflow-hidden">
                        <pre class="lq-whitespace-pre-wrap lq-break-words">
                            {move || { query.read_value().debug_key.pretty().to_string() }}
                        </pre>
                    </div>
                </div>
                <div class="lq-text-sm lq-text-lq-foreground lq-p-1 lq-bg-lq-accent lq-w-full">
                    Data
                </div>
                <div class="lq-flex-1 lq-flex lq-p-2 lq-w-full">
                    <div class="lq-flex-1 lq-p-4 lq-rounded-md lq-bg-zinc-800 lq-shadow-md lq-w-11/12 lq-text-xs lq-overflow-hidden">
                        <pre class="lq-whitespace-pre-wrap lq-break-words">
                            {move || {
                                query
                                    .read_value()
                                    .value_derivs
                                    .read()
                                    .debug_value
                                    .pretty()
                                    .to_string()
                            }}
                        </pre>
                    </div>
                </div>
            </div>
        </div>
    }
}
