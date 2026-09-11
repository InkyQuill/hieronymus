//! Deterministic Task Scheduler definitions. Executable and arguments are separate;
//! neither the scheduler nor the browser receives shell-interpolated commands.
use sha2::{Digest, Sha256};
use std::path::Path;

pub fn name(root: &Path, sid: &str, tray: bool) -> String {
    let mut hash = Sha256::new();
    hash.update(sid.as_bytes());
    hash.update([0]);
    hash.update(root.as_os_str().as_encoded_bytes());
    format!(
        "Hieronymus-{:x}-{}",
        hash.finalize(),
        if tray { "tray" } else { "daemon" }
    )
}

pub fn argument(path: &Path) -> Result<String, String> {
    let text = path
        .to_str()
        .filter(|s| path.is_absolute() && s.len() <= 512 && !s.chars().any(char::is_control))
        .ok_or("Task paths must be absolute UTF-8, at most 512 bytes, without controls")?;
    // CommandLineToArgvW/CRT quoting: backslashes before a quote or final quote double.
    let mut result = String::from("\"");
    let mut slashes = 0;
    for c in text.chars() {
        if c == '\\' {
            slashes += 1;
            continue;
        }
        result.extend(std::iter::repeat_n(
            '\\',
            if c == '"' { slashes * 2 + 1 } else { slashes },
        ));
        slashes = 0;
        result.push(c);
    }
    result.extend(std::iter::repeat_n('\\', slashes * 2));
    result.push('"');
    Ok(result)
}
fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
pub fn render(
    binary: &Path,
    root: &Path,
    sid: &str,
    tray: bool,
    enabled: bool,
    recovery: bool,
) -> Result<String, String> {
    argument(binary)?;
    let args = format!(
        "{} --data-root {}",
        if tray { "tray" } else { "daemon" },
        argument(root)?
    );
    if !sid.starts_with("S-1-")
        || sid.len() > 184
        || !sid
            .bytes()
            .all(|c| c.is_ascii_digit() || c == b'S' || c == b'-')
    {
        return Err("Invalid Windows user SID".into());
    }
    let trigger = if tray {
        format!("<LogonTrigger><Enabled>true</Enabled><UserId>{sid}</UserId></LogonTrigger>")
    } else {
        String::new()
    };
    let recovery = if recovery && !tray {
        "<RestartOnFailure><Interval>PT1M</Interval><Count>3</Count></RestartOnFailure>"
    } else {
        ""
    };
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
<RegistrationInfo><URI>{}</URI><Description>Hieronymus owned native task</Description></RegistrationInfo>
<Triggers>{trigger}</Triggers>
<Principals><Principal id="Owner"><UserId>{sid}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
<Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><AllowHardTerminate>false</AllowHardTerminate><StartWhenAvailable>false</StartWhenAvailable><RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable><IdleSettings><StopOnIdleEnd>false</StopOnIdleEnd><RestartOnIdle>false</RestartOnIdle></IdleSettings><AllowStartOnDemand>true</AllowStartOnDemand><Enabled>{enabled}</Enabled><Hidden>false</Hidden><RunOnlyIfIdle>false</RunOnlyIfIdle><WakeToRun>false</WakeToRun><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><Priority>7</Priority>{recovery}</Settings>
<Actions Context="Owner"><Exec><Command>{}</Command><Arguments>{}</Arguments></Exec></Actions>
</Task>"#,
        name(root, sid, tray),
        xml(binary.to_str().unwrap()),
        xml(&args)
    ))
}

/// Compare the complete element tree, ignoring formatting and optional defaults
/// inserted by Task Scheduler. Unknown elements, extra actions/principals/triggers,
/// or changes to any explicit setting are refused.
pub fn equivalent(actual: &str, expected: &str) -> Result<(), String> {
    fn tree(text: &str) -> Result<Vec<(String, String)>, String> {
        if text.len() > 65536 {
            return Err("Task definition is oversized".into());
        }
        let doc = roxmltree::Document::parse(text).map_err(|_| "Invalid task XML")?;
        let mut values = Vec::new();
        for node in doc.descendants().filter(|n| n.is_element()) {
            if node.tag_name().namespace()
                != Some("http://schemas.microsoft.com/windows/2004/02/mit/task")
            {
                return Err("Foreign task XML namespace".into());
            }
            let mut names: Vec<_> = node
                .ancestors()
                .filter(|n| n.is_element())
                .map(|n| n.tag_name().name())
                .collect();
            names.reverse();
            let path = names.join("/");
            // Scheduler documents inject these defaults; none grants execution authority.
            if matches!(
                (path.as_str(), node.text().unwrap_or("").trim()),
                ("Task/Settings/IdleSettings/Duration", "PT10M")
                    | ("Task/Settings/IdleSettings/WaitTimeout", "PT1H")
                    | ("Task/Settings/UseUnifiedSchedulingEngine", "false")
                    | ("Task/Settings/DisallowStartOnRemoteAppSession", "false")
            ) {
                continue;
            }
            for attr in node.attributes() {
                if path == "Task" && attr.name() == "version" {
                    continue;
                }
                values.push((format!("{path}/@{}", attr.name()), attr.value().into()));
            }
            let value = if node.children().any(|n| n.is_element()) {
                ""
            } else {
                let text = node.text().unwrap_or("");
                if text.chars().all(char::is_whitespace) {
                    ""
                } else {
                    text
                }
            };
            values.push((path, value.into()));
        }
        values.sort();
        Ok(values)
    }
    if tree(actual)? != tree(expected)? {
        return Err("Task belongs to another installation or its definition was modified".into());
    }
    Ok(())
}

/// Preserve an explicitly selected registration directory through real logon.
pub fn render_login_in_directory(
    binary: &Path,
    root: &Path,
    directory: &Path,
    sid: &str,
    enabled: bool,
) -> Result<String, String> {
    let text = render(binary, root, sid, true, enabled, false)?;
    Ok(text.replace(
        "</Arguments>",
        &format!(" --unit-dir {}</Arguments>", xml(&argument(directory)?)),
    ))
}
