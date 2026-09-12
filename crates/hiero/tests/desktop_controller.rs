use hiero::desktop::{Action, Controller, DesktopBackend, DesktopState, Event, PollSchedule};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

struct Fake {
    calls: Arc<Mutex<Vec<Action>>>,
    probes: Arc<AtomicUsize>,
    entered: mpsc::Sender<()>,
    release: Option<mpsc::Receiver<()>>,
    error: Option<String>,
}
impl DesktopBackend for Fake {
    fn probe(&mut self) -> Event {
        self.probes.fetch_add(1, Ordering::SeqCst);
        Event::Stopped
    }
    fn perform(&mut self, action: &Action) -> Result<(), String> {
        self.calls.lock().unwrap().push(action.clone());
        self.entered.send(()).unwrap();
        if let Some(release) = &self.release {
            release.recv_timeout(Duration::from_secs(1)).unwrap();
        }
        self.error.clone().map_or(Ok(()), Err)
    }
}
fn wait_event(controller: &Controller, wanted: impl Fn(&Event) -> bool) -> Event {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(event) = controller.try_event()
            && wanted(&event)
        {
            return event;
        }
        assert!(Instant::now() < deadline, "event did not arrive");
        std::thread::yield_now();
    }
}
fn schedule() -> PollSchedule {
    PollSchedule {
        poll_interval: Duration::from_millis(1),
        startup_timeout: Duration::from_millis(5),
    }
}
#[test]
fn passive_polling_never_starts_and_shutdown_joins() {
    let calls = Arc::new(Mutex::new(vec![]));
    let probes = Arc::new(AtomicUsize::new(0));
    let (entered, _) = mpsc::channel();
    let controller = Controller::spawn_with_schedule(
        Fake {
            calls: calls.clone(),
            probes: probes.clone(),
            entered,
            release: None,
            error: None,
        },
        schedule(),
    );
    wait_event(&controller, |event| matches!(event, Event::Stopped));
    controller.shutdown().unwrap();
    assert!(probes.load(Ordering::SeqCst) > 0);
    assert!(calls.lock().unwrap().is_empty());
}
#[test]
fn failed_quit_emits_ordered_error_and_never_exits_or_accepts_duplicate() {
    let calls = Arc::new(Mutex::new(vec![]));
    let (entered, began) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let controller = Controller::spawn_with_schedule(
        Fake {
            calls: calls.clone(),
            probes: Arc::new(AtomicUsize::new(0)),
            entered,
            release: Some(gate),
            error: Some("daemon did not stop".into()),
        },
        schedule(),
    );
    controller.submit(Action::Quit).unwrap();
    assert!(controller.submit(Action::Quit).is_err());
    began.recv_timeout(Duration::from_secs(1)).unwrap();
    let mut state = DesktopState::new();
    state.apply(wait_event(&controller, |event| {
        matches!(event, Event::Begin(_))
    }));
    assert!(controller.submit(Action::Start).is_err());
    release.send(()).unwrap();
    let event = wait_event(&controller, |event| matches!(event, Event::Finished { .. }));
    assert_eq!(
        event,
        Event::Finished {
            action: Action::Quit,
            error: Some("daemon did not stop".into())
        }
    );
    assert!(!state.apply(event).exit_requested);
    controller.shutdown().unwrap();
    assert_eq!(*calls.lock().unwrap(), vec![Action::Quit]);
}
#[test]
fn completed_start_without_snapshot_has_a_real_deadline() {
    let (entered, _receiver) = mpsc::channel();
    let controller = Controller::spawn_with_schedule(
        Fake {
            calls: Arc::new(Mutex::new(vec![])),
            probes: Arc::new(AtomicUsize::new(0)),
            entered,
            release: None,
            error: None,
        },
        schedule(),
    );
    controller.submit(Action::Start).unwrap();
    let mut state = DesktopState::new();
    state.apply(wait_event(&controller, |e| matches!(e, Event::Begin(_))));
    state.apply(wait_event(&controller, |e| {
        matches!(e, Event::Finished { .. })
    }));
    let view = state.apply(wait_event(&controller, |e| {
        matches!(e, Event::StartupDeadlineExpired)
    }));
    assert_eq!(view.accent, hiero::desktop::Accent::Red);
    controller.shutdown().unwrap();
}

