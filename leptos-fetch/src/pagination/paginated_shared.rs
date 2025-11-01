/// The key for a page of a paginated query scope.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct PaginatedPageKey<Key> {
    /// The actual query key, this should be the same across all pages.
    pub key: Key,
    /// The active page index, starting from 0.
    pub page_index: usize,
    /// The active page size, the maximum number of items per page.
    /// The getter will continue to internally query until this many items are available, or there are no more items.
    pub page_size: usize,
}
