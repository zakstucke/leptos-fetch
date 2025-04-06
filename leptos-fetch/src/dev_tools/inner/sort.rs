use std::borrow::Cow;

use chrono::Utc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SortConfig {
    pub(crate) sort: SortOption,
    pub(crate) order_asc: bool,
}

impl SortConfig {
    pub fn new_default_updated_at() -> Self {
        Self {
            sort: SortOption::UpdatedAt,
            order_asc: false,
        }
    }

    pub fn new_default_ascii() -> Self {
        Self {
            sort: SortOption::Ascii,
            order_asc: false,
        }
    }

    pub fn sort<T>(
        &self,
        items: &mut [T],
        get_ascii: impl Fn(&T) -> Vec<Cow<str>>,
        get_updated_at: impl Fn(&T) -> chrono::DateTime<Utc>,
        get_created_at: impl Fn(&T) -> chrono::DateTime<Utc>,
    ) {
        match self.sort {
            SortOption::Ascii => items.sort_by(|a, b| {
                get_ascii(a).iter().map(|s| s.to_lowercase()).cmp(
                    get_ascii(b)
                        .iter()
                        .map(|s| s.to_lowercase())
                        .collect::<Vec<_>>(),
                )
            }),
            SortOption::UpdatedAt => items.sort_by_key(get_updated_at),
            SortOption::CreatedAt => items.sort_by_key(get_created_at),
        };
        if !self.order_asc {
            items.reverse();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SortOption {
    UpdatedAt,
    CreatedAt,
    Ascii,
}

impl SortOption {
    pub fn as_str(&self) -> &str {
        match self {
            SortOption::UpdatedAt => "UpdatedAt",
            SortOption::CreatedAt => "CreatedAt",
            SortOption::Ascii => "Ascii",
        }
    }

    pub fn from_string(s: &str) -> Self {
        match s {
            "Ascii" => SortOption::Ascii,
            "UpdatedAt" => SortOption::UpdatedAt,
            "CreatedAt" => SortOption::CreatedAt,
            _ => SortOption::UpdatedAt,
        }
    }
}

pub(crate) fn filter_s<'a, T>(
    raw_filter: &str,
    extra_strs_to_match: &[&str],
    to_str: impl Fn(&T) -> Cow<str> + 'a,
    items: impl IntoIterator<Item = T> + 'a,
) -> impl IntoIterator<Item = T> + 'a {
    let extra_strs_to_match = extra_strs_to_match
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let raw_filter = raw_filter.trim().to_ascii_lowercase();
    let mut filters = raw_filter
        .split_whitespace()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();

    // Making separate words additive filters, all must pass between the extras and the items:
    filters.retain(|filter| {
        !filter.is_empty() && !extra_strs_to_match.iter().any(|s| s.contains(filter))
    });

    items.into_iter().filter(move |item| {
        filters.is_empty() || {
            let s = to_str(item).to_ascii_lowercase();
            filters.iter().all(|filter| s.contains(filter))
        }
    })
}
