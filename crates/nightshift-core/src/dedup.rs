use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::event::BatchedEvent;

pub trait DedupCache: Send + Sync {
    /// Returns true if this key was seen recently (duplicate). Registers the key if new.
    fn is_duplicate(&mut self, key: &str) -> bool;
}

pub struct InMemoryDedupCache {
    seen: HashMap<String, Instant>,
    ttl: Duration,
}

impl InMemoryDedupCache {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            seen: HashMap::new(),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    fn evict_expired(&mut self) {
        self.seen.retain(|_, inserted_at| inserted_at.elapsed() < self.ttl);
    }
}

impl DedupCache for InMemoryDedupCache {
    fn is_duplicate(&mut self, key: &str) -> bool {
        self.evict_expired();
        if self.seen.contains_key(key) {
            return true;
        }
        self.seen.insert(key.to_string(), Instant::now());
        false
    }
}

/// Derives a stable dedup key from an event.
/// Key = sessionId + event_type + event_name + timestamp rounded to the nearest second.
/// Rounding prevents duplicate detection across normal inter-event timing but catches
/// sendBeacon double-fires that occur within the same second.
pub fn dedup_key(event: &BatchedEvent) -> String {
    let rounded_ts = event.context.timestamp / 1000;
    format!(
        "{}:{:?}:{}:{}",
        event.context.session_id,
        event.event_type,
        event.event.as_deref().unwrap_or(""),
        rounded_ts,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{BatchedEvent, EventContext, EventType};

    fn make_event(session: &str, name: &str, ts_ms: i64) -> BatchedEvent {
        BatchedEvent {
            event_type: EventType::Track,
            event: Some(name.to_string()),
            user_id: None,
            properties: None,
            traits: None,
            error: None,
            context: EventContext {
                viewport: "1x1".to_string(),
                url: "/".to_string(),
                session_id: session.to_string(),
                app_version: "v1".to_string(),
                timestamp: ts_ms,
                ..Default::default()
            },
        }
    }

    #[test]
    fn first_event_is_not_duplicate() {
        let mut cache = InMemoryDedupCache::new(30);
        let event = make_event("sid1", "Click", 1000_000);
        assert!(!cache.is_duplicate(&dedup_key(&event)));
    }

    #[test]
    fn same_event_same_second_is_duplicate() {
        let mut cache = InMemoryDedupCache::new(30);
        let e1 = make_event("sid1", "Click", 1000_000);
        let e2 = make_event("sid1", "Click", 1000_500); // same second
        cache.is_duplicate(&dedup_key(&e1));
        assert!(cache.is_duplicate(&dedup_key(&e2)));
    }

    #[test]
    fn different_sessions_not_duplicate() {
        let mut cache = InMemoryDedupCache::new(30);
        let e1 = make_event("sid1", "Click", 1000_000);
        let e2 = make_event("sid2", "Click", 1000_000);
        cache.is_duplicate(&dedup_key(&e1));
        assert!(!cache.is_duplicate(&dedup_key(&e2)));
    }

    #[test]
    fn different_events_not_duplicate() {
        let mut cache = InMemoryDedupCache::new(30);
        let e1 = make_event("sid1", "Click", 1000_000);
        let e2 = make_event("sid1", "Purchase", 1000_000);
        cache.is_duplicate(&dedup_key(&e1));
        assert!(!cache.is_duplicate(&dedup_key(&e2)));
    }
}
