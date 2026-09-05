//! The dream tool family (plan M5): `hieronymus_dream`, the one tool the
//! M1–M4 families left unclaimed. The dispatch runs the existing
//! [`DreamService`] fail-closed path — the same seam the daemon's
//! `run_manual_dreaming` REST route uses: `DreamService::open` validates the
//! dream.conf workflow wiring against provider.conf before any run (an
//! enabled LLM-declared workflow refuses deterministic substitution), and
//! `run_all` drains pending completed-session memories with the
//! deterministic provider.
//!
//! Scope note: the configured LLM provider lanes (scheduler, draining,
//! per-workflow providers) belong to the dreaming plan (D5), which upgrades
//! this dispatch. Until then a named non-deterministic provider argument is
//! an explicit domain rejection, never a silent deterministic substitution.

use serde::Deserialize;
use serde_json::{Value, json};

use hieronymus::dreaming::{DeterministicDreamProvider, DreamService};

use super::AppError;
use super::Application;
use super::decode;
use super::domain;

/// The family dispatcher: `None` means the tool is not ours.
pub(crate) fn dispatch(
    application: &Application,
    tool: &str,
    arguments: &Value,
    _actor: &str,
) -> Option<Result<Value, AppError>> {
    match tool {
        "hieronymus_dream" => Some(dream(application, arguments)),
        _ => None,
    }
}

#[derive(Deserialize)]
struct DreamArgs {
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    wait: bool,
}

/// The Python `hieronymus_dream` port: run dreaming over all pending
/// completed-session memories and report the finished run record. The call is
/// synchronous (Python's `run_all` blocks for `owner="mcp"` too; `wait` only
/// changes lock acquisition there, and the Rust lock refuses with an
/// `already running` domain error instead of waiting).
fn dream(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<DreamArgs>(arguments)?;
    // `wait` is accepted for frozen-schema fidelity; both Python's and the
    // Rust `run_all` are synchronous for the MCP owner, so the answer always
    // reports the finished run.
    let _ = args.wait;
    if let Some(provider) = &args.provider {
        // D5 (the dreaming plan) upgrades this dispatch with the configured
        // provider lanes; today only the deterministic provider exists here.
        if provider != "deterministic" {
            return Err(AppError::Domain(format!(
                "provider {provider:?} is not available; the daemon runs the \
                 deterministic dreaming provider (fail-closed, no silent \
                 substitution)"
            )));
        }
    }
    let service =
        DreamService::open(application.config(), DeterministicDreamProvider).map_err(domain)?;
    // Same seam as the daemon's manual-dreaming route: drain everything
    // pending (ignore the minimum threshold), refuse when another cycle
    // holds the lock.
    let record = service.run_all("mcp", true, false).map_err(domain)?;
    Ok(json!({
        "cycle_id": record.cycle_id,
        "status": record.status,
        "provider": record.provider,
        "input_count": record.input_count,
        "created_crystal_count": record.created_crystal_count,
        "proposal_count": record.proposal_count,
    }))
}
