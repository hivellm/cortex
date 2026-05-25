//! Phase11s §3.2 — Vectorizer JWT token cache with pre-emptive refresh.
//!
//! The 2026-05-03 incident showed `cortex-embedder-worker` running for
//! hours with an expired JWT — every embed call returned `HTTP 401
//! Unauthorized` because [`super::vectorizer_client::LiveVectorizerClient`]
//! takes the JWT once at construction and never refreshes. The
//! Vectorizer issues tokens with `expires_in = 3600` (1 hour); the
//! worker survived its first hour and then bled silent failures
//! until manual restart.
//!
//! This module ships the canonical fix:
//!
//! 1. Track the JWT's expiry locally (parsed from the `exp` claim
//!    when present, falls back to `DEFAULT_TOKEN_TTL_SECS`).
//! 2. Refresh `REFRESH_BUFFER_SECS` before expiry so a request never
//!    races the rotation.
//! 3. Expose counters (`refreshes_total`, `refresh_errors_total`,
//!    `last_login_ts_ms`) for `/healthz` + Prometheus.
//!
//! The cache is a pure data type — no I/O, no locking on the hot
//! path. Refresh orchestration lives in `vectorizer_client.rs` so
//! tests can drive the cache deterministically without touching the
//! HTTP layer.

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::RwLock;

/// Default TTL stamped when a JWT carries no `exp` claim. Matches
/// the Vectorizer 3.0.3 server default (`expires_in: 3600`).
pub const DEFAULT_TOKEN_TTL_SECS: i64 = 3_600;

/// How far in advance of expiry to pre-emptively refresh. The
/// proposal pins this at 60 seconds — long enough to absorb a
/// retry storm against a hot Vectorizer, short enough that the
/// 5-minute prompt-cache TTL on the upstream doesn't expire
/// between refreshes.
pub const REFRESH_BUFFER_SECS: i64 = 60;

/// Token cache state. `token` is the live bearer; `expires_at_ms`
/// is the wall-clock expiry the JWT carries (or
/// `now + DEFAULT_TOKEN_TTL_SECS` when unparseable). The metrics
/// counters are bumped by the refresh orchestrator and surface
/// through `/healthz`.
#[derive(Debug)]
pub struct TokenCache {
    /// Live bearer token. `None` when the cache has never been
    /// populated (boot before first login).
    token: RwLock<Option<String>>,
    /// Unix-epoch ms when the live token expires. `0` means
    /// "never populated".
    expires_at_ms: AtomicI64,
    /// Unix-epoch ms of the most recent successful login.
    last_login_ts_ms: AtomicI64,
    /// Cumulative successful refreshes (boot login + every
    /// pre-emptive rotation + every 401-recovery re-login).
    refreshes_total: AtomicU64,
    /// Cumulative refresh failures.
    refresh_errors_total: AtomicU64,
}

impl Default for TokenCache {
    fn default() -> Self {
        Self {
            token: RwLock::new(None),
            expires_at_ms: AtomicI64::new(0),
            last_login_ts_ms: AtomicI64::new(0),
            refreshes_total: AtomicU64::new(0),
            refresh_errors_total: AtomicU64::new(0),
        }
    }
}

impl TokenCache {
    /// Empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Read the live bearer token. `None` until [`Self::record_refresh`]
    /// fires for the first time.
    pub fn token(&self) -> Option<String> {
        self.token.read().ok().and_then(|g| g.clone())
    }

    /// Read the live token's expiry (Unix-epoch ms). `0` until
    /// the first refresh.
    pub fn expires_at_ms(&self) -> i64 {
        self.expires_at_ms.load(Ordering::Relaxed)
    }

    /// Read the last-login timestamp (Unix-epoch ms).
    pub fn last_login_ts_ms(&self) -> i64 {
        self.last_login_ts_ms.load(Ordering::Relaxed)
    }

    /// Read the cumulative-refresh counter.
    pub fn refreshes_total(&self) -> u64 {
        self.refreshes_total.load(Ordering::Relaxed)
    }

    /// Read the cumulative-error counter.
    pub fn refresh_errors_total(&self) -> u64 {
        self.refresh_errors_total.load(Ordering::Relaxed)
    }

    /// Phase11s §3.2 — does the token need a refresh? `true` when
    /// the cache is empty OR the token expires within
    /// [`REFRESH_BUFFER_SECS`] of `now_ms`.
    pub fn should_refresh(&self, now_ms: i64) -> bool {
        let expires_at = self.expires_at_ms.load(Ordering::Relaxed);
        if expires_at == 0 {
            return true;
        }
        let buffer_ms = REFRESH_BUFFER_SECS.saturating_mul(1_000);
        now_ms.saturating_add(buffer_ms) >= expires_at
    }

