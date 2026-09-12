//! `hiero admin` / `hiero config`: open the local web console.
//! Browser authentication is optional (`web.conf`); the grant flow below is
//! also works when authentication is disabled, so CLI launches remain valid
//! while configuration changes are waiting for a server restart.
//!
//! There is no in-browser way to obtain a session: the daemon only hands out a
//! session cookie in exchange for a one-time launch grant, and only a
//! bearer-authenticated local command can mint one. This module is that
//! command.
//!
//! Flow (ADR 0012 as amended 2026-09-03, ADR 0013):
//!
//! 1. The shared guarded lifecycle connection discovers (and, if absent, starts) the local
//!    daemon and yields a bearer-authenticated `lifecycle::DaemonClient`.
//! 2. `POST /auth/launch-grant` mints a 60-second, single-use grant.
//! 3. The platform browser adapter is
//!    handed `http://<addr>/<page>#launch_grant=<grant>`. The grant rides in
//!    the URL *fragment*: fragments are never sent to a server and never land
//!    in server logs. The 2026-09-03 amendment prohibits query-string secrets
//!    only; `bootstrap.ts` strips the fragment synchronously (via
//!    `history.replaceState`) before its first network call, so the grant
//!    never survives into a reload, a `Referer`, or history.
//!
//! Redaction discipline: the grant and the bearer are
//! [`Secret`](hieronymus::secret::Secret)s. The opener's argument (the URL,
//! which carries the grant) is never printed or logged, and the opener's own
//! stdout/stderr are discarded. An opener failure is reported with the origin
//! and page path plus the opener's own (secret-free) cause — never the
//! fragment or the grant.

#[cfg(all(test, target_os = "linux"))]
use crate::platform::browser::open_with_command;
#[cfg(all(test, target_os = "linux"))]
use std::time::{Duration, Instant};

use serde_json::json;

use hieronymus::data_root::HieronymusConfig;

use crate::lifecycle;

/// The console pages the CLI can open. Each name is also the SPA entry path:
/// the Svelte app switches its initial view on `window.location.pathname`.
pub const PAGES: [&str; 2] = ["admin", "config"];

/// Mint a launch grant through the local daemon and open `page` in the
/// browser, already carrying the grant in its URL fragment.
///
/// `page` must be one of [`PAGES`]. Errors are plain strings safe to print:
/// they never contain the grant or the bearer.
pub fn launch(config: &HieronymusConfig, page: &str) -> Result<(), String> {
    if !PAGES.contains(&page) {
        return Err(format!(
            "unknown console page {page:?}; expected one of {PAGES:?}"
        ));
    }

    let options = lifecycle::default_service_options(config)
        .map_err(|_| "Could not locate service configuration")?;
    launch_with_options(config, &options, page)
}

/// Shared serialized console operation, including optional start and opener.
pub fn launch_with_options(
    config: &HieronymusConfig,
    options: &crate::service::ServiceOptions,
    page: &str,
) -> Result<(), String> {
    if !PAGES.contains(&page) {
        return Err("Unknown console page".into());
    }
    let operation = lifecycle::operation::LifecycleOperation::acquire(config)
        .map_err(|_| "Another lifecycle operation is in progress or the root is unavailable")?;
    let client = lifecycle::connect_guarded(config, options, &operation)
        .map_err(|error| error.to_string())?
        .with_local_credential(config, crate::daemon::discovery::LocalCredential::Console)
        .map_err(|error| error.to_string())?;
    let address = client.address();
    let origin = format!("http://{address}");
    let mut url = format!("{origin}/{page}");
    {
        // The running daemon owns the active mode. Always obtaining a grant
        // also works if web.conf changed since that daemon started.
        let response = client
            .post("/auth/launch-grant", &json!({}))
            .map_err(|error| error.to_string())?;
        let grant = response
            .get("launch_grant")
            .and_then(serde_json::Value::as_str)
            .filter(|grant| !grant.is_empty())
            .ok_or("the server did not return a launch grant")?;
        url.push_str(&format!("#launch_grant={grant}"));
    }

    // The opener diagnostic never includes the grant-bearing URL.
    crate::platform::browser::open(options, &url).map_err(|cause| {
        format!(
            "could not open a browser for {origin}/{page} ({cause}). Re-run `hiero {page}` from \
             a terminal where a browser can open."
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn browser_timeout_is_bounded_reaped_and_secret_free() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let opener = directory.path().join("opener");
        let pid_file = directory.path().join("pid");
        std::fs::write(
            &opener,
            format!(
                "#!/bin/sh\nprintf '%s' \"$$\" > '{}'\nexec sleep 30\n",
                pid_file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&opener, std::fs::Permissions::from_mode(0o700)).unwrap();
        let started = Instant::now();
        let error = open_with_command(
            opener.to_str().unwrap(),
            "http://127.0.0.1/admin#launch_grant=SECRET",
            Duration::from_millis(100),
        )
        .unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(!error.contains("SECRET"));
        assert!(!error.contains("http"));
        let pid = std::fs::read_to_string(pid_file).unwrap();
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    }

    #[test]
    fn unknown_pages_are_rejected_before_any_daemon_contact() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        let error = launch(&config, "dashboard").unwrap_err();
        assert!(error.contains("unknown console page"), "{error}");
        assert!(
            error.contains("admin") && error.contains("config"),
            "{error}"
        );
    }
}
