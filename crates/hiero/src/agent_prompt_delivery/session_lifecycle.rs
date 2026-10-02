//! Host termination has a private replay journal. Hosts without SessionEnd
//! use explicit abandoned-session recovery, never an idle-time heuristic.
use super::*;
use rusqlite::OptionalExtension;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ending {
    host: String,
    host_session_id: String,
    binding_hash: String,
    session_id: i64,
    series_id: i64,
    last_activity_at: String,
    completed: bool,
    previously_paused: bool,
}
fn load_ending(path: &std::path::Path) -> Result<Ending, DeliveryError> {
    let bytes = hieronymus::private_file::read_private(path)?;
    serde_json::from_slice(&bytes).map_err(|_| invalid("invalid saved termination"))
}
fn ending_path(config: &HieronymusConfig, h: &str, session: &str) -> PathBuf {
    config.config_root().join("host-endings").join(
        context_path(config, h, session)
            .file_name()
            .expect("context filename"),
    )
}
pub(super) fn clear_ending(
    config: &HieronymusConfig,
    h: &str,
    session: &str,
) -> Result<(), DeliveryError> {
    match std::fs::remove_file(ending_path(config, h, session)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
fn identity<'a>(h: &str, input: &'a Value, event: &str) -> Result<&'a str, DeliveryError> {
    host(h)?;
    if input["hook_event_name"] != event {
        return Err(invalid(format!("expected {event} event")));
    }
    input["session_id"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 512)
        .ok_or_else(|| invalid("missing host session"))
}
/// Validate a SessionEnd envelope and the binding for that host. Stdin does
/// not attest native-host origin against another process of the same OS user.
/// Failed transport leaves a bounded, replayable intent; it never starts a daemon.
pub fn handle_session_end(
    config: &HieronymusConfig,
    h: &str,
    input: &Value,
) -> Result<Value, DeliveryError> {
    let session = identity(h, input, "SessionEnd")?;
    let _guard = conversation_lock(config, h, session)?;
    finish(config, h, session, None)
}
/// SessionStart/resume invalidates a pending old termination under the same
/// conversation lock. Completed sessions need a new explicit active binding.
pub fn handle_session_start(
    config: &HieronymusConfig,
    h: &str,
    input: &Value,
) -> Result<Value, DeliveryError> {
    let session = identity(h, input, "SessionStart")?;
    let _guard = conversation_lock(config, h, session)?;
    // Explicit task completion need not create a host-ending journal. Do not
    // advertise its stale binding as active when the conversation resumes.
    let context = context_path(config, h, session);
    if context.try_exists()? {
        let binding: HostContext =
            serde_json::from_slice(&hieronymus::private_file::read_private(&context)?)
                .map_err(|_| invalid("invalid saved binding"))?;
        if binding.version != 1 || binding.host != h || binding.host_session_id != session {
            return Err(invalid("saved binding identity mismatch"));
        }
        let db = rusqlite::Connection::open_with_flags(
            config.database_path(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let status: Option<String> = db.query_row(
            "select t.status from task_sessions t join series s on s.slug=t.series_slug where t.id=?1 and s.id=?2",
            params![binding.session_id, binding.series_id],
            |r| r.get(0),
        ).optional()?;
        match status.as_deref() {
            Some("active") => {}
            Some(_) => return Ok(binding_required(binding.session_id)),
            None => return Err(invalid("bound session ownership mismatch")),
        }
    }
    let path = ending_path(config, h, session);
    if path.try_exists()? {
        let ending = load_ending(&path)?;
        if ending.host != h || ending.host_session_id != session {
            return Err(invalid("saved termination identity mismatch"));
        }
        let db = rusqlite::Connection::open_with_flags(
            config.database_path(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let active: bool=db.query_row("select exists(select 1 from task_sessions t join series s on s.slug=t.series_slug where t.id=?1 and s.id=?2 and t.status='active')",params![ending.session_id,ending.series_id],|r|r.get(0))?;
        if ending.completed || !active {
            return Ok(binding_required(ending.session_id));
        }
        {
            clear_ending(config, h, session)?;
            if !ending.previously_paused {
                match std::fs::remove_file(pause_path(config, h, session)) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }
    Ok(json!({"status":"active","host":h,"host_session_id":session,"authority_changed":false}))
}
fn binding_required(session_id: i64) -> Value {
    let reason = "memory session already completed; for new work use a fresh active session and explicitly bind it before capture";
    json!({
        "status":"binding_required", "session_id":session_id,
        "reason":reason, "authority_changed":false,
        "hookSpecificOutput": {
            "hookEventName":"SessionStart",
            "additionalContext":format!("Hieronymus memory session {session_id}: {reason}. Do not capture into the completed session or replay an earlier prompt.")
        }
    })
}
/// Explicit recovery requires the exact currently bound domain ID and an
/// operator assertion that the host has stopped. No bulk or idle recovery.
pub fn recover_session(
    config: &HieronymusConfig,
    h: &str,
    input: &Value,
) -> Result<Value, DeliveryError> {
    host(h)?;
    let session = input["host_session_id"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 512)
        .ok_or_else(|| invalid("missing host session"))?;
    let id = input["session_id"]
        .as_i64()
        .filter(|id| *id > 0)
        .ok_or_else(|| invalid("missing bound memory session"))?;
    if input["confirm_abandoned"] != true {
        return Err(invalid(
            "stop the host first, then explicitly confirm_abandoned",
        ));
    }
    let _guard = conversation_lock(config, h, session)?;
    finish(config, h, session, Some(id))
}
fn finish(
    config: &HieronymusConfig,
    h: &str,
    session: &str,
    expected_id: Option<i64>,
) -> Result<Value, DeliveryError> {
    let path = context_path(config, h, session);
    if !path.try_exists()? {
        return Ok(skipped("no_bound_session"));
    }
    let bytes = hieronymus::private_file::read_private(&path)?;
    let c: HostContext =
        serde_json::from_slice(&bytes).map_err(|_| invalid("invalid saved binding"))?;
    if c.version != 1
        || c.host != h
        || c.host_session_id != session
        || expected_id.is_some_and(|id| id != c.session_id)
    {
        return Err(invalid(
            "saved binding does not match the terminating conversation",
        ));
    }
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let journal = ending_path(config, h, session);
    let mut ending: Ending = if journal.try_exists()? {
        let ending = load_ending(&journal)?;
        if ending.host != h
            || ending.host_session_id != session
            || ending.binding_hash != hash
            || ending.session_id != c.session_id
            || ending.series_id != c.series_id
        {
            return Err(invalid(
                "termination snapshot is stale; bind a fresh active session",
            ));
        }
        if ending.completed {
            return Ok(
                json!({"status":"completed","session_id":ending.session_id,"replayed":true,"authority_changed":false}),
            );
        }
        ending
    } else {
        let db = rusqlite::Connection::open_with_flags(
            config.database_path(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let activity:Option<String>=db.query_row("select t.last_activity_at from task_sessions t join series s on s.slug=t.series_slug where t.id=?1 and s.id=?2 and (?3 is null or t.source_language=?3) and (?4 is null or t.target_language=?4)",params![c.session_id,c.series_id,c.source_language,c.target_language],|r|r.get(0)).optional()?;
        let activity = activity.ok_or_else(|| invalid("bound session ownership mismatch"))?;
        Ending {
            host: h.into(),
            host_session_id: session.into(),
            binding_hash: hash,
            session_id: c.session_id,
            series_id: c.series_id,
            last_activity_at: activity,
            completed: false,
            previously_paused: pause_path(config, h, session).try_exists()?,
        }
    };
    save(&journal, &ending)?;
    save(&pause_path(config, h, session), &json!({"paused":true}))?;
    let response=lifecycle::connect(config,false).and_then(|client|client.call_tool("hieronymus_session_complete",&json!({"session_id":ending.session_id,"expected_series_id":ending.series_id,"expected_last_activity_at":ending.last_activity_at})));
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            return Ok(
                json!({"status":"pending","session_id":ending.session_id,"reason":error.to_string(),"retry":"repeat the original SessionEnd or recover-session; resume cancels pending termination","authority_changed":false}),
            );
        }
    };
    if response.get("error").is_some()
        || response.pointer("/result/isError") == Some(&json!(true))
        || response.pointer("/result/structuredContent/completed") != Some(&json!(true))
    {
        return Ok(
            json!({"status":"pending","session_id":ending.session_id,"reason":"daemon did not acknowledge conditional completion; session may have changed","authority_changed":false}),
        );
    }
    ending.completed = true;
    save(&journal, &ending)?;
    Ok(
        json!({"status":"completed","session_id":ending.session_id,"replayed":false,"authority_changed":false}),
    )
}
