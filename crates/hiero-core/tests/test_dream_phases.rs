use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use chrono::Utc;
use hiero_core::{
    db::ShortTermMemoryRecord,
    domain::{ShortTermMemory, TranslationContext},
    dreaming::{
        Crystallizer, DreamPhase, DreamPhaseError, DreamProviderResolver, PhaseOutput,
        WorkflowProfile, execute_provider_passes, parse_dream_output, parse_dream_output_async,
        parse_provider_output, strip_code_fences,
    },
    provider::{
        DreamOutput, DreamProvider, PassName, ProviderError, ProviderPassOutput,
        Result as ProviderResult,
    },
};
use serde_json::json;
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

fn profile(phase: PassName, provider: &str, model: &str, limit: usize) -> WorkflowProfile {
    WorkflowProfile {
        phase,
        provider: provider.into(),
        model: model.into(),
        max_records_per_pass: limit,
    }
}

fn memory(id: i64) -> ShortTermMemory {
    ShortTermMemory {
        record: ShortTermMemoryRecord {
            id,
            session_id: 1,
            source_role: "user".into(),
            kind: "reading".into(),
            text: "Evidence.".into(),
            source_ref: String::new(),
            metadata_json: "{}".into(),
            source_credibility: Some("observation".into()),
            rule_intent: Some(String::new()),
            soft_origin: None,
            source_crystal_id: None,
            created_at: Utc::now(),
            archived_at: None,
        },
        metadata: Default::default(),
        language_tags: vec!["en".into(), "ru".into()],
        story_scopes: Vec::new(),
        semantic_tags: Vec::new(),
    }
}

struct OutputProvider {
    output: ProviderPassOutput,
    calls: Arc<Mutex<Vec<PassName>>>,
}

#[async_trait]
impl DreamProvider for OutputProvider {
    fn name(&self) -> &str {
        "output"
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
    ) -> ProviderResult<ProviderPassOutput> {
        self.calls.lock().unwrap().push(pass);
        Ok(self.output.clone())
    }
}

#[derive(Default)]
struct RecordingResolver {
    outputs: BTreeMap<(String, String), ProviderPassOutput>,
    resolutions: Mutex<Vec<(PassName, String, String)>>,
    calls: Arc<Mutex<Vec<PassName>>>,
}

