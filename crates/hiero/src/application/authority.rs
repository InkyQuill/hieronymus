//! Public evidence producers and origin-bound decisions. No providers participate.
use super::correction_parser::{self as parser, OriginReceipt, SelectionContext};
use super::{AppError, Application, decode, domain};
use crate::trusted_ingress::{DecisionDraftV1, Principal, TrustedIngress, hash};
use hieronymus::authority::{DecisionStore, EvidenceBindingV1};
use hieronymus::authority_models::*;
use hieronymus::authority_producers::{DocumentSelection, EvidenceProducer, SnapshotInput};
use hieronymus::story_applicability::{ApplicabilityV1, MetadataState};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

fn connection(app: &Application) -> Result<Connection, AppError> {
    hieronymus::db::open_migrated(&app.config().database_path()).map_err(domain)
}
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SnapshotDraft {
    File {
        path: std::path::PathBuf,
        expected_hash: String,
    },
    Retained {
        evidence_id: i64,
        expected_hash: String,
    },
}
impl SnapshotDraft {
    fn input(self) -> SnapshotInput {
        match self {
            Self::File {
                path,
                expected_hash,
            } => SnapshotInput::File {
                path,
                expected_hash,
            },
            Self::Retained {
                evidence_id,
                expected_hash,
            } => SnapshotInput::Retained {
                evidence_id,
                expected_hash,
            },
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestDraft {
    series_id: i64,
    snapshot: SnapshotDraft,
    timeline_id: Option<i64>,
    expected_revision: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectionDraft {
    start: usize,
    end: usize,
    expected_text: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureDraft {
    series_id: i64,
    snapshot: SnapshotDraft,
    kind: EvidenceKind,
    selection: SelectionDraft,
    binding: EvidenceBindingV1,
}

pub(crate) fn dispatch(
    app: &Application,
    tool: &str,
    args: &Value,
) -> Option<Result<Value, AppError>> {
    Some(match tool {
        "hieronymus_decide" | "hieronymus_correct" => {
            decide(app, args, tool == "hieronymus_correct")
        }
        "hieronymus_order_register" => register_manifest(app, args),
        "hieronymus_evidence_capture" => capture(app, args),
        _ => return None,
    })
}
fn decide(app: &Application, args: &Value, correction: bool) -> Result<Value, AppError> {
    let draft: DecisionDraftV1 = decode(args)?;
    if correction != matches!(draft.operation, OperationV1::Correct { .. }) {
        return Err(AppError::Invalid(
            "correction operations use hieronymus_correct; other decisions use hieronymus_decide"
                .into(),
        ));
    }
    let mut db = connection(app)?;
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(domain)?;
    let unresolved: bool = tx.query_row("select exists(select 1 from decision_records where decision_id=? and json_extract(canonical_request,'$.kind')='unresolved_signal')",[&draft.decision_id],|r|r.get(0)).map_err(domain)?;
    if unresolved {
        return Err(AppError::Authority(DecisionErrorV1::IdempotencyConflict));
    }
    let request = TrustedIngress::new(&tx).bind(&Principal::Agent, draft)?;
    tx.commit().map_err(domain)?;
    serde_json::to_value(DecisionStore::new(&mut db).apply(&request)?).map_err(domain)
}
fn register_manifest(app: &Application, args: &Value) -> Result<Value, AppError> {
    let draft: ManifestDraft = decode(args)?;
    let mut db = connection(app)?;
    serde_json::to_value(
        EvidenceProducer::new(&mut db)
            .register_manifest(
                draft.series_id,
                &draft.snapshot.input(),
                draft.timeline_id,
                draft.expected_revision,
            )
            .map_err(domain)?,
    )
    .map_err(domain)
}
fn capture(app: &Application, args: &Value) -> Result<Value, AppError> {
    let d: CaptureDraft = decode(args)?;
    let mut db = connection(app)?;
    serde_json::to_value(
        EvidenceProducer::new(&mut db)
            .capture(
                d.series_id,
                &d.snapshot.input(),
                d.kind,
                &DocumentSelection {
                    start: d.selection.start,
                    end: d.selection.end,
                    expected_text: d.selection.expected_text,
                },
                &d.binding,
            )
            .map_err(domain)?,
    )
    .map_err(domain)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SelectedRevision {
    pub id: i64,
    pub revision: u64,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StructuredCorrection {
    Rendering {
        canonical: String,
        approved_variants: Option<Vec<String>>,
        forbidden_variants: Option<Vec<String>>,
    },
    Invalidate,
    Qualify {
        qualification: String,
    },
}
/// Same typed context for the console form and independently delivered host events.
/// Revisions are observed values; this route never refreshes them to force a commit.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UserCorrectionV1 {
    pub version: u8,
    pub decision_id: String,
    pub event_id: String,
    pub expected_revision: u64,
    pub series_id: i64,
    pub session_id: Option<i64>,
    pub source_language: Option<String>,
    pub target_language: Option<String>,
    pub applicability: Option<ApplicabilityV1>,
    #[serde(default)]
    pub selected_sources: Vec<EvidenceRef>,
    #[serde(default)]
    pub selected_claims: Vec<SelectedRevision>,
    pub selected_rule: Option<SelectedRevision>,
    pub text: Option<String>,
    pub structured: Option<StructuredCorrection>,
}
/// Guarded routes pass their actual principal; nothing in the JSON can choose it.
pub(crate) fn user_correction(
    app: &Application,
    principal: Principal,
    args: &Value,
) -> Result<Value, AppError> {
    let input: UserCorrectionV1 = decode(args)?;
    if input.version != 1
        || input.series_id <= 0
        || input.event_id.is_empty()
        || input.text.is_some() == input.structured.is_some()
        || input.selected_claims.iter().any(|s| s.id <= 0)
        || input.selected_rule.as_ref().is_some_and(|s| s.id <= 0)
    {
        return Err(AppError::Authority(DecisionErrorV1::InvalidRequest));
    }
    if matches!(principal, Principal::HostEvent)
        && (input.session_id.is_none() || input.structured.is_some())
    {
        return Err(AppError::Authority(DecisionErrorV1::UnverifiedOrigin));
    }
    let mut db = connection(app)?;
    if !crate::trusted_ingress::valid_uuid(&input.decision_id)
        || input.expected_revision >= i64::MAX as u64
        || input.selected_sources.iter().any(|r| {
            r.id <= 0 || r.kind != EvidenceKind::SourcePassage || r.span_start >= r.span_end
        })
        || input
            .selected_claims
            .iter()
            .any(|r| r.revision >= i64::MAX as u64)
    {
        return Err(AppError::Authority(DecisionErrorV1::InvalidRequest));
    }
    let valid_series: bool = db
        .query_row(
            "select exists(select 1 from series where id=?)",
            [input.series_id],
            |r| r.get(0),
        )
        .map_err(domain)?;
    if !valid_series {
        return Err(AppError::Authority(DecisionErrorV1::UnknownTarget));
    }
    if let Some(session) = input.session_id {
        let valid:bool=db.query_row("select exists(select 1 from task_sessions t join series s on s.slug=t.series_slug where t.id=? and s.id=?)",params![session,input.series_id],|r|r.get(0)).map_err(domain)?;
        if !valid {
            return Err(AppError::Authority(DecisionErrorV1::OriginMismatch));
        }
    }
    let text = match &input.text {
        Some(text) => text.clone(),
        None => serde_json::to_string(&input.structured).map_err(domain)?,
    };
    if text.is_empty() || text.len() > 65536 {
        return Err(AppError::Authority(DecisionErrorV1::InvalidRequest));
    }
    // Preserve all unresolved authentic signals without manufacturing an operation.
    if let Some(result) = super::authority_signal::replay(&db, &principal, &input)? {
        return Ok(result);
    }
    let resolved = resolve_user(&db, &input, &text);
    let (mut draft, parsed) = match resolved {
        Ok(value) => value,
        Err(ResolveError::Tentative(reason, detail)) => {
            return super::authority_signal::persist(
                &mut db, &principal, &input, &text, reason, detail,
            );
        }
        Err(ResolveError::Error(error)) => return Err(error),
    };
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(domain)?;
    let ingress = TrustedIngress::new(&tx);
    let origin = ingress.mint(&principal, &draft, &input.event_id, &text, parsed.as_ref())?;
    draft.receipt_ref = Some(origin);
    let request = ingress.bind(&principal, draft)?;
    tx.commit().map_err(domain)?;
    serde_json::to_value(DecisionStore::new(&mut db).apply(&request)?).map_err(domain)
}

enum ResolveError {
    Tentative(TentativeReason, &'static str),
    Error(AppError),
}
impl From<rusqlite::Error> for ResolveError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Error(domain(e))
    }
}
fn uncertain<T>(reason: TentativeReason, detail: &'static str) -> Result<T, ResolveError> {
    Err(ResolveError::Tentative(reason, detail))
}
fn resolve_user(
    db: &Connection,
    input: &UserCorrectionV1,
    text: &str,
) -> Result<(DecisionDraftV1, Option<Value>), ResolveError> {
    let applicability = match &input.applicability {
        Some(a) if a.metadata_state == MetadataState::Resolved => a.clone(),
        _ => return uncertain(TentativeReason::UnknownOrder, "missing_or_unresolved_scope"),
    };
    let source_language = match &input.source_language {
        Some(v) if !v.is_empty() => v.clone(),
        _ => {
            return uncertain(
                TentativeReason::AmbiguousIdentity,
                "missing_source_language",
            );
        }
    };
    let languages: Option<(String, String)> = db
        .query_row(
            "select default_source_language,default_target_language from series where id=?",
            [input.series_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((source, target)) = languages else {
        return Err(ResolveError::Error(AppError::Authority(
            DecisionErrorV1::UnknownTarget,
        )));
    };
    if input
        .target_language
        .as_ref()
        .is_some_and(|value| value.is_empty())
    {
        return uncertain(
            TentativeReason::AmbiguousIdentity,
            "missing_target_language",
        );
    }
    for language in
        std::iter::once(source_language.as_str()).chain(input.target_language.as_deref())
    {
        let registered:bool=db.query_row("select exists(select 1 from series_language_tags where series_id=? and language_tag=?)",params![input.series_id,language],|r|r.get(0))?;
        if language != language.trim().to_lowercase()
            || !(registered
                || language == source.trim().to_lowercase()
                || language == target.trim().to_lowercase())
        {
            return uncertain(TentativeReason::AmbiguousIntent, "unsupported_language");
        }
    }
    if applicability.series_id != input.series_id {
        return Err(ResolveError::Error(AppError::Authority(
            DecisionErrorV1::OriginMismatch,
        )));
    }
    if let Some(session) = input.session_id {
        let found:bool=db.query_row("select exists(select 1 from task_sessions t join series s on s.slug=t.series_slug where t.id=? and s.id=?)",params![session,input.series_id],|r|r.get(0))?;
        if !found {
            return Err(ResolveError::Error(AppError::Authority(
                DecisionErrorV1::OriginMismatch,
            )));
        }
    }
    let mut concept_id = None;
    let mut source_text = None;
    for reference in &input.selected_sources {
        let row:Option<(String,String,i64,i64,String)>=db.query_row("select content,source_hash,span_start,span_end,binding_json from evidence_records where id=? and series_id=? and kind='source_passage'",params![reference.id,input.series_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        let Some((content, digest, start, end, binding)) = row else {
            return uncertain(TentativeReason::AmbiguousIdentity, "missing_source");
        };
        if reference.kind != EvidenceKind::SourcePassage
            || digest != reference.content_hash
            || hash(&content) != digest
            || start != reference.span_start as i64
            || end != reference.span_end as i64
        {
            return uncertain(
                TentativeReason::AmbiguousIdentity,
                "changed_source_hash_or_span",
            );
        }
        let binding: EvidenceBindingV1 =
            serde_json::from_str(&binding).map_err(|e| ResolveError::Error(domain(e)))?;
        if binding.source_language != source_language
            || binding.target_language != input.target_language
            || binding.applicability != applicability
        {
            return uncertain(
                TentativeReason::AmbiguousIdentity,
                "source_context_mismatch",
            );
        }
        concept_id = Some(binding.concept_id);
        source_text = content
            .get(reference.span_start..reference.span_end)
            .map(str::to_owned);
    }
    let mut rendering = None;
    if let Some(selected) = &input.selected_rule {
        let row:Option<(Option<i64>,String,String,String,String)>=db.query_row("select concept_id,source_text,canonical_translation,source_language,target_language from term_rules where id=?",[selected.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        let Some((concept, source_form, canonical, sl, tl)) = row else {
            return Err(ResolveError::Error(AppError::Authority(
                DecisionErrorV1::UnknownTarget,
            )));
        };
        if concept != concept_id || sl != source_language || Some(tl) != input.target_language {
            return uncertain(TentativeReason::AmbiguousIdentity, "rule_context_mismatch");
        }
        let mut forms=db.prepare("select surface,form_kind,case_sensitive from term_rule_forms where rule_id=? order by id")?;
        let rows = forms.query_map([selected.id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, bool>(2)?,
            ))
        })?;
        let mut value = RenderingV1 {
            source_forms: vec![source_form],
            canonical,
            approved_variants: vec![],
            forbidden_variants: vec![],
            case_sensitive: false,
        };
        for row in rows {
            let (form, kind, case_sensitive) = row?;
            value.case_sensitive |= case_sensitive;
            match kind.as_str() {
                "source" => {
                    if !value.source_forms.contains(&form) {
                        value.source_forms.push(form)
                    }
                }
                "approved" => {
                    if form != value.canonical {
                        value.approved_variants.push(form)
                    }
                }
                "forbidden" => value.forbidden_variants.push(form),
                _ => {}
            }
        }
        rendering = Some(value);
    }
    let claim = if input.selected_claims.len() == 1 {
        let selected = &input.selected_claims[0];
        let concept: Option<Option<i64>> = db
            .query_row(
                "select concept_id from memory_claims where id=? and series_id=?",
                params![selected.id, input.series_id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(concept) = concept else {
            return Err(ResolveError::Error(AppError::Authority(
                DecisionErrorV1::UnknownTarget,
            )));
        };
        concept_id = concept;
        Some((selected.id, selected.revision))
    } else {
        None
    };
    let context = SelectionContext {
        source_text,
        source_count: input.selected_sources.len(),
        claim,
        claim_count: input.selected_claims.len(),
        languages_resolved: true,
        scope_resolved: true,
        replaces: input.selected_rule.as_ref().map(|r| (r.id, r.revision)),
        rendering,
    };
    let (intent, parsed) = if let Some(structured) = &input.structured {
        let event = match structured {
            StructuredCorrection::Rendering { canonical, .. } => format!(
                "translate this as {}",
                serde_json::to_string(canonical).unwrap()
            ),
            StructuredCorrection::Invalidate => "that memory is wrong".into(),
            StructuredCorrection::Qualify { qualification } => format!(
                "qualify that memory as {}",
                serde_json::to_string(qualification).unwrap()
            ),
        };
        let receipt = OriginReceipt { text: event };
        let parsed = parser::parse_correction_v1(&receipt).map_err(|_| {
            ResolveError::Error(AppError::Authority(DecisionErrorV1::InvalidRequest))
        })?;
        let mut intent = parser::bind_selection(&parsed, &receipt, &context)
            .map_err(|r| ResolveError::Tentative(r, "unresolved_selection"))?;
        if let (
            StructuredCorrection::Rendering {
                approved_variants,
                forbidden_variants,
                ..
            },
            CorrectionIntentV1::Rendering { value, .. },
        ) = (structured, &mut intent)
        {
            if let Some(v) = approved_variants {
                value.approved_variants = v.clone();
            }
            if let Some(v) = forbidden_variants {
                value.forbidden_variants = v.clone();
            }
        }
        (intent, None)
    } else {
        let receipt = OriginReceipt { text: text.into() };
        let parsed = parser::parse_correction_v1(&receipt)
            .map_err(|r| ResolveError::Tentative(r, "unsupported_or_ambiguous_command"))?;
        let intent = parser::bind_selection(&parsed, &receipt, &context)
            .map_err(|r| ResolveError::Tentative(r, "unresolved_selection"))?;
        (
            intent,
            Some(serde_json::to_value(parsed).map_err(|e| ResolveError::Error(domain(e)))?),
        )
    };
    if matches!(intent, CorrectionIntentV1::Rendering { .. })
        && input.target_language.as_ref().is_none_or(|l| l.is_empty())
    {
        return uncertain(
            TentativeReason::AmbiguousIdentity,
            "missing_target_language",
        );
    }
    Ok((
        DecisionDraftV1 {
            version: input.version,
            decision_id: input.decision_id.clone(),
            expected_revision: input.expected_revision,
            evidence_refs: input.selected_sources.clone(),
            series_id: input.series_id,
            concept_id,
            source_language,
            target_language: input.target_language.clone(),
            applicability,
            operation: OperationV1::Correct { intent },
            receipt_ref: None,
            session_id: input.session_id,
        },
        parsed,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectionRequestV1 {
    series_id: i64,
    target: Option<hieronymus::claim_reads::ClaimTarget>,
    source_evidence_id: Option<i64>,
    rule_id: Option<i64>,
}
/// Consistent source inspection for a dedicated correction form. No choice or mint occurs.
pub(crate) fn user_selection(app: &Application, args: &Value) -> Result<Value, AppError> {
    let input: SelectionRequestV1 = decode(args)?;
    if input.series_id <= 0 {
        return Err(AppError::Authority(DecisionErrorV1::InvalidRequest));
    }
    let mut db = connection(app)?;
    let tx = db.transaction().map_err(domain)?;
    let (default_source, default_target): (String, String) = tx
        .query_row(
            "select default_source_language,default_target_language from series where id=?",
            [input.series_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(domain)?;
    let revision: i64 = tx
        .query_row(
            "select revision from authority_state where series_id=?",
            [input.series_id],
            |r| r.get(0),
        )
        .map_err(domain)?;
    let mut languages: Option<(String, Option<String>)> = None;
    if let Some(target) = input.target {
        use hieronymus::claim_reads::ClaimTarget;
        let pair = match target {
            ClaimTarget::ShortTerm(id) => Some(tx.query_row("select t.source_language,t.target_language from short_term_memories m join task_sessions t on t.id=m.session_id where m.id=?",[id],|r|Ok((r.get::<_,String>(0)?,Some(r.get::<_,String>(1)?)))).map_err(domain)?),
            ClaimTarget::Crystal(id) => Some(tx.query_row("select source_language,target_language from crystals where id=?",[id],|r|Ok((r.get::<_,String>(0)?,Some(r.get::<_,String>(1)?)))).map_err(domain)?),
            ClaimTarget::Facet(id) => Some(tx.query_row("select language from concept_facets where id=?",[id],|r|Ok((r.get::<_,String>(0)?,None))).map_err(domain)?),
            ClaimTarget::RagChunk(_) => None, // No language pair is stored on a RAG claim target.
        };
        if let Some((source, target)) = pair {
            bind_selection_languages(&mut languages, source, target)?;
        }
    }
    let claims = if let Some(target) = input.target {
        let query = hieronymus::story_applicability::StoryQueryV1 {
            series_id: input.series_id,
            timeline_id: None,
            position_id: None,
            viewpoint: hieronymus::story_applicability::Viewpoint::Unspecified,
            scope_predicates: vec![],
            mode: hieronymus::story_applicability::QueryMode::Current,
        };
        let annotation = hieronymus::claim_reads::read_annotation(&tx, target, &query)?;
        if annotation
            .claims
            .iter()
            .any(|c| c.applicability.series_id != input.series_id)
        {
            return Err(AppError::Authority(DecisionErrorV1::OriginMismatch));
        }
        let mut claims = serde_json::to_value(annotation.claims).map_err(domain)?;
        for claim in claims.as_array_mut().expect("serialized claim list") {
            let id = claim["claim_id"].as_i64().expect("typed claim id");
            let text: String = tx
                .query_row("select text from memory_claims where id=?", [id], |r| {
                    r.get(0)
                })
                .map_err(domain)?;
            claim["text"] = json!(text);
        }
        claims
    } else {
        json!([])
    };
    let source = if let Some(id) = input.source_evidence_id {
        let (identity,digest,start,end,content,binding):(String,String,i64,i64,String,String)=tx.query_row("select source_identity,source_hash,span_start,span_end,content,binding_json from evidence_records where id=? and series_id=? and kind='source_passage'",params![id,input.series_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).map_err(domain)?;
        if digest != hash(&content) {
            return Err(AppError::Authority(DecisionErrorV1::EvidenceMismatch));
        }
        let selected = content
            .get(start as usize..end as usize)
            .ok_or(DecisionErrorV1::EvidenceMismatch)?;
        let binding: EvidenceBindingV1 = serde_json::from_str(&binding).map_err(domain)?;
        json!({"reference":{"kind":"source_passage","id":id,"content_hash":digest,"span_start":start,"span_end":end},"source_identity":identity,"selected_text":selected,"binding":binding})
    } else {
        Value::Null
    };
    // A selected source owns its language pair; defaults are only a fallback
    // for context without bound languages, never a replacement for unknown data.
    let source_binding: Option<EvidenceBindingV1> = if source.is_null() {
        None
    } else {
        Some(serde_json::from_value(source["binding"].clone()).map_err(domain)?)
    };
    if let Some(binding) = &source_binding {
        if binding.target_language.is_none() {
            return Err(AppError::Authority(DecisionErrorV1::LanguageMismatch));
        }
        bind_selection_languages(
            &mut languages,
            binding.source_language.clone(),
            binding.target_language.clone(),
        )?;
    }
    let rule = if let Some(id) = input.rule_id {
        let row:Option<(i64,Option<i64>,String,String,String)>=tx.query_row("select revision,concept_id,canonical_translation,source_language,target_language from term_rules where id=?",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(domain)?;
        let (revision, concept_id, canonical, sl, tl) =
            row.ok_or(DecisionErrorV1::UnknownTarget)?;
        bind_selection_languages(&mut languages, sl, Some(tl))?;
        if let Some(concept) = concept_id {
            let valid:bool=tx.query_row("select exists(select 1 from concepts where id=?1 and (scope_type='global' or (scope_type='series' and scope_key=(select 'series:'||slug from series where id=?2))))",params![concept,input.series_id],|r|r.get(0)).map_err(domain)?;
            if !valid {
                return Err(AppError::Authority(DecisionErrorV1::OriginMismatch));
            }
        }
        if let Some(binding) = &source_binding
            && hieronymus::authority::rule_eligibility_at_source(&tx, id, binding)?
                != hieronymus::story_applicability::Eligibility::Current
        {
            return Err(AppError::Authority(DecisionErrorV1::ApplicabilityConflict));
        }
        json!({"id":id,"revision":revision,"concept_id":concept_id,"canonical":canonical})
    } else {
        Value::Null
    };
    let (source_language, target_language) =
        languages.unwrap_or((default_source, Some(default_target)));
    tx.commit().map_err(domain)?;
    Ok(
        json!({"series_id":input.series_id,"expected_revision":revision,"source_language":source_language,"target_language":target_language,"claims":claims,"source":source,"rule":rule,"source_inspection":true}),
    )
}

fn bind_selection_languages(
    pair: &mut Option<(String, Option<String>)>,
    source: String,
    target: Option<String>,
) -> Result<(), AppError> {
    for language in std::iter::once(&source).chain(target.iter()) {
        if language.is_empty() || *language != language.trim().to_lowercase() {
            return Err(AppError::Authority(DecisionErrorV1::LanguageMismatch));
        }
    }
    if let Some((old_source, old_target)) = pair {
        if *old_source != source
            || old_target
                .as_ref()
                .zip(target.as_ref())
                .is_some_and(|(a, b)| a != b)
        {
            return Err(AppError::Authority(DecisionErrorV1::LanguageMismatch));
        }
        if old_target.is_none() {
            *old_target = target;
        }
    } else {
        *pair = Some((source, target));
    }
    Ok(())
}
