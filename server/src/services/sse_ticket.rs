//! Single-use ticket store for SSE endpoint authentication.
//!
//! Browsers cannot set custom headers on `EventSource`, so the JWT had to be
//! passed as a URL query parameter. That leaks the long-lived user token into
//! every layer of the request path: reverse-proxy access logs, Cloudflare
//! Tunnel logs, browser history, and potentially `Referer` headers.
//!
//! To close that leak we hand the client a short-lived opaque ticket instead.
//! Properties:
//!
//! - **Cryptographically random** — 256 bits from `OsRng`, base64url-encoded.
//! - **Single-use** — consumed atomically on the SSE handshake. Replay fails.
//! - **Short-lived** — 60-second TTL. Even if a ticket leaks into a log, the
//!   window for abuse is tiny.
//! - **User-bound** — the server stores the issuing `user_id` so future SSE
//!   handlers can apply per-user authorization without re-parsing JWTs.
//! - **In-memory only** — tickets live in a single RwLock-guarded HashMap.
//!   Losing them on restart is fine (clients simply request a new one).

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use argon2::password_hash::rand_core::{OsRng, RngCore};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// Lifetime of a ticket from issuance until it becomes unusable.
/// Kept deliberately short: the client fetches a fresh ticket immediately
/// before each EventSource connection, so the end-to-end window is seconds.
const TICKET_TTL: Duration = Duration::from_secs(60);

/// Minimum interval between ticket issuances for a single user. Without
/// this, a misbehaving client (or a tight retry loop on a dropped SSE
/// connection) could mint hundreds of tickets per minute — burning the
/// entire `API_RATE_LIMIT_MAX` bucket on ticket traffic alone and
/// preventing the user's own SWR / status calls from landing.
/// 2 s is well below legitimate SSE reconnection cadence (the browser
/// `EventSource` retry starts at 3 s) but hard-caps abuse at 30/min.
const ISSUE_COOLDOWN: Duration = Duration::from_secs(2);

/// Size of the random ticket body in bytes (256 bits).
const TICKET_BYTES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TicketEntry {
    pub user_id: i32,
    pub issued_at: usize,
    pub expires_at: Instant,
}

/// Outcome of `SseTicketStore::issue`. `CoolingDown` carries the remaining
/// cooldown window so the handler can surface a useful `Retry-After`.
pub enum IssueOutcome {
    Minted(String),
    CoolingDown { retry_after_secs: u64 },
}

/// Thread-safe store of live SSE tickets.
pub struct SseTicketStore {
    tickets: RwLock<HashMap<String, TicketEntry>>,
    /// Per-user throttle for `issue()`. Keyed by `user_id`; value is the
    /// `Instant` of the most recent mint. Separate from `tickets` so the
    /// hot path doesn't iterate the ticket map just to check a user's
    /// cooldown. Entries decay implicitly — they're reused by next issue,
    /// and a bounded periodic sweep (`evict_expired`) drops stale users.
    last_issue_per_user: RwLock<HashMap<i32, Instant>>,
}

impl SseTicketStore {
    pub fn new() -> Self {
        Self {
            tickets: RwLock::new(HashMap::new()),
            last_issue_per_user: RwLock::new(HashMap::new()),
        }
    }

    /// Attempt to mint a new ticket for `user_id`. Returns `CoolingDown`
    /// when the same user issued a ticket within `ISSUE_COOLDOWN`; the
    /// handler maps that to `429` with a `Retry-After` header so the
    /// client can back off cleanly.
    pub fn issue(&self, user_id: i32, issued_at: usize) -> IssueOutcome {
        // Cooldown check first — cheap and avoids entropy draw on throttled calls.
        let now = Instant::now();
        {
            let map = match self.last_issue_per_user.read() {
                Ok(m) => m,
                Err(poisoned) => poisoned.into_inner(),
            };
            if let Some(&last) = map.get(&user_id) {
                let elapsed = now.saturating_duration_since(last);
                if elapsed < ISSUE_COOLDOWN {
                    let remaining = ISSUE_COOLDOWN.saturating_sub(elapsed);
                    return IssueOutcome::CoolingDown {
                        retry_after_secs: remaining.as_secs().max(1),
                    };
                }
            }
        }

        let mut raw = [0u8; TICKET_BYTES];
        OsRng.fill_bytes(&mut raw);
        let token = URL_SAFE_NO_PAD.encode(raw);

        let entry = TicketEntry {
            user_id,
            issued_at,
            expires_at: now + TICKET_TTL,
        };

        // Lock poisoning here is recoverable — the map is a plain HashMap and
        // a panic in another thread cannot leave it in a torn state.
        {
            let mut tickets = match self.tickets.write() {
                Ok(t) => t,
                Err(poisoned) => poisoned.into_inner(),
            };
            tickets.insert(token.clone(), entry);
        }
        {
            let mut last = match self.last_issue_per_user.write() {
                Ok(m) => m,
                Err(poisoned) => poisoned.into_inner(),
            };
            last.insert(user_id, now);
        }
        IssueOutcome::Minted(token)
    }

    /// Atomically look up and remove a ticket, returning the issuing session context
    /// on success. Returns `None` for unknown, expired, or already-consumed
    /// tickets — the caller cannot distinguish those cases, which is deliberate
    /// (no oracle for enumeration).
    pub fn consume(&self, presented: &str) -> Option<TicketEntry> {
        if presented.is_empty() {
            return None;
        }
        let mut tickets = match self.tickets.write() {
            Ok(t) => t,
            Err(poisoned) => poisoned.into_inner(),
        };
        let entry = tickets.remove(presented)?;
        if entry.expires_at <= Instant::now() {
            // Expired — removed from the map as a side effect of the lookup,
            // which is exactly what we want (lazy eviction on access).
            return None;
        }
        Some(entry)
    }

