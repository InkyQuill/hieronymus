//! Coalesced callback state survives nested Win32 loops consuming posted wakes.
use std::cell::Cell;
pub const MENU: u8 = 1;
pub const APPEARANCE: u8 = 2;
pub const EXPLORER: u8 = 4;
pub const TEARDOWN: u8 = 8;
#[derive(Default)]
pub struct PendingEvents(Cell<u8>);
impl PendingEvents {
    /// Returns true only when a new event requires a wake. Storage is constant size.
    pub fn record(&self, event: u8) -> bool {
        let previous = self.0.get();
        self.0.set(previous | event);
        previous & event != event
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
