//! Atomic completion of worker-bound prepared results. No provider or enqueue.
use crate::{
    authority::{self, AuditOwner},
    authority_models::DecisionErrorV1,
    consolidation::{self, *},
};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{OptionalExtension, Transaction, params};

fn invariant(message: &str) -> ConsolidationError {
    ConsolidationError::Invariant(message.into())
}
fn policy(error: DecisionErrorV1) -> ConsolidationError {
    match error {
        DecisionErrorV1::RevisionConflict { .. } => ConsolidationError::RevisionConflict,
        error => ConsolidationError::Policy(error),
    }
}
/// The caller owns commit. Errors roll back all this function's writes; stale
/// returns an explicit successful transition that the caller must commit.
/// Completed results replay the stored receipt independently of expired leases.
pub fn finish_result_tx(
    tx: &Transaction<'_>,
    result_id: &str,
    token: &str,
    now: DateTime<Utc>,
) -> Result<CompletionOutcome, ConsolidationError> {
    tx.execute_batch("SAVEPOINT consolidation_completion")?;
    let outcome = finish(tx, result_id, token, now);
    if outcome.is_err() {
        tx.execute_batch("ROLLBACK TO consolidation_completion")?;
    }
    tx.execute_batch("RELEASE consolidation_completion")?;
    outcome
}
fn finish(
    tx: &Transaction<'_>,
    result_id: &str,
    token: &str,
    now: DateTime<Utc>,
) -> Result<CompletionOutcome, ConsolidationError> {
    type Row = (
        String,
        String,
        i64,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
        i64,
    );
    let row: Option<Row> = tx.query_row(
        "select r.state,r.job_decision_id,r.generation,r.expected_revision,r.canonical_output,r.origin_id,r.completion_receipt,d.series_id from consolidation_results r join decision_records d on d.decision_id=r.job_decision_id where r.result_id=?",
        [result_id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional()?;
    let (state, job, generation, revision, canonical, origin, receipt, series) =
        row.ok_or_else(|| invariant("unknown result"))?;
    if state == "complete" {
        let receipt =
            serde_json::from_str(&receipt.ok_or_else(|| invariant("missing completion receipt"))?)
                .map_err(|_| invariant("corrupt completion receipt"))?;
        return Ok(CompletionOutcome::Complete { receipt });
    }
    let (leased_job, attempts) = consolidation::current_lease(tx, token, now)?;
    let current_generation: i64 = tx.query_row(
        "select result_generation from consolidation_jobs where decision_id=?",
        [&job],
        |r| r.get(0),
    )?;
    if state != "prepared" || job != leased_job || current_generation != generation {
        return Err(ConsolidationError::ExpiredLease);
    }
    let canonical = canonical.ok_or_else(|| invariant("missing prepared bytes"))?;
    let output: ConsolidationResultV1 =
        serde_json::from_str(&canonical).map_err(|_| invariant("corrupt prepared protocol"))?;
    let normalized = serde_json::to_value(&output)
        .map_err(|_| invariant("invalid canonical protocol"))?
        .to_string();
    if normalized != canonical
        || output.version != 1
        || output.result_id != result_id
        || output.job_decision_id != job
        || output.generation != generation as u64
        || Some(output.expected_revision as i64) != revision
        || output.expected_revision >= i64::MAX as u64
        || Some(&output.origin.0) != origin.as_ref()
        || output.mutations.len() > 100
        || output.selected_claims.len() > 200
        || output.evidence_refs.len() > 512
    {
        return Err(invariant("prepared result binding mismatch"));
    }
    let origin: Option<(String, String, String, String)> = tx
        .query_row(
            "select kind,text,context_json,content_hash from origin_receipts where id=?",
            [&output.origin.0],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let (kind, text, context, hash) = origin.ok_or_else(|| invariant("missing result origin"))?;
    if kind != "dream"
        || context != canonical
        || crate::authority_evidence::hash(&format!("{text}\n{context}")) != hash
    {
        return Err(invariant("result origin mismatch"));
    }
    validate_evidence(tx, &output, series)?;
    let current_revision: i64 = tx.query_row(
        "select revision from authority_state where series_id=?",
        [series],
        |r| r.get(0),
    )?;
    if current_revision != revision.expect("checked revision") {
        return stale(tx, &output, attempts, now);
    }
    let (lineage, mutations) = match validate_effects(tx, &output, series) {
        Err(ConsolidationError::RevisionConflict) => return stale(tx, &output, attempts, now),
        result => result?,
    };
    let rules =
        authority::apply_mutations_tx(tx, &mutations, AuditOwner::ConsolidationResult(result_id))
            .map_err(policy)?;
    let claims = apply_lineage(tx, &output, &lineage, now)?;
    let changed = !rules.is_empty() || !claims.is_empty();
    let resulting_revision = output.expected_revision + u64::from(changed);
    if changed {
        crate::rag::record_corpus_change(tx)?;
        tx.execute(
            "update authority_state set revision=?2 where series_id=?1",
            params![series, resulting_revision as i64],
        )?;
    }
    let receipt = CompletionReceiptV1 {
        result_id: result_id.into(),
        job_decision_id: job.clone(),
        generation: output.generation,
        resulting_revision,
        affected_rules: rules,
        affected_claims: claims,
        committed_at: consolidation::timestamp(now),
    };
    tx.execute("update consolidation_results set state='complete',completion_receipt=?2,updated_at=?3 where result_id=?1",params![result_id,serde_json::to_string(&receipt).map_err(|_|invariant("receipt serialization"))?,consolidation::timestamp(now)])?;
    tx.execute("update consolidation_jobs set state='complete',lease_token=null,lease_until=null,next_attempt_at=null,last_error_code=null,updated_at=?2 where decision_id=?1",params![job,consolidation::timestamp(now)])?;
    Ok(CompletionOutcome::Complete { receipt })
}
/// Non-mutating policy validation before a provider draft becomes durable.
/// The trusted origin must already be staged in the caller's transaction.
pub(crate) fn validate_draft_tx(
    tx: &Transaction<'_>,
    output: &ConsolidationResultV1,
    series: i64,
) -> Result<(), ConsolidationError> {
    validate_evidence(tx, output, series)?;
    let revision: i64 = tx.query_row(
        "select revision from authority_state where series_id=?",
        [series],
        |r| r.get(0),
    )?;
    if revision as u64 != output.expected_revision {
        return Err(ConsolidationError::RevisionConflict);
    }
    validate_effects(tx, output, series).map(|_| ())
}
fn validate_effects(
    tx: &Transaction<'_>,
    output: &ConsolidationResultV1,
    series: i64,
) -> Result<(LineagePlan, Vec<authority::ValidatedMutation>), ConsolidationError> {
    let lineage = validate_lineage(tx, output, series)?;
    let mut mutations = vec![];
    for (index, mutation) in output.mutations.iter().enumerate() {
        if matches!(mutation, DerivedMutationV1::LearnedRule { .. }) {
            mutations.push(
                authority::validate_result_mutation(tx, output, series, index).map_err(policy)?,
            );
        }
    }
    validate_rule_batch(tx, output, &mutations)?;
    Ok((lineage, mutations))
}
fn stale(
    tx: &Transaction<'_>,
    output: &ConsolidationResultV1,
    attempts: i64,
    now: DateTime<Utc>,
) -> Result<CompletionOutcome, ConsolidationError> {
    let next = output
        .generation
        .checked_add(1)
        .filter(|n| *n < i64::MAX as u64)
        .ok_or_else(|| invariant("generation overflow"))?;
    let delay = if attempts >= 6 {
        21600
    } else {
        [30, 120, 600, 3600, 21600][attempts.saturating_sub(1) as usize]
    };
    tx.execute(
        "update consolidation_results set state='stale',updated_at=?2 where result_id=?1",
        params![output.result_id, consolidation::timestamp(now)],
    )?;
    tx.execute("update consolidation_jobs set state=?2,result_generation=?3,lease_token=null,lease_until=null,next_attempt_at=?4,last_error_code='revision_conflict',updated_at=?5 where decision_id=?1",params![output.job_decision_id,if attempts>=6 {"degraded"} else {"retry"},next as i64,consolidation::timestamp(now+Duration::seconds(delay)),consolidation::timestamp(now)])?;
    Ok(CompletionOutcome::Stale {
        result_id: output.result_id.clone(),
        next_generation: next,
    })
}
fn validate_evidence(
    tx: &Transaction<'_>,
    output: &ConsolidationResultV1,
    series: i64,
) -> Result<(), ConsolidationError> {
    let mut ids = std::collections::HashSet::new();
    for evidence in &output.evidence_refs {
        let row:Option<(i64,String,String,i64,i64,String)>=tx.query_row("select series_id,kind,source_hash,span_start,span_end,content from evidence_records where id=?",[evidence.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
        let (owner, kind, hash, start, end, content) =
            row.ok_or_else(|| invariant("unknown selected evidence"))?;
        if !ids.insert(evidence.id)
            || owner != series
            || kind != evidence.kind.as_str()
            || hash != evidence.content_hash
            || hash != crate::authority_evidence::hash(&content)
            || usize::try_from(start).ok() != Some(evidence.span_start)
            || usize::try_from(end).ok() != Some(evidence.span_end)
            || start >= end
            || content
                .get(evidence.span_start..evidence.span_end)
                .is_none()
        {
            return Err(invariant("selected evidence mismatch"));
        }
    }
    Ok(())
}
fn validate_rule_batch(
    tx: &Transaction<'_>,
    output: &ConsolidationResultV1,
    validated: &[authority::ValidatedMutation],
) -> Result<(), ConsolidationError> {
    let mut targets = std::collections::HashSet::new();
    let mut prior = vec![];
    let operations = output
        .mutations
        .iter()
        .filter(|m| matches!(m, DerivedMutationV1::LearnedRule { .. }));
    for (mutation, checked) in operations.zip(validated) {
        let DerivedMutationV1::LearnedRule {
            concept_id,
            source_language,
            target_language,
            operation,
            ..
        } = mutation
        else {
            unreachable!()
        };
        let id = match operation.as_ref() {
            LearnedRuleOperationV1::Activate { candidate_id, .. } => *candidate_id,
            LearnedRuleOperationV1::Replace { rule_id, .. }
            | LearnedRuleOperationV1::Scope { rule_id, .. }
            | LearnedRuleOperationV1::Archive { rule_id, .. } => *rule_id,
        };
        if !targets.insert(id) {
            return Err(ConsolidationError::Draft(DraftProblem::DuplicateRuleTarget));
        }
        let key = (*concept_id, source_language, target_language);
        let effect = checked.correction_effect();
        for (other_key, other_effect) in &prior {
            let other_effect: &crate::corrections::CorrectionEffect = other_effect;
            if key == *other_key {
                let mut masks = effect.effective_exclusions.clone();
                masks.extend(other_effect.effective_exclusions.clone());
                if crate::authority_applicability::effective_overlap(
                    tx,
                    Some(&other_effect.effective_applicability),
                    &effect.effective_applicability,
                    &masks,
                )
                .map_err(policy)?
                {
                    return Err(ConsolidationError::Draft(
                        DraftProblem::OverlappingRuleOperations,
                    ));
                }
            }
        }
        prior.push((key, effect));
    }
    Ok(())
}

use crate::{claim_capture::ExistingClaimInput, claim_reads::ClaimTarget};
use std::collections::{BTreeMap, BTreeSet};
struct LineagePlan {
    edges: Vec<(i64, i64)>,
    bindings: Vec<(ClaimTarget, ExistingClaimInput)>,
    outputs: BTreeSet<i64>,
}
fn ancestors(graph: &BTreeMap<i64, BTreeSet<i64>>, id: i64) -> BTreeSet<i64> {
    let mut seen = BTreeSet::new();
    let mut todo = vec![id];
    while let Some(node) = todo.pop() {
        for parent in graph.get(&node).into_iter().flatten() {
            if seen.insert(*parent) {
                todo.push(*parent);
            }
        }
    }
    seen
}
fn validate_lineage(
    tx: &Transaction<'_>,
    output: &ConsolidationResultV1,
    series: i64,
) -> Result<LineagePlan, ConsolidationError> {
    let mut selected = BTreeMap::new();
    for selection in &output.selected_claims {
        let evidence = output
            .evidence_refs
            .iter()
            .find(|e| e.id == selection.evidence_id)
            .ok_or_else(|| invariant("unselected claim evidence"))?;
        let row:Option<(i64,i64,String,Option<i64>,i64)>=tx.query_row("select series_id,revision,text,concept_id,applicability_id from memory_claims where id=?",[selection.claim_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        let (owner, revision, text, concept, app) =
            row.ok_or_else(|| invariant("unknown selected claim"))?;
        if owner != series
            || selected.contains_key(&selection.claim_id)
            || selection.revision >= i64::MAX as u64
        {
            return Err(invariant("foreign or duplicate selected claim"));
        }
        if revision as u64 != selection.revision {
            return Err(ConsolidationError::RevisionConflict);
        }
        let (identity, content, binding): (String, String, String) = tx.query_row(
            "select source_identity,content,binding_json from evidence_records where id=?",
            [evidence.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let binding: serde_json::Value =
            serde_json::from_str(&binding).map_err(|_| invariant("corrupt claim capture"))?;
        let applicability = crate::authority_applicability::load(tx, app)
            .map_err(policy)?
            .ok_or_else(|| invariant("missing claim applicability"))?;
        if evidence.kind != crate::authority_models::EvidenceKind::Observation
            || identity != format!("claim:{}", selection.claim_id)
            || content != text
            || binding.get("claim_id").and_then(|v| v.as_i64()) != Some(selection.claim_id)
            || binding.get("applicability")
                != Some(
                    &serde_json::to_value(&applicability)
                        .map_err(|_| invariant("claim applicability encoding"))?,
                )
        {
            return Err(invariant("claim capture binding mismatch"));
        }
        let id = binding
            .get("target_id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| invariant("claim target missing"))?;
        let target = match binding.get("target_kind").and_then(|v| v.as_str()) {
            Some("short_term_id") => ClaimTarget::ShortTerm(id),
            Some("crystal_id") => ClaimTarget::Crystal(id),
            Some("facet_id") => ClaimTarget::Facet(id),
            Some("rag_chunk_id") => ClaimTarget::RagChunk(id),
            _ => return Err(invariant("unknown claim capture target")),
        };
        let mut statement=tx.prepare("select short_term_id,crystal_id,facet_id,rag_chunk_id from claim_bindings where claim_id=? order by id")?;
        let targets = statement
            .query_map([selection.claim_id], |r| {
                Ok(
                    match (
                        r.get::<_, Option<i64>>(0)?,
                        r.get::<_, Option<i64>>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                    ) {
                        (Some(id), None, None, None) => ClaimTarget::ShortTerm(id),
                        (None, Some(id), None, None) => ClaimTarget::Crystal(id),
                        (None, None, Some(id), None) => ClaimTarget::Facet(id),
                        (None, None, None, Some(id)) => ClaimTarget::RagChunk(id),
                        _ => return Err(rusqlite::Error::InvalidQuery),
                    },
                )
            })?
            .collect::<Result<Vec<_>, _>>()?;
        if targets.is_empty() {
            return Err(invariant("selected claim has no live target"));
        }
        for current in &targets {
            crate::claim_capture::validate_target(tx, *current, series, concept, true)
                .map_err(policy)?;
        }
        if !targets.contains(&target)
            && !audited_rebind(tx, selection.claim_id, &text, target, &targets)?
        {
            return Err(invariant("detached capture lacks audited lifecycle proof"));
        }
        selected.insert(selection.claim_id, targets);
    }
    let mut graph: BTreeMap<i64, BTreeSet<i64>> = BTreeMap::new();
    let mut statement=tx.prepare("select d.input_claim_id,d.output_claim_id from claim_derivations d join memory_claims c on c.id=d.output_claim_id where c.series_id=?")?;
    for row in statement.query_map([series], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (input, output) = row?;
        graph.entry(output).or_default().insert(input);
    }
    for id in graph.keys() {
        if ancestors(&graph, *id).contains(id) {
            return Err(invariant("corrupt existing lineage cycle"));
        }
    }
    let mut edges = vec![];
    let mut outputs = BTreeSet::new();
    for mutation in &output.mutations {
        if let DerivedMutationV1::ClaimLineage {
            input_claim_ids,
            output_claim_ids,
        } = mutation
        {
            if input_claim_ids.is_empty()
                || output_claim_ids.is_empty()
                || input_claim_ids.len() > 100
                || output_claim_ids.len() > 100
            {
                return Err(ConsolidationError::Draft(DraftProblem::LineageBounds));
            }
            for id in input_claim_ids.iter().chain(output_claim_ids) {
                if !selected.contains_key(id) {
                    return Err(ConsolidationError::Draft(DraftProblem::UnselectedLineage));
                }
            }
            for input in input_claim_ids {
                for out in output_claim_ids {
                    if input == out {
                        return Err(ConsolidationError::Draft(DraftProblem::SelfLineage));
                    }
                    if graph.entry(*out).or_default().insert(*input) {
                        edges.push((*input, *out));
                        outputs.insert(*out);
                    }
                }
            }
        }
    }
    for id in graph.keys() {
        if ancestors(&graph, *id).contains(id) {
            return Err(ConsolidationError::Draft(DraftProblem::LineageCycle));
        }
    }
    let mut bindings = vec![];
    for out in &outputs {
        for target in &selected[out] {
            let target = *target;
            for input in ancestors(&graph, *out) {
                let (owner, concept, app): (i64, Option<i64>, i64) = tx.query_row(
                    "select series_id,concept_id,applicability_id from memory_claims where id=?",
                    [input],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?;
                if owner != series {
                    return Err(invariant("foreign lineage ancestor"));
                }
                crate::claim_capture::validate_target(tx, target, series, concept, true)
                    .map_err(policy)?;
                bindings.push((
                    target,
                    ExistingClaimInput {
                        claim_id: input,
                        concept_id: concept,
                        applicability: crate::authority_applicability::load(tx, app)
                            .map_err(policy)?
                            .ok_or_else(|| invariant("missing ancestor applicability"))?,
                    },
                ));
            }
        }
    }
    Ok(LineagePlan {
        edges,
        bindings,
        outputs,
    })
}
fn apply_lineage(
    tx: &Transaction<'_>,
    output: &ConsolidationResultV1,
    plan: &LineagePlan,
    now: DateTime<Utc>,
) -> Result<Vec<(i64, u64)>, ConsolidationError> {
    for (target, input) in &plan.bindings {
        crate::claim_capture::bind_existing_claim_tx(tx, *target, input).map_err(policy)?;
    }
    for (input, out) in &plan.edges {
        tx.execute("insert into claim_derivations(input_claim_id,output_claim_id,consolidation_result_id) values(?1,?2,?3)",params![input,out,output.result_id])?;
    }
    let mut claims = vec![];
    for id in &plan.outputs {
        tx.execute(
            "update memory_claims set revision=revision+1,updated_at=?2 where id=?1",
            params![id, consolidation::timestamp(now)],
        )?;
        let revision: i64 =
            tx.query_row("select revision from memory_claims where id=?", [id], |r| {
                r.get(0)
            })?;
        claims.push((*id, revision as u64));
    }
    Ok(claims)
}

fn audited_rebind(
    tx: &Transaction<'_>,
    claim: i64,
    text: &str,
    original: ClaimTarget,
    targets: &[ClaimTarget],
) -> Result<bool, ConsolidationError> {
    let mut statement=tx.prepare("select content,source_hash,binding_json from evidence_records where kind='observation' and json_extract(binding_json,'$.claim_id')=? order by id")?;
    let rows = statement
        .query_map([claim], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut reachable = vec![original];
    for (content, hash, json) in rows {
        if content != text || crate::authority_evidence::hash(&content) != hash {
            continue;
        }
        let binding: serde_json::Value =
            serde_json::from_str(&json).map_err(|_| invariant("corrupt lifecycle evidence"))?;
        let parse = |key: &str| -> Option<ClaimTarget> {
            let v = binding.get(key)?;
            let id = v.get("id")?.as_i64()?;
            match v.get("kind")?.as_str()? {
                "short_term_id" => Some(ClaimTarget::ShortTerm(id)),
                "crystal_id" => Some(ClaimTarget::Crystal(id)),
                "facet_id" => Some(ClaimTarget::Facet(id)),
                "rag_chunk_id" => Some(ClaimTarget::RagChunk(id)),
                _ => None,
            }
        };
        let (Some(from), Some(to)) = (parse("from"), parse("to")) else {
            continue;
        };
        let event = binding.get("event").and_then(|v| v.as_str());
        if event == Some("rag_rebind") {
            let Some(detach) = binding
                .get("prior_detach_evidence_id")
                .and_then(|v| v.as_i64())
            else {
                continue;
            };
            let valid:bool=tx.query_row("select exists(select 1 from evidence_records where id=?1 and json_extract(binding_json,'$.event')='rag_replace_detach' and json_extract(binding_json,'$.claim_id')=?2 and json_extract(binding_json,'$.from.id')=?3 and source_hash=?4 and content=?5)",params![detach,claim,from.id(),hash,text],|r|r.get(0))?;
            if !valid {
                continue;
            }
        }
        if matches!(event, Some("copy" | "rag_rebind"))
            && reachable.contains(&from)
            && !reachable.contains(&to)
        {
            reachable.push(to);
        }
        // Explicit lineage is already an audited exact-identity assertion made
        // by the trusted domain binder, including when the source was deleted.
        if event == Some("explicit_lineage") && from == to && !reachable.contains(&to) {
            reachable.push(to);
        }
    }
    Ok(targets.iter().any(|target| reachable.contains(target)))
}
