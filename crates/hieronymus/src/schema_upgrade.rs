//! The ordered Rust schema-upgrade runner (ADR 0010: "existing databases use
//! a Rust upgrade runner that executes ordered SQL and typed converters inside
//! one exclusive SQLite transaction after the durable backup is complete").
//!
//! [`apply_steps`] moves a database from one application-owned schema version
//! to another by replaying the registered steps in order. It is deliberately
//! tiny and has exactly three rules:
//!
//! 1. It runs inside the caller's transaction and never commits. The upgrade
//!    protocol owns the single commit, so a failure anywhere — here, in the
//!    FTS rebuild, in verification, or in the durable semantic-job creation —
//!    rolls the whole thing back and the older version stays published.
//! 2. It refuses an autocommit connection. A half-applied step set outside a
//!    transaction is exactly the corruption this module exists to prevent.
//! 3. It fails closed on a gap: a requested transition with no registered step
//!    is an error, never a silent no-op that would publish a version the
//!    database has not actually reached.
//!
//! Each step's SQL publishes its own version markers (`hieronymus_meta` and
//! `PRAGMA user_version`) as its last statements, and [`apply_steps`] verifies
//! the markers really reached `to` before returning.

use rusqlite::Connection;

use crate::db::RUST_META_TABLE;

/// The schema step immediately after the `global.sql` + `terminology.sql`
/// baseline: the four durable work tables.
const STEP_001_TO_002: &str = include_str!("../migrations/002-durable-work.sql");

/// The ordered step table: `(from, to, sql)`, contiguous and monotonic. The
/// v1 baseline itself is not a step — it is the schema a fresh database and
/// the Python import boundary both build directly (`db.rs`).
const STEPS: &[(i64, i64, &str)] = &[(1, 2, STEP_001_TO_002)];

/// The lowest version the ordered runner can start from. A database below it
/// predates the application-owned schema identity and is not upgradable.
pub const BASELINE_SCHEMA_VERSION: i64 = 1;

/// A fail-closed refusal from the runner itself, shaped as the `SQLITE_MISUSE`
/// the caller would get from SQLite for the same category of mistake so it
/// flows through every existing `rusqlite::Result` path unchanged. `Display`
/// renders the message verbatim.
fn refused(message: String) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_MISUSE),
        Some(message),
    )
}

/// Apply the ordered steps that move `from` to `to` through the caller's
/// transaction (`&Transaction` derefs to `&Connection`, which is the handle
/// converters and step SQL share). Nothing is committed here.
///
/// `from == to` is a no-op. `from > to` is refused: schema versions are
/// monotonic and this line never downgrades a database.
pub fn apply_steps(connection: &Connection, from: i64, to: i64) -> rusqlite::Result<()> {
    if connection.is_autocommit() {
        return Err(refused(
            "schema upgrade requires a caller-owned transaction, not autocommit".to_string(),
        ));
    }
    if from == to {
        // Still verify: a caller that passed the wrong `from` for a database
        // at some other version must fail closed rather than be told the
        // no-op succeeded.
        return verify_markers(connection, to);
    }
    if from > to {
        return Err(refused(format!(
            "refusing to downgrade schema version {from} to {to}"
        )));
    }
    if from < BASELINE_SCHEMA_VERSION {
        return Err(refused(format!(
            "schema version {from} predates the supported baseline \
             {BASELINE_SCHEMA_VERSION}"
        )));
    }

    // Walk the requested range one registered transition at a time. A missing
    // link fails closed rather than skipping ahead.
    let mut current = from;
    while current < to {
        let step = STEPS
            .iter()
            .find(|(step_from, _, _)| *step_from == current)
            .ok_or_else(|| {
                refused(format!(
                    "no registered schema step from version {current} \
                     (upgrading {from} to {to})"
                ))
            })?;
        let (_, step_to, sql) = *step;
        if step_to > to {
            return Err(refused(format!(
                "the registered step from version {current} lands at {step_to}, \
                 past the requested target {to}"
            )));
        }
        connection.execute_batch(sql)?;
        current = step_to;
    }

    // The steps publish their own markers; confirm they really did before the
    // caller treats the database as upgraded.
    verify_markers(connection, to)
}

