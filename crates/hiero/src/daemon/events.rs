//! The admin event hub (port of Python `daemon_events.AdminEventHub`,
//! extended to the spec envelope): every admin websocket message is
//! `{version, event_id, event_type, payload}` with a daemon-lifetime
//! monotonic `event_id` starting at 1 and the fixed protocol
//! [`EVENT_PROTOCOL_VERSION`] (`1`, the frozen fixture value).
//!
//! The hub keeps a bounded retention ring of recent events so reconnecting
//! clients can resume from their last event id. Resuming from a point older
//! than the retained window delivers a `snapshot_refresh` instruction
//! instead of a partial replay — lag never silently produces a partial
//! admin view. Subscriber callbacks run inline, exactly like Python; a
//! subscriber that reports failure (Python: raises) is dropped without
//! affecting the others.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

/// The wire protocol version of admin events (frozen fixture: `"version": 1`).
pub(crate) const EVENT_PROTOCOL_VERSION: u64 = 1;

/// Recent events retained for resume; the ring is bounded, so resuming from
/// an id older than this window falls back to a snapshot refresh.
const RETAINED_EVENTS: usize = 256;

/// Event type of the lag fallback instruction.
pub(crate) const SNAPSHOT_REFRESH_EVENT: &str = "snapshot_refresh";
const SNAPSHOT_REFRESH_REASON: &str = "last event id is outside retained history";

/// A published admin event.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AdminEvent {
    pub version: u64,
    pub id: u64,
    pub event_type: String,
    pub payload: Value,
}

impl AdminEvent {
    /// The frozen wire envelope: `version`, `event_id`, `event_type`,
    /// `payload`.
    pub(crate) fn to_wire(&self) -> Value {
        json!({
            "version": self.version,
            "event_id": self.id,
            "event_type": self.event_type,
            "payload": self.payload,
        })
    }
}

/// A callback receiving every event after registration. Returns `false` to
/// signal failure (a dead websocket, for example); the hub then drops the
/// subscriber, matching Python's remove-on-exception behavior.
pub(crate) type Subscriber = Arc<dyn Fn(&AdminEvent) -> bool + Send + Sync>;

#[derive(Default)]
struct Inner {
    next_id: u64,
    next_token: u64,
    retained: VecDeque<AdminEvent>,
    subscribers: Vec<(u64, Subscriber)>,
}

/// The in-memory admin event hub; state lives exactly as long as the daemon.
#[derive(Default)]
pub(crate) struct AdminEventHub {
    inner: Mutex<Inner>,
}

impl std::fmt::Debug for AdminEventHub {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.lock();
        formatter
            .debug_struct("AdminEventHub")
            .field("next_id", &inner.next_id)
            .field("retained", &inner.retained.len())
            .field("subscribers", &inner.subscribers.len())
            .finish()
    }
}

impl AdminEventHub {
    /// Publish an event: assign the next monotonic id, retain it, and deliver
    /// it to every live subscriber (a snapshot of the list, so delivery never
    /// blocks publishing; failing subscribers are dropped).
    pub fn publish(&self, event_type: &str, payload: Value) {
        let (event, subscribers) = {
            let mut inner = self.lock();
            inner.next_id += 1;
            let event = AdminEvent {
                version: EVENT_PROTOCOL_VERSION,
                id: inner.next_id,
                event_type: event_type.to_string(),
                payload,
            };
            inner.retained.push_back(event.clone());
            while inner.retained.len() > RETAINED_EVENTS {
                inner.retained.pop_front();
            }
            (event, inner.subscribers.clone())
        };
        for (token, subscriber) in subscribers {
            if !subscriber(&event) {
                self.unsubscribe(token);
            }
        }
    }

    /// Register a subscriber; the returned token unsubscribes it. Callers
    /// must unsubscribe on disconnect — and the hub also self-heals by
    /// dropping subscribers that report failure.
    pub fn subscribe(&self, subscriber: Subscriber) -> u64 {
        let mut inner = self.lock();
        inner.next_token += 1;
        let token = inner.next_token;
        inner.subscribers.push((token, subscriber));
        token
    }