impl DreamProviderResolver for RecordingResolver {
    fn resolve(
        &self,
        workflow: &WorkflowProfile,
    ) -> Result<Arc<dyn DreamProvider>, DreamPhaseError> {
        self.resolutions.lock().unwrap().push((
            workflow.phase,
            workflow.provider.clone(),
            workflow.model.clone(),
        ));
        let output = self
            .outputs
            .get(&(workflow.provider.clone(), workflow.model.clone()))
            .cloned()
            .ok_or(DreamPhaseError::ProviderResolution)?;
        Ok(Arc::new(OutputProvider {
            output,
            calls: self.calls.clone(),
        }))
    }
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
fn parse_provider_output_recovers_every_provider_pass_shape() {
    let fixtures = [
        (PassName::Concepts, json!({"concepts": []})),
        (
            PassName::TerminologyCandidates,
            json!({"concept_proposals": []}),
        ),
        (PassName::RuleCrystals, json!({"crystals": []})),
        (PassName::KnowledgeCrystals, json!({"crystals": []})),
        (PassName::Relations, json!({"relations": []})),
        (PassName::Reinforcement, json!({"reinforce": []})),
        (PassName::CoverageAudit, json!({"covered_memory_ids": [1]})),
    ];

    for (pass, value) in fixtures {
        let json = serde_json::to_string(&value).unwrap();
        for raw in [
            format!("```json\n{json}\n```"),
            format!("provider preface {json} provider suffix"),
        ] {
            let parsed = parse_provider_output(&raw).unwrap();
            assert_eq!(parsed.value, value, "failed recovery for {pass:?}");
            assert!(parsed.recovered, "missing recovery marker for {pass:?}");
            assert_eq!(parsed.malformed_penalty, 0.2);
        }
    }
}

#[test]
fn parse_provider_output_rejects_malformed_plus_valid_balanced_candidates_as_ambiguous() {
    let raw = r#"first {"wrong": true} second {"concepts": []}"#;

    assert_eq!(
        parse_provider_output(raw).unwrap_err().to_string(),
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
        ) -> ProviderResult<ProviderPassOutput> {
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
    ) -> ProviderResult<ProviderPassOutput> {
        self.calls.lock().unwrap().push(pass);
        Ok(ProviderPassOutput::direct(match pass {
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
        }))
    }
}

#[tokio::test]
async fn execute_provider_passes_runs_resolved_workflow_sequentially_with_typed_outputs() {
    let pool = SqlitePoolOptions::new()
        .connect_lazy("sqlite::memory:")
        .unwrap();
    let provider = Arc::new(FakeProvider::default());
    let selected = provider.clone();
    let resolver = move |_: &WorkflowProfile| -> Result<Arc<dyn DreamProvider>, DreamPhaseError> {
        Ok(selected.clone())
    };
    let context = TranslationContext::new("book", "en", "ru");

    let outputs = execute_provider_passes(&pool, &resolver, &workflow(), context, vec![memory(1)])
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
        ) -> ProviderResult<ProviderPassOutput> {
            self.0.lock().unwrap().push(pass);
            Ok(ProviderPassOutput::direct(
                json!({"unexpected": "api-key-must-not-leak"}),
            ))
        }
    }

    let pool = SqlitePoolOptions::new()
        .connect_lazy("sqlite::memory:")
        .unwrap();
    let provider = Arc::new(BadProvider(Mutex::new(Vec::new())));
    let selected = provider.clone();
    let resolver = move |_: &WorkflowProfile| -> Result<Arc<dyn DreamProvider>, DreamPhaseError> {
        Ok(selected.clone())
    };
    let error = execute_provider_passes(
        &pool,
        &resolver,
        &workflow(),
        TranslationContext::new("book", "en", "ru"),
        vec![memory(1)],
    )
    .await
    .unwrap_err();

    assert_eq!(*provider.0.lock().unwrap(), vec![PassName::Concepts]);
    assert!(!error.to_string().contains("api-key"));
}

#[tokio::test]
async fn execute_provider_passes_resolves_each_profiles_provider_and_model_in_order() {
    let pool = SqlitePoolOptions::new()
        .connect_lazy("sqlite::memory:")
        .unwrap();
    let mut resolver = RecordingResolver::default();
    resolver.outputs.insert(
        ("concept-provider".into(), "concept-model".into()),
        ProviderPassOutput::direct(json!({"concepts": []})),
    );
    resolver.outputs.insert(
        ("audit-provider".into(), "audit-model".into()),
        ProviderPassOutput::direct(json!({"covered_memory_ids": [1]})),
    );
    let workflow = vec![
        profile(PassName::Concepts, "concept-provider", "concept-model", 10),
        profile(PassName::CoverageAudit, "audit-provider", "audit-model", 10),
    ];

    let outputs = execute_provider_passes(
        &pool,
        &resolver,
        &workflow,
        TranslationContext::new("book", "en", "ru"),
        vec![memory(1)],
    )
    .await
    .unwrap();

    assert_eq!(
        *resolver.resolutions.lock().unwrap(),
        vec![
            (
                PassName::Concepts,
                "concept-provider".into(),
                "concept-model".into()
            ),
            (
                PassName::CoverageAudit,
                "audit-provider".into(),
                "audit-model".into()
            )
        ]
    );
    assert_eq!(
        *resolver.calls.lock().unwrap(),
        vec![PassName::Concepts, PassName::CoverageAudit]
    );
    assert!(matches!(outputs[0], PhaseOutput::Concepts(_)));
    assert!(matches!(outputs[1], PhaseOutput::CoverageAudit(_)));
}

