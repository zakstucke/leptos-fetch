use std::ops::{Deref, DerefMut};

use send_wrapper::SendWrapper;

enum Inner<V> {
    Local(SendWrapper<V>),
    Threadsafe(V),
}

pub(crate) struct MaybeLocal<V>(Inner<V>);

unsafe impl<V> Send for MaybeLocal<V> {}
unsafe impl<V> Sync for MaybeLocal<V> {}

impl<V> MaybeLocal<V>
where
    V: Send + Sync,
{
    pub fn new(value: V) -> Self {
        Self(Inner::Threadsafe(value))
    }
}

impl<V> MaybeLocal<V> {
    pub fn new_local(value: V) -> Self {
        Self(Inner::Local(SendWrapper::new(value)))
    }

    #[track_caller]
    pub fn value(&self) -> &V {
        match &self.0 {
            Inner::Local(wrapper) => wrapper.deref(),
            Inner::Threadsafe(value) => value,
        }
    }

    #[track_caller]
    pub fn value_mut(&mut self) -> &mut V {
        match &mut self.0 {
            Inner::Local(wrapper) => wrapper.deref_mut(),
            Inner::Threadsafe(value) => value,
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(self.0, Inner::Local(_))
    }
}