    /// Remove a previously subscribed callback (idempotent).
    pub fn unsubscribe(&self, token: u64) {
        self.lock().subscribers.retain(|(id, _)| *id != token);
    }

    /// The number of live subscribers (diagnostics and close-cleanup tests).
    pub fn subscriber_count(&self) -> usize {
        self.lock().subscribers.len()
    }

    /// Resume semantics: deliver the retained events with an id greater than
    /// `resume_from` to `deliver`. When the resume point is older than the
    /// retained window, deliver a single [`snapshot_refresh_event`]
    /// instruction instead — never a partial replay. Delivering nothing means
    /// the client is current and continues to stream live.
    pub fn replay_after(&self, resume_from: u64, deliver: &Subscriber) {
        let (replay, instruction) = {
            let inner = self.lock();
            let Some(newest) = inner.retained.back().map(|event| event.id) else {
                // Nothing was ever published: no event can be missing.
                return;
            };
            if resume_from >= newest {
                // The client is already current.
                return;
            }
            let oldest = inner
                .retained
                .front()
                .map(|event| event.id)
                .unwrap_or_default();
            if oldest > resume_from.saturating_add(1) {
                (Vec::new(), Some(snapshot_refresh_event(newest)))
            } else {
                let replay = inner
                    .retained
                    .iter()
                    .filter(|event| event.id > resume_from)
                    .cloned()
                    .collect();
                (replay, None)
            }
        };
        for event in &replay {
            if !deliver(event) {
                return;
            }
        }
        if let Some(event) = instruction {
            deliver(&event);
        }
    }
}

