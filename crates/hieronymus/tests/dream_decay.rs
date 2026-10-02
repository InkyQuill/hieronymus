#[path = "support/current_story.rs"]
mod current_story;

use hieronymus::{
    crystals::{CrystalStore, NewCrystal},
    data_root::HieronymusConfig,
    db::open_migrated,
    dream_config::{default_dream_config, save_dream_config},
    dream_workflows::WorkflowResolver,
    dreaming::DreamService,
    memory_models::TranslationContext,
    registry::Registry,
    story_applicability::Viewpoint,
    workspace::{ShortTermMemoryInput, WorkspaceStore},
};
use rusqlite::params;

struct Fixture {
    _root: tempfile::TempDir,
    config: HieronymusConfig,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        for slug in ["book", "other"] {
            Registry::open(&config)
                .unwrap()
                .create_series(slug, slug, "ja", "ru", None)
                .unwrap();
            current_story::register_public(&config, slug, "I", "Opening");
        }
        Self {
            _root: root,
            config,
        }
    }
    fn context(slug: &str) -> TranslationContext {
        let mut context = current_story::context(slug, "ja", "ru", "translate");
        context.story_viewpoint = Viewpoint::Narrator;
        context
    }
    fn crystal(&self, slug: &str, new: NewCrystal) -> i64 {
        let id = CrystalStore::open(&self.config)
            .unwrap()
            .add_crystal(&Self::context(slug), &new.crystal_type, &new)
            .unwrap();
        open_migrated(&self.config.database_path()).unwrap().execute("update crystals set created_at='2026-09-01T00:00:00Z',updated_at='2026-09-01T00:00:00Z' where id=?",[id]).unwrap();
        id
    }
    fn session(&self, context: &TranslationContext, count: usize) -> i64 {
        let ws = WorkspaceStore::open(&self.config).unwrap();
        let session = ws.start_session(context).unwrap();
        for i in 0..count {
            let text = format!("A distinct new observation from this session: evidence {i}.");
            let mut input = ShortTermMemoryInput::new("note", &text);
            input.claims = vec![current_story::claim(
                &self.config,
                &context.series_slug,
                &text,
            )];
            ws.add_short_term_memory(session.id, &input).unwrap();
        }
        ws.complete_session(session.id).unwrap();
        session.id
    }
    fn run(&self) {
        DreamService::open(&self.config, WorkflowResolver::deterministic())
            .unwrap()
            .run_all("manual", true, false)
            .unwrap();
    }
    fn scores(&self, id: i64) -> (f64, f64) {
        open_migrated(&self.config.database_path())
            .unwrap()
            .query_row(
                "select strength,confidence from crystals where id=?",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    }
    fn count(&self, sql: &str) -> i64 {
        open_migrated(&self.config.database_path())
            .unwrap()
            .query_row(sql, [], |r| r.get(0))
            .unwrap()
    }
}

#[test]
fn meaningful_session_decays_only_compatible_series_and_preserves_confidence() {
    let f = Fixture::new();
    let eligible = f.crystal(
        "book",
        NewCrystal::new("lesson", "Old applicable lesson").confidence(0.8),
    );
    let other = f.crystal("other", NewCrystal::new("lesson", "Other series lesson"));
    let db = open_migrated(&f.config.database_path()).unwrap();
    let context_other = f.crystal(
        "book",
        NewCrystal::new("lesson", "Different chapter lesson"),
    );
    db.execute("update applicabilities set chapter_key='Revelation' where id in(select applicability_id from memory_claims where id in(select claim_id from claim_bindings where crystal_id=?))",[context_other]).unwrap();
    f.session(&Fixture::context("book"), 1);
    f.run();
    assert_eq!(f.scores(eligible), (0.48, 0.8));
    assert_eq!(f.scores(other), (0.5, 0.5));
    assert_eq!(f.scores(context_other), (0.5, 0.5));
    assert_eq!(f.count("select count(*) from memory_events where event_type='cycle_decay' and applied=1 and confidence_delta=0"),1);
    assert_eq!(f.count("select count(*) from dream_audit_entries where json_extract(payload_json,'$.phase_name')='salience_decay' and json_extract(payload_json,'$.decayed_crystals[0].strength_after')=0.48"),1);
}

