use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::{Arc, atomic::AtomicU64},
};

use leptos::prelude::{Owner, ScopedFuture, TimeoutHandle, untrack};

use crate::{
    UntypedQueryClient, no_reactive_diagnostics_future::NoReactiveDiagnosticsFuture,
    query_scope::ScopeCacheKey,
};

pub(crate) fn provide_cb_contexts(
    untyped_client: UntypedQueryClient,
    scope_cache_key: ScopeCacheKey,
) {
    leptos::context::provide_context(untyped_client);
    leptos::context::provide_context(scope_cache_key);
}

// The context which on_gc, on_invalidation etc should run in.
pub(crate) fn run_external_callbacks(
    untyped_client: UntypedQueryClient,
    scope_cache_key: ScopeCacheKey,
    callbacks: Vec<Box<dyn FnOnce()>>,
) {
    let maybe_parent_owner = Owner::current();
    let owner = match maybe_parent_owner.as_ref() {
        Some(o) => o.child(),
        None => Owner::default(),
    };
    owner.with(|| {
        provide_cb_contexts(untyped_client, scope_cache_key);
        for cb in callbacks {
            cb();
        }
    });
    if let Some(parent) = maybe_parent_owner {
        parent.set();
    }
}

macro_rules! defined_id_gen {
    ($name:ident) => {
        pub(crate) fn $name() -> u64 {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        }
    };
}

defined_id_gen!(new_resource_id);
defined_id_gen!(new_scope_id);
defined_id_gen!(new_buster_id);
defined_id_gen!(new_sub_listener_id);
defined_id_gen!(new_value_modified_id);
#[cfg(any(
    all(debug_assertions, feature = "devtools"),
    feature = "devtools-always"
))]
defined_id_gen!(new_subscription_id);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyHash(u64);

impl KeyHash {
    pub fn new<K: Hash>(key: &K) -> Self {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        Self(hasher.finish())
    }
}

impl Hash for KeyHash {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

#[cfg(any(
    all(debug_assertions, feature = "devtools"),
    feature = "devtools-always"
))]
#[derive(Clone, Debug)]
pub(crate) struct DebugValue {
    pretty: std::sync::Arc<String>,
    compact: std::sync::Arc<String>,
}

#[cfg(any(
    all(debug_assertions, feature = "devtools"),
    feature = "devtools-always"
))]
impl DebugValue {
    pub fn new<T: std::fmt::Debug>(value: &T) -> Self {
        Self {
            pretty: std::sync::Arc::new(format!("{value:#?}")),
            compact: std::sync::Arc::new(format!("{value:?}")),
        }
    }

    pub fn pretty(&self) -> &std::sync::Arc<String> {
        &self.pretty
    }

    pub fn compact(&self) -> &std::sync::Arc<String> {
        &self.compact
    }
}

pub(crate) struct OnDrop<F>
where
    F: FnOnce(),
{
    f: Option<F>,
}

impl<F> OnDrop<F>
where
    F: FnOnce(),
{
    pub fn new(f: F) -> Self {
        Self { f: Some(f) }
    }
}

impl<F> Drop for OnDrop<F>
where
    F: FnOnce(),
{
    fn drop(&mut self) {
        if let Some(f) = self.f.take() {
            f();
        }
    }
}

pub(crate) enum ResetInvalidated {
    Reset,
    NoReset,
}

#[allow(dead_code)]
/// Works around potential panic reported in https://github.com/zakstucke/leptos-fetch/issues/43
/// until my internal fix is upstreamed into leptos (https://github.com/leptos-rs/leptos/pull/4212)
#[track_caller]
pub(crate) fn safe_set_timeout(
    cb: impl FnOnce() + 'static,
    duration: std::time::Duration,
) -> TimeoutHandle {
    leptos::prelude::set_timeout_with_handle(
        cb,
        if duration.as_millis() > i32::MAX as _ {
            std::time::Duration::from_millis(i32::MAX as _)
        } else {
            duration
        },
    )
    .expect("leptos::prelude::set_timeout_with_handle() failed to spawn")
}

/// An owned ancestry chain of owners, preventing anything dropping until this is dropped.
#[derive(Clone)]
pub(crate) struct OwnerChain(Arc<Vec<Owner>>);

/// Accepts None to make the method usage usable even when no owner exists.
/// Will run the query in a fresh child owner.
/// The owner will contain the context of the current client.
impl OwnerChain {
    pub fn new(
        untyped_client: UntypedQueryClient,
        scope_cache_key: ScopeCacheKey,
        owner: Option<Owner>,
    ) -> Self {
        let active_owner = match &owner {
            Some(o) => o.child(),
            None => Owner::default(),
        };
        active_owner.with(|| {
            provide_cb_contexts(untyped_client, scope_cache_key);
        });
        if let Some(parent) = owner.as_ref() {
            parent.set();
        }

        let mut owners = vec![active_owner];
        let mut next_owner = owner;
        while let Some(o) = next_owner {
            next_owner = o.parent();
            owners.push(o);
        }
        Self(Arc::new(owners))
    }

    fn active_owner(&self) -> Option<&Owner> {
        self.0.first()
    }

    // Run the async future, and initial function creating it, with the active owner if there is one.
    pub async fn with<T, Fut>(&self, f: impl FnOnce() -> Fut) -> T
    where
        Fut: Future<Output = T>,
    {
        if let Some(owner) = self.active_owner() {
            // Explicitly disabling diagnostics and removing observer to prevent "leak through" reactivity in local resources
            owner
                .with(|| ScopedFuture::new_untracked(NoReactiveDiagnosticsFuture::new(untrack(f))))
                .await
        } else {
            f().await
        }
    }
}
