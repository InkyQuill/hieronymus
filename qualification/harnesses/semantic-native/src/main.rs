//! The `semantic-native` qualification CLI.
//!
//! Grammar:
//!
//! ```text
//! semantic-native scenario --work-dir <path> --state-db <path> \
//!     --mode complete|crash|cancel --generation <id> --stop-after <count>
//! semantic-native query --work-dir <path> --series <slug> --query-index <0..49>
//! semantic-native fts-fallback --work-dir <path> --series <slug> --query-index <0..49>
//! ```
//!
//! Policy enforced before anything is written:
//!
//! - work directories must live beneath
//!   `qualification/.artifacts/work/semantic-native`; the disposable test
//!   harness relocates that root through
//!   `SEMANTIC_NATIVE_QUALIFICATION_WORK_ROOT`;
//! - symlinks on the work-directory or state-database paths are rejected;
//! - any `http(s)://` argument with a non-loopback host is rejected (the CLI
//!   itself performs no downloads);
//! - the model checksum, dimensions, and generation ids are verified by the
//!   scenario library before the first batch is written.
//!
//! Everything printed is normalized evidence — counts, ids, checksums,
//! digests — never a raw vector.

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, ensure};
use hieronymus_semantic_native_qualification::fts;
#[cfg(feature = "semantic-native")]
use hieronymus_semantic_native_qualification::scenario::{self, ScenarioMode};

const WORK_ROOT_OVERRIDE: &str = "SEMANTIC_NATIVE_QUALIFICATION_WORK_ROOT";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match dispatch(&arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("semantic-native: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(arguments: &[String]) -> anyhow::Result<()> {
    reject_foreign_acquisition_urls(arguments)?;
    let (command, rest) = arguments
        .split_first()
        .context("a subcommand is required: scenario | query | fts-fallback")?;
    match command.as_str() {
        "scenario" => cmd_scenario(rest),
        "query" => cmd_query(rest),
        "fts-fallback" => cmd_fts_fallback(rest),
        other => {
            anyhow::bail!("unknown subcommand {other:?}; expected scenario | query | fts-fallback")
        }
    }
}

/// Rejects any `http(s)://` argument whose host is not loopback. The CLI
/// performs no acquisition itself; this guard keeps a non-loopback URL from
/// ever being accepted on the command line before any work starts.
fn reject_foreign_acquisition_urls(arguments: &[String]) -> anyhow::Result<()> {
    for argument in arguments {
        let Some((scheme, rest)) = argument.split_once("://") else {
            continue;
        };
        if !matches!(scheme, "http" | "https") {
            continue;
        }
        let host = rest.split(['/', '?', '#', '@', ':']).next().unwrap_or("");
        let loopback = host == "localhost"
            || host
                .parse::<Ipv4Addr>()
                .map(|ip| ip.is_loopback())
                .unwrap_or(false)
            || host.strip_prefix('[').map(|h| h == "::1]").unwrap_or(false);
        ensure!(
            loopback,
            "rejecting non-loopback acquisition URL host {host:?}"
        );
    }
    Ok(())
}

#[derive(Debug)]
struct Flags(BTreeMap<String, String>);

impl Flags {
    fn parse(arguments: &[String], allowed: &[&str]) -> anyhow::Result<Self> {
        let mut flags = BTreeMap::new();
        let mut index = 0;
        while index < arguments.len() {
            let argument = &arguments[index];
            let (name, inline) = match argument.split_once('=') {
                Some((name, value)) => (name.to_string(), Some(value.to_string())),
                None => (argument.clone(), None),
            };
            ensure!(name.starts_with("--"), "unexpected argument {argument:?}");
            ensure!(allowed.contains(&name.as_str()), "unknown flag {name}");
            ensure!(!flags.contains_key(&name), "duplicate flag {name}");
            let value = match inline {
                Some(value) => value,
                None => {
                    index += 1;
                    arguments
                        .get(index)
                        .cloned()
                        .with_context(|| format!("flag {name} requires a value"))?
                }
            };
            ensure!(!value.starts_with("--"), "flag {name} requires a value");
            flags.insert(name, value);
            index += 1;
        }
        Ok(Self(flags))
    }

    fn require(&self, name: &str) -> anyhow::Result<&str> {
        self.0
            .get(name)
            .map(String::as_str)
            .with_context(|| format!("missing required flag {name}"))
    }
}

fn allowed_work_root() -> anyhow::Result<PathBuf> {
    if let Ok(root) = std::env::var(WORK_ROOT_OVERRIDE) {
        ensure!(
            !root.trim().is_empty(),
            "work root override must not be empty"
        );
        return Ok(PathBuf::from(root));
    }
    let qualification_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .context("the harness lives two levels below the qualification root")?;
    Ok(qualification_root.join(".artifacts/work/semantic-native"))
}

/// Rejects supplied paths carrying `ParentDir`/`CurDir` components: such a
/// path can pass a lexical containment check while resolving outside the
/// allowed root, so it must never reach a `create_dir_all` or an `open`.
fn reject_trivial_components(path: &Path, what: &str, allowed_root: &Path) -> anyhow::Result<()> {
    for component in path.components() {
        if matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::CurDir
        ) {
            anyhow::bail!(
                "{what} {} is outside the allowed root {}: \
                 parent-directory and current-directory components are rejected",
                path.display(),
                allowed_root.display()
            );
        }
    }
    Ok(())
}

