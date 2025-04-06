use std::{
    fmt::Debug,
    ops::{Deref, DerefMut},
};

use send_wrapper::SendWrapper;

enum Inner<V> {
    Local {
        value: SendWrapper<V>,
        src_thread_id: std::thread::ThreadId,
    },
    Threadsafe(V),
}

pub(crate) struct MaybeLocal<V>(Inner<V>);

// SAFETY: `MaybeLocal` can *only* be given a T in three ways
// 1) via new(), which requires T: Send + Sync
// 2) via new_local(), which wraps T in a SendWrapper
// 3) via deref_mut(), which provides access to &mut T, either already in a SendWrapper, or not if it was already determined T is a threadsafe type
unsafe impl<V> Send for MaybeLocal<V> {}
unsafe impl<V> Sync for MaybeLocal<V> {}

impl<V> Clone for MaybeLocal<V>
where
    V: Clone,
{
    fn clone(&self) -> Self {
        match &self.0 {
            Inner::Local {
                value,
                src_thread_id,
            } => Self(Inner::Local {
                value: value.clone(),
                src_thread_id: *src_thread_id,
            }),
            Inner::Threadsafe(value) => Self(Inner::Threadsafe(value.clone())),
        }
    }
}

impl<V> Debug for MaybeLocal<V>
where
    V: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Inner::Local { .. } => {
                if let Some(value) = self.value_if_safe() {
                    f.debug_tuple("Local").field(value).finish()
                } else {
                    f.debug_tuple("Local")
                        .field(&"cannot access variable from this thread")
                        .finish()
                }
            }
            Inner::Threadsafe(value) => f.debug_tuple("Threadsafe").field(value).finish(),
        }
    }
}

impl<V> MaybeLocal<V>
where
    V: Send + Sync,
{
    /// Create a new threadsafe value.
    pub fn new(value: V) -> Self {
        Self(Inner::Threadsafe(value))
    }
}

impl<V> MaybeLocal<V> {
    /// Wraps `value` in a [`SendWrapper`] to make threadsafe. Access to this value will panic if called from a different thread.
    pub fn new_local(value: V) -> Self {
        Self(Inner::Local {
            value: SendWrapper::new(value),
            src_thread_id: std::thread::current().id(),
        })
    }

    /// Panics if [`Inner::Local`] and called from a different thread.
    #[track_caller]
    pub fn value_may_panic(&self) -> &V {
        match &self.0 {
            Inner::Local { value, .. } => {
                // Will panic itself if called from a different thread:
                value.deref()
            }
            Inner::Threadsafe(value) => value,
        }
    }

    /// Panics if [`Inner::Local`] and called from a different thread.
    #[track_caller]
    pub fn value_mut_may_panic(&mut self) -> &mut V {
        match &mut self.0 {
            Inner::Local { value, .. } => {
                // Will panic itself if called from a different thread:
                value.deref_mut()
            }
            Inner::Threadsafe(value) => value,
        }
    }

    #[track_caller]
    pub fn value_if_safe(&self) -> Option<&V> {
        match &self.0 {
            Inner::Local {
                value,
                src_thread_id,
            } => {
                if std::thread::current().id() == *src_thread_id {
                    Some(value.deref())
                } else {
                    None
                }
            }
            Inner::Threadsafe(value) => Some(value),
        }
    }

    #[allow(dead_code)]
    #[track_caller]
    pub fn value_mut_value_if_safe(&mut self) -> Option<&mut V> {
        match &mut self.0 {
            Inner::Local {
                value,
                src_thread_id,
            } => {
                if std::thread::current().id() == *src_thread_id {
                    Some(value.deref_mut())
                } else {
                    None
                }
            }
            Inner::Threadsafe(value) => Some(value),
        }
    }

    /// true when [`Inner::Local`]
    pub fn is_local(&self) -> bool {
        matches!(self.0, Inner::Local { .. })
    }
}
