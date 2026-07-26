use async_trait::async_trait;

use crate::{
    domain::{ShortTermMemory, TranslationContext},
    values::SOURCE_CREDIBILITY_CONFIDENCE,
};

use super::{
    CandidateCrystal, ConceptCandidate, DreamOutput, DreamProvider, PassName, ProviderPassOutput,
    Result,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct DeterministicProvider;

#[async_trait]
impl DreamProvider for DeterministicProvider {
    fn name(&self) -> &str {
        "deterministic"
    }

    async fn crystallize(
        &self,
        _context: &TranslationContext,
        memories: &[ShortTermMemory],
    ) -> Result<DreamOutput> {
        Ok(DreamOutput {
            crystals: rule_candidates(memories),
            concepts: Vec::<ConceptCandidate>::new(),
            recovered: false,
        })
    }

    async fn run_pass(
        &self,
        pass: PassName,
        _context: &TranslationContext,
        memories: &[ShortTermMemory],
    ) -> Result<ProviderPassOutput> {
        let value = match pass {
            PassName::RuleCrystals => serde_json::json!({
                "crystals": rule_candidates(memories)
                    .into_iter()
                    .zip(memories.iter().filter(|memory| is_rule(memory)))
                    .map(|(candidate, memory)| serde_json::json!({
                        "crystal_type": candidate.crystal_type,
                        "title": candidate.title,
                        "text": candidate.text,
                        "source_credibility": candidate.source_credibility,
                        "rule_intent": candidate.rule_intent,
                        "confidence": candidate.confidence,
                        "source_memory_ids": [memory.id],
                    }))
                    .collect::<Vec<_>>()
            }),
            PassName::CoverageAudit => serde_json::json!({
                "covered_memory_ids": memories.iter().map(|memory| memory.id).collect::<Vec<_>>()
            }),
            _ => serde_json::json!({}),
        };
        Ok(ProviderPassOutput::direct(value))
    }
}

fn is_rule(memory: &ShortTermMemory) -> bool {
    memory.source_credibility.as_deref() == Some("user_rule")
        || memory
            .rule_intent
            .as_deref()
            .is_some_and(|intent| !intent.trim().is_empty())
}

fn rule_candidates(memories: &[ShortTermMemory]) -> Vec<CandidateCrystal> {
    memories
        .iter()
        .filter(|memory| is_rule(memory))
        .map(|memory| {
            let credibility = memory
                .source_credibility
                .as_deref()
                .unwrap_or("observation");
            CandidateCrystal {
                crystal_type: "rule".into(),
                title: title_from_kind(&memory.kind),
                text: memory.text.split_whitespace().collect::<Vec<_>>().join(" "),
                source_credibility: credibility.into(),
                rule_intent: memory.rule_intent.clone().unwrap_or_default(),
                confidence: SOURCE_CREDIBILITY_CONFIDENCE
                    .get(credibility)
                    .copied()
                    .unwrap_or(SOURCE_CREDIBILITY_CONFIDENCE["observation"]),
                malformed_penalty: 0.0,
            }
        })
        .collect()
}

fn title_from_kind(kind: &str) -> String {
    kind.split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(4)
        .map(|word| {
            let mut characters = word.chars();
            characters.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(characters).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}