use hiero::desktop::{
    AutostartRegistration, ForegroundMode, LifecycleBackend, SettingsStore, UnsupportedAutostart,
};
use hieronymus::data_root::HieronymusConfig;
struct Registration {
    actual: bool,
    fail: bool,
    lie: bool,
    calls: Vec<&'static str>,
}
impl AutostartRegistration for Registration {
    fn set_enabled(&mut self, enabled: bool) -> Result<(), String> {
        self.calls.push("set");
        if !self.lie {
            self.actual = enabled;
        }
        if self.fail {
            Err("Registration failed".into())
        } else {
            Ok(())
        }
    }
    fn is_enabled(&mut self) -> Result<bool, String> {
        self.calls.push("read");
        Ok(self.actual)
    }
}
#[test]
fn settings_read_back_before_persistence_and_repair_interrupted_change() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let store = SettingsStore::new(&config);
    store.set_foreground(ForegroundMode::Light).unwrap();
    let mut registration = Registration {
        actual: false,
        fail: true,
        lie: false,
        calls: vec![],
    };
    assert!(store.set_autostart(&mut registration, true).is_err());
    assert_eq!(registration.calls, ["set", "read"]);
    assert!(!store.load().unwrap().autostart);
    assert!(store.reconcile(&mut registration).unwrap().autostart);
    assert_eq!(store.load().unwrap().foreground, ForegroundMode::Light);
    registration.fail = false;
    registration.lie = true;
    assert!(store.set_autostart(&mut registration, false).is_err());
    assert!(store.load().unwrap().autostart);
    registration.lie = false;
    assert!(
        !store
            .set_autostart(&mut registration, false)
            .unwrap()
            .autostart
    );
    assert!(!store.load().unwrap().autostart);
    assert!(
        store
            .set_autostart(&mut UnsupportedAutostart, true)
            .is_err()
    );
}
#[test]
fn production_probe_does_not_create_root_and_requires_owner_release() {
    let directory = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(directory.path().join("absent"));
    let mut backend = LifecycleBackend::new(config.clone(), UnsupportedAutostart).unwrap();
    assert_eq!(backend.probe(), Event::Stopped);
    assert!(!config.data_root().exists());
    let owner = hieronymus::ownership::RootOwnership::acquire(&config, "test").unwrap();
    assert_eq!(backend.probe(), Event::ProbeTimeout);
    drop(owner);
    assert_eq!(backend.probe(), Event::Stopped);
    std::fs::write(config.daemon_discovery_path(), "invalid").unwrap();
    assert_eq!(backend.probe(), Event::InvalidIdentity);
}

