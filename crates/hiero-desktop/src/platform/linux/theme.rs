//! Panel palette observation; GTK/app color-scheme does not describe GNOME Shell.
use crate::events::Sender;
use gtk::{gio, prelude::*};
use hiero::desktop::ForegroundMode;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

#[derive(Default)]
pub struct Appearance {
    pub foreground: Option<[u8; 3]>,
}
pub fn foreground(mode: ForegroundMode, panel: Option<[u8; 3]>) -> [u8; 3] {
    match mode {
        ForegroundMode::Light => [245, 245, 245],
        ForegroundMode::Dark => [16, 33, 52],
        ForegroundMode::Auto => panel.unwrap_or([245, 245, 245]),
    }
}
pub fn kde_foreground(text: &str) -> Option<[u8; 3]> {
    let mut window = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            window = line == "[Colors:Window]";
        }
        if window && let Some(value) = line.strip_prefix("ForegroundNormal=") {
            let values: Vec<_> = value
                .split(',')
                .map(|v| v.trim().parse::<u8>())
                .collect::<Result<_, _>>()
                .ok()?;
            return values.try_into().ok();
        }
    }
    None
}
pub struct ThemeObserver {
    _monitor: Option<gio::FileMonitor>,
    _shell_settings: Option<gio::Settings>,
}
impl ThemeObserver {
    pub fn new(state: Arc<Mutex<Appearance>>, wake: Sender) -> Self {
        let kde = std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .split(':')
            .any(|name| name.eq_ignore_ascii_case("KDE"));
        if !kde {
            return observe_shell(state, wake);
        }
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config"))
            });
        let Some(config) = config else {
            return Self {
                _monitor: None,
                _shell_settings: None,
            };
        };
        let file = gio::File::for_path(config.join("kdeglobals"));
        let generation = Arc::new(AtomicU64::new(0));
        refresh(&file, state.clone(), wake.clone(), generation.clone());
        // Watch the directory so atomic replacement of kdeglobals remains visible.
        let monitor = gio::File::for_path(config)
            .monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
            .ok();
        if let Some(monitor) = &monitor {
            monitor.connect_changed(move |_, changed, _, _| {
                if changed
                    .basename()
                    .is_some_and(|name| name == std::path::Path::new("kdeglobals"))
                {
                    refresh(&file, state.clone(), wake.clone(), generation.clone());
                }
            });
        }
        Self {
            _monitor: monitor,
            _shell_settings: None,
        }
    }
}
fn refresh(
    file: &gio::File,
    state: Arc<Mutex<Appearance>>,
    wake: Sender,
    generation: Arc<AtomicU64>,
) {
    let current = generation.fetch_add(1, Ordering::SeqCst) + 1;
    file.load_contents_async(gio::Cancellable::NONE, move |result| {
        if generation.load(Ordering::SeqCst) != current { return; }
        let foreground = result.ok().and_then(|(bytes, _)| std::str::from_utf8(&bytes).ok().and_then(kde_foreground));
        if foreground.is_none() { eprintln!("hiero-desktop: KDE panel palette unavailable; use the light/dark foreground override for custom panels"); }
        state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).foreground = foreground;
        wake.wake();
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn palette_uses_window_ink_and_overrides_mean_ink() {
        assert_eq!(
            kde_foreground(
                "[Colors:Button]\nForegroundNormal=0,0,0\n[Colors:Window]\nForegroundNormal=215,241,248\n"
            ),
            Some([215, 241, 248])
        );
        assert_eq!(
            kde_foreground("[Colors:Window]\nForegroundNormal=999,0,0"),
            None
        );
        assert_eq!(
            foreground(ForegroundMode::Auto, Some([215, 241, 248])),
            [215, 241, 248]
        );
        assert_eq!(
            foreground(ForegroundMode::Dark, Some([255, 255, 255])),
            [16, 33, 52]
        );
        assert_eq!(
            foreground(ForegroundMode::Light, Some([0, 0, 0])),
            [245, 245, 245]
        );
    }
}

// Inspect an available Shell stylesheet, never the app/GTK color-scheme. Only
// an unambiguous literal #panel foreground is accepted; inherited/variable or
// extension-defined styling uses the explicit override and a diagnostic.
fn shell_foreground(css: &str) -> Option<[u8; 3]> {
    let mut ink = None;
    for block in css.split('}') {
        let Some((selector, rules)) = block.rsplit_once('{') else {
            continue;
        };
        if selector.trim() != "#panel" {
            continue;
        }
        for declaration in rules.split(';') {
            if let Some(("color", value)) = declaration
                .trim()
                .split_once(':')
                .map(|(key, value)| (key.trim(), value.trim()))
            {
                let hex = value.strip_prefix('#')?;
                if hex.len() != 6 || !hex.is_ascii() {
                    return None;
                }
                ink = Some([
                    u8::from_str_radix(&hex[0..2], 16).ok()?,
                    u8::from_str_radix(&hex[2..4], 16).ok()?,
                    u8::from_str_radix(&hex[4..6], 16).ok()?,
                ]);
            }
        }
    }
    ink
}
fn observe_shell(state: Arc<Mutex<Appearance>>, wake: Sender) -> ThemeObserver {
    let settings = gio::SettingsSchemaSource::default()
        .and_then(|source| source.lookup("org.gnome.shell.extensions.user-theme", true))
        .filter(|schema| schema.has_key("name"))
        .map(|schema| gio::Settings::new_full(&schema, gio::SettingsBackend::NONE, None));
    let generation = Arc::new(AtomicU64::new(0));
    refresh_shell(
        settings.as_ref(),
        state.clone(),
        wake.clone(),
        generation.clone(),
    );
    let monitor_state = state.clone();
    let monitor_wake = wake.clone();
    let monitor_generation = generation.clone();
    if let Some(settings) = &settings {
        settings.connect_changed(Some("name"), move |settings, _| {
            refresh_shell(
                Some(settings),
                state.clone(),
                wake.clone(),
                generation.clone(),
            );
        });
    }
    let monitor = gio::File::for_path("/usr/share/gnome-shell/theme")
        .monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        .ok();
    if let Some(monitor) = &monitor {
        let settings = settings.clone();
        let state = monitor_state;
        let wake = monitor_wake;
        let generation = monitor_generation;
        monitor.connect_changed(move |_, _, _, _| {
            refresh_shell(
                settings.as_ref(),
                state.clone(),
                wake.clone(),
                generation.clone(),
            );
        });
    }
    ThemeObserver {
        _monitor: monitor,
        _shell_settings: settings,
    }
}
fn refresh_shell(
    settings: Option<&gio::Settings>,
    state: Arc<Mutex<Appearance>>,
    wake: Sender,
    generation: Arc<AtomicU64>,
) {
    let name = settings
        .map(|s| s.string("name").to_string())
        .unwrap_or_default();
    // Custom themes and extension effects can come from multiple locations. The
    // packaged default stylesheet is consulted only when no custom theme is set.
    if !name.is_empty() {
        generation.fetch_add(1, Ordering::SeqCst);
        state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .foreground = None;
        eprintln!(
            "hiero-desktop: Custom GNOME Shell panel foreground is unknown; set desktop-settings.json foreground to light or dark"
        );
        wake.wake();
        return;
    }
    let current = generation.fetch_add(1, Ordering::SeqCst) + 1;
    gio::File::for_path("/usr/share/gnome-shell/theme/gnome-shell.css").load_contents_async(gio::Cancellable::NONE, move |result| {
        if generation.load(Ordering::SeqCst) != current { return; }
        let foreground = result.ok().and_then(|(bytes,_)| std::str::from_utf8(&bytes).ok().and_then(shell_foreground));
        state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).foreground = foreground;
        if foreground.is_none() { eprintln!("hiero-desktop: GNOME Shell panel foreground unavailable (possibly a resource theme); using light ink. Set desktop-settings.json foreground to light or dark for custom panels"); }
        wake.wake();
    });
}

#[cfg(test)]
mod shell_tests {
    use super::shell_foreground;
    #[test]
    fn shell_ink_comes_from_panel_not_app_theme() {
        assert_eq!(
            shell_foreground(
                ".window { color: #000000; } #panel { color: #eeeeee; background: black; }"
            ),
            Some([238, 238, 238])
        );
        assert_eq!(shell_foreground("#panel { color: inherit; }"), None);
        assert_eq!(shell_foreground("#panel { color: #ééé; }"), None);
    }
}