#[test]
fn protects_explicit_authority_rules_recall_and_confirmation() {
    let f = Fixture::new();
    let user = f.crystal(
        "book",
        NewCrystal::new("lesson", "User authority").source_credibility("explicit_user"),
    );
    let rule = f.crystal(
        "book",
        NewCrystal::new("rule", "Always render this approved term consistently"),
    );
    let recalled = f.crystal("book", NewCrystal::new("lesson", "Recalled lesson"));
    let confirmed = f.crystal("book", NewCrystal::new("lesson", "Confirmed lesson"));
    let session = f.session(&Fixture::context("book"), 1);
    let db = open_migrated(&f.config.database_path()).unwrap();
    db.execute("insert into crystal_activations(crystal_id,session_id,recall_query,rank,score,outcome,created_at) values(?1,?2,'query',1,1,'miss','2026-09-01T00:00:00Z')",params![recalled,session]).unwrap();
    db.execute("insert into memory_events(crystal_id,event_type,source_role,applied,created_at) values(?,'confirmed_by_user','user',1,'2026-09-01T00:00:00Z')",[confirmed]).unwrap();
    f.run();
    for id in [user, rule, recalled, confirmed] {
        assert_eq!(f.scores(id), (0.5, 0.5));
    }
}

#[test]
fn capped_drain_and_restart_count_one_opportunity_and_respect_floor() {
    let f = Fixture::new();
    let id = f.crystal(
        "book",
        NewCrystal::new("lesson", "Nearly at salience floor").strength(0.21),
    );
    let mut settings = default_dream_config();
    settings.max_short_term_memories_per_run = 1;
    save_dream_config(&f.config, &settings).unwrap();
    f.session(&Fixture::context("book"), 3);
    f.run();
    assert_eq!(f.scores(id), (0.2, 0.5));
    assert_eq!(
        f.count("select count(*) from memory_events where event_type='dream_decay_opportunity'"),
        1
    );
    f.run();
    assert_eq!(
        f.count("select count(*) from memory_events where event_type='cycle_decay'"),
        1
    );
    assert_eq!(f.scores(id), (0.2, 0.5));
}

#[test]
fn unknown_context_empty_and_failed_completion_never_decay() {
    let f = Fixture::new();
    let id = f.crystal("book", NewCrystal::new("lesson", "Untouched lesson"));
    f.run();
    f.session(&Fixture::context("book").chapter("Unknown"), 1);
    f.run();
    assert_eq!(f.scores(id), (0.5, 0.5));
    f.session(&Fixture::context("book"), 1);
    open_migrated(&f.config.database_path()).unwrap().execute_batch("create trigger reject_decay before insert on memory_events when new.event_type='dream_decay_opportunity' begin select raise(abort,'fixture completion failure'); end;").unwrap();
    let result = DreamService::open(&f.config, WorkflowResolver::deterministic())
        .unwrap()
        .run_all("manual", true, false);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(f.scores(id), (0.5, 0.5));
    assert_eq!(f.count("select count(*) from memory_events where event_type in ('cycle_decay','dream_decay_opportunity')"),0);
    assert_eq!(
        f.count("select count(*) from dream_phase_runs where phase='salience_decay' and status='completed'"),
        0
    );
    assert_eq!(
        f.count("select count(*) from dream_runs where status='failed'"),
        0
    );
}

#[test]
fn preceding_persistence_consumes_budget_and_capped_opportunity_is_terminal() {
    let f = Fixture::new();
    let ids: Vec<_> = (0..3)
        .map(|i| f.crystal("book", NewCrystal::new("lesson", format!("Old lesson {i}"))))
        .collect();
    let mut settings = default_dream_config();
    settings.max_changed_crystals_per_cycle = 2;
    save_dream_config(&f.config, &settings).unwrap();
    f.session(&Fixture::context("book"), 1);
    f.run();
    assert_eq!(
        f.count("select count(*) from memory_events where event_type='cycle_decay'"),
        1
    );
    assert_eq!(ids.iter().filter(|&&id| f.scores(id).0 == 0.48).count(), 1);
    f.run();
    assert_eq!(
        f.count("select count(*) from memory_events where event_type='cycle_decay'"),
        1
    );
}

