use crate::readiness::{ReadinessLevel, ReadinessSummary};

const MAX_CONSECUTIVE_FAILURES: u8 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accent {
    Green,
    Amber,
    Red,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    OpenConsole,
    Start,
    Restart,
    SetAutostart(bool),
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub accent: Accent,
    pub reason: String,
    pub busy: bool,
    pub can_start: bool,
    pub exit_requested: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Snapshot(ReadinessSummary),
    ProbeTimeout,
    StartupDeadlineExpired,
    InvalidIdentity,
    Stopped,
    Begin(Action),
    Finished {
        action: Action,
        error: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopState {
    consecutive_failures: u8,
    view: View,
    status_view: View,
    pending_action: Option<Action>,
    operation_failure: Option<String>,
    awaiting_startup: bool,
    snapshot_observed_during_lifecycle: bool,
}

impl Default for DesktopState {
    fn default() -> Self {
        Self::new()
    }
}

impl DesktopState {
    #[must_use]
    pub fn new() -> Self {
        let view = View {
            accent: Accent::Amber,
            reason: "Checking".to_owned(),
            busy: false,
            can_start: false,
            exit_requested: false,
        };
        Self {
            consecutive_failures: 0,
            status_view: view.clone(),
            view,
            pending_action: None,
            operation_failure: None,
            awaiting_startup: false,
            snapshot_observed_during_lifecycle: false,
        }
    }

    pub fn apply(&mut self, event: Event) -> &View {
        match event {
            Event::Snapshot(summary) => self.apply_snapshot(summary),
            Event::ProbeTimeout => self.apply_probe_timeout(),
            Event::StartupDeadlineExpired => self.apply_startup_deadline_expired(),
            Event::InvalidIdentity => self.apply_invalid_identity(),
            Event::Stopped => self.apply_stopped(),
            Event::Begin(action) => self.begin(action),
            Event::Finished { action, error } => self.finish(action, error),
        }
        &self.view
    }

    fn apply_snapshot(&mut self, summary: ReadinessSummary) {
        if matches!(self.pending_action, Some(Action::Start | Action::Restart)) {
            self.snapshot_observed_during_lifecycle = true;
        }
        self.consecutive_failures = 0;
        self.awaiting_startup = false;
        self.status_view = view_from_summary(summary);
        self.show_status_when_unblocked();
    }

    fn apply_probe_timeout(&mut self) {
        if self.pending_action.is_some() || self.operation_failure.is_some() {
            self.snapshot_observed_during_lifecycle = false;
            return;
        }

        if self.awaiting_startup {
            self.view = View {
                accent: Accent::Amber,
                reason: "Starting".to_owned(),
                busy: false,
                can_start: false,
                exit_requested: false,
            };
            return;
        }

        self.consecutive_failures = self
            .consecutive_failures
            .saturating_add(1)
            .min(MAX_CONSECUTIVE_FAILURES);

        if self.consecutive_failures < MAX_CONSECUTIVE_FAILURES {
            self.status_view = View {
                accent: Accent::Amber,
                reason: "Checking".to_owned(),
                busy: false,
                can_start: false,
                exit_requested: false,
            };
        } else {
            self.status_view = View {
                accent: Accent::Red,
                reason: "Server unavailable".to_owned(),
                busy: false,
                can_start: false,
                exit_requested: false,
            };
        }
        self.view = self.status_view.clone();
    }

    fn apply_startup_deadline_expired(&mut self) {
        let pending_lifecycle =
            matches!(self.pending_action, Some(Action::Start | Action::Restart));
        if !self.awaiting_startup && !pending_lifecycle {
            return;
        }

        self.awaiting_startup = false;
        self.snapshot_observed_during_lifecycle = false;
        self.status_view = View {
            accent: Accent::Red,
            reason: "Server unavailable".to_owned(),
            busy: false,
            can_start: true,
            exit_requested: false,
        };
        self.view = self.status_view.clone();
        if pending_lifecycle {
            self.view.busy = true;
            self.view.can_start = false;
        }
    }

    fn apply_invalid_identity(&mut self) {
        self.consecutive_failures = MAX_CONSECUTIVE_FAILURES;
        self.awaiting_startup = false;
        self.snapshot_observed_during_lifecycle = false;
        self.status_view = View {
            accent: Accent::Red,
            reason: "Invalid server identity".to_owned(),
            busy: false,
            can_start: false,
            exit_requested: false,
        };
        if self.operation_failure.is_none() {
            self.view = self.status_view.clone();
            self.view.busy = self.pending_action.is_some();
        }
    }

    fn apply_stopped(&mut self) {
        let starting = self.awaiting_startup
            || matches!(self.pending_action, Some(Action::Start | Action::Restart));
        if starting {
            self.snapshot_observed_during_lifecycle = false;
            if self.pending_action.is_none() && self.operation_failure.is_none() {
                self.view = View {
                    accent: Accent::Amber,
                    reason: "Starting".to_owned(),
                    busy: false,
                    can_start: false,
                    exit_requested: false,
                };
            }
            return;
        }

        self.consecutive_failures = MAX_CONSECUTIVE_FAILURES;
        self.status_view = View {
            accent: Accent::Red,
            reason: "Stopped".to_owned(),
            busy: false,
            can_start: true,
            exit_requested: false,
        };

        if self.pending_action.is_some() || self.operation_failure.is_some() {
            return;
        }
        self.view = self.status_view.clone();
    }

    fn begin(&mut self, action: Action) {
        if self.pending_action.is_some() {
            return;
        }
        self.operation_failure = None;
        self.view.exit_requested = false;
        if matches!(action, Action::Start | Action::Restart) {
            self.snapshot_observed_during_lifecycle = false;
        }
        self.pending_action = Some(action.clone());
        self.view = View {
            accent: match action {
                Action::OpenConsole | Action::SetAutostart(_) => self.status_view.accent.clone(),
                Action::Start | Action::Restart | Action::Quit => Accent::Amber,
            },
            reason: action_progress_reason(&action).to_owned(),
            busy: true,
            can_start: false,
            exit_requested: false,
        };
    }

    fn finish(&mut self, action: Action, error: Option<String>) {
        if self.pending_action.as_ref() != Some(&action) {
            return;
        }
        self.pending_action = None;

        if let Some(error) = error {
            self.awaiting_startup = false;
            self.snapshot_observed_during_lifecycle = false;
            self.operation_failure = Some(error.clone());
            let lifecycle_failure =
                matches!(action, Action::Start | Action::Restart | Action::Quit);
            self.view = View {
                accent: if lifecycle_failure {
                    Accent::Red
                } else {
                    self.status_view.accent.clone()
                },
                reason: error,
                busy: false,
                can_start: matches!(action, Action::Start | Action::Restart),
                exit_requested: false,
            };
            return;
        }

        match action {
            Action::Quit => {
                self.awaiting_startup = false;
                self.snapshot_observed_during_lifecycle = false;
                self.view.busy = false;
                self.view.exit_requested = true;
            }
            Action::Start | Action::Restart => {
                self.consecutive_failures = 0;
                if self.snapshot_observed_during_lifecycle {
                    self.snapshot_observed_during_lifecycle = false;
                    self.awaiting_startup = false;
                    self.view = self.status_view.clone();
                } else {
                    self.awaiting_startup = true;
                    self.view = View {
                        accent: Accent::Amber,
                        reason: "Starting".to_owned(),
                        busy: false,
                        can_start: false,
                        exit_requested: false,
                    };
                }
            }
            Action::OpenConsole | Action::SetAutostart(_) => {
                self.view = self.status_view.clone();
            }
        }
    }

    fn show_status_when_unblocked(&mut self) {
        if self.pending_action.is_none() && self.operation_failure.is_none() {
            self.view = self.status_view.clone();
        }
    }
}

fn view_from_summary(summary: ReadinessSummary) -> View {
    let (accent, fallback) = match summary.level {
        ReadinessLevel::Ready => (Accent::Green, "Ready"),
        ReadinessLevel::Degraded => (Accent::Amber, "Limited functionality"),
        ReadinessLevel::Starting => (Accent::Amber, "Starting"),
    };
    View {
        accent,
        reason: if summary.reasons.is_empty() {
            fallback.to_owned()
        } else {
            summary.reasons.join("; ")
        },
        busy: false,
        can_start: false,
        exit_requested: false,
    }
}

fn action_progress_reason(action: &Action) -> &'static str {
    match action {
        Action::OpenConsole => "Opening console",
        Action::Start => "Starting",
        Action::Restart => "Restarting",
        Action::SetAutostart(_) => "Updating start at login",
        Action::Quit => "Stopping",
    }
}
