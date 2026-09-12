//! Session-scoped OS ownership. Persistent lock files are never unlinked.
use hieronymus::data_root::HieronymusConfig;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io;

/// Only generated session lock names identify persistent coordination authority.
pub(crate) fn is_session_lock_name(name: &str) -> bool {
    name.strip_prefix(".tray-")
        .and_then(|name| name.strip_suffix(".lock"))
        .is_some_and(|hash| {
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

#[derive(Debug)]
pub struct TraySingleton {
    file: File,
    pub(crate) path: std::path::PathBuf,
    pub(crate) startup_gate: Option<File>,
}
#[derive(Debug)]
pub enum SingletonOutcome {
    Acquired(TraySingleton),
    AlreadyRunning,
}
impl TraySingleton {
    /// `session` is a diagnostic hint only; it cannot choose the authority
    /// namespace. The session identity comes from the current OS process.
    pub fn acquire(config: &HieronymusConfig, session: &str) -> io::Result<Self> {
        Self::acquire_from_lookup(config, session, trusted_session)
    }
    fn acquire_from_lookup(
        config: &HieronymusConfig,
        _hint: &str,
        lookup: impl FnOnce() -> io::Result<String>,
    ) -> io::Result<Self> {
        Self::acquire_trusted(config, &lookup()?)
    }
    /// Duplicate launch is an explicit clean-success outcome for native entrypoints.
    pub fn acquire_or_existing(
        config: &HieronymusConfig,
        session: &str,
    ) -> io::Result<SingletonOutcome> {
        match Self::acquire(config, session) {
            Ok(guard) => Ok(SingletonOutcome::Acquired(guard)),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                Ok(SingletonOutcome::AlreadyRunning)
            }
            Err(error) => Err(error),
        }
    }
    pub(crate) fn session_path(config: &HieronymusConfig) -> io::Result<std::path::PathBuf> {
        let root = config.data_root().canonicalize()?;
        let mut hash = Sha256::new();
        hash.update(root.as_os_str().as_encoded_bytes());
        hash.update([0]);
        hash.update(trusted_session()?.as_bytes());
        Ok(root.join(format!(".tray-{:x}.lock", hash.finalize())))
    }
    pub(crate) fn acquire_trusted(config: &HieronymusConfig, session: &str) -> io::Result<Self> {
        std::fs::create_dir_all(config.data_root())?;
        let root = config.data_root().canonicalize()?;
        let mut hash = Sha256::new();
        hash.update(root.as_os_str().as_encoded_bytes());
        hash.update([0]);
        hash.update(session.as_bytes());
        let path = root.join(format!(".tray-{:x}.lock", hash.finalize()));
        let startup_gate = super::control::launch_gate(config).map_err(|e| {
            if e.kind() == io::ErrorKind::WouldBlock {
                io::Error::other(
                    "Desktop installation/update is in progress; retry after completion",
                )
            } else {
                e
            }
        })?;
        Ok(Self {
            file: super::control::lock(&path)?,
            path,
            startup_gate: Some(startup_gate),
        })
    }
}
impl Drop for TraySingleton {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[cfg(target_os = "linux")]
fn trusted_session() -> io::Result<String> {
    // Kernel audit login session, inherited across fork/exec, independent of
    // caller-controlled environment variables. -1 denotes no assigned session.
    if let Ok(value) = std::fs::read_to_string("/proc/self/sessionid")
        && let Ok(id) = value.trim().parse::<u32>()
        && id != u32::MAX
    {
        return Ok(format!("audit-{id}"));
    }
    if let Ok(session) = logind_session().or_else(|_| graphical_user_session()) {
        return Ok(session);
    }

    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Could not identify this desktop login session; run the helper inside a logind graphical login with /usr/bin/busctl available",
    ))
}
#[cfg(target_os = "linux")]
fn logind_session() -> io::Result<String> {
    let output = busctl(&[
        "call",
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
        "GetSessionByPID",
        "u",
        &std::process::id().to_string(),
    ])?;
    parse_logind_session(&output)
}

#[cfg(target_os = "linux")]
fn busctl(args: &[&str]) -> io::Result<String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let mut child = Command::new("/usr/bin/busctl")
        .args(["--system", "--timeout=2", "--no-pager"])
        .args(args)
        .env_remove("DBUS_SYSTEM_BUS_ADDRESS")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            result => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(result.err().unwrap_or_else(|| {
                    io::Error::new(io::ErrorKind::TimedOut, "Desktop session lookup timed out")
                }));
            }
        }
    };
    if !status.success() {
        return Err(io::Error::other(
            "No logind session was found for this process",
        ));
    }
    let mut output = String::new();
    child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("Session lookup had no output"))?
        .take(4096)
        .read_to_string(&mut output)?;
    Ok(output)
}