fn probe_reply(mut body: serde_json::Value) -> Event {
    use hiero::daemon::{
        discovery::{self, DiscoveryRecord},
        registry::PROTOCOL_REVISION,
    };
    use std::io::{Read, Write};
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let instance = "ab".repeat(16);
    let token = discovery::generate_bearer_token().unwrap();
    discovery::write_token(&config, &token).unwrap();
    discovery::write_discovery(
        &config,
        &DiscoveryRecord {
            discovery_version: discovery::DISCOVERY_VERSION,
            protocol_version: PROTOCOL_REVISION.into(),
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
            pid: std::process::id(),
            instance_id: instance.clone(),
            started_at: "2026-09-11T00:00:00Z".into(),
        },
    )
    .unwrap();
    if body.get("instance_id").is_none() {
        body["instance_id"] = instance.into();
    }
    body["protocol_revision"] = PROTOCOL_REVISION.into();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut request = vec![];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let mut bytes = [0; 1024];
            let count = stream.read(&mut bytes).unwrap();
            assert!(count > 0);
            request.extend_from_slice(&bytes[..count]);
        }
        assert!(
            String::from_utf8(request)
                .unwrap()
                .contains(token.expose_secret())
        );
        let body = body.to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });
    let mut backend = LifecycleBackend::new(config, UnsupportedAutostart).unwrap();
    let event = backend.probe();
    server.join().unwrap();
    event
}
#[test]
fn production_probe_consumes_authenticated_readiness_and_fails_closed() {
    use hiero::readiness::{ReadinessLevel, ReadinessSummary};
    let summary = ReadinessSummary {
        level: ReadinessLevel::Degraded,
        reasons: vec!["Model unavailable".into()],
        providers: vec![],
    };
    assert_eq!(
        probe_reply(serde_json::json!({"readiness": summary})),
        Event::Snapshot(summary)
    );
    assert_eq!(
        probe_reply(serde_json::json!({"version": "legacy"})),
        Event::ProbeTimeout
    );
    assert_eq!(
        probe_reply(serde_json::json!({"readiness": {"level": "ready"}})),
        Event::ProbeTimeout
    );
    assert_eq!(
        probe_reply(serde_json::json!({"instance_id": "foreign"})),
        Event::InvalidIdentity
    );
    assert_eq!(
        probe_reply(serde_json::json!({"instance_id": null})),
        Event::InvalidIdentity
    );
}
#[test]
fn production_actions_share_operation_guard_and_failed_quit_keeps_owner() {
    use hiero::{lifecycle::operation::LifecycleOperation, service::ServiceOptions};
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("data"));
    let options = ServiceOptions {
        data_root: config.data_root().into(),
        unit_dir: root.path().join("units"),
        binary: root.path().join("hiero"),
        use_manager: false,
    };
    let held = LifecycleOperation::acquire(&config).unwrap();
    let mut backend =
        LifecycleBackend::with_service_options(config.clone(), options, UnsupportedAutostart);
    for action in [
        Action::Start,
        Action::Restart,
        Action::Quit,
        Action::OpenConsole,
    ] {
        assert!(backend.perform(&action).is_err());
    }
    assert!(!root.path().join("units").exists());
    drop(held);
    let owner = hieronymus::ownership::RootOwnership::acquire(&config, "fixture").unwrap();
    assert!(backend.perform(&Action::Quit).is_err());
    assert!(!hiero::lifecycle::root_is_released(&config).unwrap());
    drop(owner);
}
#[test]
fn successful_quit_stops_worker_and_keeps_exit_event_last() {
    let (entered, _receiver) = mpsc::channel();
    let controller = Controller::spawn_with_schedule(
        Fake {
            calls: Arc::new(Mutex::new(vec![])),
            probes: Arc::new(AtomicUsize::new(0)),
            entered,
            release: None,
            error: None,
        },
        schedule(),
    );
    controller.submit(Action::Quit).unwrap();
    wait_event(&controller, |event| matches!(event, Event::Finished { .. }));
    assert_eq!(controller.try_event(), None);
    assert!(controller.submit(Action::Start).is_err());
    controller.shutdown().unwrap();
}

#[test]
fn preference_action_does_not_cancel_outstanding_startup_deadline() {
    let (entered, _receiver) = mpsc::channel();
    let controller = Controller::spawn_with_schedule(
        Fake {
            calls: Arc::new(Mutex::new(vec![])),
            probes: Arc::new(AtomicUsize::new(0)),
            entered,
            release: None,
            error: None,
        },
        PollSchedule {
            poll_interval: Duration::from_millis(1),
            startup_timeout: Duration::from_millis(100),
        },
    );
    controller.submit(Action::Start).unwrap();
    wait_event(&controller, |e| matches!(e, Event::Finished { .. }));
    controller.submit(Action::SetAutostart(true)).unwrap();
    wait_event(&controller, |e| matches!(e, Event::Finished { .. }));
    wait_event(&controller, |e| matches!(e, Event::StartupDeadlineExpired));
    controller.shutdown().unwrap();
}

