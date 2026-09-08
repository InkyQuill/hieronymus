#![allow(dead_code)]
use hieronymus::{
    claim_capture::{ClaimInput, capture_claim_tx},
    claim_reads::ClaimTarget,
    data_root::HieronymusConfig,
    db::open_migrated,
    memory_models::TranslationContext,
    story_applicability::*,
};
use rusqlite::params;

pub fn register(config: &HieronymusConfig, slug: &str) {
    let mut db = open_migrated(&config.database_path()).unwrap();
    let tx = db.transaction().unwrap();
    let series: i64 = tx
        .query_row("select id from series where slug=?", [slug], |r| r.get(0))
        .unwrap();
    tx.execute(
        "insert into story_timelines(series_id,name) values(?,'reading')",
        [series],
    )
    .unwrap();
    let timeline = tx.last_insert_rowid();
    let manifest = OrderManifest {
        series_id: series,
        timeline_id: timeline,
        positions: vec![
            ManifestPosition {
                volume_key: "I".into(),
                chapter_key: "Opening".into(),
                scene_key: "".into(),
            },
            ManifestPosition {
                volume_key: "I".into(),
                chapter_key: "Revelation".into(),
                scene_key: "".into(),
            },
        ],
    };
    let content = serde_json::to_string(&manifest).unwrap();
    tx.execute("insert into evidence_records(series_id,kind,source_identity,source_hash,span_start,span_end,content,binding_json,created_at) values(?1,'observation','test-book-manifest','fixture-manifest',0,?2,?3,'{}','now')", params![series,content.len() as i64,content]).unwrap();
    StoryApplicability::register_order_tx(&tx, &manifest, tx.last_insert_rowid(), 0).unwrap();
    tx.commit().unwrap();
}

pub fn context(slug: &str, source: &str, target: &str, task: &str) -> TranslationContext {
    TranslationContext::new(slug, source, target, task)
        .volume("I")
        .chapter("Opening")
}

pub fn claim(config: &HieronymusConfig, slug: &str, text: &str) -> ClaimInput {
    let db = open_migrated(&config.database_path()).unwrap();
    let (series,timeline) = db.query_row("select s.id,t.id from series s join story_timelines t on t.series_id=s.id where s.slug=?",[slug],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    ClaimInput {
        text: text.into(),
        concept_id: None,
        applicability: ApplicabilityV1 {
            series_id: series,
            timeline_id: Some(timeline),
            volume_key: None,
            chapter_key: None,
            scope_predicates: vec![],
            valid_from: None,
            valid_until: None,
            metadata_state: MetadataState::Resolved,
            knowledge_gates: vec![KnowledgeGateV1 {
                viewpoint: KnowledgeViewpoint::All,
                known_from: None,
                known_until: None,
            }],
        },
    }
}

pub fn crystal(
    config: &HieronymusConfig,
    kind: &str,
    text: &str,
) -> hieronymus::crystals::NewCrystal {
    let mut input = hieronymus::crystals::NewCrystal::new(kind, text);
    input.claims = vec![claim(config, "demo", text)];
    input
}
pub fn memory(
    config: &HieronymusConfig,
    kind: &str,
    text: &str,
) -> hieronymus::workspace::ShortTermMemoryInput {
    let mut input = hieronymus::workspace::ShortTermMemoryInput::new(kind, text);
    input.claims = vec![claim(config, "demo", text)];
    input
}

/// Audited fixture capture, before query/index activation. Imported assertions
/// explicitly have all-viewpoint positive gates; raw untyped import tests do not
/// call this helper and remain Unknown.
pub fn capture_chunks(config: &HieronymusConfig, slug: &str) {
    let mut db = open_migrated(&config.database_path()).unwrap();
    let rows = db.prepare("select id,text from rag_chunks r where series_slug=? and not exists(select 1 from claim_bindings b where b.rag_chunk_id=r.id)").unwrap().query_map([slug],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?))).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    for (id, text) in rows {
        let input = claim(config, slug, &text);
        let tx = db.transaction().unwrap();
        capture_claim_tx(&tx, ClaimTarget::RagChunk(id), &input).unwrap();
        tx.commit().unwrap();
    }
}

/// Register an actual disposable manifest through the current snapshot producer.
pub fn register_public(config: &HieronymusConfig, slug: &str, volume: &str, chapter: &str) {
    use hieronymus::authority_producers::{EvidenceProducer, SnapshotInput};
    use sha2::{Digest, Sha256};
    let mut db = open_migrated(&config.database_path()).unwrap();
    let series: i64 = db
        .query_row("select id from series where slug=?", [slug], |r| r.get(0))
        .unwrap();
    let manifest = serde_json::json!({"version":1,"series_id":series,"timeline_name":"reading","positions":[{"volume_key":volume,"chapter_key":chapter,"scene_key":""}]}).to_string();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("order.json");
    std::fs::write(&path, &manifest).unwrap();
    let input = SnapshotInput::File {
        path,
        expected_hash: format!("{:x}", Sha256::digest(manifest.as_bytes())),
    };
    EvidenceProducer::new(&mut db)
        .register_manifest(series, &input, None, 0)
        .unwrap();
}