#[tokio::test]
async fn execute_provider_passes_rejects_semantically_invalid_output_for_each_phase() {
    let invalid = [
        (
            PassName::Concepts,
            json!({"concepts": [{
                "canonical_name": " ",
                "facets": [["name", "Sense", "en"]],
                "source_memory_ids": [1]
            }]}),
        ),
        (
            PassName::TerminologyCandidates,
            json!({"concept_proposals": [{
                "concept_text": "",
                "source_form": "Sense",
                "canonical_rendering": "Сенс",
                "source_memory_ids": [1]
            }]}),
        ),
        (
            PassName::RuleCrystals,
            json!({"crystals": [{
                "crystal_type": "rule",
                "title": "Rule",
                "text": "",
                "source_credibility": "user_rule",
                "rule_intent": "terminology",
                "confidence": 0.9,
                "source_memory_ids": [1]
            }]}),
        ),
        (
            PassName::KnowledgeCrystals,
            json!({"crystals": [{
                "crystal_type": "observation",
                "title": "",
                "text": "Knowledge.",
                "source_credibility": "expert",
                "rule_intent": "",
                "confidence": 1.2,
                "source_memory_ids": [1]
            }]}),
        ),
        (
            PassName::Relations,
            json!({"relations": [{
                "source_id": 0,
                "target_id": 2,
                "relation": "",
                "source_memory_ids": [1]
            }]}),
        ),
        (
            PassName::Reinforcement,
            json!({"reinforce": [{
                "crystal_id": 0,
                "strength_delta": 1.2,
                "confidence_delta": 0.1,
                "source_memory_ids": []
            }]}),
        ),
        (PassName::CoverageAudit, json!({"covered_memory_ids": [0]})),
    ];
    let pool = SqlitePoolOptions::new()
        .connect_lazy("sqlite::memory:")
        .unwrap();

    for (pass, output) in invalid {
        let mut resolver = RecordingResolver::default();
        resolver.outputs.insert(
            ("invalid".into(), "invalid".into()),
            ProviderPassOutput::direct(output),
        );
        let error = execute_provider_passes(
            &pool,
            &resolver,
            &[profile(pass, "invalid", "invalid", 10)],
            TranslationContext::new("book", "en", "ru"),
            vec![memory(1)],
        )
        .await
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "dream phase output schema is invalid",
            "accepted invalid {pass:?} output"
        );
        assert_eq!(*resolver.calls.lock().unwrap(), vec![pass]);
    }
}

#[tokio::test]
async fn execute_provider_passes_rejects_source_ids_outside_selected_memory_set() {
    let pool = SqlitePoolOptions::new()
        .connect_lazy("sqlite::memory:")
        .unwrap();
    let mut resolver = RecordingResolver::default();
    resolver.outputs.insert(
        ("audit".into(), "model".into()),
        ProviderPassOutput::direct(json!({"covered_memory_ids": [2]})),
    );

    let error = execute_provider_passes(
        &pool,
        &resolver,
        &[profile(PassName::CoverageAudit, "audit", "model", 10)],
        TranslationContext::new("book", "en", "ru"),
        vec![memory(1)],
    )
    .await
    .unwrap_err();

    assert_eq!(error.to_string(), "dream phase output schema is invalid");
}

