//! In-tree hardening primitives for the MCP HTTP server (RES-3944).
//!
//! Token-bucket rate limiter + byte-size limiter. Pure `core`-compatible
//! logic — no I/O, no new dependencies, and no reliance on `std::time`
//! internally (callers supply a monotonic millisecond clock) — so this
//! module can be lifted into `resilient-runtime` (`no_std`) later without
//! changes.
//!
//! Used by [`crate::mcp_server`] to enforce:
//! - RES-3935: request body size cap (`SizeLimiter`)
//! - RES-3938: per-IP rate limiting (`TokenBucket` + `RateLimiterRegistry`)

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;

/// A fixed-capacity token bucket rate limiter.
///
/// Time is expressed as milliseconds since an arbitrary caller-chosen
/// epoch (e.g. `Instant::elapsed().as_millis()`), keeping the type free
/// of any `std::time` dependency.
#[derive(Debug, Clone, Copy)]
pub struct TokenBucket {
    capacity: f64,
    tokens: f64,
    refill_per_ms: f64,
    last_refill_ms: u64,
}

impl TokenBucket {
    /// Create a bucket that holds up to `capacity` tokens and refills at
    /// `refill_per_min` tokens per minute (a "requests per minute" limit).
    pub fn new(capacity: u32, refill_per_min: u32) -> Self {
        let capacity = capacity.max(1) as f64;
        let refill_per_ms = refill_per_min.max(1) as f64 / 60_000.0;
        TokenBucket {
            capacity,
            tokens: capacity,
            refill_per_ms,
            last_refill_ms: 0,
        }
    }

    /// Refill the bucket based on elapsed time and attempt to consume one
    /// token. Returns `true` if a token was available (request allowed).
    pub fn try_acquire(&mut self, now_ms: u64) -> bool {
        let elapsed = now_ms.saturating_sub(self.last_refill_ms) as f64;
        if elapsed > 0.0 {
            self.tokens = (self.tokens + elapsed * self.refill_per_ms).min(self.capacity);
            self.last_refill_ms = now_ms;
        }
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Per-IP token bucket registry, guarded by a mutex so it can be shared
/// across a (potentially multi-threaded) HTTP accept loop.
pub struct RateLimiterRegistry {
    capacity: u32,
    refill_per_min: u32,
    buckets: Mutex<HashMap<IpAddr, TokenBucket>>,
}

impl RateLimiterRegistry {
    pub fn new(capacity: u32, refill_per_min: u32) -> Self {
        RateLimiterRegistry {
            capacity,
            refill_per_min,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Returns `true` if `ip` is allowed to make a request at `now_ms`.
    pub fn allow(&self, ip: IpAddr, now_ms: u64) -> bool {
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        let bucket = buckets
            .entry(ip)
            .or_insert_with(|| TokenBucket::new(self.capacity, self.refill_per_min));
        bucket.try_acquire(now_ms)
    }
}

/// Error returned when a payload exceeds the configured size limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizeLimitExceeded {
    pub limit: usize,
    pub actual: usize,
}

/// A simple byte-size limiter: checks a declared or observed size against
/// a configured maximum before any buffering happens.
#[derive(Debug, Clone, Copy)]
pub struct SizeLimiter {
    limit: usize,
}

impl SizeLimiter {
    pub fn new(limit: usize) -> Self {
        SizeLimiter { limit }
    }

    /// Check `size` (e.g. a `Content-Length` header value, or bytes read
    /// so far) against the limit.
    pub fn check(&self, size: usize) -> Result<(), SizeLimitExceeded> {
        if size > self.limit {
            Err(SizeLimitExceeded {
                limit: self.limit,
                actual: size,
            })
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_bucket_allows_up_to_capacity_then_blocks() {
        let mut bucket = TokenBucket::new(3, 60);
        assert!(bucket.try_acquire(0));
        assert!(bucket.try_acquire(0));
        assert!(bucket.try_acquire(0));
        assert!(!bucket.try_acquire(0));
    }

    #[test]
    fn token_bucket_refills_over_time() {
        // 60 tokens/min == 1 token/sec == 1 token/1000ms.
        let mut bucket = TokenBucket::new(1, 60);
        assert!(bucket.try_acquire(0));
        assert!(!bucket.try_acquire(100));
        assert!(bucket.try_acquire(1_000));
    }

    #[test]
    fn token_bucket_never_exceeds_capacity() {
        let mut bucket = TokenBucket::new(2, 60);
        assert!(bucket.try_acquire(0));
        assert!(bucket.try_acquire(0));
        // Huge elapsed time should still cap refill at capacity, not
        // accumulate unboundedly.
        assert!(bucket.try_acquire(1_000_000_000));
        assert!(bucket.try_acquire(1_000_000_000));
        assert!(!bucket.try_acquire(1_000_000_000));
    }

    #[test]
    fn rate_limiter_registry_tracks_per_ip() {
        let registry = RateLimiterRegistry::new(1, 60);
        let a: IpAddr = "127.0.0.1".parse().unwrap();
        let b: IpAddr = "127.0.0.2".parse().unwrap();
        assert!(registry.allow(a, 0));
        assert!(!registry.allow(a, 0));
        // Different IP has its own independent bucket.
        assert!(registry.allow(b, 0));
    }

    #[test]
    fn size_limiter_accepts_at_and_under_limit() {
        let limiter = SizeLimiter::new(100);
        assert!(limiter.check(0).is_ok());
        assert!(limiter.check(100).is_ok());
    }

    #[test]
    fn size_limiter_rejects_over_limit() {
        let limiter = SizeLimiter::new(100);
        let err = limiter.check(101).unwrap_err();
        assert_eq!(err.limit, 100);
        assert_eq!(err.actual, 101);
    }
}
