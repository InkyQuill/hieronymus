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
fn probe_failures_during_non_lifecycle_action_surface_after_completion() {
    for action in [Action::OpenConsole, Action::SetAutostart(true)] {
        let mut state = DesktopState::new();
        state.apply(Event::Snapshot(snapshot(ReadinessLevel::Ready, &[])));
        state.apply(Event::Begin(action.clone()));

        assert!(state.apply(Event::ProbeTimeout).busy);
        assert!(state.apply(Event::ProbeTimeout).busy);
        let pending = state.apply(Event::ProbeTimeout);
        assert!(pending.busy);
        assert_eq!(
            pending.reason,
            match action {
                Action::OpenConsole => "Opening console",
                Action::SetAutostart(_) => "Updating start at login",
                _ => unreachable!(),
            }
        );

        let view = state.apply(Event::Finished {
            action,
            error: None,
        });
        assert_eq!(view.accent, Accent::Red);
        assert_eq!(view.reason, "Server unavailable");
        assert!(!view.busy);
    }
}

#[test]
fn probe_failures_during_quit_update_status_without_clearing_busy_state() {
    let mut state = DesktopState::new();
    state.apply(Event::Snapshot(snapshot(ReadinessLevel::Ready, &[])));
    state.apply(Event::Begin(Action::Quit));

    state.apply(Event::ProbeTimeout);
    state.apply(Event::ProbeTimeout);
    let pending = state.apply(Event::ProbeTimeout);
    assert_eq!(pending.accent, Accent::Amber);
    assert_eq!(pending.reason, "Stopping");
    assert!(pending.busy);

    let view = state.apply(Event::Finished {
        action: Action::Quit,
        error: None,
    });
    assert_eq!(view.accent, Accent::Red);
    assert_eq!(view.reason, "Server unavailable");
    assert!(!view.busy);
    assert!(view.exit_requested);
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
    assert!(!view.can_start);
    assert_eq!(state.apply(Event::ProbeTimeout).accent, Accent::Red);
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
fn failed_quit_keeps_reason_and_no_exit_but_tracks_current_health() {
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
    assert_eq!(view.accent, Accent::Green);
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

#[test]
fn failed_start_never_proves_absence_and_authenticated_snapshot_recovers() {
    for action in [Action::Start, Action::Restart] {
        let mut state = DesktopState::new();
        state.apply(Event::Begin(action.clone()));
        let view = state.apply(Event::Finished {
            action,
            error: Some("Operation busy".into()),
        });
        assert!(!view.can_start);
        assert_eq!(view.accent, Accent::Red);
        let view = state.apply(Event::Snapshot(snapshot(ReadinessLevel::Ready, &[])));
        assert_eq!(view.accent, Accent::Green);
        assert_eq!(view.reason, "Ready");
    }
}
#[test]
fn browser_error_reason_tracks_changing_health_accent() {
    let mut state = DesktopState::new();
    state.apply(Event::Snapshot(snapshot(ReadinessLevel::Ready, &[])));
    state.apply(Event::Begin(Action::OpenConsole));
    state.apply(Event::Finished {
        action: Action::OpenConsole,
        error: Some("Browser failed".into()),
    });
    assert_eq!(state.apply(Event::ProbeTimeout).accent, Accent::Amber);
    state.apply(Event::ProbeTimeout);
    let view = state.apply(Event::ProbeTimeout);
    assert_eq!(view.accent, Accent::Red);
    assert_eq!(view.reason, "Browser failed");
    let view = state.apply(Event::Snapshot(snapshot(ReadinessLevel::Degraded, &[])));
    assert_eq!(view.accent, Accent::Amber);
    assert_eq!(view.reason, "Browser failed");
}