#[cfg(target_os = "linux")]
fn graphical_user_session() -> io::Result<String> {
    // User-manager services are outside a login scope. Ask the system's logind
    // for this UID's graphical session; caller-supplied environment is not authority.
    let uid = rustix::process::geteuid().as_raw();
    let user = format!("/org/freedesktop/login1/user/_{uid}");
    let display = logind_property(&user, "org.freedesktop.login1.User", "Display")?;
    let path = display
        .get(1)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| io::Error::other("No graphical session"))?;
    let canonical = parse_logind_session(&format!("o \"{path}\""))?;
    let owner = logind_property(path, "org.freedesktop.login1.Session", "User")?;
    let kind = logind_property(path, "org.freedesktop.login1.Session", "Type")?;
    let remote = logind_property(path, "org.freedesktop.login1.Session", "Remote")?;
    let audit = logind_property(path, "org.freedesktop.login1.Session", "Audit")?;
    graphical_identity(uid, &owner, &kind, &remote, &audit, canonical)
}

#[cfg(target_os = "linux")]
fn logind_property(path: &str, interface: &str, property: &str) -> io::Result<serde_json::Value> {
    let output = busctl(&[
        "--json=short",
        "get-property",
        "org.freedesktop.login1",
        path,
        interface,
        property,
    ])?;
    let value: serde_json::Value = serde_json::from_str(&output).map_err(io::Error::other)?;
    value
        .get("data")
        .cloned()
        .ok_or_else(|| io::Error::other("Missing logind property data"))
}

#[cfg(target_os = "linux")]
fn graphical_identity(
    uid: u32,
    owner: &serde_json::Value,
    kind: &serde_json::Value,
    remote: &serde_json::Value,
    audit: &serde_json::Value,
    canonical: String,
) -> io::Result<String> {
    if owner.pointer("/0").and_then(serde_json::Value::as_u64) != Some(u64::from(uid))
        || !matches!(kind.as_str(), Some("wayland" | "x11"))
        || remote.as_bool() != Some(false)
    {
        return Err(io::Error::other("No local graphical session for this user"));
    }
    let id = audit
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .ok_or_else(|| io::Error::other("Invalid graphical audit session"))?;
    Ok(if id == u32::MAX {
        canonical
    } else {
        format!("audit-{id}")
    })
}

#[cfg(target_os = "linux")]
fn parse_logind_session(output: &str) -> io::Result<String> {
    let session = output
        .trim()
        .strip_prefix("o \"/org/freedesktop/login1/session/")
        .and_then(|s| s.strip_suffix('"'))
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 255
                && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        })
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid logind session identity",
            )
        })?;
    Ok(format!("logind-{session}"))
}
#[cfg(windows)]
fn trusted_session() -> io::Result<String> {
    crate::platform::windows_identity::session()
}
#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
fn trusted_session() -> io::Result<String> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Trusted desktop session lookup is not implemented for this platform",
    ))
}

#[cfg(target_os = "macos")]
fn trusted_session() -> io::Result<String> {
    crate::platform::macos_identity::session()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    #[test]
    fn graphical_service_identity_matches_login_and_rejects_foreign_sessions() {
        use serde_json::json;
        let identify = |uid, kind, remote, audit| {
            graphical_identity(
                1000,
                &json!([uid, "/org/freedesktop/login1/user/_1000"]),
                &json!(kind),
                &json!(remote),
                &json!(audit),
                "logind-_33".into(),
            )
        };
        assert_eq!(identify(1000, "wayland", false, 3u32).unwrap(), "audit-3");
        assert_eq!(
            identify(1000, "x11", false, u32::MAX).unwrap(),
            "logind-_33"
        );
        assert!(identify(1001, "wayland", false, 3).is_err());
        assert!(identify(1000, "wayland", true, 3).is_err());
        assert!(identify(1000, "tty", false, 3).is_err());
    }
    #[test]
    fn caller_hints_cannot_split_trusted_session_ownership() {
        let directory = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(directory.path());
        let mut held =
            TraySingleton::acquire_from_lookup(&config, "invented-one", || Ok("trusted".into()))
                .unwrap();
        held.startup_gate.take();
        assert_eq!(
            TraySingleton::acquire_from_lookup(&config, "invented-two", || Ok("trusted".into()))
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        assert!(
            TraySingleton::acquire_from_lookup(&config, "trust-me", || Err(io::Error::other(
                "No trusted session"
            )))
            .is_err()
        );
    }
    #[test]
    fn same_root_alias_and_session_share_lock_and_inode_survives_drop() {
        let directory = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(directory.path());
        let mut held = TraySingleton::acquire_trusted(&config, "trusted-session").unwrap();
        held.startup_gate.take();
        let alias = HieronymusConfig::new(directory.path().join("."));
        assert_eq!(
            TraySingleton::acquire_trusted(&alias, "trusted-session")
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        let other = TraySingleton::acquire_trusted(&config, "other-session").unwrap();
        drop(other);
        drop(held);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 3);
        TraySingleton::acquire_trusted(&alias, "trusted-session").unwrap();
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn logind_identity_parser_rejects_untrusted_or_malformed_output() {
        assert_eq!(
            parse_logind_session("o \"/org/freedesktop/login1/session/_32\"\n").unwrap(),
            "logind-_32"
        );
        for output in [
            "XDG_SESSION_ID=2",
            "o \"/other/session/2\"",
            "o \"/org/freedesktop/login1/session/../../x\"",
            "o \"/org/freedesktop/login1/session/\"",
        ] {
            assert!(parse_logind_session(output).is_err());
        }
    }
}
