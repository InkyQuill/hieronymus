//! `hiero admin` / `hiero config`: open an authenticated web console tab.
//!
//! There is no in-browser way to obtain a session: the daemon only hands out a
//! session cookie in exchange for a one-time launch grant, and only a
//! bearer-authenticated local command can mint one. This module is that
//! command.
//!
//! Flow (ADR 0012 as amended 2026-09-03, ADR 0013):
//!
//! 1. [`lifecycle::connect`] discovers (and, if absent, starts) the local
//!    daemon and yields a bearer-authenticated `lifecycle::DaemonClient`.
//! 2. `POST /auth/launch-grant` mints a 60-second, single-use grant.
//! 3. The platform opener (`xdg-open`; Linux is the only cutover target) is
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

use std::process::{Command, Stdio};

use serde_json::json;

use hieronymus::data_root::HieronymusConfig;

use crate::lifecycle;

/// The console pages the CLI can open. Each name is also the SPA entry path:
/// the Svelte app switches its initial view on `window.location.pathname`.
pub const PAGES: [&str; 2] = ["admin", "config"];

/// Test/debug seam: overrides the browser opener command (one executable name
/// or path, invoked with the URL as its only argument). It is honored in
/// release builds so the integration tests can point it at a capture script,
/// but it is deliberately kept out of the user-facing usage text. Unset in
/// normal use, where `xdg-open` is the opener.
const OPENER_ENV: &str = "HIERO_CONSOLE_BROWSER";

/// The default opener for the Linux cutover target (ADR 0013). No macOS `open`
/// fallback: the first Rust cutover is `x86_64-unknown-linux-gnu` only.
const DEFAULT_OPENER: &str = "xdg-open";

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

    let client = lifecycle::connect(config, true).map_err(|error| error.to_string())?;
    let response = client
        .post("/auth/launch-grant", &json!({}))
        .map_err(|error| error.to_string())?;
    let grant = response
        .get("launch_grant")
        .and_then(serde_json::Value::as_str)
        .filter(|grant| !grant.is_empty())
        .ok_or_else(|| "the daemon did not return a launch grant".to_string())?;

    let address = client.address();
    let origin = format!("http://{address}");
    // The only place the grant appears: the fragment of the URL handed to the
    // opener. Never logged, never printed.
    let url = format!("{origin}/{page}#launch_grant={grant}");

    // The cause from `open_in_browser` is secret-free (an opener name plus a
    // spawn error or exit status), so it is safe to surface. Manually browsing
    // to the page is not an option — the console cannot sign in on its own, so
    // the only recovery is to re-run this command where a browser can open.
    open_in_browser(&url).map_err(|cause| {
        format!(
            "could not open a browser for {origin}/{page} ({cause}). Re-run `hiero {page}` from \
             a terminal where a browser can open."
        )
    })
}

/// Invoke the platform opener with `url`, discarding its streams so the URL
/// (which carries the grant) cannot be echoed anywhere. Returns `Err` when the
/// opener cannot be spawned or exits non-zero.
fn open_in_browser(url: &str) -> Result<(), String> {
    let opener = std::env::var(OPENER_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_OPENER.to_string());

    let status = Command::new(&opener)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("{opener}: {error}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("{opener} exited with {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
