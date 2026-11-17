//! Rate limits and the bounded retry policy.
//!
//! ## Bucket rate limiting is delegated, deliberately
//!
//! twilight's client ratelimiter tracks Discord's per-bucket headers and
//! pre-emptively waits before a request would exceed a bucket. That is the part
//! a hand-rolled client gets wrong, so this module does not reimplement it.
//!
//! twilight also retries a `429` itself: it reads `Retry-After`, waits for the
//! bucket's next permit and re-sends. That retry loop is unbounded, which is why
//! every request in the adapter runs under a caller-configured timeout: a
//! rate-limit wait longer than the timeout surfaces as `TIMEOUT` instead of
//! blocking the server indefinitely. A `429` that does reach the adapter (when a
//! response carries no usable rate-limit headers) is surfaced as
//! `RATE_LIMITED`, never retried blindly here.
//!
//! ## What this module does own
//!
//! The retry policy for *transient transport failures* on reads: network errors,
//! client-side timeouts and Discord `5xx` responses. Retries are bounded by
//! `discord.max_retries` and use exponential backoff with jitter, so a burst of
//! callers does not resynchronise on the same retry instant.
//!
//! Reads only. This server has no write path yet, and when it gets one it will
//! not be retried here: a write whose outcome is unknown must never be replayed.

use std::time::Duration;

use twilight_http::error::ErrorType;

/// Delay before the first retry.
const BASE_BACKOFF: Duration = Duration::from_millis(100);

/// Ceiling for a single backoff delay.
const MAX_BACKOFF: Duration = Duration::from_millis(2_000);

/// Whether retrying this failure could plausibly succeed.
///
/// Deliberately narrow: only failures where Discord never processed the request,
/// or where it reported its own failure, qualify.
pub fn is_transient(error: &twilight_http::Error) -> bool {
    match error.kind() {
        ErrorType::RequestError | ErrorType::RequestTimedOut => true,
        ErrorType::Response { status, .. } => status.is_server_error(),
        _ => false,
    }
}

/// Exponential backoff with jitter for retry number `attempt` (zero-based).
pub fn backoff(attempt: u32) -> Duration {
    let factor = 1u64 << attempt.min(5);
    let ceiling = MAX_BACKOFF.as_millis() as u64;
    let base = (BASE_BACKOFF.as_millis() as u64)
        .saturating_mul(factor)
        .min(ceiling);
    // Jitter between 50% and 100% of the delay.
    let millis = (base as f64 * jitter_fraction()) as u64;
    Duration::from_millis(millis.max(1))
}

/// A pseudo-random fraction in `[0.5, 1.0)`, from the standard library's
/// randomly seeded hasher. No extra dependency is worth pulling in for jitter.
fn jitter_fraction() -> f64 {
    use std::hash::{BuildHasher, Hasher};

    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u8(0);
    0.5 + (hasher.finish() % 500) as f64 / 1_000.0
}
