//! Pure presentation and stable command identities shared by native adapters.
use hiero::desktop::{Action, DesktopSettings, View};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuId {
    Status,
    OpenConsole,
    Start,
    Restart,
    Autostart,
    Quit,
}
impl MenuId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::OpenConsole => "open-console",
            Self::Start => "start",
            Self::Restart => "restart",
            Self::Autostart => "start-at-login",
            Self::Quit => "quit",
        }
    }
    pub fn parse(id: &str) -> Option<Self> {
        [
            Self::Status,
            Self::OpenConsole,
            Self::Start,
            Self::Restart,
            Self::Autostart,
            Self::Quit,
        ]
        .into_iter()
        .find(|candidate| candidate.as_str() == id)
    }
    pub fn action(self, autostart: bool) -> Option<Action> {
        match self {
            Self::Status => None,
            Self::OpenConsole => Some(Action::OpenConsole),
            Self::Start => Some(Action::Start),
            Self::Restart => Some(Action::Restart),
            Self::Autostart => Some(Action::SetAutostart(!autostart)),
            Self::Quit => Some(Action::Quit),
        }
    }
}
#[derive(Debug)]
pub struct MenuItem {
    pub id: MenuId,
    pub label: String,
    pub enabled: bool,
    pub checked: Option<bool>,
}
#[derive(Debug)]
pub struct MenuProjection {
    pub items: Vec<MenuItem>,
}
impl MenuProjection {
    pub fn from_view(view: &View, settings: &DesktopSettings) -> Self {
        Self {
            items: vec![
                MenuItem {
                    id: MenuId::Status,
                    label: view.reason.clone(),
                    enabled: false,
                    checked: None,
                },
                MenuItem {
                    id: MenuId::OpenConsole,
                    label: "Open console".into(),
                    enabled: !view.busy,
                    checked: None,
                },
                MenuItem {
                    id: MenuId::Start,
                    label: "Start".into(),
                    enabled: view.can_start && !view.busy,
                    checked: None,
                },
                MenuItem {
                    id: MenuId::Restart,
                    label: "Restart".into(),
                    enabled: !view.busy,
                    checked: None,
                },
                MenuItem {
                    id: MenuId::Autostart,
                    label: "Start at login".into(),
                    enabled: !view.busy,
                    checked: Some(settings.autostart),
                },
                MenuItem {
                    id: MenuId::Quit,
                    label: "Quit".into(),
                    enabled: !view.busy,
                    checked: None,
                },
            ],
        }
    }
}
