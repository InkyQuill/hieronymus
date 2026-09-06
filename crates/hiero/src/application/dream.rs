//! The dream tool family (plan M5): `hieronymus_dream`, the one tool the
//! M1–M4 families left unclaimed. Since task D5 the dispatch runs through
//! the daemon's [`DreamController`]: the request coalesces into the one
//! supervised worker (scheduled, admin, and MCP dream never run
//! concurrently), providers are configured lanes resolved from
//! `provider.conf`/`dream.conf` inside the controller's worker (ADR 0007),
//! and the answer is the finished drain. There is no direct
//! [`hieronymus::dreaming::DreamService`] construction here: on an
//! application without a controller (not serving a daemon) the tool fails
//! closed with a clear domain error.
//!
//! Argument semantics over the frozen schema: `provider`, when given, is a
//! fail-closed pin — it must name the provider the run will actually
//! record (the first enabled workflow's wire provider type); anything else
//! is a domain rejection, never a silent substitution. `wait` is accepted
//! for frozen-schema fidelity: the call is synchronous either way (Python's
//! `run_all` blocks for `owner="mcp"` too), and concurrent requests join
//! the active run instead of contending on the OS lock.

use serde::Deserialize;
use serde_json::{Value, json};

use hieronymus::dreaming::resolved_provider_label;

use super::AppError;
use super::Application;
use super::decode;
use super::domain;
use crate::daemon::dream_worker::DreamRequest;

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
/// completed-session memories through the daemon's dream controller and
/// report the finished drain.
fn dream(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<DreamArgs>(arguments)?;
    // `wait` is accepted for frozen-schema fidelity; the controller path is
    // synchronous for the MCP owner, so the answer always reports the
    // finished run.
    let _ = args.wait;
    if let Some(provider) = &args.provider {
        // Fail-closed pin (argument validation first): the run uses the
        // configured workflow lanes, and the answer's provider is the
        // resolved profile's wire name.
        let configured = resolved_provider_label(application.config())
            .map_err(|error| AppError::Domain(error.to_string()))?;
        if *provider != configured {
            return Err(AppError::Domain(format!(
                "provider {provider:?} is not available; the daemon runs the \
                 configured dreaming provider ({configured:?}) (fail-closed, no \
                 silent substitution)"
            )));
        }
    }
    let Some(controller) = application.dream_controller() else {
        return Err(AppError::Domain(
            "hieronymus_dream runs on the daemon's dream controller; no controller is \
             installed, so the tool is only served by a running daemon"
                .to_string(),
        ));
    };
    let drain = controller
        .request_and_wait(DreamRequest {
            all: true,
            manual: true,
        })
        .map_err(domain)?;
    let record = &drain.record;
    Ok(json!({
        "cycle_id": record.cycle_id,
        "status": record.status,
        "provider": record.provider,
        "input_count": drain.input_count,
        "created_crystal_count": drain.created_crystal_count,
        "proposal_count": drain.proposal_count,
    }))
}