/// Resolves `path` to a canonical absolute path without creating anything:
/// the longest existing ancestor is canonicalized and the remaining
/// (already trivial-component-free) names are re-attached lexically.
#[cfg(feature = "semantic-native")]
fn resolve_existing(path: &Path) -> anyhow::Result<PathBuf> {
    let mut suffix = Vec::new();
    let mut cursor = path.to_path_buf();
    loop {
        match cursor.canonicalize() {
            Ok(canonical) => {
                let mut resolved = canonical;
                for component in suffix.into_iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) => {
                let name = cursor
                    .file_name()
                    .with_context(|| format!("could not resolve {}: {error}", path.display()))?;
                suffix.push(name.to_owned());
                cursor = cursor
                    .parent()
                    .with_context(|| format!("could not resolve {}: no parent", path.display()))?
                    .to_path_buf();
            }
        }
    }
}

fn validate_work_dir(work_dir: &Path) -> anyhow::Result<()> {
    validate_work_dir_in(&allowed_work_root()?, work_dir)
}

fn validate_work_dir_in(allowed_root: &Path, work_dir: &Path) -> anyhow::Result<()> {
    // Structural rejection first: nothing may be created for a malformed path.
    reject_trivial_components(work_dir, "work directory", allowed_root)?;
    std::fs::create_dir_all(allowed_root).with_context(|| {
        format!(
            "could not create the allowed work root {}",
            allowed_root.display()
        )
    })?;
    // Reject symlinks on every existing component at or below the root.
    let mut cursor = Some(work_dir.to_path_buf());
    while let Some(path) = cursor {
        if !path.starts_with(allowed_root) {
            break;
        }
        if let Ok(metadata) = std::fs::symlink_metadata(&path) {
            ensure!(
                !metadata.is_symlink(),
                "work directory component {} is a symlink",
                path.display()
            );
        }
        cursor = path.parent().map(Path::to_path_buf);
    }
    ensure!(
        work_dir.starts_with(allowed_root),
        "work directory {} is outside the allowed root {}",
        work_dir.display(),
        allowed_root.display()
    );
    // Only after every check may the directory be created; the canonical
    // re-check below then guards against resolution outside the root.
    std::fs::create_dir_all(work_dir)
        .with_context(|| format!("could not create the work directory {}", work_dir.display()))?;
    let canonical_work = work_dir.canonicalize()?;
    let canonical_root = allowed_root.canonicalize()?;
    ensure!(
        canonical_work.starts_with(&canonical_root),
        "work directory {} resolves outside the allowed root {}",
        canonical_work.display(),
        canonical_root.display()
    );
    Ok(())
}

#[cfg(feature = "semantic-native")]
fn validate_state_db(work_dir: &Path, state_db: &Path) -> anyhow::Result<()> {
    validate_state_db_in(&allowed_work_root()?, work_dir, state_db)
}