// Exercise the actual worker's Begin/Finished/poll/deadline ordering through
// the reducer, including an already-ready daemon before restart.
struct RestartThenAction {
    restarted: bool,
    followup_error: bool,
    recovered: Arc<std::sync::atomic::AtomicBool>,
}
impl DesktopBackend for RestartThenAction {
    fn probe(&mut self) -> Event {
        if !self.restarted || self.recovered.load(Ordering::SeqCst) {
            Event::Snapshot(hiero::readiness::ReadinessSummary {
                level: hiero::readiness::ReadinessLevel::Ready,
                reasons: vec![],
                providers: vec![],
            })
        } else {
            Event::ProbeTimeout
        }
    }
    fn perform(&mut self, action: &Action) -> Result<(), String> {
        if *action == Action::Restart {
            self.restarted = true;
            Ok(())
        } else if self.followup_error {
            Err("Desktop action failed".into())
        } else {
            Ok(())
        }
    }
}
fn apply_until(
    controller: &Controller,
    state: &mut DesktopState,
    wanted: impl Fn(&Event) -> bool,
) -> hiero::desktop::View {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(event) = controller.try_event() {
            let found = wanted(&event);
            let view = state.apply(event).clone();
            if found {
                return view;
            }
        }
        assert!(Instant::now() < deadline, "controller event did not arrive");
        std::thread::yield_now();
    }
}
fn restart_then_followup(action: Action, error: bool) {
    use hiero::desktop::Accent;
    let recovered = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let controller = Controller::spawn_with_schedule(
        RestartThenAction {
            restarted: false,
            followup_error: error,
            recovered: recovered.clone(),
        },
        PollSchedule {
            poll_interval: Duration::from_millis(200),
            startup_timeout: Duration::from_millis(100),
        },
    );
    let mut state = DesktopState::new();
    assert_eq!(
        apply_until(&controller, &mut state, |event| matches!(
            event,
            Event::Snapshot(_)
        ))
        .accent,
        Accent::Green
    );
    controller.submit(Action::Restart).unwrap();
    assert_eq!(
        apply_until(&controller, &mut state, |event| matches!(
            event,
            Event::Finished { .. }
        ))
        .accent,
        Accent::Amber
    );
    controller.submit(action).unwrap();
    let pending = apply_until(&controller, &mut state, |event| {
        matches!(event, Event::Begin(_))
    });
    assert_eq!(
        pending.accent,
        Accent::Amber,
        "follow-up action must not reuse pre-restart readiness"
    );
    assert!(pending.busy);
    let completed = apply_until(&controller, &mut state, |event| {
        matches!(event, Event::Finished { .. })
    });
    assert_eq!(completed.accent, Accent::Amber);
    assert!(!completed.busy);
    assert!(!completed.can_start);
    assert_eq!(
        completed.reason,
        if error {
            "Desktop action failed"
        } else {
            "Starting"
        }
    );
    let checking = apply_until(&controller, &mut state, |event| {
        *event == Event::ProbeTimeout
    });
    assert_eq!(checking.accent, Accent::Amber);
    assert_eq!(checking.reason, completed.reason);
    let expired = apply_until(&controller, &mut state, |event| {
        *event == Event::StartupDeadlineExpired
    });
    assert_eq!(expired.accent, Accent::Red);
    assert_eq!(
        expired.reason,
        if error {
            "Desktop action failed"
        } else {
            "Server unavailable"
        }
    );
    assert!(!expired.can_start);
    recovered.store(true, Ordering::SeqCst);
    let healthy = apply_until(&controller, &mut state, |event| {
        matches!(event, Event::Snapshot(_))
    });
    assert_eq!(healthy.accent, Accent::Green);
    assert_eq!(
        healthy.reason,
        if error {
            "Desktop action failed"
        } else {
            "Ready"
        }
    );
    controller.shutdown().unwrap();
}
#[test]
fn successful_preference_and_browser_actions_preserve_pending_restart_health() {
    for action in [Action::SetAutostart(true), Action::OpenConsole] {
        restart_then_followup(action, false);
    }
}
#[test]
fn failed_preference_and_browser_actions_keep_restart_deadline_and_error_reason() {
    for action in [Action::SetAutostart(true), Action::OpenConsole] {
        restart_then_followup(action, true);
    }
}

