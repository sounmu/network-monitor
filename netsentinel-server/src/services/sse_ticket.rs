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

/// Size of the random ticket body in bytes (256 bits).
const TICKET_BYTES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TicketEntry {
    pub user_id: i32,
    pub issued_at: usize,
    pub expires_at: Instant,
}

/// Thread-safe store of live SSE tickets.
pub struct SseTicketStore {
    tickets: RwLock<HashMap<String, TicketEntry>>,
}

impl SseTicketStore {
    pub fn new() -> Self {
        Self {
            tickets: RwLock::new(HashMap::new()),
        }
    }

    /// Mint a new ticket for `user_id` and return its opaque string form.
    /// The returned value is what the client passes back to `/api/stream?key=`.
    pub fn issue(&self, user_id: i32, issued_at: usize) -> String {
        let mut raw = [0u8; TICKET_BYTES];
        OsRng.fill_bytes(&mut raw);
        let token = URL_SAFE_NO_PAD.encode(raw);

        let entry = TicketEntry {
            user_id,
            issued_at,
            expires_at: Instant::now() + TICKET_TTL,
        };

        // Lock poisoning here is recoverable — the map is a plain HashMap and
        // a panic in another thread cannot leave it in a torn state.
        let mut tickets = match self.tickets.write() {
            Ok(t) => t,
            Err(poisoned) => poisoned.into_inner(),
        };
        tickets.insert(token.clone(), entry);
        token
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
    pub fn evict_expired(&self) {
        let now = Instant::now();
        let mut tickets = match self.tickets.write() {
            Ok(t) => t,
            Err(poisoned) => poisoned.into_inner(),
        };
        tickets.retain(|_, entry| entry.expires_at > now);
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

    #[test]
    fn test_issue_returns_base64url_without_padding() {
        let store = SseTicketStore::new();
        let token = store.issue(1, 123);
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
        let store = SseTicketStore::new();
        let a = store.issue(1, 123);
        let b = store.issue(1, 123);
        assert_ne!(a, b, "Two issues should never collide at 256-bit entropy");
    }

    #[test]
    fn test_consume_returns_user_id_once_only() {
        let store = SseTicketStore::new();
        let token = store.issue(42, 123);
        let consumed = store.consume(&token).expect("ticket should exist");
        assert_eq!(consumed.user_id, 42);
        assert_eq!(
            store.consume(&token),
            None,
            "Tickets are single-use — second consume must fail"
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
        let live_token = store.issue(1, 123);
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