#[cfg(feature = "semantic-native")]
fn validate_state_db_in(
    allowed_root: &Path,
    work_dir: &Path,
    state_db: &Path,
) -> anyhow::Result<()> {
    // Structural rejection first: a `--state-db` with `..`/`.` components can
    // pass a lexical `starts_with(work_dir)` while resolving outside the work
    // directory — and with enough `..`, outside the allowed root entirely.
    reject_trivial_components(state_db, "state database", allowed_root)?;
    if let Ok(metadata) = std::fs::symlink_metadata(state_db) {
        ensure!(
            !metadata.is_symlink(),
            "state database {} is a symlink",
            state_db.display()
        );
    }
    let mut cursor = Some(state_db.to_path_buf());
    while let Some(path) = cursor {
        if !path.starts_with(work_dir) {
            break;
        }
        if let Ok(metadata) = std::fs::symlink_metadata(&path) {
            ensure!(
                !metadata.is_symlink(),
                "state database component {} is a symlink",
                path.display()
            );
        }
        cursor = path.parent().map(Path::to_path_buf);
    }
    ensure!(
        state_db.starts_with(work_dir),
        "state database {} must live inside the work directory {}",
        state_db.display(),
        work_dir.display()
    );
    // Canonical containment against the allowed root, without creating
    // anything on the rejection path.
    let canonical_root = allowed_root.canonicalize().with_context(|| {
        format!(
            "could not resolve the allowed work root {}",
            allowed_root.display()
        )
    })?;
    let resolved = resolve_existing(state_db)?;
    ensure!(
        resolved.starts_with(&canonical_root),
        "state database {} is outside the allowed root {}",
        state_db.display(),
        allowed_root.display()
    );
    Ok(())
}

fn parse_query_index(value: &str) -> anyhow::Result<usize> {
    let index: usize = value
        .parse()
        .with_context(|| format!("query index {value:?} is not a number"))?;
    ensure!(
        index <= 49,
        "query index {index} is outside the mandated 0..49 range"
    );
    Ok(index)
}

#[cfg(feature = "semantic-native")]
fn parse_stop_after(flags: &Flags) -> anyhow::Result<u64> {
    let value = flags.require("--stop-after")?;
    let stop_after: u64 = value
        .parse()
        .with_context(|| format!("stop-after {value:?} is not a count"))?;
    ensure!(stop_after > 0, "stop-after must be positive");
    Ok(stop_after)
}

#[cfg(feature = "semantic-native")]
fn block_on<F: std::future::Future>(future: F) -> anyhow::Result<F::Output> {
    let runtime = tokio::runtime::Runtime::new().context("could not start the tokio runtime")?;
    Ok(runtime.block_on(future))
}

fn cmd_scenario(arguments: &[String]) -> anyhow::Result<()> {
    #[cfg(not(feature = "semantic-native"))]
    {
        let _ = arguments;
        anyhow::bail!(
            "the scenario subcommand requires the semantic-native feature; \
             this build ships the FTS fallback only"
        )
    }
    #[cfg(feature = "semantic-native")]
    {
        let flags = Flags::parse(
            arguments,
            &[
                "--work-dir",
                "--state-db",
                "--mode",
                "--generation",
                "--stop-after",
            ],
        )?;
        let work_dir = PathBuf::from(flags.require("--work-dir")?);
        let state_db = PathBuf::from(flags.require("--state-db")?);
        let generation = flags.require("--generation")?.to_string();
        let mode = match flags.require("--mode")? {
            "complete" => ScenarioMode::Complete,
            "crash" => ScenarioMode::Crash {
                stop_after: parse_stop_after(&flags)?,
            },
            "cancel" => ScenarioMode::Cancel {
                stop_after: parse_stop_after(&flags)?,
            },
            other => {
                anyhow::bail!("unknown scenario mode {other:?}; expected complete | crash | cancel")
            }
        };
        validate_work_dir(&work_dir)?;
        validate_state_db(&work_dir, &state_db)?;
        let summary = block_on(scenario::run_scenario(scenario::ScenarioOptions {
            work_dir,
            state_db,
            generation,
            mode,
            probe: scenario::TransactionProbe::new(),
        }))??;
        println!("{}", serde_json::to_string(&summary)?);
        Ok(())
    }
}

fn cmd_query(arguments: &[String]) -> anyhow::Result<()> {
    #[cfg(not(feature = "semantic-native"))]
    {
        let _ = arguments;
        anyhow::bail!(
            "the query subcommand requires the semantic-native feature; \
             this build ships the FTS fallback only"
        )
    }
    #[cfg(feature = "semantic-native")]
    {
        let flags = Flags::parse(arguments, &["--work-dir", "--series", "--query-index"])?;
        let work_dir = PathBuf::from(flags.require("--work-dir")?);
        let series = flags.require("--series")?.to_string();
        let query_index = parse_query_index(flags.require("--query-index")?)?;
        validate_work_dir(&work_dir)?;
        let state_db = work_dir.join("state.sqlite3");
        validate_state_db(&work_dir, &state_db)?;
        let receipt = block_on(scenario::run_active_query(
            &work_dir.join("index"),
            &state_db,
            &series,
            query_index,
        ))??;
        println!("{}", serde_json::to_string(&receipt)?);
        Ok(())
    }
}

