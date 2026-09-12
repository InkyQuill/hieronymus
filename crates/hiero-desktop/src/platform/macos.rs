//! AppKit owns native presentation on the main thread. Menu/delegate callbacks
//! retain bounded enum flags; the shared worker owns every service/settings I/O.
use crate::{
    menu::{MenuId, MenuProjection},
    render_icon,
};
use hiero::{
    desktop::{
        Accent, Action, Controller, DesktopSettings, DesktopState, Event, ForegroundMode,
        LifecycleBackend, PollSchedule, SingletonOutcome, TraySingleton, View,
    },
    service::ServiceOptions,
};
use hieronymus::data_root::HieronymusConfig;
use objc2::{
    AnyThread, MainThreadOnly, define_class, msg_send, rc::Retained, runtime::ProtocolObject,
};
use objc2_app_kit::{
    NSAppearanceCustomization, NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate,
    NSApplicationTerminateReply, NSButtonCell, NSCellStyleMask, NSColor, NSColorSpace, NSEvent,
    NSEventModifierFlags, NSEventType, NSImage,
};
use objc2_foundation::{
    MainThreadMarker, NSData, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRunLoop,
    NSRunLoopCommonModes, NSSize, NSTimer,
};
use std::{
    cell::{Cell, RefCell},
    sync::atomic::{AtomicU32, Ordering},
};
use tray_icon::{
    TrayIcon, TrayIconBuilder,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem},
};
static MENU: AtomicU32 = AtomicU32::new(0);
thread_local! {
 static LAUNCHED:Cell<bool>=const{Cell::new(false)};
 static TEARDOWN:Cell<bool>=const{Cell::new(false)};
 static UI:RefCell<Option<Runtime>>=const{RefCell::new(None)};
 static INITIAL:RefCell<Option<(HieronymusConfig,ServiceOptions,TraySingleton)>>=const{RefCell::new(None)};
 static FAILURE:RefCell<Option<String>>=const{RefCell::new(None)};
}
define_class!(
    #[unsafe(super=NSObject)]
    #[thread_kind=MainThreadOnly]
    struct Delegate;
    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl NSApplicationDelegate for Delegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_launch(&self, _: &NSNotification) {
            LAUNCHED.set(true);
        }
        #[unsafe(method(applicationShouldTerminate:))]
        fn should_terminate(&self, _: &NSApplication) -> NSApplicationTerminateReply {
            TEARDOWN.set(true);
            NSApplicationTerminateReply::TerminateLater
        }
    }
);
struct NativeMenu {
    menu: Menu,
    items: Vec<MenuItem>,
    autostart: CheckMenuItem,
}
impl NativeMenu {
    fn new() -> Result<Self, String> {
        let menu = Menu::new();
        let items = [
            MenuId::Status,
            MenuId::OpenConsole,
            MenuId::Start,
            MenuId::Restart,
            MenuId::Quit,
        ]
        .into_iter()
        .map(|id| MenuItem::with_id(id.as_str(), "", false, None))
        .collect::<Vec<_>>();
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
        .map_err(|_| "Could not build macOS menu")?;
        Ok(Self {
            menu,
            items,
            autostart,
        })
    }
    fn update(&self, view: &View, settings: &DesktopSettings, known: bool) {
        let projection = MenuProjection::from_view(view, settings);
        let mut normal = self.items.iter();
        for item in projection.items {
            if item.id == MenuId::Autostart {
                self.autostart.set_checked(item.checked.unwrap_or(false));
                self.autostart.set_enabled(item.enabled && known);
                self.autostart.set_text(if known {
                    &item.label
                } else {
                    "Start at login (unavailable)"
                });
            } else if let Some(native) = normal.next() {
                native.set_text(item.label);
                native.set_enabled(item.enabled);
            }
        }
    }
}
type IconKey = ([u8; 3], [u8; 3], [u8; 3], u32);
struct Runtime {
    controller: Option<Controller>,
    _singleton: TraySingleton,
    native: NativeMenu,
    tray: TrayIcon,
    state: DesktopState,
    view: View,
    settings: DesktopSettings,
    known: bool,
    error: Option<String>,
    last: Option<IconKey>,
}
impl Runtime {
    fn new(
        config: HieronymusConfig,
        options: ServiceOptions,
        mut singleton: TraySingleton,
    ) -> Result<Self, String> {
        let native = NativeMenu::new()?;
        // This function is called only by a running main-loop timer after AppKit's
        // applicationDidFinishLaunching notification, never before NSApplication.run.
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(native.menu.clone()))
            .with_icon_as_template(false)
            .build()
            .map_err(|_| "Could not create macOS status item")?;
        let registration =
            hiero::desktop::macos_registration::MacosRegistration::new(options.clone());
        let mut controller = Controller::spawn_with_schedule(
            LifecycleBackend::with_service_options(config.clone(), options, registration),
            PollSchedule::default(),
        );
        controller.attach_control(&config, &mut singleton)?;
        let mut state = DesktopState::new();
        let view = state
            .apply(Event::Preferences {
                settings: None,
                error: None,
            })
            .clone();
        let mut runtime = Self {
            controller: Some(controller),
            _singleton: singleton,
            native,
            tray,
            state,
            view,
            settings: DesktopSettings::default(),
            known: false,
            error: None,
            last: None,
        };
        runtime.refresh()?;
        if !std::env::args().any(|a| a == "--resume") {
            runtime.controller.as_ref().unwrap().submit(Action::Start)?;
        }
        Ok(runtime)
    }
    fn refresh(&mut self) -> Result<(), String> {
        let controller = self.controller.as_ref().unwrap();
        while let Some(event) = controller.try_event() {
            if let Event::Preferences { settings, error } = &event {
                self.known = settings.is_some();
                if let Some(settings) = settings {
                    self.settings = settings.clone();
                }
                self.error = error.clone();
            }
            self.view = self.state.apply(event).clone();
        }
        let mut projected = self.view.clone();
        if let Some(error) = &self.error {
            projected.reason.push_str(" — ");
            projected.reason.push_str(error);
        }
        self.native.update(&projected, &self.settings, self.known);
        let flags = MENU.swap(0, Ordering::AcqRel);
        if !TEARDOWN.get() {
            for (bit, id) in [
                (1, MenuId::OpenConsole),
                (2, MenuId::Start),
                (4, MenuId::Restart),
                (8, MenuId::Autostart),
                (16, MenuId::Quit),
            ] {
                if flags & bit != 0
                    && MenuProjection::from_view(&self.view, &self.settings)
                        .items
                        .iter()
                        .any(|i| i.id == id && i.enabled)
                    && (id != MenuId::Autostart || self.known)
                {
                    if let Some(action) = id.action(self.settings.autostart) {
                        let _ = controller.submit(action);
                    }
                    break;
                }
            }
        }
        let item = self
            .tray
            .ns_status_item()
            .ok_or("Status item is unavailable")?;
        let button = item
            .button(MainThreadMarker::new().ok_or("AppKit requires main thread")?)
            .ok_or("Status button is unavailable")?;
        let appearance = button.effectiveAppearance();
        let colors = Cell::new(None);
        appearance.performAsCurrentDrawingAppearance(&block2::RcBlock::new(|| {
            let resolve = |color: Retained<NSColor>| {
                color
                    .colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())
                    .map(|c| {
                        [c.redComponent(), c.greenComponent(), c.blueComponent()]
                            .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
                    })
            };
            colors.set(
                resolve(NSColor::labelColor()).zip(resolve(NSColor::selectedMenuItemTextColor())),
            );
        }));
        let (mut normal, highlight) = colors
            .get()
            .ok_or("Could not resolve status-button foreground")?;
        match self.settings.foreground {
            ForegroundMode::Light => normal = [245; 3],
            ForegroundMode::Dark => normal = [24; 3],
            ForegroundMode::Auto => {}
        }
        let scale = button
            .window()
            .map(|w| w.backingScaleFactor())
            .unwrap_or(1.0)
            .clamp(1.0, 4.0);
        let pixels = super::macos_pixels::raster_size(scale);
        let accent = match self.view.accent {
            Accent::Green => [46, 173, 104],
            Accent::Amber => [229, 167, 43],
            Accent::Red => [217, 74, 72],
        };
        if self.last != Some((normal, highlight, accent, pixels)) {
            let normal_image = image(normal, accent, pixels)?;
            let alternate = image(highlight, accent, pixels)?;
            button.setImage(Some(&normal_image));
            button.setAlternateImage(Some(&alternate));
            if let Some(cell) = button
                .cell()
                .and_then(|c| c.downcast::<NSButtonCell>().ok())
            {
                cell.setHighlightsBy(
                    NSCellStyleMask::ContentsCellMask | NSCellStyleMask::ChangeBackgroundCellMask,
                );
                cell.setImageDimsWhenDisabled(false);
            }
            self.last = Some((normal, highlight, accent, pixels));
        }
        Ok(())
    }
}
fn image(ink: [u8; 3], accent: [u8; 3], pixels: u32) -> Result<Retained<NSImage>, String> {
    let rgba = render_icon(ink, accent, pixels)?;
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, pixels, pixels);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .map_err(|_| "Could not encode status image")?
            .write_image_data(&rgba)
            .map_err(|_| "Could not encode status image")?;
    }
    let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(&bytes))
        .ok_or("Could not create status image")?;
    image.setSize(NSSize::new(18.0, 18.0));
    image.setTemplate(false);
    Ok(image)
}
fn stop_loop(app: &NSApplication) {
    let mtm = MainThreadMarker::new().expect("AppKit owner thread");
    UI.with_borrow(|ui| {
        if let Some(menu) = ui
            .as_ref()
            .and_then(|ui| ui.tray.ns_status_item())
            .and_then(|item| item.menu(mtm))
        {
            menu.cancelTracking();
        }
    });
    app.stop(None);
    if let Some(event)=NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(NSEventType::ApplicationDefined,NSPoint::new(0.0,0.0),NSEventModifierFlags::empty(),0.0,0,None,0,0,0){app.postEvent_atStart(&event,true);}
}
fn tick() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    if TEARDOWN.get() {
        stop_loop(&app);
        return;
    }
    if !LAUNCHED.get() {
        return;
    }
    let result = UI.with_borrow_mut(|ui| {
        if ui.is_none() {
            let (config, options, singleton) = INITIAL
                .with_borrow_mut(Option::take)
                .ok_or("Missing desktop initialization")?;
            *ui = Some(Runtime::new(config, options, singleton)?);
        }
        let ui = ui.as_mut().unwrap();
        ui.refresh()?;
        Ok::<_, String>(ui.view.exit_requested)
    });
    match result {
        Ok(true) => stop_loop(&app),
        Ok(false) => {}
        Err(error) => {
            FAILURE.with_borrow_mut(|e| *e = Some(error));
            stop_loop(&app);
        }
    }
}
pub fn run(config: HieronymusConfig) -> Result<(), String> {
    let options = ServiceOptions {
        data_root: config.data_root().into(),
        unit_dir: hiero::service::default_unit_dir(),
        binary: hiero::desktop::launch::stable_cli(
            &std::env::current_exe().map_err(|_| "Could not locate helper")?,
        )?,
        use_manager: true,
    };
    run_with_service_options(config, options)
}
pub fn run_with_service_options(
    config: HieronymusConfig,
    options: ServiceOptions,
) -> Result<(), String> {
    let singleton = match TraySingleton::acquire_or_existing(&config, "native")
        .map_err(|_| "Could not own the macOS graphical session")?
    {
        SingletonOutcome::AlreadyRunning => return Ok(()),
        SingletonOutcome::Acquired(guard) => guard,
    };
    let mtm = MainThreadMarker::new().ok_or("AppKit must run on the main thread")?;
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(mtm), init] };
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    INITIAL.with_borrow_mut(|initial| *initial = Some((config, options, singleton)));
    MenuEvent::set_event_handler(Some(|event: MenuEvent| {
        let bit = match MenuId::parse(event.id.as_ref()) {
            Some(MenuId::OpenConsole) => 1,
            Some(MenuId::Start) => 2,
            Some(MenuId::Restart) => 4,
            Some(MenuId::Autostart) => 8,
            Some(MenuId::Quit) => {
                UI.with_borrow(|ui| {
                    if let Some(controller) = ui.as_ref().and_then(|r| r.controller.as_ref()) {
                        controller
                            .quit_intent_handle()
                            .store(true, Ordering::Release);
                    }
                });
                16
            }
            _ => 0,
        };
        MENU.fetch_or(bit, Ordering::Release);
    }));
    // A capture-free block is sendable; its runtime access is main-thread checked.
    // Common modes also refresh appearance/scale during status-menu tracking.
    let block = block2::RcBlock::new(|_| tick());
    let timer = unsafe { NSTimer::timerWithTimeInterval_repeats_block(0.1, true, &block) };
    unsafe { NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
    app.run();
    timer.invalidate();
    MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
    let runtime = UI.with_borrow_mut(Option::take);
    let shutdown = super::shutdown_owner::shutdown_owned(
        runtime,
        |runtime| runtime.controller.take(),
        Controller::shutdown,
    );
    if TEARDOWN.get() {
        app.replyToApplicationShouldTerminate(true);
    }
    app.setDelegate(None);
    shutdown?;
    match FAILURE.with_borrow_mut(Option::take) {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
