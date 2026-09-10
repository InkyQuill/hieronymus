//! Admin domain surface for the web console.
//!
//! * `views` — the read-only projections behind the ten "Memory" views
//!   ([`snapshot`], [`VIEW_NAMES`]); ports the Python `AdminStore` result
//!   shapes (plan W2).
//! * `actions` — the 13 typed, audited mutations ([`run_action`],
//!   [`validate_action_request`], [`ACTION_NAMES`]); the actor is always the
//!   transport-authenticated identity, never a request field (plan W3).
//! * `audit` — the shared audit-row / crystal-row / FTS / rule-guard
//!   helpers both surfaces use.

mod actions;
mod audit;
mod views;

pub use actions::{ACTION_NAMES, run_action, validate_action_request};
pub use views::{VIEW_NAMES, snapshot};

use rusqlite::Connection;

use hieronymus::data_root::HieronymusConfig;
use hieronymus::db::open_migrated;

use crate::application::AppError;

/// Open a fresh migrated connection to the data-root database; any failure
/// collapses to a generic "admin store is unavailable" (no path or SQL text
/// leaks).
fn open_db(config: &HieronymusConfig) -> Result<Connection, AppError> {
    open_migrated(&config.database_path())
        .map_err(|_| AppError::Domain("admin store is unavailable".to_string()))
}

/// Map any late store failure (a transaction that will not begin/commit, an
/// unexpected SQL error) to the generic unavailable message.
fn store_unavailable<E>(_: E) -> AppError {
    AppError::Domain("admin store is unavailable".to_string())
}

/// RFC 3339 seconds-precision UTC, the timestamp format every admin row uses.
fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Clamp a strength/confidence score to `[0, 1]`.
fn clamp_score(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}
