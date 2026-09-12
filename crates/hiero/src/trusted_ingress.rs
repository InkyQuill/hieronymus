//! Transport-owned actor enrichment. Public drafts cannot name actors or origins.
use hieronymus::authority::OriginContextV1;
use hieronymus::authority_models::*;
use hieronymus::story_applicability::ApplicabilityV1;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionDraftV1 {
    pub version: u8,
    pub decision_id: String,
    pub expected_revision: u64,
    pub evidence_refs: Vec<EvidenceRef>,
    pub series_id: i64,
    pub concept_id: Option<i64>,
    pub source_language: String,
    pub target_language: Option<String>,
    pub applicability: ApplicabilityV1,
    pub operation: OperationV1,
    #[serde(default)]
    pub receipt_ref: Option<OriginReceiptId>,
    #[serde(default)]
    pub session_id: Option<i64>,
}
/// Only route guards construct user principals. The existing actor string is never used.
#[derive(Debug, Clone)]
pub(crate) enum Principal {
    Agent,
    Console(String),
    LocalConsole,
    HostEvent,
}
impl Principal {
    fn identity(&self) -> (&str, &str, ActorKind) {
        match self {
            Self::Agent => ("agent", "ordinary-mcp", ActorKind::Agent),
            Self::Console(session) => ("console_user", session, ActorKind::ExplicitUser),
            Self::LocalConsole => (
                "console_user",
                "local-desktop-console",
                ActorKind::ExplicitUser,
            ),
            Self::HostEvent => (
                "host_user_event",
                "local-host-event",
                ActorKind::ExplicitUser,
            ),
        }
    }
}
pub(crate) struct TrustedIngress<'a> {
    db: &'a Connection,
}
pub(crate) fn hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
pub(crate) fn valid_uuid(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
}
pub(crate) fn new_id() -> Result<String, DecisionErrorV1> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| DecisionErrorV1::StorageUnavailable)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let s: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &s[..8],
        &s[8..12],
        &s[12..16],
        &s[16..20],
        &s[20..]
    ))
}
impl<'a> TrustedIngress<'a> {
    pub(crate) fn new(db: &'a Connection) -> Self {
        Self { db }
    }
    pub(crate) fn bind(
        &self,
        principal: &Principal,
        draft: DecisionDraftV1,
    ) -> Result<DecisionRequestV1, DecisionErrorV1> {
        self.validate(&draft)?;
        let (kind, identity, actor_kind) = principal.identity();
        let origin = if let Some(id) = &draft.receipt_ref {
            let row: Option<(String, String, Option<i64>)> = self
                .db
                .query_row(
                    "select kind,principal,session_id from origin_receipts where id=?",
                    [&id.0],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            if row != Some((kind.into(), identity.into(), draft.session_id)) {
                return Err(DecisionErrorV1::UnverifiedOrigin);
            }
            let (text, context, digest): (String, String, String) = self.db.query_row(
                "select text,context_json,content_hash from origin_receipts where id=?",
                [&id.0],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            let stored: OriginContextV1 =
                serde_json::from_str(&context).map_err(|_| DecisionErrorV1::OriginMismatch)?;
            if hash(&format!("{text}\n{context}")) != digest
                || stored.decision_id != draft.decision_id
                || stored.expected_revision != draft.expected_revision
                || stored.series_id != draft.series_id
                || stored.concept_id != draft.concept_id
                || stored.source_language != draft.source_language
                || stored.target_language != draft.target_language
                || stored.applicability != draft.applicability
                || stored.operation != draft.operation
                || stored.evidence_ids
                    != draft.evidence_refs.iter().map(|e| e.id).collect::<Vec<_>>()
            {
                return Err(DecisionErrorV1::OriginMismatch);
            }
            let mut normalized = draft.clone();
            normalized.receipt_ref = None;
            let expected = serde_json::to_value(&normalized)?;
            let stored_draft = if matches!(principal, Principal::Agent) {
                serde_json::from_str::<serde_json::Value>(&text)?
            } else {
                let metadata:String=self.db.query_row("select binding_json from evidence_records where kind='user_event' and source_identity=?",[format!("origin:{}",id.0)],|r|r.get(0))?;
                let metadata = serde_json::from_str::<serde_json::Value>(&metadata)?;
                if !metadata["parsed"].is_null() {
                    let reparsed = crate::application::correction_parser::parse_correction_v1(
                        &crate::application::correction_parser::OriginReceipt {
                            text: text.clone(),
                        },
                    )
                    .map_err(|_| DecisionErrorV1::OriginMismatch)?;
                    if serde_json::to_value(reparsed)? != metadata["parsed"] {
                        return Err(DecisionErrorV1::OriginMismatch);
                    }
                }
                metadata["draft"].clone()
            };
            if stored_draft != expected {
                return Err(DecisionErrorV1::OriginMismatch);
            }
            id.clone()
        } else if matches!(principal, Principal::Agent) {
            let text = serde_json::to_string(&draft)?;
            self.mint(principal, &draft, &draft.decision_id, &text, None)?
        } else {
            return Err(DecisionErrorV1::UnverifiedOrigin);
        };
        Ok(DecisionRequestV1 {
            version: draft.version,
            decision_id: draft.decision_id,
            expected_revision: draft.expected_revision,
            actor_kind,
            origin,
            evidence_refs: draft.evidence_refs,
            series_id: draft.series_id,
            concept_id: draft.concept_id,
            source_language: draft.source_language,
            target_language: draft.target_language,
            applicability: draft.applicability,
            operation: draft.operation,
        })
    }
    fn validate(&self, draft: &DecisionDraftV1) -> Result<(), DecisionErrorV1> {
        if draft.version != 1 {
            return Err(DecisionErrorV1::UnsupportedVersion);
        }
        if !valid_uuid(&draft.decision_id)
            || draft.expected_revision >= i64::MAX as u64
            || draft
                .receipt_ref
                .as_ref()
                .is_some_and(|id| !valid_uuid(&id.0))
        {
            return Err(DecisionErrorV1::InvalidRequest);
        }
        let target = match &draft.operation {
            OperationV1::Activate {
                candidate_id,
                candidate_revision,
            } => Some((*candidate_id, *candidate_revision)),
            OperationV1::Archive {
                rule_id,
                rule_revision,
            }
            | OperationV1::Replace {
                rule_id,
                rule_revision,
                ..
            }
            | OperationV1::Scope {
                rule_id,
                rule_revision,
                ..
            } => Some((*rule_id, *rule_revision)),
            OperationV1::Correct {
                intent:
                    CorrectionIntentV1::Fact {
                        claim_id,
                        claim_revision,
                        ..
                    },
            } => Some((*claim_id, *claim_revision)),
            OperationV1::Correct {
                intent: CorrectionIntentV1::Rendering { replaces, .. },
            } => *replaces,
            _ => None,
        };
        if target.is_some_and(|(id, revision)| id <= 0 || revision >= i64::MAX as u64) {
            return Err(DecisionErrorV1::InvalidRequest);
        }
        if draft.series_id <= 0
            || draft.concept_id.is_some_and(|id| id <= 0)
            || draft.session_id.is_some_and(|id| id <= 0)
            || draft
                .evidence_refs
                .iter()
                .any(|e| e.id <= 0 || e.span_start >= e.span_end)
        {
            return Err(DecisionErrorV1::InvalidRequest);
        }
        if let Some(session) = draft.session_id {
            let valid:bool=self.db.query_row("select exists(select 1 from task_sessions t join series s on s.slug=t.series_slug where t.id=? and s.id=?)",params![session,draft.series_id],|r|r.get(0))?;
            if !valid {
                return Err(DecisionErrorV1::OriginMismatch);
            }
        }
        Ok(())
    }
    /// Called only after a console form or deterministic event has been resolved.
    pub(crate) fn mint(
        &self,
        principal: &Principal,
        draft: &DecisionDraftV1,
        event_id: &str,
        text: &str,
        parsed: Option<&serde_json::Value>,
    ) -> Result<OriginReceiptId, DecisionErrorV1> {
        self.validate(draft)?;
        if event_id.is_empty() || text.is_empty() || text.len() > 65536 {
            return Err(DecisionErrorV1::InvalidRequest);
        }
        let (kind, identity, _) = principal.identity();
        let selected_source = draft
            .evidence_refs
            .iter()
            .find(|e| e.kind == EvidenceKind::SourcePassage)
            .cloned();
        let context = OriginContextV1 {
            decision_id: draft.decision_id.clone(),
            expected_revision: draft.expected_revision,
            selected_source,
            series_id: draft.series_id,
            concept_id: draft.concept_id,
            source_language: draft.source_language.clone(),
            target_language: draft.target_language.clone(),
            applicability: draft.applicability.clone(),
            evidence_ids: draft.evidence_refs.iter().map(|e| e.id).collect(),
            operation: draft.operation.clone(),
        };
        let context = serde_json::to_string(&context)?;
        let digest = hash(&format!("{text}\n{context}"));
        let existing:Option<(String,String,Option<i64>)>=self.db.query_row("select id,content_hash,session_id from origin_receipts where kind=? and principal=? and event_id=?",params![kind,identity,event_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        if let Some((id, old, session)) = existing {
            if old != digest || session != draft.session_id {
                return Err(DecisionErrorV1::OriginMismatch);
            }
            return Ok(OriginReceiptId(id));
        }
        let id = new_id()?;
        self.db.execute("insert into origin_receipts(id,kind,principal,session_id,event_id,text,context_json,content_hash,created_at) values(?,?,?,?,?,?,?,?,?)",params![id,kind,identity,draft.session_id,event_id,text,context,digest,chrono::Utc::now().to_rfc3339()])?;
        if !matches!(principal, Principal::Agent) {
            // Immutable event snapshot retains exact parser spans and decoded values alongside
            // the full draft/selection binding. This evidence is never terminology support.
            let metadata = serde_json::json!({"origin_id":id,"draft":draft,"parsed":parsed,"principal":identity,"session_id":draft.session_id});
            self.db.execute("insert into evidence_records(series_id,kind,source_identity,source_hash,span_start,span_end,content,binding_json,created_at) values(?,'user_event',?,?,0,?,?,?,?)",params![draft.series_id,format!("origin:{id}"),hash(text),text.len() as i64,text,metadata.to_string(),chrono::Utc::now().to_rfc3339()])?;
        }
        Ok(OriginReceiptId(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (tempfile::TempDir, Connection, DecisionDraftV1) {
        let root = tempfile::tempdir().unwrap();
        let config = hieronymus::data_root::HieronymusConfig::new(root.path());
        let registry = hieronymus::registry::Registry::open(&config).unwrap();
        registry
            .create_series("book", "Book", "en", "ru", None)
            .unwrap();
        let db = hieronymus::db::open_migrated(&config.database_path()).unwrap();
        let draft = DecisionDraftV1 {
            version: 1,
            decision_id: new_id().unwrap(),
            expected_revision: 0,
            evidence_refs: vec![],
            series_id: 1,
            concept_id: None,
            source_language: "en".into(),
            target_language: Some("ru".into()),
            applicability: ApplicabilityV1 {
                series_id: 1,
                timeline_id: None,
                volume_key: None,
                chapter_key: None,
                scope_predicates: vec![],
                valid_from: None,
                valid_until: None,
                metadata_state: hieronymus::story_applicability::MetadataState::Resolved,
                knowledge_gates: vec![],
            },
            operation: OperationV1::Archive {
                rule_id: 1,
                rule_revision: 0,
            },
            receipt_ref: None,
            session_id: None,
        };
        (root, db, draft)
    }
    #[test]
    fn user_receipt_does_not_rebind_to_mcp_or_context() {
        let (_root, db, mut draft) = setup();
        let ingress = TrustedIngress::new(&db);
        let console = Principal::Console("test-browser-session".into());
        let id = ingress
            .mint(&console, &draft, "event-1", "that memory is wrong", None)
            .unwrap();
        draft.receipt_ref = Some(id);
        assert!(matches!(
            ingress.bind(&Principal::Agent, draft.clone()),
            Err(DecisionErrorV1::UnverifiedOrigin)
        ));
        assert!(matches!(
            ingress.bind(&Principal::Console("other-session".into()), draft.clone()),
            Err(DecisionErrorV1::UnverifiedOrigin)
        ));
        let bound = ingress.bind(&console, draft.clone()).unwrap();
        assert_eq!(bound.actor_kind, ActorKind::ExplicitUser);
        for change in 0..6 {
            let mut changed = draft.clone();
            match change {
                0 => changed.concept_id = Some(9),
                1 => changed.expected_revision = 7,
                2 => {
                    changed.operation = OperationV1::Archive {
                        rule_id: 2,
                        rule_revision: 0,
                    }
                }
                3 => changed.applicability.chapter_key = Some("other".into()),
                4 => changed.source_language = "fr".into(),
                _ => changed.evidence_refs.push(EvidenceRef {
                    kind: EvidenceKind::Observation,
                    id: 1,
                    content_hash: "forged".into(),
                    span_start: 0,
                    span_end: 1,
                }),
            }
            assert!(matches!(
                ingress.bind(&console, changed),
                Err(DecisionErrorV1::OriginMismatch)
            ));
        }
        let mut invented = draft.clone();
        invented.receipt_ref = Some(OriginReceiptId(new_id().unwrap()));
        assert!(matches!(
            ingress.bind(&Principal::Agent, invented),
            Err(DecisionErrorV1::UnverifiedOrigin)
        ));
        draft.receipt_ref = None;
        let host = Principal::HostEvent;
        let id = ingress
            .mint(
                &host,
                &draft,
                "host-owned-event",
                "that memory is wrong",
                None,
            )
            .unwrap();
        draft.receipt_ref = Some(id);
        assert_eq!(
            ingress.bind(&host, draft.clone()).unwrap().actor_kind,
            ActorKind::ExplicitUser
        );
        draft.session_id = Some(999);
        assert!(matches!(
            ingress.bind(&host, draft),
            Err(DecisionErrorV1::OriginMismatch)
        ));
    }
}
