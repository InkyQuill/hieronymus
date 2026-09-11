use hiero::desktop::{Accent, Action, DesktopState, Event};
use hiero::readiness::{ReadinessLevel, ReadinessSummary};

fn snapshot(level: ReadinessLevel, reasons: &[&str]) -> ReadinessSummary {
    ReadinessSummary {
        level,
        reasons: reasons.iter().map(|reason| (*reason).to_owned()).collect(),
        providers: Vec::new(),
    }
}

#[test]
fn three_timeouts_confirm_failure() {
    let mut state = DesktopState::new();
    assert_eq!(state.apply(Event::ProbeTimeout).accent, Accent::Amber);
    assert_eq!(state.apply(Event::ProbeTimeout).accent, Accent::Amber);
    assert_eq!(state.apply(Event::ProbeTimeout).accent, Accent::Red);
    assert_eq!(state.apply(Event::ProbeTimeout).accent, Accent::Red);
}

#[test]
fn authenticated_snapshot_recovers_after_probe_failure() {
    let mut state = DesktopState::new();
    state.apply(Event::ProbeTimeout);
    state.apply(Event::ProbeTimeout);
    state.apply(Event::ProbeTimeout);

    let view = state.apply(Event::Snapshot(snapshot(ReadinessLevel::Ready, &[])));

    assert_eq!(view.accent, Accent::Green);
    assert_eq!(view.reason, "Ready");
    assert!(!view.busy);
    assert!(!view.can_start);
}

#[test]
fn status_rows_project_to_expected_views() {
    let cases = [
        (ReadinessLevel::Ready, &[][..], Accent::Green, "Ready"),
        (
            ReadinessLevel::Starting,
            &["Loading multilingual model"][..],
            Accent::Amber,
            "Loading multilingual model",
        ),
        (
            ReadinessLevel::Degraded,
            &["Generation provider unavailable"][..],
            Accent::Amber,
            "Generation provider unavailable",
        ),
    ];

    for (level, reasons, accent, reason) in cases {
        let mut state = DesktopState::new();
        let view = state.apply(Event::Snapshot(snapshot(level, reasons)));
        assert_eq!(view.accent, accent);
        assert_eq!(view.reason, reason);
        assert!(!view.busy);
        assert!(!view.can_start);
        assert!(!view.exit_requested);
    }
}

#[test]
fn pending_restart_takes_precedence_over_snapshots_and_timeouts() {
    let mut state = DesktopState::new();
    state.apply(Event::Snapshot(snapshot(ReadinessLevel::Ready, &[])));
    state.apply(Event::Begin(Action::Restart));

    let view = state.apply(Event::Snapshot(snapshot(
        ReadinessLevel::Degraded,
        &["Provider failed"],
    )));
    assert_eq!(view.accent, Accent::Amber);
    assert_eq!(view.reason, "Restarting");
    assert!(view.busy);

    let view = state.apply(Event::ProbeTimeout);
    assert_eq!(view.accent, Accent::Amber);
    assert_eq!(view.reason, "Restarting");
    assert!(view.busy);
}

#[test]
fn stopped_server_enables_start() {
    let mut state = DesktopState::new();
    let view = state.apply(Event::Stopped);

    assert_eq!(view.accent, Accent::Red);
    assert_eq!(view.reason, "Stopped");
    assert!(view.can_start);
    assert!(!view.busy);
}

#[test]
fn missing_record_during_explicit_start_remains_transitional() {
    let mut state = DesktopState::new();
    state.apply(Event::Begin(Action::Start));

    let view = state.apply(Event::Stopped);

    assert_eq!(view.accent, Accent::Amber);
    assert_eq!(view.reason, "Starting");
    assert!(view.busy);
    assert!(!view.can_start);
}

#[test]
fn successful_start_keeps_missing_record_transitional_until_deadline_expires() {
    let mut state = DesktopState::new();
    state.apply(Event::Begin(Action::Start));
    state.apply(Event::Finished {
        action: Action::Start,
        error: None,
    });

    assert_eq!(state.apply(Event::Stopped).accent, Accent::Amber);
    assert_eq!(state.apply(Event::ProbeTimeout).accent, Accent::Amber);
    assert_eq!(state.apply(Event::ProbeTimeout).accent, Accent::Amber);
    assert_eq!(state.apply(Event::ProbeTimeout).accent, Accent::Amber);
    assert_eq!(state.apply(Event::Stopped).accent, Accent::Amber);

    let view = state.apply(Event::StartupDeadlineExpired);
    assert_eq!(view.accent, Accent::Red);
    assert_eq!(view.reason, "Server unavailable");
    assert!(view.can_start);
}

#[test]
fn ready_snapshot_during_start_survives_successful_completion() {
    let mut state = DesktopState::new();
    state.apply(Event::Begin(Action::Start));
    state.apply(Event::Snapshot(snapshot(ReadinessLevel::Ready, &[])));

    let view = state.apply(Event::Finished {
        action: Action::Start,
        error: None,
    });

    assert_eq!(view.accent, Accent::Green);
    assert_eq!(view.reason, "Ready");
    assert!(!view.busy);
}

#[test]
fn ready_snapshot_during_restart_survives_successful_completion() {
    let mut state = DesktopState::new();
    state.apply(Event::Begin(Action::Restart));
    state.apply(Event::Snapshot(snapshot(ReadinessLevel::Ready, &[])));

    let view = state.apply(Event::Finished {
        action: Action::Restart,
        error: None,
    });

    assert_eq!(view.accent, Accent::Green);
    assert_eq!(view.reason, "Ready");
    assert!(!view.busy);
}

#[test]
fn failed_quit_keeps_tray_open_and_failure_visible_across_snapshot() {
    let mut state = DesktopState::new();
    state.apply(Event::Begin(Action::Quit));
    let view = state.apply(Event::Finished {
        action: Action::Quit,
        error: Some("daemon did not stop".to_owned()),
    });

    assert_eq!(view.accent, Accent::Red);
    assert_eq!(view.reason, "daemon did not stop");
    assert!(!view.busy);
    assert!(!view.exit_requested);

    let view = state.apply(Event::Snapshot(snapshot(ReadinessLevel::Ready, &[])));
    assert_eq!(view.accent, Accent::Red);
    assert_eq!(view.reason, "daemon did not stop");
    assert!(!view.exit_requested);
}

#[test]
fn successful_quit_requests_exit_only_after_matching_completion() {
    let mut state = DesktopState::new();
    state.apply(Event::Begin(Action::Quit));
    let view = state.apply(Event::Finished {
        action: Action::Quit,
        error: None,
    });

    assert!(view.exit_requested);
    assert!(!view.busy);
}

#[test]
fn stale_completion_cannot_clear_a_newer_action() {
    let mut state = DesktopState::new();
    state.apply(Event::Begin(Action::Restart));
    state.apply(Event::Finished {
        action: Action::Restart,
        error: None,
    });
    state.apply(Event::Begin(Action::Quit));

    let view = state.apply(Event::Finished {
        action: Action::Restart,
        error: None,
    });

    assert_eq!(view.reason, "Stopping");
    assert!(view.busy);
    assert!(!view.exit_requested);
}

#[test]
fn invalid_identity_is_immediately_red_without_losing_busy_operation() {
    let mut state = DesktopState::new();
    state.apply(Event::Begin(Action::Restart));
    let view = state.apply(Event::InvalidIdentity);

    assert_eq!(view.accent, Accent::Red);
    assert_eq!(view.reason, "Invalid server identity");
    assert!(view.busy);
}
