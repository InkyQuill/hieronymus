//! Main-thread Win32 presentation. A hidden TOP-LEVEL window receives broadcasts;
//! callbacks retain bounded flags across modal loops; the common worker owns lifecycle I/O.
use crate::{
    menu::{MenuId, MenuProjection},
    render_icon,
};
use hiero::{
    desktop::{
        Accent, Action, Controller, DesktopSettings, DesktopState, Event, ForegroundMode,
        LifecycleBackend, PollSchedule, SingletonOutcome, TraySingleton,
        windows_registration::WindowsRegistration,
    },
    service::{self, ServiceOptions},
};
use hieronymus::data_root::HieronymusConfig;
use std::{
    mem::{size_of, zeroed},
    ptr,
    sync::OnceLock,
};
use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{LibraryLoader::GetModuleHandleW, Registry::*},
    UI::{HiDpi::*, Shell::*, WindowsAndMessaging::*},
};
const WAKE: u32 = WM_APP + 1;
const TRAY: u32 = WM_APP + 2;
use super::windows_events::{APPEARANCE, EXPLORER, MENU, PendingEvents, TEARDOWN};
thread_local! {
    // One owner window per UI thread. Callbacks never borrow the presentation state.
    static PENDING: PendingEvents = PendingEvents::default();
}
const KEYSELECT: u32 = NIN_SELECT | NINF_KEY;
static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // No COM, settings, browser or daemon calls in native callbacks.
    if message == WAKE {
        PENDING.with(PendingEvents::controller_wake);
        return 0;
    }
    let event = if TASKBAR_CREATED.get().is_some_and(|id| *id == message) {
        Some(EXPLORER)
    } else {
        match message {
            TRAY if matches!(
                lparam as u32 & 0xffff,
                WM_RBUTTONUP | WM_CONTEXTMENU | WM_LBUTTONUP | NIN_SELECT | KEYSELECT
            ) =>
            {
                Some(MENU)
            }
            WM_SETTINGCHANGE | WM_THEMECHANGED | WM_DPICHANGED | WM_DISPLAYCHANGE => {
                Some(APPEARANCE)
            }
            WM_QUERYENDSESSION => return 1,
            WM_ENDSESSION if wparam != 0 => Some(TEARDOWN),
            WM_CLOSE => Some(TEARDOWN),
            _ => None,
        }
    };
    if let Some(event) = event {
        if PENDING.with(|pending| pending.record(event)) {
            unsafe {
                PostMessageW(window, WAKE, 0, 0);
            }
        }
        if event == TEARDOWN {
            // Finish menu modality so the owner can drain confirmed teardown promptly.
            unsafe {
                EndMenu();
            }
        }
        return 0;
    }
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}
struct Window(HWND);
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.0);
        }
    }
}
struct NativeIcon {
    window: HWND,
    handle: HICON,
    present: bool,
}
impl NativeIcon {
    fn data(&self) -> NOTIFYICONDATAW {
        let mut data: NOTIFYICONDATAW = unsafe { zeroed() };
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = self.window;
        data.uID = 1;
        data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
        data.uCallbackMessage = TRAY;
        data.hIcon = self.handle;
        for (slot, value) in data.szTip.iter_mut().zip("Hieronymus".encode_utf16()) {
            *slot = value;
        }
        data
    }
    fn update(&mut self, ink: [u8; 3], accent: [u8; 3], size: u32) -> Result<(), String> {
        let rgba = render_icon(ink, accent, size)?;
        let next = icon_handle(&rgba, size)?;
        let previous = self.handle;
        self.handle = next;
        let mut data = self.data();
        if unsafe { Shell_NotifyIconW(if self.present { NIM_MODIFY } else { NIM_ADD }, &data) } == 0
        {
            self.handle = previous;
            unsafe {
                DestroyIcon(next);
            }
            return Err("Could not publish the Windows notification icon".into());
        }
        self.present = true;
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        unsafe {
            Shell_NotifyIconW(NIM_SETVERSION, &data);
            if !previous.is_null() {
                DestroyIcon(previous);
            }
        }
        Ok(())
    }
}
impl Drop for NativeIcon {
    fn drop(&mut self) {
        unsafe {
            if self.present {
                Shell_NotifyIconW(NIM_DELETE, &self.data());
            }
            if !self.handle.is_null() {
                DestroyIcon(self.handle);
            }
        }
    }
}
fn icon_handle(rgba: &[u8], size: u32) -> Result<HICON, String> {
    let (bgra, mask_bytes) = super::windows_pixels::convert(rgba, size)?;
    unsafe {
        let mut header: BITMAPINFO = zeroed();
        header.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        header.bmiHeader.biWidth = size as i32;
        header.bmiHeader.biHeight = -(size as i32);
        header.bmiHeader.biPlanes = 1;
        header.bmiHeader.biBitCount = 32;
        header.bmiHeader.biCompression = BI_RGB;
        let mut pixels = ptr::null_mut();
        let color = CreateDIBSection(
            ptr::null_mut(),
            &header,
            DIB_RGB_COLORS,
            &mut pixels,
            ptr::null_mut(),
            0,
        );
        if color.is_null() || pixels.is_null() {
            if !color.is_null() {
                DeleteObject(color);
            }
            return Err("Could not allocate native tray bitmap".into());
        }
        std::slice::from_raw_parts_mut(pixels.cast::<u8>(), bgra.len()).copy_from_slice(&bgra);
        // Both color and AND-mask DIBs are top-down. Mask rows are DWORD aligned.
        #[repr(C)]
        struct MaskInfo {
            header: BITMAPINFOHEADER,
            colors: [RGBQUAD; 2],
        }
        let mut mask_info: MaskInfo = zeroed();
        mask_info.header = header.bmiHeader;
        mask_info.header.biBitCount = 1;
        mask_info.header.biClrUsed = 2;
        mask_info.colors[1] = RGBQUAD {
            rgbBlue: 255,
            rgbGreen: 255,
            rgbRed: 255,
            rgbReserved: 0,
        };
        let mut mask_pixels = ptr::null_mut();
        let mask = CreateDIBSection(
            ptr::null_mut(),
            (&mask_info as *const MaskInfo).cast::<BITMAPINFO>(),
            DIB_RGB_COLORS,
            &mut mask_pixels,
            ptr::null_mut(),
            0,
        );
        if mask.is_null() || mask_pixels.is_null() {
            if !mask.is_null() {
                DeleteObject(mask);
            }
            DeleteObject(color);
            return Err("Could not allocate tray mask".into());
        }
        std::slice::from_raw_parts_mut(mask_pixels.cast::<u8>(), mask_bytes.len())
            .copy_from_slice(&mask_bytes);
        let info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info);
        DeleteObject(mask);
        DeleteObject(color);
        if icon.is_null() {
            Err("Could not create native tray icon".into())
        } else {
            Ok(icon)
        }
    }
}
/// System taskbar theme, high contrast and primary taskbar DPI. Explicit ink wins.
fn appearance(mode: ForegroundMode) -> ([u8; 3], u32, bool) {
    unsafe {
        let tray = FindWindowW(wide("Shell_TrayWnd").as_ptr(), ptr::null());
        let dpi = if tray.is_null() {
            96
        } else {
            GetDpiForWindow(tray).max(96)
        };
        let requested = GetSystemMetricsForDpi(SM_CXSMICON, dpi).max(16) as u32;
        let size = [16, 20, 24, 32, 40, 48, 64, 128]
            .into_iter()
            .find(|s| *s >= requested)
            .unwrap_or(128);
        let mut high: HIGHCONTRASTW = zeroed();
        high.cbSize = size_of::<HIGHCONTRASTW>() as u32;
        let contrast = SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            high.cbSize,
            (&mut high as *mut HIGHCONTRASTW).cast(),
            0,
        ) != 0
            && high.dwFlags & HCF_HIGHCONTRASTON != 0;
        let mut light: u32 = 0;
        let mut bytes = size_of::<u32>() as u32;
        let known = RegGetValueW(
            HKEY_CURRENT_USER,
            wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize").as_ptr(),
            wide("SystemUsesLightTheme").as_ptr(),
            RRF_RT_REG_DWORD,
            ptr::null_mut(),
            (&mut light as *mut u32).cast(),
            &mut bytes,
        ) == ERROR_SUCCESS
            && light <= 1;
        let ink = match mode {
            ForegroundMode::Light => [245; 3],
            ForegroundMode::Dark => [32; 3],
            ForegroundMode::Auto if contrast => {
                let color = GetSysColor(COLOR_BTNTEXT);
                [color as u8, (color >> 8) as u8, (color >> 16) as u8]
            }
            ForegroundMode::Auto if known && light == 1 => [32; 3],
            ForegroundMode::Auto => [245; 3],
        };
        (
            ink,
            size,
            mode == ForegroundMode::Auto && !contrast && !known,
        )
    }
}
fn show_menu(window: HWND, projection: &MenuProjection, preferences_known: bool) -> Option<MenuId> {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return None;
        }
        for (index, item) in projection.items.iter().enumerate() {
            let enabled = item.enabled && (item.id != MenuId::Autostart || preferences_known);
            let flags = MF_STRING
                | if enabled { MF_ENABLED } else { MF_GRAYED }
                | if item.checked == Some(true) {
                    MF_CHECKED
                } else {
                    MF_UNCHECKED
                };
            AppendMenuW(menu, flags, index + 1, wide(&item.label).as_ptr());
        }
        let mut point: POINT = zeroed();
        GetCursorPos(&mut point);
        SetForegroundWindow(window);
        let chosen = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            window,
            ptr::null(),
        ) as usize;
        PostMessageW(window, WM_NULL, 0, 0);
        DestroyMenu(menu);
        chosen
            .checked_sub(1)
            .and_then(|index| projection.items.get(index))
            .filter(|item| item.enabled && (item.id != MenuId::Autostart || preferences_known))
            .map(|item| item.id)
    }
}
pub fn run(config: HieronymusConfig) -> Result<(), String> {
    let cli = hiero::desktop::launch::stable_cli(
        &std::env::current_exe().map_err(|_| "Could not locate helper")?,
    )?;
    let options = ServiceOptions {
        data_root: config.data_root().to_path_buf(),
        unit_dir: service::default_unit_dir(),
        binary: cli,
        use_manager: true,
    };
    run_with_service_options(config, options)
}
pub fn run_with_service_options(
    config: HieronymusConfig,
    mut options: ServiceOptions,
) -> Result<(), String> {
    if options.data_root != config.data_root() {
        return Err("Desktop options belong to a different root".into());
    }
    let mut singleton = match TraySingleton::acquire_or_existing(&config, "native")
        .map_err(|_| "Could not own the native desktop session")?
    {
        SingletonOutcome::AlreadyRunning => return Ok(()),
        SingletonOutcome::Acquired(guard) => guard,
    };
    let root = config
        .data_root()
        .canonicalize()
        .map_err(|_| "Desktop root is unavailable")?;
    let config = HieronymusConfig::new(root);
    let window = unsafe {
        // Must precede GUI creation; broadcasts go to this top-level window, never HWND_MESSAGE.
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let taskbar = *TASKBAR_CREATED
            .get_or_init(|| RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()));
        if taskbar == 0 {
            return Err("Could not register Explorer recovery message".into());
        }
        let name = wide("HieronymusDesktopWindow");
        let instance = GetModuleHandleW(ptr::null());
        let mut class: WNDCLASSW = zeroed();
        class.lpfnWndProc = Some(window_proc);
        class.hInstance = instance;
        class.lpszClassName = name.as_ptr();
        if RegisterClassW(&class) == 0 {
            return Err("Could not register native tray window".into());
        }
        let window = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            name.as_ptr(),
            name.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null(),
        );
        if window.is_null() {
            return Err("Could not create native tray window".into());
        }
        Window(window)
    };
    let mut settings = DesktopSettings::default();
    let mut icon = NativeIcon {
        window: window.0,
        handle: ptr::null_mut(),
        present: false,
    };
    let (ink, size, _) = appearance(settings.foreground);
    icon.update(ink, [229, 167, 43], size)?;
    options.data_root = config.data_root().to_path_buf();
    let registration = WindowsRegistration::new(options.clone());
    let hwnd = window.0 as usize;
    let mut controller = Controller::spawn_with_notifier(
        LifecycleBackend::with_service_options(config.clone(), options, registration),
        PollSchedule::default(),
        move || unsafe {
            PostMessageW(hwnd as HWND, WAKE, 0, 0);
        },
    );
    controller.attach_control(&config, &mut singleton)?;
    let mut state = DesktopState::new();
    let mut view = state
        .apply(Event::Preferences {
            settings: None,
            error: None,
        })
        .clone();
    let mut preferences_known = false;
    let mut diagnostic = None;
    let mut last_icon = (ink, [229, 167, 43], size);
    if !std::env::args().any(|a| a == "--resume") {
        controller.submit(Action::Start)?;
    }
    let mut message: MSG = unsafe { zeroed() };
    let result = loop {
        // Modal menus can consume every posted wake, so consult persistent state
        // before blocking. Dispatch first so this message's flags are drained too.
        if PENDING.with(|pending| pending.pending() == 0) {
            let received = unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) };
            if received == -1 {
                break Err("Windows tray message loop failed".into());
            }
            if received == 0 {
                break Ok(());
            }
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        let events = PENDING.with(PendingEvents::take);
        if events & TEARDOWN != 0 {
            // Logoff belongs to this session; never submit Quit against another
            // session's shared root. Controller shutdown only joins its worker.
            break Ok(());
        }
        while let Some(event) = controller.try_event() {
            if let Event::Preferences {
                settings: actual,
                error,
            } = &event
            {
                preferences_known = actual.is_some();
                if let Some(actual) = actual {
                    settings = actual.clone();
                }
                diagnostic = error.clone();
            }
            view = state.apply(event).clone();
        }
        if view.exit_requested {
            break Ok(());
        }
        let (ink, size, ambiguous) = appearance(settings.foreground);
        let accent = match view.accent {
            Accent::Green => [46, 173, 104],
            Accent::Amber => [229, 167, 43],
            Accent::Red => [217, 74, 72],
        };
        if events & EXPLORER != 0 {
            // Same window/id only: tolerate repeated broadcasts without duplicates.
            unsafe {
                Shell_NotifyIconW(NIM_DELETE, &icon.data());
            }
            icon.present = false;
        }
        if !icon.present || last_icon != (ink, accent, size) {
            if let Err(error) = icon.update(ink, accent, size) {
                diagnostic = Some(error);
            } else {
                last_icon = (ink, accent, size);
            }
        }
        let mut projected = view.clone();
        if let Some(error) = &diagnostic {
            projected.reason.push_str(&format!(" — {error}"));
        }
        if ambiguous {
            projected.reason.push_str(" — Taskbar theme unknown; set foreground to light or dark in desktop-settings.json");
        }
        if events & MENU != 0
            && let Some(id) = show_menu(
                window.0,
                &MenuProjection::from_view(&projected, &settings),
                preferences_known,
            )
            && PENDING.with(|pending| pending.pending() & TEARDOWN == 0)
            && let Some(action) = id.action(settings.autostart)
        {
            let _ = controller.submit(action);
        }
    };
    controller.shutdown()?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    const FINISH_MODAL_TEST: u32 = WM_APP + 40;
    thread_local! {
        static RETIREMENT_TEST: Cell<bool> = const { Cell::new(false) };
        static DELIVERY: RefCell<VecDeque<Event>> = const { RefCell::new(VecDeque::new()) };
        static CONFIRM_SESSION: Cell<bool> = const { Cell::new(false) };
        static INSIDE_MODAL: Cell<bool> = const { Cell::new(false) };
        static SEEN_EVENTS: Cell<u8> = const { Cell::new(0) };
    }
    unsafe extern "system" fn modal_test_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == WM_ENTERMENULOOP {
            INSIDE_MODAL.set(true);
            if RETIREMENT_TEST.get() {
                // Same event/wake delivery contract as the worker notifier, while
                // the real nested native dispatcher owns posted messages.
                DELIVERY.with_borrow_mut(|events| events.push_back(Event::RetiredForUpdate));
                unsafe {
                    PostMessageW(window, WAKE, 0, 0);
                    PostMessageW(window, FINISH_MODAL_TEST, 0, 0);
                }
                return 0;
            }
            // These queued messages run through the real TrackPopupMenu dispatcher.
            unsafe {
                PostMessageW(window, *TASKBAR_CREATED.get().unwrap(), 0, 0);
                PostMessageW(window, WM_THEMECHANGED, 0, 0);
                PostMessageW(window, WM_QUERYENDSESSION, 0, 0);
                PostMessageW(window, WM_ENDSESSION, CONFIRM_SESSION.get() as usize, 0);
                PostMessageW(window, FINISH_MODAL_TEST, 0, 0);
            }
            return 0;
        }
        if message == WM_TIMER {
            // Bound a broken regression's modal wait; missing fixture events fail assertions.
            unsafe {
                EndMenu();
            }
            return 0;
        }
        if message == FINISH_MODAL_TEST {
            // Consuming wakes via the window procedure cannot consume the event copy.
            if !RETIREMENT_TEST.get() {
                unsafe {
                    SendMessageW(window, WAKE, 0, 0);
                }
            }
            SEEN_EVENTS.set(PENDING.with(PendingEvents::pending));
            unsafe {
                EndMenu();
            }
            return 0;
        }
        unsafe { window_proc(window, message, wparam, lparam) }
    }
    #[test]
    #[ignore = "requires an interactive disposable Windows desktop; opens a fixture popup"]
    fn native_modal_menu_retains_explorer_theme_and_confirmed_session_end() {
        run_modal_regression(false);
    }
    #[test]
    #[ignore = "requires an interactive disposable Windows desktop; opens a fixture popup"]
    fn native_modal_menu_drains_retirement_after_only_wake_is_consumed() {
        run_modal_regression(true);
    }
    fn run_modal_regression(retirement: bool) {
        RETIREMENT_TEST.set(retirement);
        unsafe {
            TASKBAR_CREATED.get_or_init(|| RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()));
            assert_ne!(*TASKBAR_CREATED.get().unwrap(), 0);
            let name = wide(if retirement {
                "HieronymusModalRetirementWindow"
            } else {
                "HieronymusModalRegressionWindow"
            });
            let instance = GetModuleHandleW(ptr::null());
            let mut class: WNDCLASSW = zeroed();
            class.lpfnWndProc = Some(modal_test_proc);
            class.hInstance = instance;
            class.lpszClassName = name.as_ptr();
            assert_ne!(RegisterClassW(&class), 0);
            let window = Window(CreateWindowExW(
                WS_EX_TOOLWINDOW,
                name.as_ptr(),
                name.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                10,
                10,
                200,
                100,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                ptr::null(),
            ));
            assert!(!window.0.is_null());
            ShowWindow(window.0, SW_SHOWNORMAL);
            let menu = CreatePopupMenu();
            assert!(!menu.is_null());
            assert_ne!(
                AppendMenuW(menu, MF_STRING, 1, wide("Disposable regression").as_ptr()),
                0
            );
            for confirmed in [false, true] {
                PENDING.with(PendingEvents::take);
                CONFIRM_SESSION.set(confirmed);
                INSIDE_MODAL.set(false);
                SEEN_EVENTS.set(0);
                SetForegroundWindow(window.0);
                assert_ne!(SetTimer(window.0, 1, 2000, None), 0);
                TrackPopupMenu(
                    menu,
                    TPM_RETURNCMD | TPM_NONOTIFY,
                    20,
                    20,
                    0,
                    window.0,
                    ptr::null(),
                );
                KillTimer(window.0, 1);
                assert!(
                    INSIDE_MODAL.get(),
                    "the native modal loop must actually run"
                );
                if retirement {
                    // The nested dispatcher consumed the sole worker wake before
                    // menu dismissal. No later synthetic/user event may rescue it.
                    assert_eq!(SEEN_EVENTS.get(), super::super::windows_events::CONTROLLER);
                    let mut message: MSG = zeroed();
                    assert_eq!(
                        PeekMessageW(&mut message, window.0, WAKE, WAKE, PM_REMOVE),
                        0
                    );
                    assert_ne!(PENDING.with(PendingEvents::pending), 0);
                    assert_eq!(
                        PENDING.with(PendingEvents::take),
                        super::super::windows_events::CONTROLLER
                    );
                    let mut state = DesktopState::new();
                    let event = DELIVERY
                        .with_borrow_mut(|events| events.pop_front())
                        .unwrap();
                    assert!(state.apply(event).exit_requested);
                    assert_eq!(PENDING.with(PendingEvents::pending), 0);
                    break;
                }
                let expected = EXPLORER | APPEARANCE | if confirmed { TEARDOWN } else { 0 };
                if !confirmed {
                    assert_eq!(
                        SEEN_EVENTS.get(),
                        expected | super::super::windows_events::CONTROLLER
                    );
                }
                assert_eq!(
                    PENDING.with(PendingEvents::take) & !super::super::windows_events::CONTROLLER,
                    expected
                );
                assert_eq!(PENDING.with(PendingEvents::pending), 0);
            }
            DestroyMenu(menu);
            drop(window);
            assert_ne!(UnregisterClassW(name.as_ptr(), instance), 0);
        }
    }
}
