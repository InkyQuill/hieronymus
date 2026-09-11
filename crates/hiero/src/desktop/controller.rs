//! A single owned worker keeps blocking lifecycle work off the native UI loop.
use std::collections::VecDeque;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, SyncSender},
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::{Action, AutostartRegistration, Event, SettingsStore};
use crate::{
    console,
    lifecycle::{self, DiscoveryHealth},
    service::ServiceOptions,
};
use hieronymus::data_root::HieronymusConfig;

/// Implementations must bound every call and return sanitized, secret-free errors.
/// Shutdown joins the worker; it never detaches a blocked implementation. The
/// production implementation preserves the existing longer lifecycle deadlines.
pub trait DesktopBackend: Send + 'static {
    fn probe(&mut self) -> Event;
    fn perform(&mut self, action: &Action) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy)]
pub struct PollSchedule {
    pub poll_interval: Duration,
    /// Maximum wait for a snapshot after a successful lifecycle operation.
    pub startup_timeout: Duration,
}
impl Default for PollSchedule {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(3),
            startup_timeout: Duration::from_secs(20),
        }
    }
}

#[derive(Default)]
struct Delivery {
    events: VecDeque<Event>,
    // Cleared only when Finished is consumed, bounding queued operation events
    // even if the UI stops draining. Begin/Finished are never coalesced.
    busy: bool,
}
impl Delivery {
    fn push(&mut self, event: Event) {
        match (&event, self.events.back()) {
            (Event::Snapshot(_) | Event::Stopped | Event::InvalidIdentity, _) => {
                // A definitive observation replaces only the trailing health
                // segment. Operation/deadline events are ordering fences.
                while self.events.back().is_some_and(is_health) {
                    self.events.pop_back();
                }
            }
            // Preserve failure confidence, but don't grow forever while idle.
            (Event::ProbeTimeout, Some(Event::ProbeTimeout))
                if self
                    .events
                    .iter()
                    .rev()
                    .take_while(|e| matches!(e, Event::ProbeTimeout))
                    .count()
                    >= 3 =>
            {
                return;
            }
            _ => {}
        }
        self.events.push_back(event);
    }
}

fn is_health(event: &Event) -> bool {
    matches!(
        event,
        Event::Snapshot(_) | Event::Stopped | Event::InvalidIdentity | Event::ProbeTimeout
    )
}

/// Dropping the controller also joins its worker. Native adapters should use
/// shutdown explicitly to report a worker panic as a sanitized error.
pub struct Controller {
    commands: SyncSender<Option<Action>>,
    delivery: Arc<Mutex<Delivery>>,
    stopping: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Controller {
    pub fn spawn(backend: impl DesktopBackend) -> Self {
        Self::spawn_with_schedule(backend, PollSchedule::default())
    }
    /// Inject short polling intervals for tests; zero intervals are clamped.
    pub fn spawn_with_schedule(mut backend: impl DesktopBackend, schedule: PollSchedule) -> Self {
        let (commands, receiver) = mpsc::sync_channel::<Option<Action>>(1);
        let delivery = Arc::new(Mutex::new(Delivery::default()));
        let output = delivery.clone();
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = stopping.clone();
        let worker = std::thread::spawn(move || {
            let interval = schedule.poll_interval.max(Duration::from_millis(1));
            let mut next_probe = Instant::now();
            let mut startup_deadline: Option<Instant> = None;
            while !stop.load(Ordering::Acquire) {
                let wake_at =
                    startup_deadline.map_or(next_probe, |deadline| deadline.min(next_probe));
                match receiver.recv_timeout(wake_at.saturating_duration_since(Instant::now())) {
                    Ok(Some(action)) => {
                        output
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push(Event::Begin(action.clone()));
                        let error = backend.perform(&action).err();
                        match action {
                            Action::Start | Action::Restart => {
                                startup_deadline = error
                                    .is_none()
                                    .then(|| Instant::now() + schedule.startup_timeout);
                            }
                            Action::Quit => startup_deadline = None,
                            Action::OpenConsole | Action::SetAutostart(_) => {}
                        }
                        let quit = action == Action::Quit && error.is_none();
                        if quit {
                            stop.store(true, Ordering::Release);
                        }
                        output
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push(Event::Finished { action, error });
                        if quit {
                            break;
                        }
                        next_probe = Instant::now();
                    }
                    Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if startup_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                            output
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .push(Event::StartupDeadlineExpired);
                            startup_deadline = None;
                        }
                        if Instant::now() >= next_probe {
                            let probe_started = Instant::now();
                            let event = backend.probe();
                            if matches!(event, Event::Snapshot(_) | Event::InvalidIdentity) {
                                startup_deadline = None;
                            }
                            output
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .push(event);
                            next_probe = probe_started + interval;
                        }
                    }
                }
            }
        });
        Self {
            commands,
            delivery,
            stopping,
            worker: Some(worker),
        }
    }
    pub fn submit(&self, action: Action) -> Result<(), String> {
        let mut delivery = self
            .delivery
            .lock()
            .map_err(|_| "Desktop controller unavailable")?;
        if delivery.busy {
            return Err("Another desktop action is in progress".into());
        }
        if self.stopping.load(Ordering::Acquire) {
            return Err("Desktop controller is shutting down".into());
        }
        delivery.busy = true;
        if self.commands.try_send(Some(action)).is_err() {
            delivery.busy = false;
            return Err("Desktop worker unavailable".into());
        }
        Ok(())
    }
    pub fn try_event(&self) -> Option<Event> {
        let mut delivery = self
            .delivery
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let event = delivery.events.pop_front()?;
        if matches!(event, Event::Finished { .. }) {
            delivery.busy = false;
        }
        Some(event)
    }
    pub fn shutdown(mut self) -> Result<(), String> {
        self.join()
    }
    fn join(&mut self) -> Result<(), String> {
        self.stopping.store(true, Ordering::Release);
        let _ = self.commands.try_send(None);
        self.worker.take().map_or(Ok(()), |worker| {
            worker.join().map_err(|_| "Desktop worker failed".into())
        })
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        let _ = self.join();
    }
}

