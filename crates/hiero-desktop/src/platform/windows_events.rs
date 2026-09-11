//! Coalesced callback state survives nested Win32 loops consuming posted wakes.
use std::cell::Cell;
pub const MENU: u8 = 1;
pub const APPEARANCE: u8 = 2;
pub const EXPLORER: u8 = 4;
pub const TEARDOWN: u8 = 8;
pub const CONTROLLER: u8 = 16;
#[derive(Default)]
pub struct PendingEvents(Cell<u8>);
impl PendingEvents {
    /// Returns true only when a new event requires a wake. Storage is constant size.
    pub fn record(&self, event: u8) -> bool {
        let previous = self.0.get();
        self.0.set(previous | event);
        previous & event != event
    }
    /// A consumed posted wake still requires owner-side controller delivery.
    /// Native callbacks latch only; never recursively post a replacement wake.
    pub fn controller_wake(&self) {
        self.record(CONTROLLER);
    }
    pub fn pending(&self) -> u8 {
        self.0.get()
    }
    pub fn take(&self) -> u8 {
        self.0.replace(0)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controller_retirement_wake_survives_nested_dispatch_without_another_message() {
        let pending = PendingEvents::default();
        let mut delivery =
            std::collections::VecDeque::from([hiero::desktop::Event::RetiredForUpdate]);
        // TrackPopupMenu consumes the worker's sole wake before returning no action.
        pending.controller_wake();
        assert_ne!(
            pending.pending(),
            0,
            "owner must drain instead of blocking in GetMessage"
        );
        assert_eq!(pending.take(), CONTROLLER);
        assert!(matches!(
            delivery.pop_front(),
            Some(hiero::desktop::Event::RetiredForUpdate)
        ));
        assert_eq!(pending.pending(), 0);
    }
    #[test]
    fn nested_loop_can_consume_every_wake_without_losing_events() {
        let pending = PendingEvents::default();
        for event in [MENU, APPEARANCE, EXPLORER, TEARDOWN] {
            assert!(pending.record(event));
            // A nested dispatcher consumes the posted wake; it never owns the event.
            for _ in 0..1000 {
                assert!(!pending.record(event));
            }
        }
        assert_eq!(pending.take(), MENU | APPEARANCE | EXPLORER | TEARDOWN);
        assert_eq!(pending.pending(), 0);
        assert!(pending.record(EXPLORER));
        assert_eq!(pending.take(), EXPLORER);
    }
}
