use std::sync::atomic::AtomicU64;

pub(crate) fn random_u64_rolling() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}
