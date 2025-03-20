use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::atomic::AtomicU64,
};

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
defined_id_gen!(new_subscription_id);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyHash(u64);

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
