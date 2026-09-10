/// Deterministic concept authority (ADR 0011): one concept with multilingual
/// facets; rule crystals are an advisory projection of these records.
#[derive(Debug, Clone, PartialEq)]
pub struct ConceptRecord {
    pub id: i64,
    pub canonical_name: String,
    pub description: String,
    /// Public lifecycle status: `candidate`, `established`, `archived`,
    /// `merged` (legacy `vague`/`solid` map to the first two).
    pub status: String,
    pub confidence: f64,
    pub scope_type: String,
    pub scope_key: String,
    pub tags: Vec<String>,
    pub merged_into_concept_id: Option<i64>,
}

impl ConceptRecord {
    pub fn validate(&self) -> Result<(), String> {
        if self.scope_type == "global" {
            if !self.scope_key.is_empty() {
                return Err("global concept scope requires an empty key".to_string());
            }
            return Ok(());
        }
        if self.scope_key.is_empty() {
            return Err("non-global concept scope requires a key".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConceptFacetRecord {
    pub id: i64,
    pub concept_id: i64,
    pub language: String,
    /// Storage kind: `name`/`rendering`/`description`/`note` plus the
    /// compatibility kinds `alias`/`former_label`.
    pub facet_type: String,
    pub value: String,
    pub confidence: f64,
    pub source_crystal_id: Option<i64>,
    pub language_tags: Vec<String>,
    pub story_scopes: Vec<String>,
    pub semantic_tags: Vec<String>,
    pub is_canonical: bool,
}

impl ConceptFacetRecord {
    /// Compatibility kinds (`alias`, `former_label`) present as name facets.
    pub fn kind(&self) -> &str {
        if self.facet_type == "alias" || self.facet_type == "former_label" {
            return "name";
        }
        &self.facet_type
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConceptLinkRecord {
    pub crystal_id: i64,
    pub concept_id: i64,
    pub link_type: String,
    pub confidence: f64,
}
