use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::atomic::AtomicU64,
};

use leptos::prelude::TimeoutHandle;

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
pub fn safe_set_timeout(
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
