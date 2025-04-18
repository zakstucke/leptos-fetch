pub trait QueryMaybeKey<K, V> {
    type MappedValue;

    fn into_maybe_key(self) -> Option<K>;

    fn prepare_mapped_value(v: Option<V>) -> Self::MappedValue;

    fn mapped_to_maybe_value(v: Self::MappedValue) -> Option<V>;

    fn mapped_value_is_some(v: &Self::MappedValue) -> bool;
}

impl<K, V> QueryMaybeKey<K, V> for K {
    type MappedValue = V;

    fn into_maybe_key(self) -> Option<K> {
        Some(self)
    }

    fn prepare_mapped_value(v: Option<V>) -> V {
        v.expect(
            "QueryMaybeKey::prepare_mapped_value: value is None when the key was always available, this is a bug"
        )
    }

    fn mapped_to_maybe_value(v: Self::MappedValue) -> Option<V> {
        Some(v)
    }

    fn mapped_value_is_some(_v: &Self::MappedValue) -> bool {
        true
    }
}

impl<K, V> QueryMaybeKey<K, V> for Option<K> {
    type MappedValue = Option<V>;

    fn into_maybe_key(self) -> Option<K> {
        self
    }

    fn prepare_mapped_value(v: Option<V>) -> Option<V> {
        v
    }

    fn mapped_to_maybe_value(v: Self::MappedValue) -> Option<V> {
        v
    }

    fn mapped_value_is_some(v: &Self::MappedValue) -> bool {
        v.is_some()
    }
}