#[test]
fn notifier_wakes_for_begin_before_blocking_action_and_finished_after_release() {
    let (entered, began) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let (wake, woken) = mpsc::channel();
    let controller = Controller::spawn_with_notifier(
        Fake {
            calls: Arc::new(Mutex::new(vec![])),
            probes: Arc::new(AtomicUsize::new(0)),
            entered,
            release: Some(gate),
            error: None,
        },
        PollSchedule {
            poll_interval: Duration::from_secs(60),
            ..schedule()
        },
        move || {
            let _ = wake.send(());
        },
    );
    controller.submit(Action::Quit).unwrap();
    began.recv_timeout(Duration::from_secs(1)).unwrap();
    woken.recv_timeout(Duration::from_secs(1)).unwrap();
    let mut events = vec![];
    while let Some(event) = controller.try_event() {
        events.push(event);
    }
    assert!(events.contains(&Event::Begin(Action::Quit)));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::Finished { .. }))
    );
    release.send(()).unwrap();
    woken.recv_timeout(Duration::from_secs(1)).unwrap();
    wait_event(&controller, |event| {
        matches!(
            event,
            Event::Finished {
                action: Action::Quit,
                error: None
            }
        )
    });
    controller.shutdown().unwrap();
}

#[test]
fn actual_registration_survives_preference_persistence_failure() {
    let directory = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(directory.path());
    std::fs::create_dir(directory.path().join("desktop-settings.json")).unwrap();
    let mut backend = LifecycleBackend::new(
        config,
        Registration {
            actual: true,
            fail: false,
            lie: false,
            calls: vec![],
        },
    )
    .unwrap();
    // Actual registration is known even though preferences cannot be persisted/read.
    assert!(
        matches!(backend.preferences(), Some(Event::Preferences { settings: Some(settings), error: Some(_) }) if settings.autostart)
    );
}

#[test]
fn preferences_event_does_not_change_lifecycle_state() {
    let mut state = DesktopState::new();
    let before = state.apply(Event::Begin(Action::Restart)).clone();
    let after = state.apply(Event::Preferences {
        settings: None,
        error: Some("Unavailable".into()),
    });
    assert_eq!(after, &before);
}

#[test]
fn failed_toggle_delivers_actual_registration_before_completion() {
    let directory = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(directory.path());
    let backend = LifecycleBackend::new(
        config,
        Registration {
            actual: false,
            fail: true,
            lie: false,
            calls: vec![],
        },
    )
    .unwrap();
    let controller = Controller::spawn(backend);
    wait_event(
        &controller,
        |event| matches!(event, Event::Preferences { settings: Some(settings), error: None } if !settings.autostart),
    );
    controller.submit(Action::SetAutostart(true)).unwrap();
    wait_event(&controller, |event| {
        matches!(event, Event::Begin(Action::SetAutostart(true)))
    });
    let readback = wait_event(&controller, |event| {
        matches!(event, Event::Preferences { .. } | Event::Finished { .. })
    });
    assert!(
        matches!(readback, Event::Preferences { settings: Some(settings), error: None } if settings.autostart)
    );
    let finished = wait_event(&controller, |event| matches!(event, Event::Finished { .. }));
    assert!(matches!(
        finished,
        Event::Finished {
            action: Action::SetAutostart(true),
            error: Some(_)
        }
    ));
    controller.shutdown().unwrap();
}
