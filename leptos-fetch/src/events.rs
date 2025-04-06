use core::fmt;
use std::{any::TypeId, ops::Deref};

use crate::{cache::ScopeLookup, utils::KeyHash};

#[derive(Debug)]
pub(crate) struct Events {
    scope_lookup: ScopeLookup,
    cache_key: TypeId,
    key_hash: KeyHash,
    events: Vec<Event>,
}

impl Deref for Events {
    type Target = Vec<Event>;

    fn deref(&self) -> &Self::Target {
        &self.events
    }
}

impl Events {
    pub fn new(
        scope_lookup: &ScopeLookup,
        cache_key: TypeId,
        key_hash: KeyHash,
        events: Vec<Event>,
    ) -> Self {
        let mut self_ = Self {
            scope_lookup: *scope_lookup,
            cache_key,
            key_hash,
            events: vec![],
        };
        // So notifications are triggered on any initial events use our custom extend method:
        self_.extend(events);
        self_
    }

    pub fn push(&mut self, event: Event) {
        self.extend(std::iter::once(event));
    }

    pub fn extend(&mut self, events: impl IntoIterator<Item = Event>) {
        let mut iter = events.into_iter();
        if let Some(first) = iter.next() {
            self.events.extend(std::iter::once(first).chain(iter));
            self.scope_lookup
                .scope_subscriptions_mut()
                .notify_events_updated(self.cache_key, self.key_hash, self.events.deref());
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Event {
    pub recorded_at: chrono::DateTime<chrono::Utc>,
    pub variant: EventVariant,
}

impl Event {
    pub fn new(variant: EventVariant) -> Self {
        Self {
            recorded_at: chrono::Utc::now(),
            variant,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum EventVariant {
    Fetched { elapsed_ms: i64 },
    DeclarativeUpdate,
    DeclarativeSet,
    StreamedFromServer,
    Invalidated,
    RefetchTriggeredViaInvalidation,
}

impl fmt::Display for EventVariant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventVariant::Fetched { elapsed_ms } => write!(f, "fetched in {}ms", elapsed_ms),
            EventVariant::DeclarativeUpdate => write!(f, "declarative update"),
            EventVariant::DeclarativeSet => write!(f, "declarative set"),
            EventVariant::StreamedFromServer => write!(f, "streamed from server"),
            EventVariant::Invalidated => write!(f, "invalidated"),
            EventVariant::RefetchTriggeredViaInvalidation => {
                write!(f, "refetch triggered via invalidation")
            }
        }
    }
}