/// Production, non-GUI adapter. Registration is supplied by the native package;
/// no unsupported platform is represented by a successful no-op.
pub struct LifecycleBackend<R> {
    config: HieronymusConfig,
    options: ServiceOptions,
    registration: R,
    settings: SettingsStore,
}
impl<R: AutostartRegistration> LifecycleBackend<R> {
    /// For an in-process CLI host. A separately packaged GUI helper must use
    /// `with_service_options` with the installed CLI binary, not its own path.
    pub fn new(config: HieronymusConfig, registration: R) -> Result<Self, String> {
        let options = lifecycle::default_service_options(&config)
            .map_err(|_| "Could not locate the local service configuration")?;
        Ok(Self::with_service_options(config, options, registration))
    }
    pub fn with_service_options(
        config: HieronymusConfig,
        options: ServiceOptions,
        registration: R,
    ) -> Self {
        let settings = SettingsStore::new(&config);
        Self {
            config,
            options,
            registration,
            settings,
        }
    }
}
impl<R: AutostartRegistration> DesktopBackend for LifecycleBackend<R> {
    fn probe(&mut self) -> Event {
        match lifecycle::probe_with_deadline(&self.config, Instant::now() + Duration::from_secs(2))
        {
            DiscoveryHealth::Live { status, .. } => status
                .get("readiness")
                .cloned()
                .and_then(|value| serde_json::from_value(value).ok())
                .map_or(Event::ProbeTimeout, Event::Snapshot),
            DiscoveryHealth::NoRecord { .. } => {
                if lifecycle::root_is_released(&self.config).unwrap_or(false) {
                    Event::Stopped
                } else {
                    Event::ProbeTimeout
                }
            }
            DiscoveryHealth::Unreachable { .. } => Event::ProbeTimeout,
            _ => Event::InvalidIdentity,
        }
    }
    fn perform(&mut self, action: &Action) -> Result<(), String> {
        if self.options.data_root != self.config.data_root() {
            return Err("Desktop service options belong to a different data root".into());
        }
        // Each public lifecycle/console entry point acquires exactly one
        // shared operation lock around its complete action.
        match action {
            Action::Start => lifecycle::start(&self.options)
                .map(|_| ())
                .map_err(|_| "Could not start the server; check the local service".into()),
            Action::Restart => lifecycle::restart(&self.config, &self.options)
                .map(|_| ())
                .map_err(|_| "Could not restart the server; it may still be running".into()),
            Action::Quit => lifecycle::stop(&self.config, &self.options)
                .map(|_| ())
                .map_err(|_| "Could not stop the server; the desktop helper remains open".into()),
            Action::OpenConsole => {
                console::launch_with_options(&self.config, &self.options, "admin")
                    .map_err(|_| "Could not open the console; check the server and browser".into())
            }
            Action::SetAutostart(enabled) => self
                .settings
                .set_autostart(&mut self.registration, *enabled)
                .map(|_| ()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        desktop::{Accent, DesktopState},
        readiness::{ReadinessLevel, ReadinessSummary},
    };
    fn ready() -> Event {
        Event::Snapshot(ReadinessSummary {
            level: ReadinessLevel::Ready,
            reasons: vec![],
            providers: vec![],
        })
    }
    #[test]
    fn delayed_consumer_retains_failure_confidence_and_operation_fences() {
        let mut delivery = Delivery::default();
        for _ in 0..1000 {
            delivery.push(ready());
        }
        for _ in 0..1000 {
            delivery.push(Event::ProbeTimeout);
        }
        assert_eq!(delivery.events.len(), 4);
        let mut state = DesktopState::new();
        for event in delivery.events.drain(..) {
            state.apply(event);
        }
        assert_eq!(state.apply(Event::ProbeTimeout).accent, Accent::Red);

        delivery.push(Event::Begin(Action::Restart));
        for _ in 0..1000 {
            delivery.push(Event::ProbeTimeout);
        }
        delivery.push(Event::Finished {
            action: Action::Restart,
            error: None,
        });
        delivery.push(Event::StartupDeadlineExpired);
        for _ in 0..1000 {
            delivery.push(Event::ProbeTimeout);
            delivery.push(ready());
        }
        assert_eq!(delivery.events.len(), 7);
        assert!(matches!(delivery.events[0], Event::Begin(_)));
        assert!(matches!(delivery.events[4], Event::Finished { .. }));
        assert_eq!(delivery.events[5], Event::StartupDeadlineExpired);
        for event in delivery.events {
            state.apply(event);
        }
        assert_eq!(state.apply(ready()).accent, Accent::Green);
    }
}
