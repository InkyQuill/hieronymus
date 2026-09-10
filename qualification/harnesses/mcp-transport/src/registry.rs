use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;

pub const PROTOCOL_REVISION: &str = "2026-07-28";
const MAX_FIXTURE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

pub trait RegistryProbe: Sized {
    fn load(snapshot: &Path) -> Result<Self>;
    fn list_tools(&self) -> &[ToolDefinition];
    fn fixture_result(&self, name: &str, arguments: &Value) -> Result<Value>;
}

#[derive(Deserialize)]
struct Snapshot {
    tools: Vec<SnapshotTool>,
}

#[derive(Deserialize)]
struct SnapshotTool {
    name: String,
    description: String,
    input_schema: Value,
}

#[derive(Clone, Debug)]
struct FixtureResult {
    arguments: Value,
    result: Value,
}

#[derive(Clone, Debug)]
pub struct FrozenRegistry {
    tools: Vec<ToolDefinition>,
    fixture_results: BTreeMap<String, FixtureResult>,
    digest: String,
}

impl RegistryProbe for FrozenRegistry {
    fn load(snapshot: &Path) -> Result<Self> {
        let bytes = read_bounded(snapshot)?;
        let digest = hex_digest(&bytes);
        let snapshot: Snapshot =
            serde_json::from_slice(&bytes).map_err(|_| anyhow!("invalid registry JSON"))?;
        if snapshot.tools.is_empty() {
            bail!("registry contains no tools");
        }
        let mut names = BTreeSet::new();
        let mut tools = Vec::with_capacity(snapshot.tools.len());
        for tool in snapshot.tools {
            if tool.name.is_empty()
                || tool.description.is_empty()
                || !tool.input_schema.is_object()
                || !names.insert(tool.name.clone())
            {
                bail!("invalid registry tool definition");
            }
            let definition = ToolDefinition {
                name: tool.name,
                description: tool.description,
                input_schema: tool.input_schema,
            };
            prove_rmcp_tool_round_trip(&definition)?;
            tools.push(definition);
        }
        Ok(Self {
            tools,
            fixture_results: BTreeMap::new(),
            digest,
        })
    }

    fn list_tools(&self) -> &[ToolDefinition] {
        &self.tools
    }

    fn fixture_result(&self, name: &str, arguments: &Value) -> Result<Value> {
        let fixture = self
            .fixture_results
            .get(name)
            .ok_or_else(|| anyhow!("no canned result for tool"))?;
        if &fixture.arguments != arguments {
            bail!("arguments do not match canned fixture");
        }
        Ok(fixture.result.clone())
    }
}

impl FrozenRegistry {
    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub fn contains_tool(&self, name: &str) -> bool {
        self.tools.iter().any(|tool| tool.name == name)
    }

    pub fn load_fixture_result(&mut self, protocol_target: &Value) -> Result<()> {
        let request = protocol_target
            .pointer("/tools_call/request")
            .ok_or_else(|| anyhow!("missing call request fixture"))?;
        let result = protocol_target
            .pointer("/tools_call/response/result")
            .ok_or_else(|| anyhow!("missing call result fixture"))?;
        let name = request
            .pointer("/params/name")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("invalid call name fixture"))?;
        let arguments = request
            .pointer("/params/arguments")
            .ok_or_else(|| anyhow!("missing call arguments fixture"))?;
        if !self.tools.iter().any(|tool| tool.name == name) || !result.is_object() {
            bail!("call fixture is not represented by the registry");
        }
        let rmcp_result: rmcp::model::CallToolResult = serde_json::from_value(result.clone())
            .map_err(|_| anyhow!("rmcp cannot represent call result fixture"))?;
        if serde_json::to_value(rmcp_result)
            .map_err(|_| anyhow!("rmcp call result cannot serialize"))?
            != *result
        {
            bail!("rmcp call result wire shape mismatch");
        }
        self.fixture_results.insert(
            name.to_owned(),
            FixtureResult {
                arguments: arguments.clone(),
                result: result.clone(),
            },
        );
        Ok(())
    }
}

pub fn read_json_bounded(path: &Path) -> Result<Value> {
    let bytes = read_bounded(path)?;
    serde_json::from_slice(&bytes).map_err(|_| anyhow!("invalid fixture JSON"))
}

fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    let file = File::open(path).map_err(|_| anyhow!("fixture is unavailable"))?;
    let mut bytes = Vec::new();
    file.take(MAX_FIXTURE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow!("fixture cannot be read"))?;
    if bytes.len() as u64 > MAX_FIXTURE_BYTES {
        bail!("fixture exceeds size limit");
    }
    Ok(bytes)
}

fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn prove_rmcp_tool_round_trip(definition: &ToolDefinition) -> Result<()> {
    let wire = serde_json::to_value(definition)
        .map_err(|_| anyhow!("tool definition cannot serialize"))?;
    let rmcp_tool: rmcp::model::Tool = serde_json::from_value(wire.clone())
        .map_err(|_| anyhow!("rmcp cannot represent tool definition"))?;
    let round_trip = serde_json::to_value(rmcp_tool)
        .map_err(|_| anyhow!("rmcp tool definition cannot serialize"))?;
    if round_trip != wire {
        bail!("rmcp tool definition wire shape mismatch");
    }
    Ok(())
}