#[test]
fn decay_audit_failure_rolls_back_score_ledger_and_success_status() {
    let f = Fixture::new();
    let id = f.crystal("book", NewCrystal::new("lesson", "Audited lesson"));
    f.session(&Fixture::context("book"), 1);
    open_migrated(&f.config.database_path()).unwrap().execute_batch("create trigger reject_decay_audit before insert on dream_audit_entries when new.event_type='phase_completed' and json_extract(new.payload_json,'$.phase_name')='salience_decay' begin select raise(abort,'fixture audit failure'); end;").unwrap();
    let result = DreamService::open(&f.config, WorkflowResolver::deterministic())
        .unwrap()
        .run_all("manual", true, false);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(f.scores(id), (0.5, 0.5));
    assert_eq!(f.count("select count(*) from memory_events where event_type in ('cycle_decay','dream_decay_opportunity')"),0);
    assert_eq!(
        f.count("select count(*) from dream_runs where status='completed'"),
        1
    );
    assert_eq!(
        f.count("select count(*) from dream_phase_runs where phase='salience_decay' and status='completed'"),
        0
    );
}

#[test]
fn ambiguous_context_at_completion_skips_decay_without_failing_run() {
    let f = Fixture::new();
    let id = f.crystal(
        "book",
        NewCrystal::new("lesson", "Ambiguous context lesson"),
    );
    let session = f.session(&Fixture::context("book"), 1);
    let config = f.config.clone();
    let mut service = DreamService::open(&f.config, WorkflowResolver::deterministic()).unwrap();
    service.set_phase_observer(std::sync::Arc::new(move |_,_,phase| {
        if phase=="persistence" {
            let db=open_migrated(&config.database_path()).unwrap();
            db.execute("insert into story_positions(timeline_id,volume_key,chapter_key,ordinal,evidence_id) select timeline_id,'II','Opening',2,evidence_id from story_positions where chapter_key='Opening' and volume_key='I' and timeline_id in(select id from story_timelines where series_id=(select id from series where slug='book'))",[]).unwrap();
            db.execute("update task_sessions set volume='' where id=?",[session]).unwrap();
            db.execute("delete from task_session_story_scopes where session_id=? and story_scope like 'volume:%'",[session]).unwrap();
        }
    }));
    let result = service.run_cycle("manual", false).unwrap();
    assert_eq!(result.status, "completed");
    assert_eq!(f.scores(id), (0.5, 0.5));
    assert_eq!(
        f.count("select count(*) from memory_events where event_type='dream_decay_opportunity'"),
        0
    );
}

#[test]
fn overlapping_sessions_each_decay_without_masking_real_edits() {
    let f = Fixture::new();
    let eligible = f.crystal("book", NewCrystal::new("lesson", "Shared unused lesson"));
    let edited = f.crystal(
        "book",
        NewCrystal::new("lesson", "Edited during overlapping work"),
    );
    f.session(&Fixture::context("book"), 1);
    f.session(&Fixture::context("book"), 1);
    let db = open_migrated(&f.config.database_path()).unwrap();
    db.execute(
        "update crystals set text='A real correction',updated_at='2099-01-01T00:00:00Z' where id=?",
        [edited],
    )
    .unwrap();
    f.run();
    assert!((f.scores(eligible).0 - 0.46).abs() < 1e-9);
    assert_eq!(f.scores(edited), (0.5, 0.5));
    assert_eq!(
        f.count("select count(*) from memory_events where event_type='dream_decay_opportunity'"),
        2
    );
    f.run();
    assert!((f.scores(eligible).0 - 0.46).abs() < 1e-9);
}

#[test]
fn advisory_failure_keeps_persistence_history_and_drains_remaining_sessions() {
    let f = Fixture::new();
    f.crystal("book", NewCrystal::new("lesson", "Eligible lesson"));
    f.session(&Fixture::context("book"), 1);
    f.session(&Fixture::context("book"), 1);
    open_migrated(&f.config.database_path()).unwrap().execute_batch("create trigger reject_decay before insert on memory_events when new.event_type='dream_decay_opportunity' begin select raise(abort,'fixture completion failure'); end;").unwrap();
    let result = DreamService::open(&f.config, WorkflowResolver::deterministic())
        .unwrap()
        .run_all("manual", true, false)
        .unwrap();
    assert_eq!(result.outcome, "completed");
    assert_eq!(result.input_count, 2);
    assert_eq!(result.created_crystal_count, 2);
    assert!(result.record.error.contains("salience_decay skipped"));
    assert_eq!(
        f.count(
            "select count(*) from dream_phase_runs where phase='persistence' and status='completed'"
        ),
        2
    );
    assert_eq!(f.count("select count(*) from dream_audit_entries where event_type='phase_failed' and json_extract(payload_json,'$.phase_name')='salience_decay'"),2);
    assert_eq!(f.count("select count(*) from dream_runs where status='completed' and input_count=1 and created_crystal_count=1"),2);
}

