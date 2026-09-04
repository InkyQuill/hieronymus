//! Browser authentication state (ADR 0012 as amended 2026-09-03): one-time,
//! short-lived, in-memory launch grants minted by bearer-authenticated local
//! commands, exchanged once for a daemon-lifetime browser session cookie.
//! The separate CSRF token layer is waived; browser mutations are guarded by
//! the session cookie plus `Origin`/`Host` validation (see `rest`).
//!
//! Redaction discipline: grant and session tokens are [`Secret`]s, never
//! logged, and never appear in URLs — only in response bodies and cookie
//! headers of the exchange response that hands them over.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hieronymus::secret::Secret;

/// Launch grants expire after this long if not exchanged.
const GRANT_TTL: Duration = Duration::from_secs(60);

/// Session and grant tokens are 32 random bytes, hex-encoded.
const TOKEN_BYTES: usize = 32;

const SESSION_COOKIE_NAME: &str = "hieronymus_session";

#[derive(Debug)]
enum GrantState {
    /// Minted, not yet exchanged.
    Available { expires_at: Instant },
    /// Already exchanged once; kept until expiry so a replay is reported as
    /// reuse instead of unknown.
    Consumed { expires_at: Instant },
}

#[derive(Debug, Default)]
struct Inner {
    grants: HashMap<String, GrantState>,
    sessions: HashMap<String, ()>,
}

/// In-memory grant + session store; state lives exactly as long as the
/// daemon (a restart invalidates every session and grant).
#[derive(Debug, Default)]
pub(crate) struct SessionStore {
    inner: Mutex<Inner>,
}

/// Outcome of a grant exchange attempt.
pub(crate) enum ExchangeOutcome {
    /// Grant was valid and is now consumed; the session cookie value is
    /// returned.
    Exchanged(Secret<String>),
    /// The grant was already exchanged once.
    AlreadyUsed,
    /// Unknown or expired grant (indistinguishable to keep state small).
    Unknown,
}

impl SessionStore {
    /// Mint a fresh one-time launch grant.
    pub fn mint_grant(&self) -> Result<Secret<String>, getrandom::Error> {
        let grant = random_token()?;
        let mut inner = self.lock();
        prune_expired(&mut inner.grants);
        inner.grants.insert(
            grant.expose_secret().clone(),
            GrantState::Available {
                expires_at: Instant::now() + GRANT_TTL,
            },
        );
        Ok(grant)
    }

    /// Exchange a presented grant for a browser session. One-time: the grant
    /// is consumed on success, and a replay reports [`ExchangeOutcome::AlreadyUsed`].
    pub fn exchange_grant(&self, presented: &str) -> Result<ExchangeOutcome, getrandom::Error> {
        let session = random_token()?;
        let mut inner = self.lock();
        prune_expired(&mut inner.grants);
        match inner.grants.get_mut(presented) {
            Some(state @ GrantState::Available { .. }) => {
                // Consumed grants linger for the TTL window so a replay is
                // reported as reuse rather than unknown.
                *state = GrantState::Consumed {
                    expires_at: Instant::now() + GRANT_TTL,
                };
                inner.sessions.insert(session.expose_secret().clone(), ());
                Ok(ExchangeOutcome::Exchanged(session))
            }
            Some(GrantState::Consumed { .. }) => Ok(ExchangeOutcome::AlreadyUsed),
            None => Ok(ExchangeOutcome::Unknown),
        }
    }

    /// Whether a presented `hieronymus_session` cookie value is a live session.
    pub fn session_is_valid(&self, presented: &str) -> bool {
        let inner = self.lock();
        inner.sessions.contains_key(presented)
    }
}

impl SessionStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Expired grants are dropped entirely; consumed ones only after expiry so a
/// replay inside the TTL window still reports reuse.
fn prune_expired(grants: &mut HashMap<String, GrantState>) {
    let now = Instant::now();
    grants.retain(|_, state| match state {
        GrantState::Available { expires_at } | GrantState::Consumed { expires_at } => {
            *expires_at > now
        }
    });
}

fn random_token() -> Result<Secret<String>, getrandom::Error> {
    let mut buffer = vec![0_u8; TOKEN_BYTES];
    getrandom::fill(&mut buffer)?;
    let mut text = String::with_capacity(TOKEN_BYTES * 2);
    for byte in buffer {
        text.push_str(&format!("{byte:02x}"));
    }
    Ok(Secret::new(text))
}

/// The `Set-Cookie` header value for a freshly exchanged session, mirroring
/// the frozen route target (minus the waived CSRF machinery).
pub(crate) fn session_cookie_header(session: &Secret<String>) -> String {
    format!(
        "{}={}; Path=/; HttpOnly; SameSite=Strict",
        SESSION_COOKIE_NAME,
        session.expose_secret()
    )
}

/// Extract the `hieronymus_session` cookie value from a `Cookie` header.
pub(crate) fn session_from_cookie_header(header: &str) -> Option<&str> {
    header.split(';').map(str::trim).find_map(|item| {
        item.strip_prefix(SESSION_COOKIE_NAME)
            .and_then(|rest| rest.strip_prefix('='))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grants_exchange_once_then_report_reuse() {
        let store = SessionStore::default();
        let grant = store.mint_grant().unwrap();
        let session = match store.exchange_grant(grant.expose_secret()).unwrap() {
            ExchangeOutcome::Exchanged(session) => session,
            ExchangeOutcome::AlreadyUsed | ExchangeOutcome::Unknown => {
                panic!("first exchange must succeed")
            }
        };
        assert_eq!(session.expose_secret().len(), 64);
        assert!(store.session_is_valid(session.expose_secret()));
        assert!(matches!(
            store.exchange_grant(grant.expose_secret()).unwrap(),
            ExchangeOutcome::AlreadyUsed
        ));
        assert!(matches!(
            store.exchange_grant("never-minted").unwrap(),
            ExchangeOutcome::Unknown
        ));
    }

    #[test]
    fn cookie_values_are_extracted_from_the_header() {
        let header = "hieronymus_session=abc123; other=value";
        assert_eq!(session_from_cookie_header(header), Some("abc123"));
        assert_eq!(
            session_from_cookie_header("other=value; hieronymus_session=xyz"),
            Some("xyz")
        );
        assert_eq!(session_from_cookie_header("other=value"), None);
    }

    #[test]
    fn cookie_header_matches_the_frozen_shape() {
        let session = Secret::new("tok".to_string());
        assert_eq!(
            session_cookie_header(&session),
            "hieronymus_session=tok; Path=/; HttpOnly; SameSite=Strict"
        );
    }
}
