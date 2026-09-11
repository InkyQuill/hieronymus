//! GTK owns every native object for the loop lifetime. Callbacks enqueue IDs;
//! authenticated requests, settings readback and service actions run on one worker.
mod host;
mod theme;
use crate::{
    events,
    menu::{MenuId, MenuProjection},
    render_icon,
};
use gtk::glib;
use hiero::{
    desktop::{
        Accent, Action, Controller, DesktopSettings, DesktopState, Event, LifecycleBackend,
        PollSchedule, SettingsStore, SingletonOutcome, TraySingleton, UnsupportedAutostart, View,
    },
    service,
};
use hieronymus::data_root::HieronymusConfig;
use std::{
    cell::RefCell,
    os::fd::AsRawFd,
    rc::Rc,
    sync::{Arc, Mutex},
};
use tray_icon::{
    TrayIcon, TrayIconBuilder,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem},
};

struct NativeMenu {
    menu: Menu,
    items: Vec<MenuItem>,
    autostart: CheckMenuItem,
}
impl NativeMenu {
    fn new() -> Result<Self, String> {
        let menu = Menu::new();
        let items: Vec<_> = [
            MenuId::Status,
            MenuId::OpenConsole,
            MenuId::Start,
            MenuId::Restart,
            MenuId::Quit,
        ]
        .into_iter()
        .map(|id| MenuItem::with_id(id.as_str(), "", false, None))
        .collect();
        let autostart = CheckMenuItem::with_id(
            MenuId::Autostart.as_str(),
            "Start at login (unavailable)",
            false,
            false,
            None,
        );
        menu.append_items(&[
            &items[0], &items[1], &items[2], &items[3], &autostart, &items[4],
        ])
        .map_err(|error| error.to_string())?;
        Ok(Self {
            menu,
            items,
            autostart,
        })
    }
    fn update(&self, projection: &MenuProjection, preferences_known: bool) {
        let mut normal = self.items.iter();
        for item in &projection.items {
            if item.id == MenuId::Autostart {
                self.autostart.set_text(if preferences_known {
                    &item.label
                } else {
                    "Start at login (unavailable)"
                });
                self.autostart.set_checked(item.checked.unwrap_or(false));
                self.autostart
                    .set_enabled(item.enabled && preferences_known);
            } else if let Some(native) = normal.next() {
                native.set_text(&item.label);
                native.set_enabled(item.enabled);
            }
        }
    }
}
struct Presentation {
    state: DesktopState,
    view: View,
    settings: DesktopSettings,
    preferences_known: bool,
    preferences_error: Option<String>,
    host_error: Arc<Mutex<Option<String>>>,
    appearance: Arc<Mutex<theme::Appearance>>,
    native: NativeMenu,
    tray: TrayIcon,
    last_icon: Option<([u8; 3], [u8; 3])>,
    last_diagnostic: String,
}
impl Presentation {
    fn drain(&mut self, controller: &Controller) {
        while let Some(event) = controller.try_event() {
            if let Event::Preferences { settings, error } = &event {
                self.preferences_known = settings.is_some();
                if let Some(settings) = settings {
                    self.settings = settings.clone();
                }
                self.preferences_error = error.clone();
            }
            self.view = self.state.apply(event).clone();
        }
    }
    fn update(&mut self) {
        let mut projected = self.view.clone();
        let host = self
            .host_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let diagnostics: Vec<_> = host
            .iter()
            .chain(self.preferences_error.iter())
            .cloned()
            .collect();
        let diagnostic = diagnostics.join("; ");
        if diagnostic != self.last_diagnostic {
            if !diagnostic.is_empty() {
                eprintln!("hiero-desktop: {diagnostic}");
            } else if !self.last_diagnostic.is_empty() {
                eprintln!("hiero-desktop: Tray host and desktop preferences available");
            }
            self.last_diagnostic = diagnostic.clone();
        }
        if !diagnostic.is_empty() {
            projected.reason.push_str(" — ");
            projected.reason.push_str(&diagnostic);
        }
        self.native.update(
            &MenuProjection::from_view(&projected, &self.settings),
            self.preferences_known,
        );
        let panel = self
            .appearance
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .foreground;
        let ink = theme::foreground(self.settings.foreground, panel);
        let accent = match self.view.accent {
            Accent::Green => [46, 173, 104],
            Accent::Amber => [229, 167, 43],
            Accent::Red => [217, 74, 72],
        };
        if self.last_icon != Some((ink, accent)) {
            match icon(ink, accent).and_then(|icon| {
                self.tray
                    .set_icon(Some(icon))
                    .map_err(|error| error.to_string())
            }) {
                Ok(()) => self.last_icon = Some((ink, accent)),
                Err(_) => {
                    self.native.items[0].set_text(format!(
                        "{} — Could not update the tray icon",
                        projected.reason
                    ));
                }
            }
        }
    }
}
fn icon(ink: [u8; 3], accent: [u8; 3]) -> Result<tray_icon::Icon, String> {
    tray_icon::Icon::from_rgba(render_icon(ink, accent, 48)?, 48, 48)
        .map_err(|error| error.to_string())
}
pub fn run(config: HieronymusConfig) -> Result<(), String> {
    let _singleton = match TraySingleton::acquire_or_existing(&config, "native")
        .map_err(|error| format!("Could not own the desktop session: {error}"))?
    {
        SingletonOutcome::AlreadyRunning => return Ok(()),
        SingletonOutcome::Acquired(guard) => guard,
    };
    let executable =
        std::env::current_exe().map_err(|_| "Could not locate the desktop installation")?;
    let cli = hiero::desktop::launch::sibling_binary(&executable, "hiero")?;
    gtk::init().map_err(
        |_| "Could not connect to the graphical desktop; run hiero status for headless diagnostics",
    )?;
    let (sender, receiver) =
        events::channel().map_err(|_| "Could not create the desktop wake channel")?;
    let native = NativeMenu::new()?;
    // Per-helper temporary names in a private directory avoid cross-root icon collisions.
    let icon_directory = config.data_root().join("tray-icons");
    std::fs::create_dir_all(&icon_directory)
        .map_err(|_| "Could not create the tray icon directory")?;
    let tray = TrayIconBuilder::new().with_id(format!("hieronymus-{}", std::process::id())).with_menu(Box::new(native.menu.clone())).with_temp_dir_path(&icon_directory).with_icon(icon([245,245,245], [229,167,43])?).build().map_err(|error| format!("Could not create AppIndicator: {error}; install the desktop native library prerequisites"))?;
    let settings = SettingsStore::new(&config).load().unwrap_or_default();
    let host_error = Arc::new(Mutex::new(Some(host::MISSING.into())));
    let appearance = Arc::new(Mutex::new(theme::Appearance::default()));
    let _host = host::HostObserver::new(host_error.clone(), sender.clone());
    let _theme = theme::ThemeObserver::new(appearance.clone(), sender.clone());
    let options = service::ServiceOptions {
        data_root: config.data_root().to_path_buf(),
        unit_dir: service::default_unit_dir(),
        binary: cli,
        use_manager: true,
    };
    let wake = sender.clone();
    let controller = Rc::new(Controller::spawn_with_notifier(
        LifecycleBackend::with_service_options(config, options, UnsupportedAutostart),
        PollSchedule::default(),
        move || wake.wake(),
    ));
    let mut state = DesktopState::new();
    let view = state
        .apply(Event::Preferences {
            settings: None,
            error: None,
        })
        .clone();
    let presentation = Rc::new(RefCell::new(Presentation {
        state,
        view,
        settings,
        preferences_known: false,
        preferences_error: None,
        host_error,
        appearance,
        native,
        tray,
        last_icon: None,
        last_diagnostic: String::new(),
    }));
    presentation.borrow_mut().update();
    let menu_sender = sender.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if let Some(id) = MenuId::parse(event.id.as_ref()) {
            menu_sender.submit(id);
        }
    }));
    let loop_controller = controller.clone();
    let loop_presentation = presentation.clone();
    let source = glib::source::unix_fd_add_local(
        receiver.socket.as_raw_fd(),
        glib::IOCondition::IN,
        move |_, _| {
            let action = receiver.drain();
            let mut ui = loop_presentation.borrow_mut();
            ui.drain(&loop_controller);
            if ui.view.exit_requested {
                gtk::main_quit();
                return glib::ControlFlow::Continue;
            }
            if let Some(id) = action {
                let eligible = MenuProjection::from_view(&ui.view, &ui.settings)
                    .items
                    .iter()
                    .any(|item| item.id == id && item.enabled)
                    && (id != MenuId::Autostart || ui.preferences_known);
                if eligible && let Some(action) = id.action(ui.settings.autostart) {
                    let _ = loop_controller.submit(action);
                }
            }
            ui.update();
            glib::ControlFlow::Continue
        },
    );
    // Only a fresh, initialized helper requests startup. Polling stays passive.
    controller.submit(Action::Start)?;
    gtk::main();
    MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
    source.remove();
    drop(presentation);
    Rc::try_unwrap(controller)
        .map_err(|_| "Desktop controller still in use")?
        .shutdown()
}