#[tokio::test]
async fn recovered_crystal_phase_marks_output_and_applies_malformed_penalty() {
    let raw = r#"```json
{"crystals":[{
  "crystal_type":"observation",
  "title":"Evidence",
  "text":"Recovered knowledge.",
  "source_credibility":"expert",
  "rule_intent":"",
  "confidence":0.8,
  "source_memory_ids":[1]
}]}
```"#;
    let pool = SqlitePoolOptions::new()
        .connect_lazy("sqlite::memory:")
        .unwrap();
    let mut resolver = RecordingResolver::default();
    resolver.outputs.insert(
        ("knowledge".into(), "model".into()),
        parse_provider_output(raw).unwrap(),
    );

    let outputs = execute_provider_passes(
        &pool,
        &resolver,
        &[profile(
            PassName::KnowledgeCrystals,
            "knowledge",
            "model",
            10,
        )],
        TranslationContext::new("book", "en", "ru"),
        vec![memory(1)],
    )
    .await
    .unwrap();

    let PhaseOutput::KnowledgeCrystals(output) = &outputs[0] else {
        panic!("knowledge phase output expected");
    };
    assert!(output.recovery.recovered);
    assert_eq!(output.recovery.malformed_penalty, 0.2);
    assert!((output.crystals[0].crystal.confidence - 0.6).abs() < f64::EPSILON);
    assert_eq!(output.crystals[0].crystal.malformed_penalty, 0.2);
}

#[tokio::test]
async fn execute_provider_passes_rejects_invalid_recovery_metadata() {
    let crystal = json!({"crystals":[{
        "crystal_type": "observation",
        "title": "Evidence",
        "text": "Knowledge.",
        "source_credibility": "expert",
        "rule_intent": "",
        "confidence": 0.8,
        "source_memory_ids": [1]
    }]});
    let invalid_metadata = [
        (true, f64::NAN),
        (true, f64::INFINITY),
        (true, -0.2),
        (false, 0.2),
        (true, 0.1),
    ];
    let pool = SqlitePoolOptions::new()
        .connect_lazy("sqlite::memory:")
        .unwrap();

    for (recovered, malformed_penalty) in invalid_metadata {
        let mut resolver = RecordingResolver::default();
        resolver.outputs.insert(
            ("knowledge".into(), "model".into()),
            ProviderPassOutput {
                value: crystal.clone(),
                recovered,
                malformed_penalty,
            },
        );

        let error = execute_provider_passes(
            &pool,
            &resolver,
            &[profile(
                PassName::KnowledgeCrystals,
                "knowledge",
                "model",
                10,
            )],
            TranslationContext::new("book", "en", "ru"),
            vec![memory(1)],
        )
        .await
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "dream phase output schema is invalid",
            "accepted recovered={recovered} malformed_penalty={malformed_penalty:?}"
        );
    }
}

#[tokio::test]
async fn terminology_limit_uses_raw_count_before_dedup_and_short_circuits() {
    let proposal = json!({
        "concept_text": "Sense",
        "source_form": "Sense",
        "canonical_rendering": "Сенс",
        "source_memory_ids": [1]
    });
    let pool = SqlitePoolOptions::new()
        .connect_lazy("sqlite::memory:")
        .unwrap();

    for (count, should_pass) in [(2, true), (3, false)] {
        let mut resolver = RecordingResolver::default();
        resolver.outputs.insert(
            ("terms".into(), "model".into()),
            ProviderPassOutput::direct(json!({
                "concept_proposals": vec![proposal.clone(); count]
            })),
        );
        resolver.outputs.insert(
            ("audit".into(), "model".into()),
            ProviderPassOutput::direct(json!({"covered_memory_ids": [1]})),
        );
        let result = execute_provider_passes(
            &pool,
            &resolver,
            &[
                profile(PassName::TerminologyCandidates, "terms", "model", 2),
                profile(PassName::CoverageAudit, "audit", "model", 10),
            ],
            TranslationContext::new("book", "en", "ru"),
            vec![memory(1)],
        )
        .await;

        if should_pass {
            let outputs = result.unwrap();
            assert!(
                matches!(&outputs[0], PhaseOutput::TerminologyCandidates(value) if value.concept_proposals.len() == 1)
            );
            assert_eq!(
                *resolver.calls.lock().unwrap(),
                vec![PassName::TerminologyCandidates, PassName::CoverageAudit]
            );
        } else {
            assert_eq!(
                result.unwrap_err().to_string(),
                "dream phase output exceeds max_records_per_pass"
            );
            assert_eq!(
                *resolver.calls.lock().unwrap(),
                vec![PassName::TerminologyCandidates]
            );
        }
    }
}
