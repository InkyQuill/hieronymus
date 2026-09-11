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
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\"', "&quot;")
        .replace('\'', "&apos;")
}
pub fn render(binary: &Path, root: &Path, directory: &Path, tray: bool) -> Result<String, String> {
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
        if tray { "true" } else { "false" }
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

/// launchctl's human readback format is deliberately parsed narrowly. An OS
/// format change is an error until qualified, never evidence of ownership.
pub fn validate_loaded(text: &str, path: &Path, expected: &[String]) -> Result<(), String> {
    if text.len() > 65536 {
        return Err("LaunchAgent readback exceeds its bound".into());
    }
    let field = |key: &str| -> Result<Option<&str>, String> {
        let prefix = format!("{key} = ");
        let values = text
            .lines()
            .map(str::trim)
            .filter_map(|line| line.strip_prefix(&prefix))
            .collect::<Vec<_>>();
        match values.as_slice() {
            [] => Ok(None),
            [value] => Ok(Some(*value)),
            _ => Err("Duplicate LaunchAgent readback field".into()),
        }
    };
    if field("path")? != path.to_str() {
        return Err("Loaded LaunchAgent belongs to another registration".into());
    }
    if field("program")? != expected.first().map(String::as_str) {
        return Err("Loaded LaunchAgent executable differs".into());
    }
    if field("properties")?.is_some_and(|properties| {
        properties
            .split('|')
            .any(|value| value.trim().eq_ignore_ascii_case("keepalive"))
    }) || text.lines().any(|line| {
        line.trim_start()
            .to_ascii_lowercase()
            .starts_with("keepalive =")
    }) {
        return Err("Loaded LaunchAgent has an unapproved recovery policy".into());
    }
    if text
        .lines()
        .filter(|line| line.trim() == "arguments = {")
        .count()
        != 1
    {
        return Err("Invalid loaded LaunchAgent argument lists".into());
    }
    let mut lines = text.lines().map(str::trim);
    lines.find(|line| *line == "arguments = {");
    let args = lines.take_while(|line| *line != "}").collect::<Vec<_>>();
    if args != expected {
        return Err("Loaded LaunchAgent arguments differ".into());
    }
    Ok(())
}
pub fn disabled_state(text: &str, label: &str) -> Result<bool, String> {
    if text.len() > 65536
        || !text.trim_start().starts_with("disabled services = {")
        || !text.trim_end().ends_with('}')
    {
        return Err("Invalid LaunchAgent enabled-state readback".into());
    }
    let prefix = format!("\"{label}\" => ");
    let values = text
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix(&prefix))
        .collect::<Vec<_>>();
    match values.as_slice() {
        [] => Ok(false),
        ["true"] => Ok(true),
        ["false"] => Ok(false),
        _ => Err("Invalid LaunchAgent enabled-state readback".into()),
    }
}
