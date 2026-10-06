//! Retrieval categories are independent of authority and story applicability.

/// Agent-selected categories of durable memory. Omission keeps mixed recall.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    Terms,
    Concepts,
    Lessons,
    Knowledge,
}

/// Limits and optional category selection for one coherent recall.
#[derive(Debug, Clone, Copy)]
pub struct RecallOptions<'a> {
    pub limit: usize,
    pub required_decision_id: Option<&'a str>,
    pub memory_types: Option<&'a [MemoryType]>,
}

impl RecallOptions<'_> {
    pub(crate) fn crystal_types_json(&self) -> Option<String> {
        self.memory_types.map(|types| {
            let mut crystal_types = Vec::new();
            if types.contains(&MemoryType::Concepts) {
                crystal_types.extend(["concept", "concept_note"]);
            }
            if types.contains(&MemoryType::Lessons) {
                crystal_types.push("lesson");
            }
            if types.contains(&MemoryType::Knowledge) {
                crystal_types.extend(["rule", "thought", "observation", "erudition"]);
            }
            serde_json::json!(crystal_types).to_string()
        })
    }
}
