//! Deterministic LaunchAgent definitions; no shell command interpolation.
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
pub const BUNDLE_ID: &str = "net.inkyquill.hieronymus";
pub fn label(root: &Path, tray: bool) -> String {
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    format!(
        "{BUNDLE_ID}.{}.{:x}",
        if tray { "tray" } else { "daemon" },
        Sha256::digest(canonical.as_os_str().as_encoded_bytes())
    )
}
pub fn domain(uid: u32) -> String {
    format!("gui/{uid}")
}
pub fn arguments(
    binary: &Path,
    root: &Path,
    directory: &Path,
    tray: bool,
) -> Result<Vec<String>, String> {
    let path = |p: &Path| -> Result<String, String> {
        let s = p
            .to_str()
            .filter(|s| p.is_absolute() && s.len() <= 512 && !s.chars().any(char::is_control))
            .ok_or("LaunchAgent requires a bounded absolute UTF-8 path")?;
        Ok(s.into())
    };
    if tray {
        Ok(vec![
            path(binary)?,
            "tray".into(),
            "--data-root".into(),
            path(root)?,
            "--unit-dir".into(),
            path(directory)?,
            "--binary".into(),
            path(binary)?,
        ])
    } else {
        Ok(vec![
            path(binary)?,
            "daemon".into(),
            "--data-root".into(),
            path(root)?,
        ])
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonMode {
    Headless,
    Desktop,
}
pub fn owned_mode(
    text: &str,
    binary: &Path,
    root: &Path,
    directory: &Path,
) -> Result<DaemonMode, String> {
    for mode in [DaemonMode::Headless, DaemonMode::Desktop] {
        if text == render_mode(binary, root, directory, false, mode)? {
            return Ok(mode);
        }
    }
    Err("Foreign daemon login definition".into())
}
/// Changing a loaded headless definition cannot be rolled back without a new
/// RunAtLoad execution. Require an authenticated Stop/unload before conversion.
pub fn check_mode_change(
    before: Option<DaemonMode>,
    loaded: bool,
    after: DaemonMode,
) -> Result<(), String> {
    if loaded && before != Some(after) {
        return Err(
            "Stop the daemon before changing its login mode; loaded registration was not changed"
                .into(),
        );
    }
    Ok(())
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\"', "&quot;")
        .replace('\'', "&apos;")
}
pub fn render(binary: &Path, root: &Path, directory: &Path, tray: bool) -> Result<String, String> {
    render_mode(binary, root, directory, tray, DaemonMode::Desktop)
}
pub fn render_mode(
    binary: &Path,
    root: &Path,
    directory: &Path,
    tray: bool,
    mode: DaemonMode,
) -> Result<String, String> {
    let args = arguments(binary, root, directory, tray)?;
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{}</string>
<key>ProgramArguments</key><array>{}</array>
<key>LimitLoadToSessionType</key><string>Aqua</string>
<key>RunAtLoad</key><{}/>
<key>KeepAlive</key><false/>
<key>ProcessType</key><string>Interactive</string>
<key>StandardOutPath</key><string>/dev/null</string>
<key>StandardErrorPath</key><string>/dev/null</string>
</dict></plist>
"#,
        label(root, tray),
        args.iter()
            .map(|a| format!("<string>{}</string>", escape(a)))
            .collect::<String>(),
        if tray || mode == DaemonMode::Headless {
            "true"
        } else {
            "false"
        }
    ))
}
pub fn parse(text: &str) -> Result<(PathBuf, PathBuf), String> {
    let doc = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .map_err(|_| "Invalid LaunchAgent plist")?;
    let args = doc
        .descendants()
        .find(|n| n.has_tag_name("array"))
        .ok_or("Missing LaunchAgent arguments")?
        .children()
        .filter(|n| n.is_element())
        .map(|n| n.text().unwrap_or(""))
        .collect::<Vec<_>>();
    match args.as_slice() {
        [binary, "daemon", "--data-root", root] => Ok(((*binary).into(), (*root).into())),
        _ => Err("Invalid daemon LaunchAgent arguments".into()),
    }
}
/// The exact definition must be restored when native publication fails. This
/// primitive is also used by pending-transaction recovery in the native backend.
pub fn restore_definition(path: &Path, before: Option<&[u8]>) -> Result<(), String> {
    match before {
        Some(bytes) => hieronymus::atomic::atomic_write(path, bytes)
            .map_err(|_| "Could not restore LaunchAgent definition".into()),
        None => match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("Could not remove LaunchAgent definition".into()),
        },
    }
}

