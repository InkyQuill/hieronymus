use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessLevel {
    Ready,
    Degraded,
    Starting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCondition {
    Untested,
    Healthy,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProviderReadiness {
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub provider: String,
    pub model: String,
    pub revision: u64,
    pub condition: ProviderCondition,
    pub observed_at: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ReadinessSummary {
    pub level: ReadinessLevel,
    pub reasons: Vec<String>,
    pub providers: Vec<ProviderReadiness>,
}
