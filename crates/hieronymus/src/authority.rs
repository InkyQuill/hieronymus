//! Trusted, transactional terminology decisions. No provider calls occur here.
pub use crate::authority_evidence::EvidenceBindingV1;
use crate::story_applicability::MetadataState;
use crate::{
    authority_applicability as applicability, authority_evidence as evidence,
    authority_models::*,
    terminology::{self, RuleAction, RuleActionRequest, TermRule},
};
use DecisionErrorV1 as Error;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

/// Stored trusted-ingress selection. This is not a transport draft or a mint API.
/// Ingress must verify text/structured fields before writing immutable receipts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginContextV1 {
    pub decision_id: String,
    pub expected_revision: u64,
    pub selected_source: Option<EvidenceRef>,
    pub series_id: i64,
    pub concept_id: Option<i64>,
    pub source_language: String,
    pub target_language: Option<String>,
    pub applicability: crate::story_applicability::ApplicabilityV1,
    pub evidence_ids: Vec<i64>,
    pub operation: OperationV1,
}
pub struct DecisionStore<'a> {
    db: &'a mut Connection,
}
impl<'a> DecisionStore<'a> {
    pub fn new(db: &'a mut Connection) -> Self {
        Self { db }
    }
    pub fn apply(&mut self, request: &DecisionRequestV1) -> Result<DecisionResultV1, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = apply_decision_tx(&tx, request)?;
        tx.commit()?;
        Ok(result)
    }
}
fn uuid(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
}
fn canonical(r: &DecisionRequestV1) -> Result<String, Error> {
    Ok(serde_json::to_value(r)?.to_string())
}
fn actor(a: ActorKind) -> &'static str {
    match a {
        ActorKind::Agent => "agent",
        ActorKind::Dream => "dream",
        ActorKind::ExplicitUser => "explicit_user",
    }
}
fn origin(db: &Connection, r: &DecisionRequestV1) -> Result<OriginContextV1, Error> {
    let row: Option<(String, String, String, String)> = db
        .query_row(
            "select kind,text,context_json,content_hash from origin_receipts where id=?",
            [&r.origin.0],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let (kind, text, context, h) = row.ok_or(Error::UnverifiedOrigin)?;
    if evidence::hash(&format!("{text}\n{context}")) != h {
        return Err(Error::OriginMismatch);
    }
    if !matches!(
        (r.actor_kind, kind.as_str()),
        (ActorKind::ExplicitUser, "console_user" | "host_user_event")
            | (ActorKind::Agent, "agent")
            | (ActorKind::Dream, "dream")
    ) {
        return Err(Error::UnverifiedOrigin);
    }
    let context: OriginContextV1 =
        serde_json::from_str(&context).map_err(|_| Error::OriginMismatch)?;
    if context.decision_id != r.decision_id
        || context.expected_revision != r.expected_revision
        || context.series_id != r.series_id
        || context.concept_id != r.concept_id
        || context.source_language != r.source_language
        || context.target_language != r.target_language
        || context.applicability != r.applicability
        || context.operation != r.operation
        || r.evidence_refs
            .iter()
            .any(|e| !context.evidence_ids.contains(&e.id))
    {
        return Err(Error::OriginMismatch);
    }
    if r.actor_kind == ActorKind::ExplicitUser
        && let Some(selected) = &context.selected_source
    {
        if selected.kind != EvidenceKind::SourcePassage || !r.evidence_refs.contains(selected) {
            return Err(Error::OriginMismatch);
        }
        let content: String = db.query_row(
            "select content from evidence_records where id=?",
            [selected.id],
            |row| row.get(0),
        )?;
        let source = content
            .get(selected.span_start..selected.span_end)
            .ok_or(Error::EvidenceMismatch)?;
        let forms = match &r.operation {
            OperationV1::Replace { rendering, .. }
            | OperationV1::Correct {
                intent:
                    CorrectionIntentV1::Rendering {
                        value: rendering, ..
                    },
            } => Some(&rendering.source_forms),
            _ => None,
        };
        if forms.is_some_and(|forms| !forms.iter().any(|form| form == source)) {
            return Err(Error::OriginMismatch);
        }
    }
    Ok(context)
}
fn target(op: &OperationV1) -> Option<(i64, u64)> {
    match op {
        OperationV1::Activate {
            candidate_id,
            candidate_revision,
        } => Some((*candidate_id, *candidate_revision)),
        OperationV1::Replace {
            rule_id,
            rule_revision,
            ..
        }
        | OperationV1::Scope {
            rule_id,
            rule_revision,
            ..
        }
        | OperationV1::Archive {
            rule_id,
            rule_revision,
        } => Some((*rule_id, *rule_revision)),
        OperationV1::Correct {
            intent: CorrectionIntentV1::Rendering { replaces, .. },
        } => *replaces,
        _ => None,
    }
}
fn term_error(e: terminology::TermbaseError) -> Error {
    match e {
        terminology::TermbaseError::UnknownRule(_) => Error::UnknownTarget,
        terminology::TermbaseError::RevisionConflict {
            current_revision, ..
        } => Error::RevisionConflict {
            current_revision: current_revision as u64,
        },
        terminology::TermbaseError::Database(_)
        | terminology::TermbaseError::ProjectionFailure { .. } => Error::StorageUnavailable,
        _ => Error::InvalidRequest,
    }
}
fn rendering(db: &Connection, rule: &TermRule) -> Result<RenderingV1, Error> {
    let mut s = db.prepare(
        "select form_kind,surface,case_sensitive from term_rule_forms where rule_id=? order by id",
    )?;
    let forms = s
        .query_map([rule.id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, bool>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RenderingV1 {
        source_forms: forms
            .iter()
            .filter(|(k, _, _)| k == "source")
            .map(|(_, s, _)| s.clone())
            .collect(),
        canonical: rule.canonical_translation.clone(),
        approved_variants: forms
            .iter()
            .filter(|(k, s, _)| k == "approved" && s != &rule.canonical_translation)
            .map(|(_, s, _)| s.clone())
            .collect(),
        forbidden_variants: rule.forbidden_variants.clone(),
        case_sensitive: forms.iter().any(|(_, _, c)| *c),
    })
}

fn validate_rendering(v: &RenderingV1) -> Result<(), Error> {
    if v.source_forms.is_empty()
        || v.source_forms
            .iter()
            .chain(std::iter::once(&v.canonical))
            .chain(&v.approved_variants)
            .chain(&v.forbidden_variants)
            .any(|s| s.trim().is_empty() || s.chars().any(char::is_control))
        || v.forbidden_variants
            .iter()
            .any(|s| s == &v.canonical || v.approved_variants.contains(s))
    {
        return Err(Error::InvalidRequest);
    }
    Ok(())
}
/// The opaque checked mutation cannot be constructed by a transport.
pub(crate) struct ValidatedMutation {
    request: DecisionRequestV1,
    value: RenderingV1,
    old: Option<TermRule>,
    exclusions: Vec<crate::story_applicability::ApplicabilityV1>,
}
#[derive(Clone, Copy)]
pub(crate) enum AuditOwner<'a> {
    Decision(&'a str),
    #[allow(dead_code)] // Task 5 owns the completion wrapper.
    ConsolidationResult(&'a str),
}
impl<'a> AuditOwner<'a> {
    fn fields(self) -> (Option<&'a str>, Option<&'a str>) {
        match self {
            Self::Decision(id) => (Some(id), None),
            Self::ConsolidationResult(id) => (None, Some(id)),
        }
    }
    fn key(self) -> String {
        let (d, r) = self.fields();
        format!(
            "{}:{}",
            if d.is_some() { "decision" } else { "result" },
            d.or(r).expect("owner")
        )
    }
}

pub(crate) fn validate_request(
    db: &Connection,
    r: &DecisionRequestV1,
) -> Result<OriginContextV1, Error> {
    if r.version != 1 {
        return Err(Error::UnsupportedVersion);
    }
    if !uuid(&r.decision_id)
        || !uuid(&r.origin.0)
        || r.series_id <= 0
        || r.expected_revision >= i64::MAX as u64
        || r.concept_id.is_some_and(|id| id <= 0)
        || r.evidence_refs.iter().any(|e| e.id <= 0)
    {
        return Err(Error::InvalidRequest);
    }
    let origin_context = origin(db, r)?;
    if r.applicability.series_id != r.series_id {
        return Err(Error::ApplicabilityConflict);
    }
    applicability::validate(db, &r.applicability)?;
    let languages: Option<(String, String)> = db
        .query_row(
            "select default_source_language,default_target_language from series where id=?",
            [r.series_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (source, target_language) = languages.ok_or(Error::UnknownTarget)?;
    for language in std::iter::once(&r.source_language).chain(r.target_language.iter()) {
        let registered:bool=db.query_row("select exists(select 1 from series_language_tags where series_id=?1 and language_tag=?2)",params![r.series_id,language],|row|row.get(0))?;
        if language.is_empty()
            || language != &language.trim().to_lowercase()
            || !(registered
                || language == &source.trim().to_lowercase()
                || language == &target_language.trim().to_lowercase())
        {
            return Err(Error::LanguageMismatch);
        }
    }
    if let Some(concept) = r.concept_id {
        let valid:bool=db.query_row("select exists(select 1 from concepts c join series s on s.id=?2 where c.id=?1 and (c.scope_type='global' or (c.scope_type='series' and c.scope_key='series:'||s.slug)))",params![concept,r.series_id],|r|r.get(0))?;
        if !valid {
            return Err(Error::UnknownTarget);
        }
    }
    let revision: Option<i64> = db
        .query_row(
            "select revision from authority_state where series_id=?",
            [r.series_id],
            |r| r.get(0),
        )
        .optional()?;
    let revision = revision.unwrap_or(0) as u64;
    if revision != r.expected_revision {
        return Err(Error::RevisionConflict {
            current_revision: revision,
        });
    }
    Ok(origin_context)
}

pub(crate) fn validate_mutation(
    db: &Connection,
    r: &DecisionRequestV1,
) -> Result<(ValidatedMutation, Vec<TentativeReason>), Error> {
    let origin_context = validate_request(db, r)?;
    if r.target_language.is_none() {
        return Err(Error::LanguageMismatch);
    }
    let mut reasons = vec![];
    if r.concept_id.is_none()
        || (r.actor_kind == ActorKind::ExplicitUser && origin_context.selected_source.is_none())
    {
        reasons.push(TentativeReason::AmbiguousIdentity);
    }
    let resolved = evidence::resolve_all(db, r)?;
    let old = if let Some((id, revision)) = target(&r.operation) {
        let rule = terminology::hydrate_rule(db, id).map_err(term_error)?;
        if !(0..i64::MAX).contains(&rule.revision) {
            return Err(Error::InvalidRequest);
        }
        if rule.revision as u64 != revision {
            return Err(Error::RevisionConflict {
                current_revision: rule.revision as u64,
            });
        }
        if r.concept_id.is_some() && rule.concept_id != r.concept_id {
            return Err(Error::EvidenceMismatch);
        }
        if rule.source_language != r.source_language
            || Some(&rule.target_language) != r.target_language.as_ref()
        {
            return Err(Error::LanguageMismatch);
        }
        Some(rule)
    } else {
        None
    };
    let value = match &r.operation {
        OperationV1::Replace { rendering, .. }
        | OperationV1::Correct {
            intent:
                CorrectionIntentV1::Rendering {
                    value: rendering, ..
                },
        } => rendering.clone(),
        OperationV1::Activate { .. } | OperationV1::Scope { .. } | OperationV1::Archive { .. } => {
            rendering(db, old.as_ref().ok_or(Error::UnknownTarget)?)?
        }
        _ => return Err(Error::InvalidRequest), // factual/relevance lanes belong to Task 4
    };
    validate_rendering(&value)?;
    if let Some(old) = &old {
        match r.operation {
            OperationV1::Activate { .. } if old.status != "candidate" => {
                return Err(Error::InvalidRequest);
            }
            OperationV1::Activate { .. } => {}
            _ if old.status != "active" => return Err(Error::InvalidRequest),
            _ => {}
        }
    }
    if r.applicability.metadata_state != MetadataState::Resolved
        || r.applicability.timeline_id.is_none()
        || r.applicability.knowledge_gates.is_empty()
    {
        reasons.push(TentativeReason::UnknownOrder)
    }
    if let OperationV1::Scope {
        new_applicability, ..
    } = &r.operation
    {
        applicability::validate(db, new_applicability)?;
        if new_applicability != &r.applicability {
            return Err(Error::ApplicabilityConflict);
        }
    }
    let mut effective = r.clone();
    let mut exclusions = vec![];
    if let Some(old) = &old
        && !matches!(
            r.operation,
            OperationV1::Activate { .. } | OperationV1::Scope { .. }
        )
    {
        let stored: Option<i64> = db
            .query_row(
                "select applicability_id from rule_authority where rule_id=?",
                [old.id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(stored) = stored
            && let Some(old_app) = applicability::load(db, stored)?
        {
            effective.applicability = applicability::intersection(db, &old_app, &r.applicability)?
                .ok_or(Error::ApplicabilityConflict)?;
        }
    }
    if let Some(old) = &old
        && !matches!(r.operation, OperationV1::Activate { .. })
    {
        for mask in applicability::exclusions(db, old.id)? {
            if let Some(mask) = applicability::intersection(db, &mask, &effective.applicability)? {
                exclusions.push(mask);
            }
        }
        if !applicability::effective_overlap(db, None, &effective.applicability, &exclusions)? {
            return Err(Error::ApplicabilityConflict);
        }
    }
    if effective.applicability.metadata_state == MetadataState::Unspecified
        && !reasons.contains(&TentativeReason::UnknownOrder)
    {
        reasons.push(TentativeReason::UnknownOrder);
    }
    let r = &effective;
    // Hard authority masks are evaluated before any scoring or mutation.
    let mut stmt=db.prepare("select tr.id,coalesce(ra.authority,'explicit_user'),ra.applicability_id from term_rules tr left join rule_authority ra on tr.id=ra.rule_id where tr.status='active' and tr.concept_id=?1 and tr.source_language=?2 and tr.target_language=?3")?;
    let active = stmt
        .query_map(
            params![r.concept_id, r.source_language, r.target_language],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    for (id, authority, a) in active {
        let a = match a {
            Some(id) => applicability::load(db, id)?,
            None => None,
        };
        let mut masks = applicability::exclusions(db, id)?;
        masks.extend(exclusions.clone());
        let overlap = applicability::effective_overlap(db, a.as_ref(), &r.applicability, &masks)?;
        if overlap && authority == "explicit_user" && r.actor_kind != ActorKind::ExplicitUser {
            return Err(Error::AuthorityConflict);
        }
        if overlap && old.as_ref().is_none_or(|old| old.id != id) {
            return Err(Error::AuthorityConflict);
        }
    }
    if r.actor_kind != ActorKind::ExplicitUser && reasons.is_empty() {
        let old_id = if matches!(
            r.operation,
            OperationV1::Activate { .. } | OperationV1::Scope { .. }
        ) {
            None
        } else {
            old.as_ref().map(|o| o.id)
        };
        reasons.extend(evidence::learned_policy(
            db,
            r,
            &resolved,
            &value.canonical,
            old_id,
            &exclusions,
        )?);
    }
    if r.actor_kind == ActorKind::ExplicitUser
        && !resolved
            .iter()
            .any(|e| e.kind == EvidenceKind::SourcePassage && e.binding.identity_anchor)
    {
        reasons.push(TentativeReason::AmbiguousIdentity)
    }
    Ok((
        ValidatedMutation {
            request: r.clone(),
            value,
            old,
            exclusions,
        },
        reasons,
    ))
}
/// Ingestion wrapper: receipt and durable intent belong here, never to the
/// shared mutation primitive. Savepoint also protects caller-owned transactions.
pub fn apply_decision_tx(
    tx: &Transaction<'_>,
    r: &DecisionRequestV1,
) -> Result<DecisionResultV1, Error> {
    tx.execute_batch("SAVEPOINT authority_decision")?;
    let result = ingest(tx, r);
    if result.is_err() {
        tx.execute_batch("ROLLBACK TO authority_decision")?;
    }
    tx.execute_batch("RELEASE authority_decision")?;
    result
}
fn ingest(tx: &Transaction<'_>, r: &DecisionRequestV1) -> Result<DecisionResultV1, Error> {
    let canonical = canonical(r)?;
    let existing:Option<(String,String,String)>=tx.query_row("select canonical_request,origin_id,result_json from decision_records where decision_id=?",[&r.decision_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    if let Some((stored, origin, result)) = existing {
        if stored != canonical || origin != r.origin.0 {
            return Err(Error::IdempotencyConflict);
        }
        let result: DecisionResultV1 =
            serde_json::from_str(&result).map_err(|_| Error::StorageUnavailable)?;
        return Ok(match result {
            DecisionResultV1::Applied { receipt } | DecisionResultV1::Replayed { receipt } => {
                DecisionResultV1::Replayed { receipt }
            }
            tentative => tentative,
        });
    }
    let correction_intent = match &r.operation {
        OperationV1::Correct {
            intent:
                intent @ (CorrectionIntentV1::Fact { .. } | CorrectionIntentV1::Relevance { .. }),
        } => Some(intent),
        _ => None,
    };
    let (mutation, correction, reasons) = if let Some(intent) = correction_intent {
        let (effect, reasons) = crate::corrections::validate_correction(tx, r, intent)?;
        (None, Some(effect), reasons)
    } else {
        let (mutation, reasons) = validate_mutation(tx, r)?;
        (Some(mutation), None, reasons)
    };
    let effective_applicability = mutation
        .as_ref()
        .map(|m| &m.request.applicability)
        .or_else(|| correction.as_ref().map(|e| &e.effective_applicability))
        .expect("validated operation")
        .clone();
    let effective_exclusions = mutation
        .as_ref()
        .map(|m| m.exclusions.clone())
        .unwrap_or_default();
    let now = chrono::Utc::now().to_rfc3339();
    let revision = r.expected_revision + 1;
    // FK ownership exists before mutations; provisional bytes never escape the
    // transaction and are replaced by the exact result before commit.
    tx.execute("insert into decision_records(decision_id,series_id,origin_id,actor_kind,expected_revision,resulting_revision,canonical_request,result_json,status,created_at) values(?1,?2,?3,?4,?5,?6,?7,'{}',?8,?9)",params![r.decision_id,r.series_id,r.origin.0,actor(r.actor_kind),r.expected_revision as i64,revision as i64,canonical,if reasons.is_empty(){"applied"}else{"tentative"},now])?;
    let mut claims = vec![];
    let rules = if reasons.is_empty() {
        if let Some(mutation) = mutation {
            apply_mutations_tx(tx, &[mutation], AuditOwner::Decision(&r.decision_id))?
        } else {
            let effect = crate::corrections::apply_correction_tx(
                tx,
                r,
                correction_intent.expect("correction"),
            )?;
            claims = effect.affected_claims;
            effect.affected_rules
        }
    } else {
        vec![]
    };
    if !rules.is_empty() || !claims.is_empty() {
        crate::rag::record_corpus_change(tx)?;
    }
    for (i, e) in r.evidence_refs.iter().enumerate() {
        tx.execute("insert into decision_evidence(decision_id,ordinal,kind,source_id,hash,span_start,span_end) values(?1,?2,?3,?4,?5,?6,?7)",params![r.decision_id,i as i64,e.kind.as_str(),e.id,e.content_hash,e.span_start as i64,e.span_end as i64])?;
    }
    tx.execute("insert into authority_state(series_id,revision) values(?1,?2) on conflict(series_id) do update set revision=excluded.revision",params![r.series_id,revision as i64])?;
    tx.execute("insert or ignore into provider_recovery_state(provider_slot_id,config_fingerprint,next_recovery_at,updated_at) values('default','unconfigured',?1,?1)",[&now])?;
    tx.execute("insert into consolidation_jobs(decision_id,state,attempts,provider_slot_id,created_at,updated_at) values(?1,'pending',0,'default',?2,?2)",params![r.decision_id,now])?;
    let receipt = DecisionReceiptV1 {
        decision_id: r.decision_id.clone(),
        resulting_revision: revision,
        affected_rules: rules,
        affected_claims: claims,
        effective_applicability,
        effective_exclusions,
        effect: if reasons.is_empty() {
            correction
                .as_ref()
                .map(|e| e.effect.as_str())
                .unwrap_or("terminology")
        } else {
            "tentative"
        }
        .into(),
        consolidation_job_id: r.decision_id.clone(),
        origin: r.origin.clone(),
        committed_at: now,
    };
    let result = if reasons.is_empty() {
        DecisionResultV1::Applied { receipt }
    } else {
        DecisionResultV1::Tentative { receipt, reasons }
    };
    tx.execute(
        "update decision_records set result_json=?2 where decision_id=?1",
        params![r.decision_id, serde_json::to_string(&result)?],
    )?;
    Ok(result)
}
fn create_rule(tx: &Transaction<'_>, m: &ValidatedMutation) -> Result<i64, Error> {
    let r = &m.request;
    let v = &m.value;
    let now = chrono::Utc::now().to_rfc3339();
    tx.execute("insert into term_rules(concept_id,source_language,target_language,source_text,canonical_translation,forbidden_variants_json,status,provenance,created_at,updated_at) values(?1,?2,?3,?4,?5,?6,'candidate','autonomous_authority',?7,?7)",params![r.concept_id,r.source_language,r.target_language,v.source_forms[0],v.canonical,serde_json::to_string(&v.forbidden_variants)?,now])?;
    let id = tx.last_insert_rowid();
    for (kind, forms, language) in [
        ("source", v.source_forms.clone(), &r.source_language),
        (
            "approved",
            std::iter::once(v.canonical.clone())
                .chain(v.approved_variants.clone())
                .collect(),
            r.target_language.as_ref().ok_or(Error::LanguageMismatch)?,
        ),
        (
            "forbidden",
            v.forbidden_variants.clone(),
            r.target_language.as_ref().ok_or(Error::LanguageMismatch)?,
        ),
    ] {
        for form in forms {
            tx.execute("insert or ignore into term_rule_forms(rule_id,form_kind,surface,language,case_sensitive) values(?1,?2,?3,?4,?5)",params![id,kind,form,language,v.case_sensitive])?;
        }
    }
    Ok(id)
}
fn lifecycle(
    tx: &Transaction<'_>,
    id: i64,
    revision: u64,
    action: RuleAction,
    m: &ValidatedMutation,
    owner: AuditOwner<'_>,
) -> Result<TermRule, Error> {
    terminology::apply_lifecycle_tx(
        tx,
        &RuleActionRequest {
            rule_id: id,
            action,
            actor: actor(m.request.actor_kind).into(),
            reason: owner.key(),
            expected_revision: revision as i64,
            idempotency_key: format!("{}:{id}", owner.key()),
        },
        true,
    )
    .map_err(term_error)
}
/// Only policy-checked domain mutations and lifecycle audit; no ingestion,
/// consolidation enqueue, receipt write, authority-state bump or commit.
pub(crate) fn apply_mutations_tx(
    tx: &Transaction<'_>,
    mutations: &[ValidatedMutation],
    owner: AuditOwner<'_>,
) -> Result<Vec<(i64, u64)>, Error> {
    let mut affected = vec![];
    for m in mutations {
        let r = &m.request;
        let app = applicability::store(tx, &r.applicability)?;
        let (decision, result) = owner.fields();
        let old_id = m.old.as_ref().map(|o| o.id);
        let activating = matches!(r.operation, OperationV1::Activate { .. });
        let archiving = matches!(r.operation, OperationV1::Archive { .. });
        let mut full = false;
        if let Some(old) = &m.old
            && !activating
        {
            let old_app: Option<i64> = tx
                .query_row(
                    "select applicability_id from rule_authority where rule_id=?",
                    [old.id],
                    |r| r.get(0),
                )
                .optional()?;
            full = matches!(r.operation, OperationV1::Scope { .. })
                || match old_app {
                    Some(id) => applicability::load(tx, id)?.is_some_and(|a| a == r.applicability),
                    None => false,
                };
            if !full {
                tx.execute("insert into rule_exclusions(rule_id,applicability_id,decision_id,consolidation_result_id) values(?1,?2,?3,?4)",params![old.id,app,decision,result])?;
                tx.execute(
                    "update term_rules set revision=revision+1,updated_at=?2 where id=?1",
                    params![old.id, chrono::Utc::now().to_rfc3339()],
                )?;
                audit_scope(tx, old, m, owner)?;
                affected.push((old.id, old.revision as u64 + 1));
            }
        }
        if archiving {
            if full {
                let old = m.old.as_ref().ok_or(Error::UnknownTarget)?;
                let rule = lifecycle(
                    tx,
                    old.id,
                    old.revision as u64,
                    RuleAction::Archive,
                    m,
                    owner,
                )?;
                affected.push((rule.id, rule.revision as u64));
            }
            continue;
        }
        let id = if activating {
            old_id.ok_or(Error::UnknownTarget)?
        } else {
            create_rule(tx, m)?
        };
        tx.execute("insert into rule_authority(rule_id,authority,origin_id,decision_id,consolidation_result_id,applicability_id,legacy_protected) values(?1,?2,?3,?4,?5,?6,0) on conflict(rule_id) do update set authority=excluded.authority,origin_id=excluded.origin_id,decision_id=excluded.decision_id,consolidation_result_id=excluded.consolidation_result_id,applicability_id=excluded.applicability_id,legacy_protected=0",params![id,if r.actor_kind==ActorKind::ExplicitUser{"explicit_user"}else{"learned"},r.origin.0,decision,result,app])?;
        for mask in &m.exclusions {
            let mask = applicability::store(tx, mask)?;
            tx.execute("insert into rule_exclusions(rule_id,applicability_id,decision_id,consolidation_result_id) values(?1,?2,?3,?4)",params![id,mask,decision,result])?;
        }
        let rule = if full && !activating {
            let old = m.old.as_ref().ok_or(Error::UnknownTarget)?;
            let rule = lifecycle(
                tx,
                old.id,
                old.revision as u64,
                RuleAction::Replace { replacement_id: id },
                m,
                owner,
            )?;
            affected.push((old.id, old.revision as u64 + 1));
            rule
        } else {
            let rev = terminology::hydrate_rule(tx, id)
                .map_err(term_error)?
                .revision;
            lifecycle(tx, id, rev as u64, RuleAction::Approve, m, owner)?
        };
        affected.push((rule.id, rule.revision as u64));
    }
    Ok(affected)
}
fn audit_scope(
    tx: &Transaction<'_>,
    old: &TermRule,
    m: &ValidatedMutation,
    owner: AuditOwner<'_>,
) -> Result<(), Error> {
    let now = chrono::Utc::now().to_rfc3339();
    let canonical = serde_json::json!({"audit_owner":owner.key(),"operation":m.request.operation});
    tx.execute("insert into term_rule_revisions(rule_id,actor,reason,prior_status,new_status,created_at) values(?1,?2,?3,?4,?4,?5)",params![old.id,actor(m.request.actor_kind),owner.key(),old.status,now])?;
    tx.execute("insert into term_rule_actions(rule_id,actor,reason,action,expected_revision,resulting_revision,idempotency_key,request_canonical,result_json,created_at) values(?1,?2,?3,'scope',?4,?5,?6,?7,?8,?9)",params![old.id,actor(m.request.actor_kind),owner.key(),old.revision,old.revision+1,format!("{}:scope:{}",owner.key(),old.id),canonical.to_string(),serde_json::to_string(&terminology::hydrate_rule(tx,old.id).map_err(term_error)?)?,now])?;
    Ok(())
}