    /// Phase11s §3.2 — install a freshly-minted JWT. `expires_at_ms`
    /// is the absolute wall-clock expiry (parsed from the `exp`
    /// claim when present); pass `now_ms + DEFAULT_TOKEN_TTL_SECS *
    /// 1000` for tokens with no claim.
    pub fn record_refresh(&self, token: String, expires_at_ms: i64, now_ms: i64) {
        if let Ok(mut g) = self.token.write() {
            *g = Some(token);
        }
        self.expires_at_ms.store(expires_at_ms, Ordering::Relaxed);
        self.last_login_ts_ms.store(now_ms, Ordering::Relaxed);
        self.refreshes_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Phase11s §3.2 — bump the refresh-error counter without
    /// touching the live token. Called by the refresh orchestrator
    /// when `/auth/login` returns a 5xx or transport error so the
    /// dashboard can flag persistent auth failures even when the
    /// previous token is still valid (no functional impact yet).
    pub fn record_error(&self) {
        self.refresh_errors_total.fetch_add(1, Ordering::Relaxed);
    }
}

/// Phase11s §3.2 — best-effort parse of the JWT `exp` claim into
/// Unix-epoch ms. Returns `None` for malformed tokens, missing
/// claims, or non-numeric values; callers fall back to
/// `now_ms + DEFAULT_TOKEN_TTL_SECS * 1000` per the cache contract.
///
/// Standard JWT format: `header.payload.signature`, each segment
/// base64url-encoded. The payload is JSON; the `exp` claim is a
/// Unix-epoch SECONDS integer (RFC 7519 §4.1.4).
pub fn parse_jwt_exp_ms(token: &str) -> Option<i64> {
    let mut segments = token.split('.');
    let _header = segments.next()?;
    let payload_b64 = segments.next()?;
    let _signature = segments.next()?;
    if segments.next().is_some() {
        return None;
    }
    let decoded = b64url_decode(payload_b64)?;
    let json: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    let exp_secs = json.get("exp")?.as_i64()?;
    exp_secs.checked_mul(1_000)
}

/// URL-safe base64 decoder without padding (JWT segments drop `=`).
/// Returns `None` on malformed input.
fn b64url_decode(input: &str) -> Option<Vec<u8>> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    URL_SAFE_NO_PAD.decode(input.as_bytes()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(secs: i64) -> i64 {
        secs.saturating_mul(1_000)
    }

    #[test]
    fn empty_cache_should_refresh() {
        let cache = TokenCache::new();
        assert!(cache.should_refresh(ms(1_000)));
        assert!(cache.token().is_none());
        assert_eq!(cache.expires_at_ms(), 0);
        assert_eq!(cache.last_login_ts_ms(), 0);
        assert_eq!(cache.refreshes_total(), 0);
    }

    #[test]
    fn record_refresh_populates_state_and_bumps_counter() {
        let cache = TokenCache::new();
        cache.record_refresh("jwt-token".into(), ms(2_000), ms(1_000));
        assert_eq!(cache.token().as_deref(), Some("jwt-token"));
        assert_eq!(cache.expires_at_ms(), ms(2_000));
        assert_eq!(cache.last_login_ts_ms(), ms(1_000));
        assert_eq!(cache.refreshes_total(), 1);
        assert_eq!(cache.refresh_errors_total(), 0);
    }

    #[test]
    fn should_refresh_fires_within_buffer_of_expiry() {
        // Phase11s §3.2 — refresh fires `REFRESH_BUFFER_SECS`
        // ahead of expiry. Pin the buffer so a future tweak
        // surfaces here before the live worker starts racing
        // its expiring token.
        assert_eq!(REFRESH_BUFFER_SECS, 60);
        let cache = TokenCache::new();
        // Token expires at t = 3600s.
        let expiry_ms = ms(3_600);
        cache.record_refresh("t".into(), expiry_ms, ms(0));
        // 30s before expiry — inside the buffer → must refresh.
        assert!(cache.should_refresh(ms(3_570)));
        // Exactly 60s before expiry — at the edge → must refresh.
        assert!(cache.should_refresh(ms(3_540)));
        // 61s before expiry — outside the buffer → no refresh.
        assert!(!cache.should_refresh(ms(3_539)));
    }

    #[test]
    fn record_error_bumps_only_the_error_counter() {
        // Phase11s §3.2 — error counter is independent of the
        // live token state. A failed refresh attempt against a
        // still-valid token surfaces in metrics without flipping
        // the token to None.
        let cache = TokenCache::new();
        cache.record_refresh("good".into(), ms(2_000), ms(1_000));
        cache.record_error();
        cache.record_error();
        assert_eq!(cache.refresh_errors_total(), 2);
        assert_eq!(cache.refreshes_total(), 1);
        assert_eq!(cache.token().as_deref(), Some("good"));
    }

    #[test]
    fn parse_jwt_exp_ms_extracts_standard_claim() {
        // Hand-crafted JWT: header={"alg":"none","typ":"JWT"},
        // payload={"sub":"alice","exp":2000000000}, no signature.
        // Built via Python:
        //   import base64, json
        //   def b64(d): return base64.urlsafe_b64encode(d).rstrip(b'=').decode()
        //   header = b64(json.dumps({"alg":"none","typ":"JWT"}).encode())
        //   payload = b64(json.dumps({"sub":"alice","exp":2000000000}).encode())
        //   token = f"{header}.{payload}."
        let token = concat!(
            "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.",
            "eyJzdWIiOiJhbGljZSIsImV4cCI6MjAwMDAwMDAwMH0.",
            ""
        );
        let exp = parse_jwt_exp_ms(token).expect("standard JWT parses");
        assert_eq!(exp, 2_000_000_000_i64.saturating_mul(1_000));
    }

    #[test]
    fn parse_jwt_exp_ms_rejects_malformed_token() {
        // Wrong segment count, garbage base64, missing claim:
        // every defensive branch yields `None` so the caller
        // falls back to DEFAULT_TOKEN_TTL_SECS.
        assert!(parse_jwt_exp_ms("not.enough").is_none());
        assert!(parse_jwt_exp_ms("not.enough.parts.4").is_none());
        assert!(parse_jwt_exp_ms("!!!.@@@.###").is_none());
        // Valid base64 segments but the payload is not JSON.
        let token = "aGVsbG8.aGVsbG8.aGVsbG8";
        assert!(parse_jwt_exp_ms(token).is_none());
    }

    #[test]
    fn default_ttl_matches_vectorizer_server() {
        // Phase11s §3.2 — the Vectorizer 3.0.3 server returns
        // `expires_in: 3600`; the cache fallback must agree.
        assert_eq!(DEFAULT_TOKEN_TTL_SECS, 3_600);
    }
}
