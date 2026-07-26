use std::sync::Mutex;

use async_trait::async_trait;
use hiero_core::{
    domain::{ShortTermMemory, TranslationContext},
    dreaming::{
        Crystallizer, DreamPhase, PhaseOutput, WorkflowProfile, execute_provider_passes,
        parse_dream_output, parse_dream_output_async, strip_code_fences,
    },
    provider::{DreamOutput, DreamProvider, PassName, ProviderError, Result as ProviderResult},
};
use serde_json::{Value, json};
use sqlx::sqlite::SqlitePoolOptions;

const VALID_OUTPUT: &str = r#"{
  "crystals": [{
    "crystal_type": "observation",
    "title": "Evidence",
    "text": "A supported conclusion.",
    "source_credibility": "expert",
    "rule_intent": "",
    "confidence": 0.8
  }],
  "concepts": []
}"#;

fn workflow() -> Vec<WorkflowProfile> {
    PassName::ALL
        .into_iter()
        .map(|phase| WorkflowProfile {
            phase,
            provider: "fake".into(),
            model: "fake".into(),
            max_records_per_pass: 10,
        })
        .collect()
}

#[test]
fn strip_code_fences_returns_the_borrowed_json_body() {
    assert_eq!(
        strip_code_fences("```json\n{\"crystals\":[],\"concepts\":[]}\n```"),
        "{\"crystals\":[],\"concepts\":[]}"
    );
}

#[test]
fn parse_dream_output_accepts_direct_json_without_penalty() {
    let output = parse_dream_output(VALID_OUTPUT).unwrap();

    assert!(!output.recovered);
    assert_eq!(output.crystals[0].confidence, 0.8);
    assert_eq!(output.crystals[0].malformed_penalty, 0.0);
}

#[test]
fn parse_dream_output_recovers_one_fenced_block_and_penalizes_candidates() {
    let output =
        parse_dream_output(&format!("model preface\n```json\n{VALID_OUTPUT}\n```\n")).unwrap();

    assert!(output.recovered);
    assert!((output.crystals[0].confidence - 0.6).abs() < f64::EPSILON);
    assert_eq!(output.crystals[0].malformed_penalty, 0.2);
}

#[test]
fn parse_dream_output_recovers_one_balanced_object_from_surrounding_prose() {
    let output = parse_dream_output(&format!(
        "Here is the result: {VALID_OUTPUT} End of result."
    ))
    .unwrap();

    assert!(output.recovered);
    assert!((output.crystals[0].confidence - 0.6).abs() < f64::EPSILON);
}

#[test]
fn parse_dream_output_rejects_multiple_fenced_candidates_as_ambiguous() {
    let raw = format!("```json\n{VALID_OUTPUT}\n```\n```json\n{VALID_OUTPUT}\n```");

    assert_eq!(
        parse_dream_output(&raw).unwrap_err().to_string(),
        "dream payload recovery is ambiguous"
    );
}

#[test]
fn parse_dream_output_rejects_multiple_balanced_candidates_as_ambiguous() {
    let raw = format!("first {VALID_OUTPUT} second {VALID_OUTPUT}");

    assert_eq!(
        parse_dream_output(&raw).unwrap_err().to_string(),
        "dream payload recovery is ambiguous"
    );
}

#[test]
fn parse_dream_output_rejects_empty_truncated_invalid_and_schema_mismatched_input() {
    for raw in [
        "",
        r#"{"crystals":["#,
        "not json",
        r#"{"crystals":[],"concepts":[],"secret":"must not be echoed"}"#,
    ] {
        let error = parse_dream_output(raw).unwrap_err().to_string();
        assert!(error.starts_with("dream payload"));
        assert!(!error.contains("secret"));
    }
}

