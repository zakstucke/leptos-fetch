use std::{fmt::Debug, ops::Deref};

use leptos::prelude::{
    ArcSignal, DefinedAt, Get, LocalStorage, ReadUntracked, Signal, Track,
    guards::{Mapped, ReadGuard},
};
use send_wrapper::SendWrapper;

/// A local variant of an [`ArcSignal`], that will panic if accessed from a different thread.
///
/// Used for the [`QueryClient::subscribe_value_arc_local`] return type.
pub struct ArcLocalSignal<T: 'static>(ArcSignal<SendWrapper<T>>);

impl<T> Debug for ArcLocalSignal<T>
where
    T: Debug + 'static,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ArcLocalSignal").field(&self.0).finish()
    }
}

impl<T> Clone for ArcLocalSignal<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> ArcLocalSignal<T> {
    /// Like [`ArcSignal::derive`], but the value is not threadsafe and will panic if accessed from a different thread.
    pub fn derive_local(derive_fn: impl Fn() -> T + 'static) -> Self {
        let derive_fn = SendWrapper::new(derive_fn);
        Self(ArcSignal::derive(move || {
            let value = derive_fn();
            SendWrapper::new(value)
        }))
    }
}

impl<T> DefinedAt for ArcLocalSignal<T> {
    fn defined_at(&self) -> Option<&'static std::panic::Location<'static>> {
        self.0.defined_at()
    }
}

impl<T> ReadUntracked for ArcLocalSignal<T> {
    type Value = ReadGuard<T, Mapped<<ArcSignal<SendWrapper<T>> as ReadUntracked>::Value, T>>;

    fn try_read_untracked(&self) -> Option<Self::Value> {
        self.0
            .try_read_untracked()
            .map(|g| ReadGuard::new(Mapped::new_with_guard(g, |v| v.deref())))
    }
}

impl<T> Track for ArcLocalSignal<T> {
    fn track(&self) {
        self.0.track()
    }
}

impl<T: Clone> From<ArcLocalSignal<T>> for Signal<T, LocalStorage> {
    fn from(value: ArcLocalSignal<T>) -> Self {
        Signal::derive_local(move || value.get())
    }
}

#[cfg(test)]
mod tests {
    use std::marker::PhantomData;
    use std::ops::Deref;
    use std::ptr::NonNull;

    use super::*;
    use leptos::prelude::*;

    #[test]
    fn test_local_arc_signal() {
        #[derive(Debug)]
        struct UnsyncValue(u64, PhantomData<NonNull<()>>);
        impl PartialEq for UnsyncValue {
            fn eq(&self, other: &Self) -> bool {
                self.0 == other.0
            }
        }
        impl Eq for UnsyncValue {}
        impl Clone for UnsyncValue {
            fn clone(&self) -> Self {
                Self(self.0, PhantomData)
            }
        }
        impl UnsyncValue {
            fn new(value: u64) -> Self {
                Self(value, PhantomData)
            }
        }

        let signal = ArcLocalSignal::derive_local(|| UnsyncValue::new(42));
        assert_eq!(signal.get_untracked().0, 42);
        assert_eq!(signal.read_untracked().0, 42);

        // Should be no SendWrapper in public interface:
        let foo = signal.read_untracked().deref().clone();
        assert_eq!(foo, UnsyncValue::new(42));
    }
}
