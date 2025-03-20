use std::time::Duration;

pub(crate) const DEFAULT_STALE_TIME: Duration = Duration::from_secs(10);
pub(crate) const DEFAULT_GC_TIME: Duration = Duration::from_secs(300);

/// Configuration to be used with [`crate::QueryClient`] and individual query types.
#[derive(Debug, Clone, Copy, Default)]
pub struct QueryOptions {
    stale_time: Option<Duration>,
    gc_time: Option<Duration>,
    refetch_interval: Option<Duration>,
}

impl QueryOptions {
    /// Create new [`QueryOptions`] with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the duration that should pass before a query is considered stale.
    ///
    /// Once stale, after any new interaction with the query, a new resource using it, declarative interactions etc, the query will be refetched in the background, and update active resources.
    ///
    /// To never mark as stale, set [`std::time::Duration::MAX`].
    ///
    /// Default: `10 seconds`
    #[track_caller]
    pub fn set_stale_time(mut self, stale_time: Duration) -> Self {
        if let Some(gc_time) = self.gc_time {
            // If stale_time is greater than gc_time, stale_time will be set to gc_time.
            if stale_time > gc_time {
                self.stale_time = Some(gc_time);
                return self;
            }
        }
        self.stale_time = Some(stale_time);
        self
    }

    /// Set the duration that should pass before an unused query is garbage collected.
    ///
    /// After this time, if the query isn't being used by any resources, the query will be removed from the cache, to minimise the cache's size. If the query is in active use, the gc will be scheduled to check again after the same time interval.
    ///
    /// To never garbage collect, set [`std::time::Duration::MAX`].
    ///
    /// Default: `5 minutes`
    #[track_caller]
    pub fn set_gc_time(mut self, gc_time: Duration) -> Self {
        if let Some(stale_time) = self.stale_time {
            if stale_time > gc_time {
                // If stale_time is greater than gc_time, stale_time will be set to gc_time.
                self.stale_time = Some(gc_time);
                return self;
            }
        }
        self.gc_time = Some(gc_time);
        self
    }

    /// Set the interval after which to automatically refetch the query if there are any active resources.
    ///
    /// If the query is being used by any resources, it will be invalidated and refetched in the background, updating active resources according to this interval.
    ///
    /// Default: No refetching
    #[track_caller]
    pub fn set_refetch_interval(mut self, refetch_interval: Duration) -> Self {
        self.refetch_interval = Some(refetch_interval);
        self
    }

    /// The duration that should pass before a query is considered stale.
    ///
    /// Once stale, after any new interaction with the query, a new resource using it, declarative interactions etc, the query will be refetched in the background, and update active resources.
    ///
    /// Default: `10 seconds`
    pub fn stale_time(&self) -> Duration {
        self.stale_time.unwrap_or(DEFAULT_STALE_TIME)
    }

    /// The duration that should pass before an unused query is garbage collected.
    ///
    /// After this time, if the query isn't being used by any resources, the query will be removed from the cache, to minimise the cache's size. If the query is in active use, the gc will be scheduled to check again after the same time interval.
    ///
    /// Default: `5 minutes`
    pub fn gc_time(&self) -> Duration {
        self.gc_time.unwrap_or(DEFAULT_GC_TIME)
    }

    /// The interval (if any) after which to automatically refetch the query if there are any active resources.
    ///
    /// If the query is being used by any resources, it will be invalidated and refetched in the background, updating active resources according to this interval.
    ///
    /// Default: No refetching
    pub fn refetch_interval(&self) -> Option<Duration> {
        self.refetch_interval
    }
}

pub(crate) fn options_combine(base: QueryOptions, scope: Option<QueryOptions>) -> QueryOptions {
    if let Some(scope) = scope {
        QueryOptions {
            stale_time: scope.stale_time.or(base.stale_time),
            gc_time: scope.gc_time.or(base.gc_time),
            refetch_interval: scope.refetch_interval.or(base.refetch_interval),
        }
    } else {
        base
    }
}
