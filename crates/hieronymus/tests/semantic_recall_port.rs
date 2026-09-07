//! Task 9: the query-time semantic lane, reciprocal-rank fusion with the FTS
//! chunk lane, degraded-mode warnings, corrupt-hit repair scheduling, and the
//! `conflicts_with_rule_ids` contract markers on advisory recall hits.
//! Tests never egress: the fake embedding provider plus the pinned WordPiece
//! tokenizer (the committed fixture) stand in for the model, over real SQLite
//! and LanceDB.

use std::fs;
use std::path::PathBuf;

use hieronymus::crystals::{CrystalStore, NewCrystal};
use hieronymus::data_root::HieronymusConfig;
use hieronymus::memory_models::TranslationContext;
use hieronymus::rag::{RagImport, RagStore};
use hieronymus::recall::{
    RecallHit, RecallResponse, RecallService, RecallWarning, WARNING_REPAIR_SCHEDULED,
    WARNING_SEMANTIC_UNAVAILABLE,
};
use hieronymus::registry::Registry;
use hieronymus::semantic_embeddings::{
    EMBEDDING_DIMENSIONS, EmbeddingProvider, FakeEmbeddingProvider,
};
use hieronymus::semantic_recall::SemanticLane;
use hieronymus::semantic_store::{SemanticChunk, SemanticSample, SemanticStore};
use hieronymus::semantic_tokenizer::ModelTokenizer;
use hieronymus::terminology::{ProposeFields, Termbase};
use hieronymus::workspace::WorkspaceStore;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

struct Fixture {
    #[allow(dead_code)] // holds the temp directory alive for the config paths
    root: tempfile::TempDir,
    config: HieronymusConfig,
    session_id: i64,
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    let registry = Registry::open(&config).unwrap();
    registry
        .create_series("demo", "demo", "ja", "en", None)
        .unwrap();
    let workspace = WorkspaceStore::open(&config).unwrap();
    let context = TranslationContext::new("demo", "ja", "en", "translation");
    let session = workspace.start_session(&context).unwrap();
    Fixture {
        root,
        config,
        session_id: session.id,
    }
}

fn write_source(fixture: &Fixture, name: &str, content: &str) -> PathBuf {
    let path = fixture.root.path().join(name);
    fs::write(&path, content).unwrap();
    path
}

fn import_text(fixture: &Fixture, name: &str, content: &str) {
    let path = write_source(fixture, name, content);
    RagStore::open(&fixture.config)
        .unwrap()
        .import_file("demo", &path, &RagImport::new())
        .unwrap();
}

/// Builds and activates a whole-corpus generation over the fixture's chunks
/// using the fake provider and the pinned WordPiece tokenizer (the exact
/// tokenizer the armed recall lane uses for queries, so document and query
/// vectors share one mapping).
fn model_tokenizer() -> ModelTokenizer {
    ModelTokenizer::from_bytes(include_bytes!("fixtures/minilm-tokenizer.json")).unwrap()
}
fn activate_generation(fixture: &Fixture) {
    let store = SemanticStore::open(&fixture.config).unwrap();
    let tokenizer = model_tokenizer();
    let mut provider = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    store
        .begin_generation("gen-a", provider.identity())
        .unwrap();
    loop {
        let pending = store.pending_chunk_ids("gen-a", 8).unwrap();
        if pending.is_empty() {
            break;
        }
        let chunks: Vec<SemanticChunk> = pending
            .iter()
            .map(|chunk_id| {
                let (series_slug, text) = store
                    .chunk_row(*chunk_id)
                    .unwrap()
                    .expect("chunk exists in the authoritative store");
                SemanticChunk {
                    chunk_id: *chunk_id,
                    series_slug,
                    token_ids: tokenizer.encode(&text).unwrap(),
                }
            })
            .collect();
        store.write_batch("gen-a", &mut provider, &chunks).unwrap();
    }
    store
        .activate_generation(
            "gen-a",
            &mut provider,
            &SemanticSample {
                series_slug: "demo".to_string(),
                token_ids: tokenizer.encode("probe").unwrap(),
            },
        )
        .unwrap();
}