fn cmd_fts_fallback(arguments: &[String]) -> anyhow::Result<()> {
    let flags = Flags::parse(arguments, &["--work-dir", "--series", "--query-index"])?;
    let work_dir = PathBuf::from(flags.require("--work-dir")?);
    let series = flags.require("--series")?.to_string();
    let query_index = parse_query_index(flags.require("--query-index")?)?;
    validate_work_dir(&work_dir)?;
    let database_path = work_dir.join("fts-fallback.sqlite3");
    let fallback = fts::FtsFallback::open_at(&database_path)?;
    let receipt = fallback.search(query_index, &series)?;

    // The result must be exactly the independently derived eligible id set.
    let expected = fts::expected_fts_id_digests()?;
    ensure!(
        expected.len() == fts::FTS_QUERY_COUNT,
        "the recipe produced {} expected digests",
        expected.len()
    );
    ensure!(
        receipt.id_digest == expected[query_index],
        "FTS fallback ids for {} diverge from the corpus recipe",
        receipt.query_id
    );

    // Prove delete/rebuild equivalence inline on the same disposable store.
    fallback.delete_index()?;
    let emptied = fallback.search(query_index, &series)?;
    ensure!(
        emptied.matched_ids.is_empty(),
        "FTS index still returns rows after delete-all"
    );
    fallback.rebuild_index()?;
    let rebuilt = fallback.search(query_index, &series)?;
    ensure!(
        rebuilt.id_digest == expected[query_index],
        "rebuilt FTS index diverges from the corpus recipe"
    );

    println!("{}", serde_json::to_string(&rebuilt)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_dir_policy_rejects_trivial_components_before_any_write() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let allowed_root = root.path().join("allowed");

        // `..` must be rejected before the work directory is created, so no
        // stray directory may materialize outside the allowed root.
        let escape = allowed_root.join("../escape");
        let error = validate_work_dir_in(&allowed_root, &escape)
            .expect_err("parent-dir component must be rejected");
        ensure!(
            error.to_string().contains("outside the allowed root"),
            "unexpected rejection error: {error}"
        );
        assert!(
            !root.path().join("escape").exists(),
            "a rejected work directory must not be created on disk"
        );

        // `.` components (which survive in `components()` only as a leading
        // `./`) are equally rejected.
        let curdir = Path::new("./stray");
        assert!(
            validate_work_dir_in(&allowed_root, curdir).is_err(),
            "curdir component must be rejected"
        );
        let nested_trivial = allowed_root.join("work/sub/../deeper");
        assert!(
            validate_work_dir_in(&allowed_root, &nested_trivial).is_err(),
            "embedded parent-dir component must be rejected"
        );
        assert!(
            !root.path().join("escape").exists() && !Path::new("stray").exists(),
            "no directory may be created for a rejected path"
        );

        // A clean path inside the root is still accepted (and created).
        let work = allowed_root.join("work");
        validate_work_dir_in(&allowed_root, &work)?;
        assert!(work.is_dir());
        Ok(())
    }

    #[cfg(feature = "semantic-native")]
    #[test]
    fn state_db_policy_rejects_escape_before_any_write() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let allowed_root = root.path().join("allowed");
        let work = allowed_root.join("work");
        std::fs::create_dir_all(&work)?;

        // `--state-db <work-dir>/../../../outside.sqlite3` passes a lexical
        // starts_with(work_dir) check; validation must reject it (and must not
        // create anything outside the allowed root).
        let escape = work.join("../../../outside.sqlite3");
        let error = validate_state_db_in(&allowed_root, &work, &escape)
            .expect_err("parent-dir escape must be rejected");
        ensure!(
            error.to_string().contains("outside the allowed root"),
            "unexpected rejection error: {error}"
        );
        assert!(
            !root.path().join("outside.sqlite3").exists(),
            "rejected state database path must not be created"
        );
        let curdir = Path::new("./state.sqlite3");
        assert!(
            validate_state_db_in(&allowed_root, &work, curdir).is_err(),
            "curdir component must be rejected"
        );

        // A state database inside the work directory is still accepted, and a
        // symlinked one is still rejected.
        validate_state_db_in(&allowed_root, &work, &work.join("state.sqlite3"))?;
        std::fs::write(work.join("plain"), b"")?;
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(work.join("plain"), work.join("link.sqlite3"))?;
            assert!(
                validate_state_db_in(&allowed_root, &work, &work.join("link.sqlite3")).is_err(),
                "symlinked state database must be rejected"
            );
        }
        Ok(())
    }
}