#[test]
fn unavailable_warning_audit_keeps_committed_counts_and_phase_history() {
    let f = Fixture::new();
    f.crystal("book", NewCrystal::new("lesson", "Applicable lesson"));
    f.session(&Fixture::context("book"), 1);
    open_migrated(&f.config.database_path()).unwrap().execute_batch("create trigger reject_all_decay_audits before insert on dream_audit_entries when json_extract(new.payload_json,'$.phase_name')='salience_decay' begin select raise(abort,'fixture diagnostic storage failure'); end;").unwrap();
    let result = DreamService::open(&f.config, WorkflowResolver::deterministic())
        .unwrap()
        .run_all("manual", true, false);
    assert!(result.is_err());
    assert_eq!(f.count("select count(*) from dream_runs where status='failed' and input_count=1 and created_crystal_count=1"),1);
    assert_eq!(
        f.count(
            "select count(*) from dream_phase_runs where phase='persistence' and status='completed'"
        ),
        1
    );
    assert_eq!(
        f.count("select count(*) from short_term_memories where archived_at is not null"),
        1
    );
}

#[test]
fn unavailable_warning_audit_keeps_committed_reconsolidation_history() {
    let f = Fixture::new();
    let text = "The binding ritual needs chalk.";
    let mut original_input = NewCrystal::new("lesson", text);
    original_input.claims = vec![current_story::claim(&f.config, "book", text)];
    let original = f.crystal("book", original_input);
    let workspace = WorkspaceStore::open(&f.config).unwrap();
    let session = workspace
        .start_session(&Fixture::context("book"))
        .unwrap()
        .id;
    for text in [text, "The binding ritual also requires fresh water."] {
        let mut input = ShortTermMemoryInput::new("note", text);
        input.claims = vec![current_story::claim(&f.config, "book", text)];
        workspace.add_short_term_memory(session, &input).unwrap();
    }
    workspace.complete_session(session).unwrap();
    let db = open_migrated(&f.config.database_path()).unwrap();
    // Turn one captured observation into an unchanged working copy; the other
    // observation still exercises provider persistence before reconsolidation.
    db.execute(
        "update short_term_memories set source_crystal_id=?1,
         text='The binding ritual needs chalk.'
         where id=(select min(id) from short_term_memories where session_id=?2)",
        params![original, session],
    )
    .unwrap();
    db.execute("delete from claim_bindings where short_term_id=(select min(id) from short_term_memories where session_id=?)",[session]).unwrap();
    db.execute("insert into claim_bindings(claim_id,short_term_id) select claim_id,(select min(id) from short_term_memories where session_id=?2) from claim_bindings where crystal_id=?1",params![original,session]).unwrap();
    db.execute_batch("create trigger reject_all_decay_audits before insert on dream_audit_entries when json_extract(new.payload_json,'$.phase_name')='salience_decay' begin select raise(abort,'fixture diagnostic storage failure'); end;").unwrap();
    let result = DreamService::open(&f.config, WorkflowResolver::deterministic())
        .unwrap()
        .run_all("manual", true, false);
    assert!(result.is_err(), "{result:?}");
    assert_eq!(f.count("select count(*) from dream_runs where status='failed' and input_count=1 and created_crystal_count=1"), 1);
    assert_eq!(
        f.count("select count(*) from crystals where last_reinforced_cycle is not null"),
        1
    );
    assert_eq!(f.count("select count(*) from dream_phase_runs where phase in ('persistence','reconsolidation') and status='completed'"), 2);
    assert_eq!(
        f.count("select count(*) from short_term_memories where archived_at is not null"),
        2
    );
}
