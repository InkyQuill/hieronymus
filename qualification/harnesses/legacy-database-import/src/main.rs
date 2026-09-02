//! `legacy-database-import` — read-only classification and neutral probe
//! import for the frozen SQLite compatibility fixtures.
//!
//! Exit codes:
//! - `0` success,
//! - `1` classification/contract/probe failure,
//! - `2` path-boundary violation, rejected before SQLite ever opens.

mod classify;
mod probe_import;
mod report;

use anyhow::Result;
use classify::{
    DatabaseContract, validate_contract_path, validate_fixture_root, validate_source,
    validate_target, validate_work_root,
};
use probe_import::{ProbeRefusal, probe_import};
use report::ProbeRefusalReport;
use std::path::{Path, PathBuf};

const USAGE: &str = "usage:
  legacy-database-import classify --fixture-root <dir> --source-name <basename> --contract <file>
  legacy-database-import probe-import --fixture-root <dir> --source-name <basename> --contract <file> --work-root <dir> --target-name <basename>";

struct ClassifyArgs {
    fixture_root: PathBuf,
    source_name: String,
    contract: PathBuf,
}

struct ProbeImportArgs {
    fixture_root: PathBuf,
    source_name: String,
    contract: PathBuf,
    work_root: PathBuf,
    target_name: String,
}

enum Invocation {
    Classify(ClassifyArgs),
    ProbeImport(ProbeImportArgs),
}

fn parse_invocation(args: &[String]) -> Option<Invocation> {
    let (verb, flags) = args.split_first()?;
    let mut fixture_root = None;
    let mut source_name = None;
    let mut contract = None;
    let mut work_root = None;
    let mut target_name = None;
    let mut index = 0;
    while index < flags.len() {
        let value = flags.get(index + 1)?;
        match flags[index].as_str() {
            "--fixture-root" => fixture_root = Some(PathBuf::from(value.as_str())),
            "--source-name" => source_name = Some(value.clone()),
            "--contract" => contract = Some(PathBuf::from(value.as_str())),
            "--work-root" => work_root = Some(PathBuf::from(value.as_str())),
            "--target-name" => target_name = Some(value.clone()),
            _ => return None,
        }
        index += 2;
    }
    match verb.as_str() {
        "classify" => Some(Invocation::Classify(ClassifyArgs {
            fixture_root: fixture_root?,
            source_name: source_name?,
            contract: contract?,
        })),
        "probe-import" => Some(Invocation::ProbeImport(ProbeImportArgs {
            fixture_root: fixture_root?,
            source_name: source_name?,
            contract: contract?,
            work_root: work_root?,
            target_name: target_name?,
        })),
        _ => None,
    }
}

/// Every path boundary is validated before SQLite opens anything.
fn validate_classify_paths(fixture_root: &Path, source_name: &str, contract: &Path) -> Result<()> {
    validate_fixture_root(fixture_root)?;
    let source = fixture_root.join(source_name);
    validate_source(&source, fixture_root)?;
    validate_contract_path(contract)?;
    Ok(())
}

fn validate_probe_paths(args: &ProbeImportArgs) -> Result<()> {
    validate_classify_paths(&args.fixture_root, &args.source_name, &args.contract)?;
    let target = args.work_root.join(&args.target_name);
    validate_work_root(&args.work_root)?;
    validate_target(&target, &args.work_root)?;
    Ok(())
}

fn run_classify(args: &ClassifyArgs) -> i32 {
    if let Err(error) =
        validate_classify_paths(&args.fixture_root, &args.source_name, &args.contract)
    {
        eprintln!("rejected: {error:#}");
        return 2;
    }
    let source = args.fixture_root.join(&args.source_name);
    match classify::classify_read_only(&source, &args.fixture_root, &args.contract) {
        Ok(classification) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&classification).expect("classification serializes")
            );
            0
        }
        Err(error) => {
            eprintln!("classify failed: {error:#}");
            1
        }
    }
}

fn run_probe_import(args: &ProbeImportArgs) -> i32 {
    if let Err(error) = validate_probe_paths(args) {
        eprintln!("rejected: {error:#}");
        return 2;
    }
    let contract = match DatabaseContract::load(&args.contract) {
        Ok(contract) => contract,
        Err(error) => {
            eprintln!("contract load failed: {error:#}");
            return 1;
        }
    };
    let source = args.fixture_root.join(&args.source_name);
    let target = args.work_root.join(&args.target_name);
    match probe_import(
        &source,
        &target,
        &args.fixture_root,
        &args.work_root,
        &contract,
    ) {
        Ok(receipt) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&receipt).expect("receipt serializes")
            );
            0
        }
        Err(error) => {
            if let Some(refusal) = error.downcast_ref::<ProbeRefusal>() {
                let report = ProbeRefusalReport {
                    ok: false,
                    source_name: args.source_name.clone(),
                    classification: refusal.classification.clone(),
                    safe_to_convert: false,
                    error_code: refusal.error_code.clone(),
                };
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).expect("refusal report serializes")
                );
            } else {
                eprintln!("probe-import failed: {error:#}");
            }
            1
        }
    }
}

fn run(args: &[String]) -> i32 {
    let Some(invocation) = parse_invocation(args) else {
        eprintln!("{USAGE}");
        return 2;
    };
    match invocation {
        Invocation::Classify(args) => run_classify(&args),
        Invocation::ProbeImport(args) => run_probe_import(&args),
    }
}

fn main() {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    std::process::exit(run(&args));
}