/// Both version markers must agree with `expected`. The step SQL publishes
/// them itself, so this is the runner's proof that the database really is at
/// the version the caller is about to treat it as.
fn verify_markers(connection: &Connection, expected: i64) -> rusqlite::Result<()> {
    let marked = marked_version(connection)?;
    if marked != Some(expected) {
        return Err(refused(format!(
            "schema version marker is {marked:?}, expected {expected}"
        )));
    }
    let user_version: i64 =
        connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if user_version != expected {
        return Err(refused(format!(
            "user_version is {user_version}, expected {expected}"
        )));
    }
    Ok(())
}

/// The version currently published by the `hieronymus_meta` marker table, or
/// `None` when the table is absent or empty.
pub fn marked_version(connection: &Connection) -> rusqlite::Result<Option<i64>> {
    let present: i64 = connection.query_row(
        "select count(*) from sqlite_master where type = 'table' and name = ?1",
        [RUST_META_TABLE],
        |row| row.get(0),
    )?;
    if present == 0 {
        return Ok(None);
    }
    let mut statement = connection.prepare(&format!(
        "select schema_version from {RUST_META_TABLE} order by schema_version desc limit 1"
    ))?;
    let mut rows = statement.query([])?;
    match rows.next()? {
        Some(row) => Ok(Some(row.get(0)?)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SUPPORTED_RUST_SCHEMA_VERSION;

    #[test]
    fn the_step_table_is_contiguous_and_reaches_the_supported_version() {
        let mut expected = BASELINE_SCHEMA_VERSION;
        for (from, to, _) in STEPS {
            assert_eq!(*from, expected, "steps must be contiguous");
            assert_eq!(*to, from + 1, "steps advance one version at a time");
            expected = *to;
        }
        assert_eq!(
            expected, SUPPORTED_RUST_SCHEMA_VERSION,
            "the last registered step must land on the supported version"
        );
    }

    #[test]
    fn autocommit_connections_are_refused() {
        let connection = Connection::open_in_memory().unwrap();
        let error = apply_steps(&connection, 1, 2).unwrap_err();
        assert!(error.to_string().contains("autocommit"), "{error}");
    }

    #[test]
    fn downgrades_and_gaps_fail_closed() {
        let mut connection = Connection::open_in_memory().unwrap();
        let transaction = connection.transaction().unwrap();
        let error = apply_steps(&transaction, 2, 1).unwrap_err();
        assert!(error.to_string().contains("downgrade"), "{error}");
        // No step is registered from the supported version onwards.
        let error = apply_steps(
            &transaction,
            SUPPORTED_RUST_SCHEMA_VERSION,
            SUPPORTED_RUST_SCHEMA_VERSION + 1,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("no registered schema step"),
            "{error}"
        );
        let error = apply_steps(&transaction, 0, 2).unwrap_err();
        assert!(error.to_string().contains("baseline"), "{error}");
    }

    /// A no-op transition still proves the database really is at the version
    /// the caller claimed: `from == to` must not rubber-stamp a wrong `from`.
    #[test]
    fn a_no_op_transition_still_verifies_the_markers() {
        let mut connection = Connection::open_in_memory().unwrap();
        let transaction = connection.transaction().unwrap();

        // No markers at all: the claim cannot be verified, so it fails closed.
        let error = apply_steps(&transaction, 2, 2).unwrap_err();
        assert!(error.to_string().contains("marker"), "{error}");

        // Markers that agree with the claim: a genuine no-op, writing nothing.
        transaction
            .execute_batch(
                "create table hieronymus_meta (schema_version integer not null unique);
                 insert into hieronymus_meta values (2);
                 pragma user_version = 2;",
            )
            .unwrap();
        apply_steps(&transaction, 2, 2).unwrap();
        let tables: i64 = transaction
            .query_row(
                "select count(*) from sqlite_master where type = 'table'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tables, 1, "the no-op created nothing");

        // Markers that disagree: the caller's `from` was wrong.
        transaction
            .execute_batch("update hieronymus_meta set schema_version = 1;")
            .unwrap();
        let error = apply_steps(&transaction, 2, 2).unwrap_err();
        assert!(error.to_string().contains("marker"), "{error}");
    }
}