fn armed_service(fixture: &Fixture) -> RecallService {
    RecallService::open(&fixture.config)
        .unwrap()
        .with_semantic_lane(SemanticLane::new(
            Box::new(FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS)),
            Box::new(model_tokenizer()),
        ))
}

fn rag_hit_ids(response: &RecallResponse) -> Vec<i64> {
    response
        .hits
        .iter()
        .filter_map(|hit| match hit {
            RecallHit::Rag {
                chunk,
                conflicts_with_rule_ids: _,
                ..
            } => Some(chunk.id),
            _ => None,
        })
        .collect()
}

fn warnings_of(response: &RecallResponse) -> &[RecallWarning] {
    &response.warnings
}

fn context() -> TranslationContext {
    TranslationContext::new("demo", "ja", "en", "translation")
}

// ---------------------------------------------------------------------------
// Degraded mode
// ---------------------------------------------------------------------------

#[test]
fn degraded_lane_returns_fts_results_with_a_structured_warning() {
    let fixture = fixture();
    import_text(&fixture, "a.txt", "Cooking Talent appears here.");

    let service = armed_service(&fixture);
    let degraded = service
        .recall(fixture.session_id, &context(), "Cooking Talent", 10)
        .unwrap();

    // The warning is structured and named: kind + reason.
    let unavailable: Vec<&RecallWarning> = warnings_of(&degraded)
        .iter()
        .filter(|warning| warning.kind == WARNING_SEMANTIC_UNAVAILABLE)
        .collect();
    assert_eq!(unavailable.len(), 1, "expected one degraded warning");
    assert!(
        !unavailable[0].reason.is_empty(),
        "degraded warnings carry a reason"
    );

    // The FTS results are exactly the unarmed service's results.
    let plain = RecallService::open(&fixture.config)
        .unwrap()
        .recall(fixture.session_id, &context(), "Cooking Talent", 10)
        .unwrap();
    assert_eq!(degraded.hits, plain.hits);
    assert!(
        plain
            .hits
            .iter()
            .any(|hit| matches!(hit, RecallHit::Rag { .. })),
        "the FTS lane must still answer the query"
    );

    // Degraded mode schedules nothing: no generations, no jobs.
    let connection = rusqlite::Connection::open_with_flags(
        fixture.config.database_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let generations: i64 = connection
        .query_row("select count(*) from semantic_generations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(generations, 0);
}

/// An unarmed lane is NOT a silent supported mode (task C5, review finding
/// A5): the FTS lane still answers, but the response says outright that
/// required semantics never ran. Until C5 this case carried no warning at
/// all, so a cold or misconfigured semantic runtime was indistinguishable
/// from a complete hybrid answer.
#[test]
fn recall_without_a_semantic_lane_reports_the_missing_lane() {
    let fixture = fixture();
    import_text(&fixture, "a.txt", "Cooking Talent appears here.");
    let response = RecallService::open(&fixture.config)
        .unwrap()
        .recall(fixture.session_id, &context(), "Cooking Talent", 10)
        .unwrap();
    let unavailable: Vec<&RecallWarning> = warnings_of(&response)
        .iter()
        .filter(|warning| warning.kind == WARNING_SEMANTIC_UNAVAILABLE)
        .collect();
    assert_eq!(unavailable.len(), 1, "{:?}", warnings_of(&response));
    assert!(!unavailable[0].reason.is_empty());
    assert!(
        response
            .hits
            .iter()
            .any(|hit| matches!(hit, RecallHit::Rag { .. }))
    );
}

/// The service-level half of the same fact: the lane ran clean, but the owner
/// of the shared semantic service reports it cannot serve the current corpus
/// (an in-flight rebuild, an acquiring runtime, a failure). The answer is
/// still incomplete, so it is still reported — the lane's own success is not
/// evidence that every required lane ran.
#[test]
fn recall_reports_a_semantic_service_that_is_not_ready() {
    use hieronymus::recall::SemanticAvailability;

    let fixture = fixture();
    import_text(&fixture, "a.txt", "Cooking Talent appears here.");
    let response = RecallService::open(&fixture.config)
        .unwrap()
        .with_semantic_status(std::sync::Arc::new(|| {
            SemanticAvailability::Unavailable("semantic indexing is still in progress".to_string())
        }))
        .recall(fixture.session_id, &context(), "Cooking Talent", 10)
        .unwrap();
    let unavailable: Vec<&RecallWarning> = warnings_of(&response)
        .iter()
        .filter(|warning| warning.kind == WARNING_SEMANTIC_UNAVAILABLE)
        .collect();
    assert_eq!(unavailable.len(), 1, "{:?}", warnings_of(&response));
    assert_eq!(
        unavailable[0].reason, "semantic indexing is still in progress",
        "the service's own reason rides through verbatim"
    );
}

#[test]
fn recall_degrades_when_the_lane_identity_differs_from_the_generation() {
    let fixture = fixture();
    import_text(&fixture, "a.txt", "Cooking Talent appears here.");
    activate_generation(&fixture);

    // The generation was built under the default fake model identity; a lane
    // armed with another identity can never mix them and must degrade.
    let mismatched = RecallService::open(&fixture.config)
        .unwrap()
        .with_semantic_lane(SemanticLane::new(
            Box::new(FakeEmbeddingProvider::with_model(
                EMBEDDING_DIMENSIONS,
                "other-model",
            )),
            Box::new(model_tokenizer()),
        ));
    let response = mismatched
        .recall(fixture.session_id, &context(), "Cooking Talent", 10)
        .unwrap();
    let unavailable = warnings_of(&response)
        .iter()
        .find(|warning| warning.kind == WARNING_SEMANTIC_UNAVAILABLE)
        .expect("identity mismatch must degrade the lane");
    assert!(
        unavailable.reason.contains("identity"),
        "the reason must name the identity mismatch: {}",
        unavailable.reason
    );
    assert!(
        response
            .hits
            .iter()
            .any(|hit| matches!(hit, RecallHit::Rag { .. })),
        "FTS results remain available in degraded mode"
    );
}

// ---------------------------------------------------------------------------
// Fusion and eligibility
// ---------------------------------------------------------------------------

#[test]
fn rrf_fusion_promotes_the_semantic_winner_over_the_boosted_fts_hit() {
    let fixture = fixture();
    // Chunk X (id 1): exactly the query text — the semantic lane's rank-1 hit
    // (identical token stream, distance 0). Imported first so it also wins the
    // chunk-id tie-break.
    import_text(&fixture, "x.txt", "wintersong ritual");
    // Chunk Y (id 2): a glossary entry carrying both query words; the glossary
    // boost makes it the FTS lane winner.
    let glossary = write_source(
        &fixture,
        "y.json",
        r#"[{"source": "wintersong ritual", "target": "wintry song"}]"#,
    );
    let glossary_import = RagImport::new().language_tags(["ja"]);
    RagStore::open(&fixture.config)
        .unwrap()
        .import_file("demo", &glossary, &glossary_import)
        .unwrap();
    activate_generation(&fixture);

    let plain = RecallService::open(&fixture.config)
        .unwrap()
        .recall(fixture.session_id, &context(), "wintersong ritual", 10)
        .unwrap();
    let fused = armed_service(&fixture)
        .recall(fixture.session_id, &context(), "wintersong ritual", 10)
        .unwrap();

    let plain_ids = rag_hit_ids(&plain);
    let fused_ids = rag_hit_ids(&fused);
    assert_eq!(
        plain_ids,
        vec![2, 1],
        "FTS-only ranking must put the boosted glossary hit first: {plain_ids:?}"
    );
    assert!(
        warnings_of(&fused).is_empty(),
        "a healthy lane must not warn: {:?}",
        warnings_of(&fused)
    );
    assert_eq!(
        fused_ids.first(),
        Some(&1),
        "RRF fusion must lift the semantic rank-1 chunk over the boosted FTS hit: {fused_ids:?}"
    );
    assert!(fused_ids.contains(&2), "the FTS winner stays in the pool");
}

#[test]
fn lane_eligibility_is_series_parity_and_cross_series_is_impossible() {
    let fixture = fixture();
    Registry::open(&fixture.config)
        .unwrap()
        .create_series("other", "other", "ja", "en", None)
        .unwrap();
    // The globally closest chunk lives in an ineligible series: its token
    // stream equals the query stream, so it is the semantic lane's rank-1 hit
    // worldwide — and must still never surface.
    let other_path = write_source(&fixture, "other.txt", "The Cooking Talent appears here.");
    RagStore::open(&fixture.config)
        .unwrap()
        .import_file("other", &other_path, &RagImport::new())
        .unwrap();
    import_text(&fixture, "demo.txt", "Demo series note about weather.");
    activate_generation(&fixture);

    let service = armed_service(&fixture);
    let response = service
        .recall(
            fixture.session_id,
            &context(),
            "The Cooking Talent appears here.",
            10,
        )
        .unwrap();
    assert!(
        warnings_of(&response).is_empty(),
        "a healthy lane must not warn: {:?}",
        warnings_of(&response)
    );
    for hit in &response.hits {
        if let RecallHit::Rag { chunk, .. } = hit {
            assert_eq!(
                chunk.series_slug, "demo",
                "cross-series chunk {} leaked into the recall response",
                chunk.id
            );
        }
    }
    assert!(
        !response.hits.iter().any(|hit| matches!(hit,
            RecallHit::Rag { chunk, .. } if chunk.text.contains("Cooking Talent"))),
        "the ineligible series must be absent from both lanes"
    );

    // Parity: a demo-series chunk is eligible through the fused lane too —
    // the exact-text query ranks it first in both lanes. (The demo chunk has
    // id 2: the ineligible series' import took id 1.)
    let parity = service
        .recall(
            fixture.session_id,
            &context(),
            "Demo series note about weather.",
            10,
        )
        .unwrap();
    let parity_ids = rag_hit_ids(&parity);
    assert_eq!(parity_ids.first(), Some(&2), "eligible in both lanes");
    assert!(parity.hits.iter().any(|hit| matches!(hit,
            RecallHit::Rag { chunk, conflicts_with_rule_ids, .. }
            if chunk.text.contains("Demo series note") && conflicts_with_rule_ids.is_empty())));
}

// ---------------------------------------------------------------------------
// Corrupt hits and repair scheduling
// ---------------------------------------------------------------------------

#[test]
fn corrupt_semantic_hits_are_excluded_and_repair_is_scheduled() {
    let fixture = fixture();
    import_text(&fixture, "a.txt", "Alpha paragraph one.");
    activate_generation(&fixture);

    // Corrupt the authoritative row underneath the active generation: the
    // stored checksum no longer matches the chunk text. Both FTS-indexed
    // columns change, so the FTS lane loses its match as well.
    let connection = rusqlite::Connection::open(fixture.config.database_path()).unwrap();
    connection
        .execute(
            "update rag_chunks
             set text = 'Rewritten paragraph one.',
                 display_text = 'Rewritten paragraph one.'
             where id = 1",
            [],
        )
        .unwrap();
    drop(connection);

    let response = armed_service(&fixture)
        .recall(fixture.session_id, &context(), "Alpha", 10)
        .unwrap();

    // The stale hit is excluded: neither lane returns the corrupted chunk.
    assert!(
        !response
            .hits
            .iter()
            .any(|hit| matches!(hit, RecallHit::Rag { .. })),
        "corrupt semantic hits must be excluded: {:?}",
        response.hits
    );
    // The lane itself ran fine, so there is no degraded warning — only the
    // repair notice.
    assert!(
        warnings_of(&response)
            .iter()
            .all(|warning| warning.kind != WARNING_SEMANTIC_UNAVAILABLE)
    );
    assert!(
        warnings_of(&response)
            .iter()
            .any(|warning| warning.kind == WARNING_REPAIR_SCHEDULED),
        "corrupt hits must schedule repair: {:?}",
        warnings_of(&response)
    );

    // The repair path is a durable Task 8 rebuild job over a fresh generation.
    let connection = rusqlite::Connection::open(fixture.config.database_path()).unwrap();
    let (generation_id, status): (String, String) = connection
        .query_row(
            "select generation_id, status from semantic_generations
             where generation_id like 'repair-%'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "building");
    let job_status: String = connection
        .query_row(
            "select status from semantic_jobs where generation_id = ?1",
            [generation_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(job_status, "queued");
}

#[test]
fn corrupt_hit_repair_does_not_schedule_twice_while_a_rebuild_is_building() {
    let fixture = fixture();
    import_text(&fixture, "a.txt", "Alpha paragraph one.");
    activate_generation(&fixture);
    let connection = rusqlite::Connection::open(fixture.config.database_path()).unwrap();
    connection
        .execute(
            "update rag_chunks set text = 'Rewritten paragraph one.' where id = 1",
            [],
        )
        .unwrap();
    drop(connection);

    let service = armed_service(&fixture);
    let first = service
        .recall(fixture.session_id, &context(), "Alpha", 10)
        .unwrap();
    let second = service
        .recall(fixture.session_id, &context(), "Alpha", 10)
        .unwrap();
    assert!(
        warnings_of(&first)
            .iter()
            .any(|warning| warning.kind == WARNING_REPAIR_SCHEDULED)
    );
    let connection = rusqlite::Connection::open(fixture.config.database_path()).unwrap();
    let repairs: i64 = connection
        .query_row(
            "select count(*) from semantic_generations where generation_id like 'repair-%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(repairs, 1, "one repair at a time");
    assert!(
        warnings_of(&second)
            .iter()
            .any(|warning| warning.kind == WARNING_REPAIR_SCHEDULED),
        "the still-corrupt lane keeps reporting the pending repair"
    );
}

// ---------------------------------------------------------------------------
// Contract awareness
// ---------------------------------------------------------------------------

fn propose_magic_rule(fixture: &Fixture) -> i64 {
    let termbase = Termbase::open(&fixture.config, &context()).unwrap();
    let rule = termbase
        .propose(
            "魔法",
            "magic",
            &ProposeFields {
                forbidden_variants: vec!["sorcery".to_string()],
                ..Default::default()
            },
        )
        .unwrap();
    termbase.approve(rule.id, "tester", "test fixture").unwrap();
    rule.id
}

#[test]
fn contract_conflicts_are_marked_on_advisory_hits() {
    let fixture = fixture();
    let rule_id = propose_magic_rule(&fixture);
    import_text(
        &fixture,
        "conflict.txt",
        "The old scroll renders 魔法 as sorcery.",
    );
    import_text(&fixture, "clean.txt", "A quiet village scene.");

    let response = RecallService::open(&fixture.config)
        .unwrap()
        .recall(fixture.session_id, &context(), "魔法 scroll", 10)
        .unwrap();
    let conflicting = response
        .hits
        .iter()
        .filter_map(|hit| match hit {
            RecallHit::Rag {
                chunk,
                conflicts_with_rule_ids,
                ..
            } => Some((chunk.text.clone(), conflicts_with_rule_ids.clone())),
            _ => None,
        })
        .find(|(text, _)| text.contains("sorcery"))
        .expect("the conflicting chunk must be recalled");
    assert_eq!(
        conflicting.1,
        vec![rule_id],
        "the forbidden variant must be marked with the active rule id"
    );

    // Clean advisory evidence stays unmarked.
    let clean = RecallService::open(&fixture.config)
        .unwrap()
        .recall(fixture.session_id, &context(), "quiet village", 10)
        .unwrap();
    for hit in &clean.hits {
        if let RecallHit::Rag {
            conflicts_with_rule_ids,
            ..
        } = hit
        {
            assert!(conflicts_with_rule_ids.is_empty());
        }
    }
}

#[test]
fn adversarial_advisory_ranking_cannot_alter_the_deterministic_contract() {
    let fixture = fixture();
    let rule_id = propose_magic_rule(&fixture);
    // The chunk text equals the query text, so with the semantic lane armed it
    // is the fused rank-1 advisory hit — the strongest adversarial placement.
    import_text(&fixture, "conflict.txt", "魔法 is rendered as sorcery.");
    activate_generation(&fixture);

    let query = "魔法 is rendered as sorcery.";
    let termbase = Termbase::open(&fixture.config, &context()).unwrap();
    let contract_before = termbase.contract(query).unwrap();

    let response = armed_service(&fixture)
        .recall(fixture.session_id, &context(), query, 10)
        .unwrap();

    // Retrieval cannot mutate, satisfy, or suppress the separate contract:
    // recomputing it after the recall yields exactly the same terms.
    let contract_after = termbase.contract(query).unwrap();
    assert_eq!(contract_before, contract_after);
    assert_eq!(contract_before.len(), 1);

    // The top advisory hit is the conflicting chunk, and it only ever carries
    // markers — never contract authority.
    let top = response
        .hits
        .iter()
        .find(|hit| matches!(hit, RecallHit::Rag { .. }))
        .expect("the conflicting chunk must be recalled");
    match top {
        RecallHit::Rag {
            chunk,
            conflicts_with_rule_ids,
            ..
        } => {
            assert_eq!(chunk.text, query);
            assert_eq!(conflicts_with_rule_ids, &vec![rule_id]);
        }
        _ => unreachable!(),
    }
    assert!(response.hits.iter().all(|hit| {
        matches!(
            hit,
            RecallHit::LongTerm { .. } | RecallHit::ShortTerm { .. } | RecallHit::Rag { .. }
        )
    }));
}

// ---------------------------------------------------------------------------
// Boosts / ledger parity under an armed lane
// ---------------------------------------------------------------------------

#[test]
fn armed_lane_leaves_long_term_boosts_and_activations_unchanged() {
    let fixture = fixture();
    let crystals = CrystalStore::open(&fixture.config).unwrap();
    let crystal_id = crystals
        .add_crystal(
            &context(),
            "lesson",
            &NewCrystal::new("lesson", "Activations are recorded on recall."),
        )
        .unwrap();
    import_text(&fixture, "a.txt", "Cooking Talent appears here.");
    activate_generation(&fixture);

    let service = armed_service(&fixture);
    let response = service
        .recall(fixture.session_id, &context(), "activations recorded", 10)
        .unwrap();

    // The long-term hit keeps its boost path and writes the activation ledger
    // exactly as before.
    let long_term = response
        .hits
        .iter()
        .filter_map(|hit| match hit {
            RecallHit::LongTerm {
                crystal,
                activation_id,
                ..
            } => Some((crystal.id, *activation_id)),
            _ => None,
        })
        .find(|(id, _)| *id == crystal_id)
        .expect("the crystal must still be recalled");
    assert!(long_term.1 > 0, "activation id recorded for the hit");

    let connection = rusqlite::Connection::open_with_flags(
        fixture.config.database_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let count: i64 = connection
        .query_row(
            "select count(*) from crystal_activations where crystal_id = ?1",
            [crystal_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

// ---------------------------------------------------------------------------
// Session-less hybrid search (task C5)
// ---------------------------------------------------------------------------

/// `search_series` is the session-less half of the same hybrid retrieval:
/// FTS chunk lane fused with the semantic chunk lane, series-isolated, with
/// no session, no ledger, and no deterministic contract. A query with no
/// lexical overlap can only have come from the semantic lane, which is what
/// makes the tool's provenance marker meaningful.
#[test]
fn search_series_fuses_both_chunk_lanes_within_one_series() {
    use hieronymus::semantic_recall::SEMANTIC_MATCH_REASON;

    let fixture = fixture();
    import_text(&fixture, "a.txt", "Cooking Talent appears here.");
    // A second series whose chunks must never appear in a demo search.
    Registry::open(&fixture.config)
        .unwrap()
        .create_series("ghost", "ghost", "ja", "en", None)
        .unwrap();
    let foreign = write_source(&fixture, "b.txt", "Cooking Talent appears here too.");
    RagStore::open(&fixture.config)
        .unwrap()
        .import_file("ghost", &foreign, &RagImport::new())
        .unwrap();
    activate_generation(&fixture);

    let service = armed_service(&fixture);

    // The lexical carrier keeps its lane reason when both lanes agree.
    let lexical = service.search_series("demo", "Cooking Talent", 10).unwrap();
    assert!(!lexical.is_empty());
    for hit in &lexical {
        assert_eq!(
            hit.chunk.series_slug, "demo",
            "foreign series must never leak: {hit:?}"
        );
        // RRF scores are bounded by the sum of two rank-1 contributions.
        assert!(hit.score > 0.0 && hit.score <= 2.0 / 61.0, "{hit:?}");
    }

    // Semantic-only provenance: nothing in the query occurs in the chunk.
    let semantic = service
        .search_series("demo", "zzqxj nonlexical probe", 10)
        .unwrap();
    assert!(
        semantic
            .iter()
            .any(|hit| hit.reason == SEMANTIC_MATCH_REASON),
        "the semantic lane must surface the chunk: {semantic:?}"
    );
    assert!(
        semantic.iter().all(|hit| hit.chunk.series_slug == "demo"),
        "{semantic:?}"
    );
}

/// The refusal: a series that HAS indexed text may never be answered with the
/// lexical half alone, because a caller cannot tell that answer apart from a
/// complete one. This is review finding A5 at the library boundary.
#[test]
fn search_series_refuses_a_lexical_only_answer_over_indexed_text() {
    let fixture = fixture();
    import_text(&fixture, "a.txt", "Cooking Talent appears here.");

    let error = RecallService::open(&fixture.config)
        .unwrap()
        .search_series("demo", "Cooking Talent", 10)
        .expect_err("an unarmed lane cannot serve a strict hybrid search");
    assert!(
        matches!(
            error,
            hieronymus::recall::RecallError::SemanticUnavailable(_)
        ),
        "{error:?}"
    );
    // The FTS lane would have answered this query; that is the point.
    assert!(
        !RagStore::open(&fixture.config)
            .unwrap()
            .search("demo", "Cooking Talent", 10, &[], &[], &[])
            .unwrap()
            .is_empty()
    );
}

/// The one honest empty answer: a series with no chunks has nothing to
/// retrieve, so an empty result withholds nothing even with the semantic half
/// missing (the ready-for-ingest case).
#[test]
fn search_series_answers_empty_for_a_series_with_no_chunks() {
    let fixture = fixture();
    let rows = RecallService::open(&fixture.config)
        .unwrap()
        .search_series("demo", "Cooking Talent", 10)
        .expect("an empty series is a complete empty answer");
    assert!(rows.is_empty());

    // Limit validation stays ahead of everything else.
    assert!(matches!(
        RecallService::open(&fixture.config)
            .unwrap()
            .search_series("demo", "Cooking Talent", 0),
        Err(hieronymus::recall::RecallError::LimitTooSmall)
    ));
}
