//! Versioned technical CWS bindings. Selection confers no trust or write authority.

use std::{collections::BTreeMap, fs, path::Path};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cws_project::{CwsError, CwsProject, metadata, safe_project_path, valid_identity};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectedBinding {
    pub series_slug: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    #[serde(rename = "binding_version")]
    _binding_version: u64,
    #[serde(rename = "project_contract_version")]
    _project_contract_version: u64,
    directions: BTreeMap<String, SelectedBinding>,
}

/// Read only the discovered project's own marker. Legacy fields cannot supply
/// CWS context without a valid `cws` object. Direction maps are independent of
/// the optional common-work slug; selection never falls back to the root slug.
pub fn select_binding(
    project: &CwsProject,
    direction: Option<&str>,
) -> Result<Option<SelectedBinding>, CwsError> {
    let path = safe_project_path(project, Path::new(".hieronymus.json"))?;
    let mut directions = BTreeMap::new();
    let common = match metadata(&path)? {
        None => None,
        Some(info) => {
            if !info.is_file() {
                return Err(CwsError::InvalidBinding);
            }
            let raw: Value =
                serde_json::from_slice(&fs::read(path)?).map_err(|_| CwsError::InvalidBinding)?;
            let raw = raw.as_object().ok_or(CwsError::InvalidBinding)?;
            if let Some(cws) = raw.get("cws") {
                // A future version need not have v1's fields. Read versions
                // independently before deserializing the supported shape.
                let version = cws
                    .get("binding_version")
                    .and_then(Value::as_u64)
                    .ok_or(CwsError::InvalidBinding)?;
                if version != 1 {
                    return Err(CwsError::UnsupportedBinding(version));
                }
                let contract = cws
                    .get("project_contract_version")
                    .and_then(Value::as_u64)
                    .ok_or(CwsError::InvalidBinding)?;
                if contract != 1 {
                    return Err(CwsError::UnsupportedContractVersion(contract));
                }
                let binding: Binding =
                    serde_json::from_value(cws.clone()).map_err(|_| CwsError::InvalidBinding)?;
                for (id, selected) in &binding.directions {
                    if !valid_identity(id) || selected.series_slug.trim().is_empty() {
                        return Err(CwsError::InvalidBinding);
                    }
                }
                directions = binding.directions;
                match raw.get("series_slug") {
                    None => None,
                    Some(Value::String(slug)) if !slug.trim().is_empty() => Some(SelectedBinding {
                        series_slug: slug.clone(),
                    }),
                    _ => return Err(CwsError::InvalidBinding),
                }
            } else {
                None
            }
        }
    };
    match direction {
        Some(id) if !valid_identity(id) => Err(CwsError::InvalidBinding),
        Some(id) => Ok(directions.remove(id)),
        None => Ok(common),
    }
}
