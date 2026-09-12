use hiero::desktop::{Action, DesktopSettings, DesktopState, Event};
use hiero_desktop::menu::{MenuId, MenuProjection};

#[test]
fn menu_has_stable_spec_order_and_disabled_text_status() {
    let mut state = DesktopState::new();
    let view = state.apply(Event::Stopped);
    let menu = MenuProjection::from_view(view, &DesktopSettings::default());
    assert_eq!(
        menu.items.iter().map(|i| i.id).collect::<Vec<_>>(),
        vec![
            MenuId::Status,
            MenuId::OpenConsole,
            MenuId::Start,
            MenuId::Restart,
            MenuId::Autostart,
            MenuId::Quit
        ]
    );
    assert!(!menu.items[0].enabled);
    assert!(menu.items[0].label.contains("Stopped"));
    assert!(menu.items[2].enabled);
}

#[test]
fn start_requires_verified_absence_and_busy_disables_every_action() {
    let mut state = DesktopState::new();
    let settings = DesktopSettings {
        autostart: true,
        ..Default::default()
    };
    let menu = MenuProjection::from_view(state.apply(Event::ProbeTimeout), &settings);
    assert!(!menu.items[2].enabled);
    assert_eq!(menu.items[4].checked, Some(true));
    let menu = MenuProjection::from_view(state.apply(Event::Begin(Action::Restart)), &settings);
    assert!(menu.items.iter().all(|i| !i.enabled));
    assert_eq!(
        MenuId::parse("restart").unwrap().action(false),
        Some(Action::Restart)
    );
    assert_eq!(
        MenuId::parse("start-at-login").unwrap().action(true),
        Some(Action::SetAutostart(false))
    );
    assert_eq!(MenuId::parse("Restart"), None);
}
