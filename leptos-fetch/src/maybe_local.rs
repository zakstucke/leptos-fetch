use std::ops::{Deref, DerefMut};

use send_wrapper::SendWrapper;

enum Inner<V> {
    Local(SendWrapper<V>),
    Threadsafe(V),
}

pub(crate) struct MaybeLocal<V>(Inner<V>);

// SAFETY: `MaybeLocal` can *only* be given a T in three ways
// 1) via new(), which requires T: Send + Sync
// 2) via new_local(), which wraps T in a SendWrapper
// 3) via deref_mut(), which provides access to &mut T, either already ina SendWrapper, or not if it was already determined T is a threadsafe type
unsafe impl<V> Send for MaybeLocal<V> {}
unsafe impl<V> Sync for MaybeLocal<V> {}

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
        Self(Inner::Local(SendWrapper::new(value)))
    }

    /// Panics if [`Inner::Local`] and called from a different thread.
    #[track_caller]
    pub fn value(&self) -> &V {
        match &self.0 {
            Inner::Local(wrapper) => wrapper.deref(),
            Inner::Threadsafe(value) => value,
        }
    }

    /// Panics if [`Inner::Local`] and called from a different thread.
    #[track_caller]
    pub fn value_mut(&mut self) -> &mut V {
        match &mut self.0 {
            Inner::Local(wrapper) => wrapper.deref_mut(),
            Inner::Threadsafe(value) => value,
        }
    }

    /// true when [`Inner::Local`]
    pub fn is_local(&self) -> bool {
        matches!(self.0, Inner::Local(_))
    }
}