#[test]
fn parse_dream_output_rejects_semantically_invalid_candidates() {
    let invalid = VALID_OUTPUT.replace(r#""confidence": 0.8"#, r#""confidence": 1.2"#);

    assert_eq!(
        parse_dream_output(&invalid).unwrap_err().to_string(),
        "dream payload schema is invalid"
    );
}

#[tokio::test]
async fn parse_dream_output_async_handles_large_payload_off_the_async_worker() {
    let padding = " ".repeat(70 * 1024);
    let output = parse_dream_output_async(format!("{VALID_OUTPUT}{padding}"))
        .await
        .unwrap();

    assert!(!output.recovered);
}

#[tokio::test]
async fn crystallizer_runs_the_async_provider_and_returns_typed_output() {
    struct CrystallizeProvider;

    #[async_trait]
    impl DreamProvider for CrystallizeProvider {
        fn name(&self) -> &str {
            "crystallize-fake"
        }

        async fn crystallize(
            &self,
            _context: &TranslationContext,
            _memories: &[ShortTermMemory],
        ) -> ProviderResult<DreamOutput> {
            Ok(parse_dream_output(VALID_OUTPUT).unwrap())
        }

        async fn run_pass(
            &self,
            _pass: PassName,
            _context: &TranslationContext,
            _memories: &[ShortTermMemory],
        ) -> ProviderResult<Value> {
            Err(ProviderError::MissingOutput)
        }
    }

    let pool = SqlitePoolOptions::new()
        .connect_lazy("sqlite::memory:")
        .unwrap();
    let output = Crystallizer::new(&CrystallizeProvider)
        .run(
            &pool,
            (TranslationContext::new("book", "en", "ru"), Vec::new()),
        )
        .await
        .unwrap();

    assert_eq!(output.crystals[0].text, "A supported conclusion.");
}

#[derive(Default)]
struct FakeProvider {
    calls: Mutex<Vec<PassName>>,
}

#[async_trait]
impl DreamProvider for FakeProvider {
    fn name(&self) -> &str {
        "fake"
    }

    async fn crystallize(
        &self,
        _context: &TranslationContext,
        _memories: &[ShortTermMemory],
    ) -> ProviderResult<DreamOutput> {
        Err(ProviderError::MissingOutput)
    }

    async fn run_pass(
        &self,
        pass: PassName,
        _context: &TranslationContext,
        _memories: &[ShortTermMemory],
    ) -> ProviderResult<Value> {
        self.calls.lock().unwrap().push(pass);
        Ok(match pass {
            PassName::Concepts => json!({
                "concepts": [{
                    "canonical_name": "Sense",
                    "facets": [["rendering", "Сенс", "ru"]],
                    "source_memory_ids": [1]
                }]
            }),
            PassName::TerminologyCandidates => json!({
                "concept_proposals": [
                    {
                        "concept_text": "Sense",
                        "source_form": "Sense",
                        "canonical_rendering": "Сенс",
                        "source_memory_ids": [1]
                    },
                    {
                        "concept_text": "sense",
                        "source_form": "Sense",
                        "canonical_rendering": "Сенс",
                        "source_memory_ids": [1]
                    }
                ]
            }),
            PassName::RuleCrystals => json!({
                "crystals": [{
                    "crystal_type": "rule",
                    "title": "Rendering",
                    "text": "Render Sense as Сенс.",
                    "source_credibility": "user_rule",
                    "rule_intent": "terminology",
                    "confidence": 0.9,
                    "source_memory_ids": [1]
                }]
            }),
            PassName::KnowledgeCrystals => json!({
                "crystals": [{
                    "crystal_type": "observation",
                    "title": "Evidence",
                    "text": "Sense is a named concept.",
                    "source_credibility": "expert",
                    "rule_intent": "",
                    "confidence": 0.8,
                    "source_memory_ids": [1]
                }]
            }),
            PassName::Relations => json!({
                "relations": [{
                    "source_id": 2,
                    "target_id": 3,
                    "relation": "related",
                    "source_memory_ids": [1]
                }]
            }),
            PassName::Reinforcement => json!({
                "reinforce": [{
                    "crystal_id": 4,
                    "strength_delta": 0.1,
                    "confidence_delta": 0.05,
                    "source_memory_ids": [1]
                }]
            }),
            PassName::CoverageAudit => json!({"covered_memory_ids": [1]}),
        })
    }
}

#[tokio::test]
async fn execute_provider_passes_runs_resolved_workflow_sequentially_with_typed_outputs() {
    let pool = SqlitePoolOptions::new()
        .connect_lazy("sqlite::memory:")
        .unwrap();
    let provider = FakeProvider::default();
    let context = TranslationContext::new("book", "en", "ru");

    let outputs = execute_provider_passes(&pool, &provider, &workflow(), context, Vec::new())
        .await
        .unwrap();

    assert_eq!(*provider.calls.lock().unwrap(), PassName::ALL);
    assert!(matches!(&outputs[0], PhaseOutput::Concepts(value) if value.concepts.len() == 1));
    assert!(
        matches!(&outputs[1], PhaseOutput::TerminologyCandidates(value) if value.concept_proposals.len() == 1)
    );
    assert!(matches!(&outputs[2], PhaseOutput::RuleCrystals(value) if value.crystals.len() == 1));
    assert!(
        matches!(&outputs[3], PhaseOutput::KnowledgeCrystals(value) if value.crystals.len() == 1)
    );
    assert!(matches!(&outputs[4], PhaseOutput::Relations(value) if value.relations.len() == 1));
    assert!(matches!(&outputs[5], PhaseOutput::Reinforcement(value) if value.reinforce.len() == 1));
    assert!(
        matches!(&outputs[6], PhaseOutput::CoverageAudit(value) if value.covered_memory_ids == vec![1])
    );
}

#[tokio::test]
async fn execute_provider_passes_rejects_schema_mismatch_without_running_later_phases() {
    struct BadProvider(Mutex<Vec<PassName>>);

    #[async_trait]
    impl DreamProvider for BadProvider {
        fn name(&self) -> &str {
            "bad"
        }

        async fn crystallize(
            &self,
            _context: &TranslationContext,
            _memories: &[ShortTermMemory],
        ) -> ProviderResult<DreamOutput> {
            Err(ProviderError::MissingOutput)
        }

        async fn run_pass(
            &self,
            pass: PassName,
            _context: &TranslationContext,
            _memories: &[ShortTermMemory],
        ) -> ProviderResult<Value> {
            self.0.lock().unwrap().push(pass);
            Ok(json!({"unexpected": "api-key-must-not-leak"}))
        }
    }

    let pool = SqlitePoolOptions::new()
        .connect_lazy("sqlite::memory:")
        .unwrap();
    let provider = BadProvider(Mutex::new(Vec::new()));
    let error = execute_provider_passes(
        &pool,
        &provider,
        &workflow(),
        TranslationContext::new("book", "en", "ru"),
        Vec::new(),
    )
    .await
    .unwrap_err();

    assert_eq!(*provider.0.lock().unwrap(), vec![PassName::Concepts]);
    assert!(!error.to_string().contains("api-key"));
}
