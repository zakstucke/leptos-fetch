use std::{sync::Arc, time::Duration};

use leptos::prelude::TimeoutHandle;
use parking_lot::Mutex;
use send_wrapper::SendWrapper;

use crate::maybe_local::MaybeLocal;

pub(crate) struct GcValue<V> {
    value: Option<MaybeLocal<V>>, // Only None temporarily after into_value() before drop()
    gc_handle: GcHandle,
    refetch_handle: RefetchHandle,
}

impl<V> GcValue<V> {
    pub fn new(value: MaybeLocal<V>, gc_handle: GcHandle, refetch_handle: RefetchHandle) -> Self {
        Self {
            value: Some(value),
            gc_handle,
            refetch_handle,
        }
    }

    /// Reset after updating the value:
    pub fn reset_callbacks(&mut self, gc_handle: GcHandle, refetch_handle: RefetchHandle) {
        self.gc_handle.cancel();
        self.refetch_handle.cancel();
        self.gc_handle = gc_handle;
        self.refetch_handle = refetch_handle;
    }

    #[track_caller]
    pub fn value(&self) -> &MaybeLocal<V> {
        self.value.as_ref().expect("value already taken, bug")
    }

    #[track_caller]
    pub fn value_mut(&mut self) -> &mut MaybeLocal<V> {
        self.value.as_mut().expect("value already taken, bug")
    }
}

/// Cancel the callbacks if the value is dropped for any reason, e.g. invalidation or replacement with something new.
impl<V> Drop for GcValue<V> {
    fn drop(&mut self) {
        self.gc_handle.cancel();
        self.refetch_handle.cancel();
    }
}

#[derive(Debug)]
pub(crate) enum GcHandle {
    None,
    #[allow(dead_code)]
    Wasm(Arc<Mutex<Option<TimeoutHandle>>>),
    #[cfg(all(test, not(target_arch = "wasm32")))]
    #[allow(dead_code)]
    Tokio(tokio::task::JoinHandle<()>),
}

impl GcHandle {
    // gc_cb returns true if gc happened, false if should call again after same delay.
    pub fn new(gc_cb: Option<Arc<SendWrapper<Box<dyn Fn() -> bool>>>>, duration: Duration) -> Self {
        if let Some(gc_cb) = gc_cb {
            #[cfg(any(not(test), target_arch = "wasm32"))]
            {
                use crate::utils::safe_set_timeout;

                let handle = Arc::new(Mutex::new(None));
                fn call(
                    handle: Arc<Mutex<Option<TimeoutHandle>>>,
                    gc_cb: impl Fn() -> bool + 'static,
                    duration: Duration,
                ) -> TimeoutHandle {
                    safe_set_timeout(
                        move || {
                            let gced = gc_cb();
                            if !gced {
                                let handle_clone = handle.clone();
                                *handle.lock() = Some(call(handle_clone, gc_cb, duration));
                            }
                        },
                        duration,
                    )
                }

                let handle_clone = handle.clone();
                *handle.lock() = Some(call(handle_clone, move || gc_cb(), duration));
                GcHandle::Wasm(handle)
            }
            #[cfg(all(test, not(target_arch = "wasm32")))]
            {
                // Just for testing, tokio tests are single threaded so SendWrapper is fine:
                // (because not sure why but spawn_local hangs.)
                let handle = tokio::task::spawn(SendWrapper::new(async move {
                    tokio::time::sleep(duration).await;
                    while !gc_cb() {
                        tokio::time::sleep(duration).await;
                    }
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
            GcHandle::Wasm(handle) => {
                if let Some(handle) = handle.lock().take() {
                    handle.clear();
                }
            }
            #[cfg(all(test, not(target_arch = "wasm32")))]
            GcHandle::Tokio(handle) => handle.abort(),
        }
        *self = GcHandle::None;
    }
}

pub(crate) enum RefetchCbResult {
    Ok,
    PostponedWhilstRefetchDisabled,
}

#[derive(Debug)]
pub(crate) enum RefetchHandle {
    None,
    #[allow(dead_code)]
    Wasm(Arc<Mutex<Option<TimeoutHandle>>>),
    #[cfg(all(test, not(target_arch = "wasm32")))]
    #[allow(dead_code)]
    Tokio(tokio::task::JoinHandle<()>),
}

impl RefetchHandle {
    pub fn new(
        refetch_cb: Option<Arc<SendWrapper<Box<dyn Fn() -> RefetchCbResult>>>>,
        duration: Option<Duration>,
    ) -> Self {
        if let Some(refetch_cb) = refetch_cb {
            let duration = duration.expect("refetch_cb is Some but duration is None (bug)");

            #[cfg(any(not(test), target_arch = "wasm32"))]
            {
                use crate::utils::safe_set_timeout;

                let handle = Arc::new(Mutex::new(None));
                fn call(
                    handle: Arc<Mutex<Option<TimeoutHandle>>>,
                    refetch_cb: Arc<SendWrapper<Box<dyn Fn() -> RefetchCbResult>>>,
                    duration: Duration,
                ) -> TimeoutHandle {
                    safe_set_timeout(
                        move || match refetch_cb() {
                            RefetchCbResult::Ok => {}
                            RefetchCbResult::PostponedWhilstRefetchDisabled => {
                                let handle_clone = handle.clone();
                                *handle.lock() = Some(call(handle_clone, refetch_cb, duration));
                            }
                        },
                        duration,
                    )
                }
                let handle_clone = handle.clone();
                *handle.lock() = Some(call(handle_clone, refetch_cb, duration));
                RefetchHandle::Wasm(handle)
            }
            #[cfg(all(test, not(target_arch = "wasm32")))]
            {
                // Just for testing, tokio tests are single threaded so SendWrapper is fine:
                // (because not sure why but spawn_local hangs.)
                let handle = tokio::task::spawn(SendWrapper::new(async move {
                    loop {
                        tokio::time::sleep(duration).await;
                        match refetch_cb() {
                            RefetchCbResult::Ok => break,
                            RefetchCbResult::PostponedWhilstRefetchDisabled => {}
                        }
                    }
                }));
                RefetchHandle::Tokio(handle)
            }
        } else {
            Self::None
        }
    }

    fn cancel(&mut self) {
        match self {
            RefetchHandle::None => {}
            RefetchHandle::Wasm(handle) => {
                if let Some(handle) = handle.lock().take() {
                    handle.clear();
                }
            }
            #[cfg(all(test, not(target_arch = "wasm32")))]
            RefetchHandle::Tokio(handle) => handle.abort(),
        }
        *self = RefetchHandle::None;
    }
}
