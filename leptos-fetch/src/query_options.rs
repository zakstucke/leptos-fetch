use std::time::Duration;

pub(crate) const DEFAULT_STALE_TIME: Duration = Duration::from_secs(10);
pub(crate) const DEFAULT_GC_TIME: Duration = Duration::from_secs(300);

/**
 * Options for a query [`use_query()`](crate::use_query())
 */
#[derive(Debug, Clone, Copy, Default)]
pub struct QueryOptions {
    stale_time: Option<Duration>,
    gc_time: Option<Duration>,
}

impl QueryOptions {
    /// Create a new set of query options
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the time after which a query is considered stale
    #[track_caller]
    pub fn set_stale_time(mut self, stale_time: Duration) -> Self {
        if let Some(gc_time) = self.gc_time {
            if stale_time > gc_time {
                panic!("stale_time must be less than gc_time");
            }
        }
        self.stale_time = Some(stale_time);
        self
    }

    /// Set the time after which a query is garbage collected
    #[track_caller]
    pub fn set_gc_time(mut self, gc_time: Duration) -> Self {
        if let Some(stale_time) = self.stale_time {
            if gc_time < stale_time {
                panic!("gc_time must be greater than stale_time");
            }
        }
        self.gc_time = Some(gc_time);
        self
    }

    /// Get the time after which a query is considered stale
    pub fn stale_time(&self) -> Duration {
        self.stale_time.unwrap_or(DEFAULT_STALE_TIME)
    }

    /// Get the time after which a query is garbage collected
    pub fn gc_time(&self) -> Duration {
        self.gc_time.unwrap_or(DEFAULT_GC_TIME)
    }
}

pub(crate) fn options_combine(base: QueryOptions, scope: Option<QueryOptions>) -> QueryOptions {
    if let Some(scope) = scope {
        QueryOptions {
            stale_time: scope.stale_time.or(base.stale_time),
            gc_time: scope.gc_time.or(base.gc_time),
        }
    } else {
        base
    }
}