    /// Drop every entry whose TTL has elapsed. Called from a background task;
    /// lazy eviction on `consume` handles the hot path.
    ///
    /// Also prunes the per-user cooldown map of entries older than the
    /// cooldown window so an idle-but-numerous userbase does not grow the
    /// map without bound.
    pub fn evict_expired(&self) {
        let now = Instant::now();
        {
            let mut tickets = match self.tickets.write() {
                Ok(t) => t,
                Err(poisoned) => poisoned.into_inner(),
            };
            tickets.retain(|_, entry| entry.expires_at > now);
        }
        {
            let mut last = match self.last_issue_per_user.write() {
                Ok(m) => m,
                Err(poisoned) => poisoned.into_inner(),
            };
            last.retain(|_, ts| now.saturating_duration_since(*ts) < ISSUE_COOLDOWN);
        }
    }
}

impl Default for SseTicketStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue_token(store: &SseTicketStore, user_id: i32, iat: usize) -> String {
        match store.issue(user_id, iat) {
            IssueOutcome::Minted(t) => t,
            IssueOutcome::CoolingDown { retry_after_secs } => {
                panic!("unexpected cooldown (retry={retry_after_secs}s) in test helper")
            }
        }
    }

    #[test]
    fn test_issue_returns_base64url_without_padding() {
        let store = SseTicketStore::new();
        let token = issue_token(&store, 1, 123);
        assert!(!token.is_empty());
        assert!(
            !token.contains('='),
            "URL_SAFE_NO_PAD must not emit '=' padding"
        );
        assert!(
            !token.contains('+') && !token.contains('/'),
            "URL-safe base64 must not contain '+' or '/'"
        );
    }

    #[test]
    fn test_issue_produces_unique_tickets() {
        // Different users on the same tick must not collide. Using the same
        // user twice trips the new cooldown, which is a separate concern
        // covered by `test_issue_cooldown_throttles_same_user`.
        let store = SseTicketStore::new();
        let a = issue_token(&store, 1, 123);
        let b = issue_token(&store, 2, 123);
        assert_ne!(a, b, "Two issues should never collide at 256-bit entropy");
    }

    #[test]
    fn test_consume_returns_user_id_once_only() {
        let store = SseTicketStore::new();
        let token = issue_token(&store, 42, 123);
        let consumed = store.consume(&token).expect("ticket should exist");
        assert_eq!(consumed.user_id, 42);
        assert_eq!(
            store.consume(&token),
            None,
            "Tickets are single-use — second consume must fail"
        );
    }

    #[test]
    fn test_issue_cooldown_throttles_same_user() {
        // Two back-to-back issues for the same user must trip the per-user
        // cooldown — the first mints a real token, the second returns
        // `CoolingDown` with a positive retry hint.
        let store = SseTicketStore::new();
        let first = store.issue(1, 123);
        let second = store.issue(1, 123);
        assert!(
            matches!(first, IssueOutcome::Minted(_)),
            "first call must succeed"
        );
        match second {
            IssueOutcome::CoolingDown { retry_after_secs } => {
                assert!(retry_after_secs >= 1, "Retry-After must be at least 1 s");
            }
            IssueOutcome::Minted(_) => panic!("second call must be throttled"),
        }
    }

    #[test]
    fn test_issue_cooldown_is_per_user() {
        // Different users are independent — one user's cooldown must not
        // lock out another user's parallel SSE handshake.
        let store = SseTicketStore::new();
        assert!(matches!(store.issue(1, 123), IssueOutcome::Minted(_)));
        assert!(
            matches!(store.issue(2, 123), IssueOutcome::Minted(_)),
            "a different user must not inherit the first user's cooldown"
        );
    }

    #[test]
    fn test_consume_unknown_ticket_returns_none() {
        let store = SseTicketStore::new();
        assert_eq!(store.consume("nope"), None);
        assert_eq!(store.consume(""), None);
    }

    #[test]
    fn test_consume_expired_ticket_returns_none() {
        let store = SseTicketStore::new();
        // Inject a synthetic already-expired entry to avoid a sleep.
        let token = "expired-marker".to_string();
        {
            let mut tickets = store.tickets.write().unwrap();
            tickets.insert(
                token.clone(),
                TicketEntry {
                    user_id: 7,
                    issued_at: 123,
                    expires_at: Instant::now() - Duration::from_secs(1),
                },
            );
        }
        assert_eq!(store.consume(&token), None);
    }

    #[test]
    fn test_evict_expired_removes_only_stale_entries() {
        let store = SseTicketStore::new();
        let live_token = issue_token(&store, 1, 123);
        {
            let mut tickets = store.tickets.write().unwrap();
            tickets.insert(
                "stale".to_string(),
                TicketEntry {
                    user_id: 2,
                    issued_at: 456,
                    expires_at: Instant::now() - Duration::from_secs(1),
                },
            );
        }
        store.evict_expired();
        let consumed = store
            .consume(&live_token)
            .expect("live ticket should exist");
        assert_eq!(consumed.user_id, 1);
        assert_eq!(store.consume("stale"), None);
    }
}