/// Commit publication with exact rollback on normal failure. Native backends
/// retain their transaction journal if rollback cannot complete.
pub fn publish_with_rollback<T>(
    publish: impl FnOnce() -> Result<T, String>,
    restore: impl FnOnce() -> Result<(), String>,
) -> Result<T, String> {
    match publish() {
        Ok(value) => Ok(value),
        Err(error) => {
            restore().map_err(|_| "LaunchAgent registration failed; rollback remains pending")?;
            Err(error)
        }
    }
}

/// A bounded parse result, including the process and login-policy fields that
/// must be known before a loaded job can be removed or replaced.
#[derive(Debug, PartialEq, Eq)]
pub struct LoadedState {
    pub target: String,
    pub pid: Option<u32>,
    pub run_at_load: bool,
    pub idle: bool,
}
#[derive(Debug)]
enum NativeValue<'a> {
    Scalar(&'a str),
    Dictionary(Vec<NativeEntry<'a>>),
    Arguments(Vec<&'a str>),
}
#[derive(Debug)]
struct NativeEntry<'a> {
    key: &'a str,
    value: NativeValue<'a>,
}
fn native_document(text: &str) -> Result<NativeEntry<'_>, String> {
    if text.len() > 65536 || text.contains('\r') {
        return Err("Invalid native readback size or line endings".into());
    }
    let mut lines = text.lines().peekable();
    let header = lines
        .find(|line| !line.trim().is_empty())
        .ok_or("Missing native readback envelope")?
        .trim();
    let (key, value) = header
        .split_once(" = ")
        .ok_or("Invalid native readback envelope")?;
    if key.is_empty() || value != "{" {
        return Err("Invalid native readback envelope".into());
    }
    let entries = native_dictionary(&mut lines, 0)?;
    if lines.any(|line| !line.trim().is_empty()) {
        return Err("Trailing native readback data".into());
    }
    Ok(NativeEntry {
        key,
        value: NativeValue::Dictionary(entries),
    })
}
fn native_dictionary<'a>(
    lines: &mut impl Iterator<Item = &'a str>,
    depth: usize,
) -> Result<Vec<NativeEntry<'a>>, String> {
    if depth > 16 {
        return Err("Native readback nesting exceeds its bound".into());
    }
    let mut entries = Vec::new();
    let mut keys = std::collections::BTreeSet::new();
    while let Some(raw) = lines.next() {
        let line = raw.trim_start();
        if line.trim() == "}" {
            return Ok(entries);
        }
        if line.is_empty() {
            continue;
        }
        // Canonical separators only. Unknown spacing is rejected, never skipped.
        let (key, value) = line
            .split_once(" => ")
            .or_else(|| line.split_once(" = "))
            .ok_or("Malformed native readback field")?;
        if key.is_empty() || key.trim() != key || !keys.insert(key) {
            return Err("Invalid or duplicate native readback field".into());
        }
        let value = if value == "{" {
            if key == "arguments" {
                let mut args = Vec::new();
                let mut closed = false;
                for argument in lines.by_ref() {
                    if argument.trim() == "}" {
                        closed = true;
                        break;
                    }
                    // Only indentation is removed: literal trailing spaces belong
                    // to argv and must compare exactly against the owned vector.
                    args.push(argument.trim_start());
                }
                if !closed {
                    return Err("Unterminated native argument block".into());
                }
                NativeValue::Arguments(args)
            } else {
                NativeValue::Dictionary(native_dictionary(lines, depth + 1)?)
            }
        } else {
            if value.is_empty() || value == "}" {
                return Err("Malformed native scalar".into());
            }
            NativeValue::Scalar(value)
        };
        entries.push(NativeEntry { key, value });
    }
    Err("Unterminated native readback dictionary".into())
}
/// Unknown authority syntax and incomplete documents cannot prove ownership or
/// process absence. Every known top-level field is unique and structurally read.
pub fn loaded_state(text: &str, path: &Path, expected: &[String]) -> Result<LoadedState, String> {
    let document = native_document(text)?;
    let parts = document.key.split('/').collect::<Vec<_>>();
    if !matches!(parts.as_slice(),["gui",uid,label] if uid.parse::<u32>().is_ok()&&!label.is_empty())
    {
        return Err("Invalid loaded LaunchAgent target".into());
    }
    let NativeValue::Dictionary(entries) = document.value else {
        unreachable!()
    };
    const FIELDS: &[&str] = &[
        "active count",
        "path",
        "type",
        "state",
        "program",
        "stdout path",
        "stderr path",
        "arguments",
        "inherited environment",
        "default environment",
        "environment",
        "domain",
        "asid",
        "minimum runtime",
        "exit timeout",
        "runs",
        "pid",
        "immediate reason",
        "forks",
        "execs",
        "initialized",
        "trampolined",
        "started suspended",
        "proxy started",
        "proxy started suspended",
        "extension alive",
        "trial factors memory limit",
        "checked allocations",
        "checked allocations reason",
        "checked allocations flags",
        "pended spawn",
        "pended nondemand spawn",
        "spawn reason filter",
        "last exit code",
        "last terminating signal",
        "jetsam priority",
        "jetsam memory limit (active)",
        "jetsam memory limit (inactive)",
        "jetsamproperties category",
        "jetsam thread limit",
        "cpumon",
        "resource coalition",
        "jetsam coalition",
        "spawn type",
        "properties",
    ];
    if entries.iter().any(|entry| !FIELDS.contains(&entry.key)) {
        return Err("Unknown loaded LaunchAgent field".into());
    }
    let scalar = |key: &str| -> Result<Option<&str>, String> {
        match entries
            .iter()
            .find(|entry| entry.key == key)
            .map(|entry| &entry.value)
        {
            None => Ok(None),
            Some(NativeValue::Scalar(value)) => Ok(Some(*value)),
            _ => Err("Invalid native authority field type".into()),
        }
    };
    if scalar("path")? != path.to_str() {
        return Err("Loaded LaunchAgent belongs to another registration".into());
    }
    if scalar("program")? != expected.first().map(String::as_str) {
        return Err("Loaded LaunchAgent executable differs".into());
    }
    for key in ["stdout path", "stderr path"] {
        if scalar(key)?.is_some_and(|value| value != "/dev/null") {
            return Err("Loaded LaunchAgent output path differs".into());
        }
    }
    match entries
        .iter()
        .find(|entry| entry.key == "arguments")
        .map(|entry| &entry.value)
    {
        Some(NativeValue::Arguments(args)) if args == expected => {}
        _ => return Err("Loaded LaunchAgent arguments differ".into()),
    }
    let properties = scalar("properties")?.ok_or("Missing loaded LaunchAgent properties")?;
    let properties = properties.split('|').map(str::trim).collect::<Vec<_>>();
    if properties.iter().any(|p| p.is_empty())
        || properties.contains(&"keepalive")
        || properties
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != properties.len()
    {
        return Err("Unapproved loaded LaunchAgent recovery policy".into());
    }
    // Refuse unrecognized policy flags rather than assuming no future startup.
    if properties.iter().any(|p| {
        !matches!(
            *p,
            "inferred program"
                | "runatload"
                | "supports transactions"
                | "partial import"
                | "dirty tracking"
                | "managed"
                | "system service"
                | "tle system"
        )
    }) {
        return Err("Unknown loaded LaunchAgent property".into());
    }
    let pid = scalar("pid")?
        .map(|value| {
            value
                .parse::<u32>()
                .ok()
                .filter(|pid| *pid > 0)
                .ok_or("Invalid LaunchAgent PID")
        })
        .transpose()?;
    let state = scalar("state")?.ok_or("Missing loaded LaunchAgent process state")?;
    let idle = match (state, pid) {
        ("not running", None) => true,
        ("running" | "xpcproxy", Some(_)) => false,
        ("spawn scheduled" | "spawn pending" | "waiting" | "exited", None) => false,
        _ => return Err("Inconsistent or unknown LaunchAgent process state".into()),
    };
    Ok(LoadedState {
        target: document.key.into(),
        pid,
        run_at_load: properties.contains(&"runatload"),
        idle,
    })
}
pub fn validate_loaded(text: &str, path: &Path, expected: &[String]) -> Result<(), String> {
    loaded_state(text, path, expected).map(|_| ())
}
pub fn disabled_state(text: &str, label: &str) -> Result<bool, String> {
    let document = native_document(text)?;
    if document.key != "disabled services" {
        return Err("Invalid disabled-service envelope".into());
    }
    let NativeValue::Dictionary(entries) = document.value else {
        unreachable!()
    };
    let mut disabled = None;
    for entry in entries {
        let name = entry
            .key
            .strip_prefix('"')
            .and_then(|key| key.strip_suffix('"'))
            .filter(|name| !name.is_empty() && !name.contains('"'))
            .ok_or("Invalid disabled-service label")?;
        let value = match entry.value {
            NativeValue::Scalar("true" | "disabled") => true,
            NativeValue::Scalar("false" | "enabled") => false,
            _ => return Err("Invalid disabled-service state".into()),
        };
        if name == label {
            disabled = Some(value);
        }
    }
    Ok(disabled.unwrap_or(false))
}
