//! SQLite-owned correction scheduling. Callers must obtain provider admission
//! before leasing work that has no prepared result. No provider calls here.
pub use crate::consolidation_completion::finish_result_tx;
pub use crate::consolidation_models::*;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

pub(crate) fn timestamp(now: DateTime<Utc>) -> String {
    now.to_rfc3339_opts(SecondsFormat::Millis, true)
}
pub(crate) fn uuid(db: &Connection) -> Result<String, ConsolidationError> {
    Ok(db.query_row("select lower(hex(randomblob(4)))||'-'||lower(hex(randomblob(2)))||'-4'||substr(lower(hex(randomblob(2))),2)||'-8'||substr(lower(hex(randomblob(2))),2)||'-'||lower(hex(randomblob(6)))", [], |r|r.get(0))?)
}
/// Borrowed store; each public mutation owns a short immediate transaction.
pub struct ConsolidationStore<'a> {
    db: &'a mut Connection,
}
impl<'a> ConsolidationStore<'a> {
    pub fn new(db: &'a mut Connection) -> Self {
        Self { db }
    }

    /// Assign unfinished jobs to the selected catalog entry. Fingerprints are
    /// normalized nonsecret configuration supplied by the trusted worker.
    /// Neither slot deadlines nor parked job deadlines reset on configuration.
    pub fn assign_provider(
        &mut self,
        slot: &str,
        fingerprint: &str,
        now: DateTime<Utc>,
    ) -> Result<(), ConsolidationError> {
        if slot.is_empty() || fingerprint.is_empty() {
            return Err(ConsolidationError::Invariant(
                "empty provider assignment".into(),
            ));
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = timestamp(now);
        tx.execute("insert into provider_recovery_state(provider_slot_id,config_fingerprint,next_recovery_at,updated_at) values(?1,?2,?3,?3) on conflict(provider_slot_id) do update set config_fingerprint=excluded.config_fingerprint,updated_at=excluded.updated_at",params![slot,fingerprint,now])?;
        tx.execute("update consolidation_jobs set provider_slot_id=?1,updated_at=?2 where state in ('pending','retry','degraded') and provider_slot_id != ?1",params![slot,now])?;
        tx.commit()?;
        Ok(())
    }
    /// Reclaim expired leases at startup and every idle scheduler tick. Prepared
    /// results remain prepared and are eligible for provider-free completion.
    pub fn recovery_tick(&mut self, now: DateTime<Utc>) -> Result<usize, ConsolidationError> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let count = recover(&tx, now)?;
        tx.commit()?;
        Ok(count)
    }
    /// Reserve one due job in a series. A degraded call atomically consumes its
    /// stable provider slot's six-hour recovery allowance before external work.
    pub fn lease_next(
        &mut self,
        now: DateTime<Utc>,
        series_id: i64,
    ) -> Result<Option<ConsolidationLease>, ConsolidationError> {
        self.lease(now, series_id, false)
    }
    /// Provider-free recovery path, usable even while Dream admission is busy.
    pub fn lease_prepared(
        &mut self,
        now: DateTime<Utc>,
        series_id: i64,
    ) -> Result<Option<ConsolidationLease>, ConsolidationError> {
        self.lease(now, series_id, true)
    }
    fn lease(
        &mut self,
        now: DateTime<Utc>,
        series_id: i64,
        prepared_only: bool,
    ) -> Result<Option<ConsolidationLease>, ConsolidationError> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        recover(&tx, now)?;
        let t = timestamp(now);
        let running:bool=tx.query_row("select exists(select 1 from consolidation_jobs j join decision_records d on d.decision_id=j.decision_id where d.series_id=?1 and j.state='leased')",[series_id],|r|r.get(0))?;
        if running {
            tx.commit()?;
            return Ok(None);
        }
        // Enforce parked fairness across series, independent of daemon order.
        #[allow(clippy::type_complexity)]
        let job:Option<(String,String,i64,i64,String,i64,Option<String>,Option<String>)>=tx.query_row(
            "select j.decision_id, j.state, j.attempts, j.result_generation,
                    j.provider_slot_id,
                    case when r.state='prepared' then r.expected_revision else a.revision end,
                    r.result_id, r.canonical_output
             from consolidation_jobs j
             join decision_records d on d.decision_id=j.decision_id
             join authority_state a on a.series_id=d.series_id
             join provider_recovery_state p on p.provider_slot_id=j.provider_slot_id
             left join consolidation_results r on r.job_decision_id=j.decision_id
                  and r.generation=j.result_generation
             where d.series_id=?1 and j.state in ('pending','retry','degraded')
               and (?3=0 or r.state='prepared')
               and (r.state='prepared' or (
                 (j.next_attempt_at is null or julianday(j.next_attempt_at)<=julianday(?2))
                 and (j.state!='degraded' or (
                   julianday(p.next_recovery_at)<=julianday(?2)
                   and j.decision_id=(
                     select k.decision_id from consolidation_jobs k
                     join decision_records kd on kd.decision_id=k.decision_id
                     left join consolidation_results kr on kr.job_decision_id=k.decision_id
                          and kr.generation=k.result_generation
                     where k.provider_slot_id=j.provider_slot_id and k.state='degraded'
                       and coalesce(kr.state,'reserved')!='prepared'
                       and (k.next_attempt_at is null or julianday(k.next_attempt_at)<=julianday(?2))
                       and not exists(
                         select 1 from consolidation_jobs busy
                         join decision_records bd on bd.decision_id=busy.decision_id
                         where busy.state='leased' and bd.series_id=kd.series_id)
                     order by julianday(coalesce(k.last_attempt_at,k.created_at)), k.decision_id
                     limit 1)))))
             order by case when r.state='prepared' then 0 else 1 end,
                      julianday(coalesce(j.last_attempt_at,j.created_at)), j.decision_id
             limit 1",
            params![series_id,t,prepared_only],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional()?;
        let Some((
            decision_id,
            state,
            attempts,
            generation,
            slot,
            revision,
            result_id,
            canonical_output,
        )) = job
        else {
            tx.commit()?;
            return Ok(None);
        };
        let prepared = canonical_output.is_some();
        let attempts = attempts
            .checked_add(i64::from(!prepared))
            .ok_or_else(|| ConsolidationError::Invariant("attempt counter overflow".into()))?;
        let expires_at = now + Duration::seconds(120);
        let token = uuid(&tx)?;
        let result_id = result_id.map(Ok).unwrap_or_else(|| uuid(&tx))?;
        tx.execute("insert or ignore into consolidation_results(result_id,job_decision_id,generation,state,created_at,updated_at) values(?1,?2,?3,'reserved',?4,?4)",params![result_id,decision_id,generation,t])?;
        tx.execute("update consolidation_results set expected_revision=?2 where result_id=?1 and state='reserved'",params![result_id,revision])?;
        tx.execute("update consolidation_jobs set state='leased',attempts=?2,lease_token=?3,lease_until=?4,last_attempt_at=case when ?6 then last_attempt_at else ?5 end,updated_at=?5 where decision_id=?1",params![decision_id,attempts,token,timestamp(expires_at),t,prepared])?;
        if state == "degraded" && !prepared {
            let next = timestamp(now + Duration::hours(6));
            tx.execute(
                "update consolidation_jobs set next_attempt_at=?2 where decision_id=?1",
                params![decision_id, next],
            )?;
            tx.execute("update provider_recovery_state set next_recovery_at=?2,updated_at=?3 where provider_slot_id=?1",params![slot,next,t])?;
        }
        tx.commit()?;
        Ok(Some(ConsolidationLease {
            decision_id,
            result_id,
            generation: generation as u64,
            token,
            expires_at,
            attempts: attempts as u64,
            expected_revision: revision as u64,
            series_id,
            provider_slot_id: slot,
            canonical_output,
        }))
    }
    /// Persist normalized bytes once, bound to the worker's immutable Dream
    /// receipt whose context is this complete selected result. This is a trusted
    /// domain API; provider drafts must never be treated as origin receipts.
    pub fn prepare_result(
        &mut self,
        token: &str,
        result_id: &str,
        canonical_output: &str,
        now: DateTime<Utc>,
    ) -> Result<(), ConsolidationError> {
        let output: ConsolidationResultV1 = serde_json::from_str(canonical_output)
            .map_err(|e| ConsolidationError::Invariant(format!("invalid stored protocol: {e}")))?;
        let canonical = serde_json::to_value(&output)
            .map_err(|e| ConsolidationError::Invariant(e.to_string()))?
            .to_string();
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (job, _) = current_lease(&tx, token, now)?;
        let row:Option<(i64,i64,Option<String>)>=tx.query_row("select r.generation,r.expected_revision,r.canonical_output from consolidation_results r join consolidation_jobs j on j.decision_id=r.job_decision_id and j.result_generation=r.generation where r.result_id=?1 and r.job_decision_id=?2 and r.state in ('reserved','prepared')",params![result_id,job],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let (generation, revision, stored) = row.ok_or(ConsolidationError::ExpiredLease)?;
        if let Some(stored) = stored {
            if stored != canonical {
                return Err(ConsolidationError::IdempotencyConflict);
            }
            tx.commit()?;
            return Ok(());
        }
        if output.version != 1
            || output.result_id != result_id
            || output.job_decision_id != job
            || output.generation != generation as u64
            || output.expected_revision != revision as u64
            || output.mutations.len() > 100
        {
            return Err(ConsolidationError::Invariant(
                "result identity, version, revision or mutation bound".into(),
            ));
        }
        let origin: Option<(String, String, String, String)> = tx
            .query_row(
                "select kind,text,context_json,content_hash from origin_receipts where id=?1",
                [&output.origin.0],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let Some((kind, text, context, hash)) = origin else {
            return Err(ConsolidationError::Invariant(
                "missing trusted Dream origin".into(),
            ));
        };
        if kind != "dream"
            || context != canonical
            || crate::authority_evidence::hash(&format!("{text}\n{context}")) != hash
        {
            return Err(ConsolidationError::Invariant(
                "Dream origin does not bind selected result".into(),
            ));
        }
        tx.execute("update consolidation_results set state='prepared',canonical_output=?2,origin_id=?3,updated_at=?4 where result_id=?1",params![result_id,canonical,output.origin.0,timestamp(now)])?;
        tx.commit()?;
        Ok(())
    }
    pub fn fail(
        &mut self,
        token: &str,
        kind: FailureKind,
        error_code: &str,
        now: DateTime<Utc>,
    ) -> Result<(), ConsolidationError> {
        if error_code.is_empty()
            || error_code.len() > 128
            || !error_code
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(ConsolidationError::Invariant(
                "invalid diagnostic code".into(),
            ));
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (job, attempts) = current_lease(&tx, token, now)?;
        let (state, delay) = match kind {
            FailureKind::Deterministic => ("failed", None),
            FailureKind::Transient if attempts >= 6 => ("degraded", Some(21600)),
            FailureKind::Transient => (
                "retry",
                Some([30, 120, 600, 3600, 21600][attempts.saturating_sub(1) as usize]),
            ),
        };
        tx.execute("update consolidation_jobs set state=?2,next_attempt_at=?3,lease_token=null,lease_until=null,last_error_code=?4,updated_at=?5 where decision_id=?1",params![job,state,delay.map(|d|timestamp(now+Duration::seconds(d))),error_code,timestamp(now)])?;
        tx.commit()?;
        Ok(())
    }
}
pub(crate) fn current_lease(
    tx: &Connection,
    token: &str,
    now: DateTime<Utc>,
) -> Result<(String, i64), ConsolidationError> {
    tx.query_row("select decision_id,attempts from consolidation_jobs where state='leased' and lease_token=?1 and julianday(lease_until)>julianday(?2)",params![token,timestamp(now)],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or(ConsolidationError::ExpiredLease)
}
fn recover(tx: &Transaction<'_>, now: DateTime<Utc>) -> Result<usize, ConsolidationError> {
    // A sixth initial attempt can crash before fail() parks it. Preserve an
    // already reserved recovery deadline, otherwise park from recovery time.
    Ok(tx.execute("update consolidation_jobs set state=case when attempts>=6 then 'degraded' else 'retry' end,next_attempt_at=case when attempts>=6 and (next_attempt_at is null or julianday(next_attempt_at)<=julianday(?1)) then ?2 else next_attempt_at end,lease_token=null,lease_until=null,updated_at=?1 where state='leased' and julianday(lease_until)<=julianday(?1)",params![timestamp(now),timestamp(now+Duration::hours(6))])?)
}