impl AdminEventHub {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The lag fallback: an instruction to refresh the admin snapshot instead of
/// a partial replay. Its id is the newest retained event id, so the client
/// can resume from it afterwards.
fn snapshot_refresh_event(newest: u64) -> AdminEvent {
    AdminEvent {
        version: EVENT_PROTOCOL_VERSION,
        id: newest,
        event_type: SNAPSHOT_REFRESH_EVENT.to_string(),
        payload: json!({
            "reason": SNAPSHOT_REFRESH_REASON,
            "request": {
                "method": "GET",
                "path": "/api/admin/snapshot?view=Crystals&selected_id=1",
            },
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A subscriber collecting every delivered event.
    fn collector(events: Arc<Mutex<Vec<AdminEvent>>>) -> Subscriber {
        Arc::new(move |event: &AdminEvent| {
            events.lock().unwrap().push(event.clone());
            true
        })
    }

    fn collected(events: &Arc<Mutex<Vec<AdminEvent>>>) -> Vec<AdminEvent> {
        events.lock().unwrap().clone()
    }

    #[test]
    fn publish_envelopes_events_with_monotonic_ids() {
        let hub = AdminEventHub::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        hub.subscribe(collector(Arc::clone(&events)));

        hub.publish("dream_started", json!({"trigger": "manual"}));
        hub.publish(
            "dream_completed",
            json!({"trigger": "manual", "result": {}}),
        );

        let events = collected(&events);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].id, 1, "daemon-lifetime ids start at 1");
        assert_eq!(events[1].id, 2);
        assert_eq!(events[0].version, EVENT_PROTOCOL_VERSION);
        assert_eq!(
            events[0].to_wire(),
            json!({
                "version": 1,
                "event_id": 1,
                "event_type": "dream_started",
                "payload": {"trigger": "manual"},
            })
        );
    }

    #[test]
    fn failing_subscribers_are_dropped_without_affecting_others() {
        let hub = AdminEventHub::default();
        let failures = Arc::new(Mutex::new(0_u32));
        let failing: Subscriber = {
            let failures = Arc::clone(&failures);
            Arc::new(move |_event: &AdminEvent| {
                *failures.lock().unwrap() += 1;
                false
            })
        };
        hub.subscribe(failing);
        let events = Arc::new(Mutex::new(Vec::new()));
        hub.subscribe(collector(Arc::clone(&events)));

        hub.publish("dream_started", json!({}));
        hub.publish("dream_started", json!({}));

        assert_eq!(
            *failures.lock().unwrap(),
            1,
            "the failing subscriber is dropped"
        );
        assert_eq!(
            collected(&events).len(),
            2,
            "healthy subscribers keep receiving"
        );
        assert_eq!(hub.subscriber_count(), 1);
    }

    #[test]
    fn unsubscribe_stops_delivery() {
        let hub = AdminEventHub::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let token = hub.subscribe(collector(Arc::clone(&events)));
        hub.unsubscribe(token);
        hub.unsubscribe(token);
        hub.publish("dream_started", json!({}));
        assert!(collected(&events).is_empty());
    }

    #[test]
    fn resume_within_the_window_replays_only_later_events() {
        let hub = AdminEventHub::default();
        for id in 1..=3 {
            hub.publish("dream_phase_progress", json!({"step": id}));
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let subscriber = collector(Arc::clone(&events));
        hub.replay_after(1, &subscriber);
        let replayed: Vec<u64> = collected(&events).iter().map(|event| event.id).collect();
        assert_eq!(replayed, vec![2, 3]);
    }

    #[test]
    fn resume_at_the_newest_event_replays_nothing() {
        let hub = AdminEventHub::default();
        hub.publish("dream_started", json!({}));
        let events = Arc::new(Mutex::new(Vec::new()));
        let subscriber = collector(Arc::clone(&events));
        hub.replay_after(1, &subscriber);
        assert!(collected(&events).is_empty());
        // A resume point ahead of every published event is current, not lag.
        hub.replay_after(1_000, &subscriber);
        assert!(collected(&events).is_empty());
    }

    #[test]
    fn an_empty_hub_replays_nothing() {
        let hub = AdminEventHub::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let subscriber = collector(Arc::clone(&events));
        hub.replay_after(41, &subscriber);
        assert!(collected(&events).is_empty());
    }

    #[test]
    fn retention_is_bounded() {
        let hub = AdminEventHub::default();
        for id in 1..=(RETAINED_EVENTS as u64 + 1) {
            hub.publish("dream_phase_progress", json!({"step": id}));
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let subscriber = collector(Arc::clone(&events));
        // Resuming from 1 is still inside the window (the oldest retained id
        // is 2), so the retained tail replays contiguously.
        hub.replay_after(1, &subscriber);
        let replayed = collected(&events);
        assert_eq!(replayed.len(), RETAINED_EVENTS);
        assert_eq!(replayed.first().unwrap().id, 2, "event 1 was evicted");
    }

    #[test]
    fn lagged_resume_gets_snapshot_refresh_instead_of_partial_replay() {
        let hub = AdminEventHub::default();
        for id in 1..=(RETAINED_EVENTS as u64 + 1) {
            hub.publish("dream_phase_progress", json!({"step": id}));
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let subscriber = collector(Arc::clone(&events));
        hub.replay_after(0, &subscriber);
        let delivered = collected(&events);
        assert_eq!(
            delivered.len(),
            1,
            "lag must deliver exactly one instruction"
        );
        let instruction = &delivered[0];
        assert_eq!(instruction.event_type, SNAPSHOT_REFRESH_EVENT);
        assert_eq!(
            instruction.id,
            RETAINED_EVENTS as u64 + 1,
            "the instruction carries the newest retained id"
        );
        assert_eq!(
            instruction.to_wire(),
            json!({
                "version": 1,
                "event_id": RETAINED_EVENTS as u64 + 1,
                "event_type": "snapshot_refresh",
                "payload": {
                    "reason": "last event id is outside retained history",
                    "request": {
                        "method": "GET",
                        "path": "/api/admin/snapshot?view=Crystals&selected_id=1",
                    },
                },
            })
        );
    }
}
