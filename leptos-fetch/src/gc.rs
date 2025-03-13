use std::{sync::Arc, time::Duration};

use leptos::prelude::TimeoutHandle;
use send_wrapper::SendWrapper;

pub(crate) struct GcValue<V> {
    value: Option<V>, // Only None temporarily after into_value() before drop()
    gc_handle: GcHandle,
}

impl<V> GcValue<V> {
    pub fn new(value: V, gc_handle: GcHandle) -> Self {
        Self {
            value: Some(value),
            gc_handle,
        }
    }

    /// Consumes and returns the value, cancelling the gc cleanup timeout.
    pub fn into_value(mut self) -> V {
        self.gc_handle.cancel();
        self.value.take().expect("value already taken, bug")
    }

    pub fn value(&self) -> &V {
        self.value.as_ref().expect("value already taken, bug")
    }
}

/// Cancel the gc cleanup timeout if the value is dropped for any reason, e.g. invalidation or replacement with something new.
impl<V> Drop for GcValue<V> {
    fn drop(&mut self) {
        self.gc_handle.cancel();
    }
}

#[derive(Debug)]
pub(crate) enum GcHandle {
    None,
    #[allow(dead_code)]
    Wasm(TimeoutHandle),
    #[cfg(all(test, not(target_arch = "wasm32")))]
    #[allow(dead_code)]
    Tokio(tokio::task::JoinHandle<()>),
}

impl GcHandle {
    pub fn new(gc_cb: Option<Arc<SendWrapper<Box<dyn Fn()>>>>, duration: Duration) -> Self {
        if let Some(gc_cb) = gc_cb {
            #[cfg(any(not(test), target_arch = "wasm32"))]
            {
                let handle = leptos::prelude::set_timeout_with_handle(move || gc_cb(), duration)
                    .expect("leptos::prelude::set_timeout_with_handle() failed to spawn");
                GcHandle::Wasm(handle)
            }
            #[cfg(all(test, not(target_arch = "wasm32")))]
            {
                // Just for testing, tokio tests are single threaded so SendWrapper is fine:
                let handle = tokio::task::spawn(SendWrapper::new(async move {
                    tokio::time::sleep(duration).await;
                    gc_cb();
                }));
                GcHandle::Tokio(handle)
            }
        } else {
            Self::None
        }
    }

    fn cancel(&mut self) {
        match self {
            GcHandle::None => {}
            GcHandle::Wasm(handle) => handle.clear(),
            #[cfg(all(test, not(target_arch = "wasm32")))]
            GcHandle::Tokio(handle) => handle.abort(),
        }
        *self = GcHandle::None;
    }
}
