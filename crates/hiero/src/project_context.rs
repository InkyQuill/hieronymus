//! Read-only projection of CWS project metadata for the installed CLI.

use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use hieronymus::{
    cws_binding::select_binding,
    cws_project::{CwsError, CwsProject, discover},
};
use serde_json::{Value, json};

/// Inspect the nearest CWS project without loading daemon configuration or
/// returning manuscript and project-instruction contents.
pub fn inspect(cwd: &Path, direction: Option<&str>) -> Value {
    let project = match discover(cwd) {
        Ok(Some(project)) => project,
        Ok(None) => return empty_report("not_found", direction, "project_not_found"),
        Err(error) => return error_report(error, direction),
    };

    let mut report = project_report(&project, direction);
    match instructions_path(&project) {
        Ok(path) => report["instructions_path"] = json!(path),
        Err(error) => {
            report["status"] = json!("invalid");
            report["diagnostics"] = json!([diagnostic(&error)]);
            return report;
        }
    }

    match select_binding(&project, direction) {
        Ok(Some(binding)) => {
            report["status"] = json!("ready");
            report["binding"] = json!(binding);
        }
        Ok(None) => report["status"] = json!("unbound"),
        Err(error) => {
            report["status"] = json!(status(&error));
            report["diagnostics"] = json!([diagnostic(&error)]);
        }
    }
    report
}

/// Map the public inspection status to the installed CLI's process status.
pub fn exit_code(report: &Value) -> u8 {
    match report.get("status").and_then(Value::as_str) {
        Some("ready" | "unbound") => 0,
        Some("ambiguous" | "unsupported" | "not_found") => 1,
        _ => 2,
    }
}

/// Render every inspection field in contract order without exposing file
/// contents or underlying filesystem diagnostics.
pub fn render_human(report: &Value) -> String {
    const FIELDS: [&str; 10] = [
        "version",
        "status",
        "root",
        "schema_version",
        "instructions_path",
        "binding",
        "direction_id",
        "source_language",
        "target_language",
        "diagnostics",
    ];
    let mut output = String::new();
    for field in FIELDS {
        output.push_str(field);
        output.push_str(": ");
        output.push_str(&human_value(report.get(field)));
        output.push('\n');
    }
    output
}

fn project_report(project: &CwsProject, direction: Option<&str>) -> Value {
    let source_language = (project.project_kind == "authoring").then_some(&project.language);
    json!({
        "version": 1,
        "status": null,
        "root": project.root,
        "schema_version": project.schema_version,
        "instructions_path": null,
        "binding": null,
        "direction_id": direction,
        "source_language": source_language,
        "target_language": null,
        "diagnostics": [],
    })
}

fn empty_report(report_status: &str, direction: Option<&str>, code: &str) -> Value {
    json!({
        "version": 1,
        "status": report_status,
        "root": null,
        "schema_version": null,
        "instructions_path": null,
        "binding": null,
        "direction_id": direction,
        "source_language": null,
        "target_language": null,
        "diagnostics": [code],
    })
}

fn error_report(error: CwsError, direction: Option<&str>) -> Value {
    let mut report = empty_report(status(&error), direction, diagnostic(&error));
    if let CwsError::UnsupportedSchema(version) = error {
        report["schema_version"] = json!(version);
    }
    report
}

fn instructions_path(project: &CwsProject) -> Result<Option<PathBuf>, CwsError> {
    let path = project.root.join("AGENTS.md");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(CwsError::UnsafePath)
        }
        Ok(_) => Ok(Some(path)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(CwsError::Io(error)),
    }
}

fn status(error: &CwsError) -> &'static str {
    match error {
        CwsError::UnsupportedSchema(_)
        | CwsError::UnsupportedBinding(_)
        | CwsError::UnsupportedContractVersion(_)
        | CwsError::UnsupportedDirectionSelection => "unsupported",
        CwsError::AmbiguousDirection => "ambiguous",
        CwsError::UnknownDirection(_) => "not_found",
        CwsError::UnsafePath
        | CwsError::InvalidManifest
        | CwsError::InvalidBinding
        | CwsError::Io(_) => "invalid",
    }
}

fn diagnostic(error: &CwsError) -> &'static str {
    match error {
        CwsError::UnsafePath => "unsafe_path",
        CwsError::InvalidManifest => "invalid_manifest",
        CwsError::UnsupportedSchema(_) => "unsupported_schema",
        CwsError::InvalidBinding => "invalid_binding",
        CwsError::UnsupportedBinding(_) => "unsupported_binding_version",
        CwsError::UnsupportedContractVersion(_) => "unsupported_contract_version",
        CwsError::UnsupportedDirectionSelection => "unsupported_direction_selection",
        CwsError::AmbiguousDirection => "ambiguous_direction",
        CwsError::UnknownDirection(_) => "unknown_direction",
        CwsError::Io(_) => "filesystem_error",
    }
}

fn human_value(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "none".to_string(),
        Some(Value::String(value)) => value.clone(),
        Some(Value::Array(values)) if values.is_empty() => "none".to_string(),
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(","),
        Some(value) => value.to_string(),
    }
}
