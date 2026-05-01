//! Per-caller token-bucket rate limiter. Spec 11 §Rate limiting:
//! 30 rps sustained / 60 rps burst per caller. Buckets refill at the
//! sustained rate; bursts above the bucket capacity get a 429 with
//! `Retry-After`.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Decision returned by [`RateLimiter::admit`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateDecision {
    /// Caller is admitted; remaining tokens after this admission.
    Admit {
        /// Tokens remaining in the bucket.
        remaining: u32,
    },
    /// Caller is rate-limited; carries the suggested
    /// `Retry-After` window.
    Limit {
        /// Wait window before the next admission.
        retry_after: Duration,
    },
}

/// Configuration for the token bucket.
#[derive(Debug, Clone, Copy)]
pub struct RateConfig {
    /// Sustained rate in requests per second.
    pub rps_sustained: u32,
    /// Burst capacity in requests.
    pub rps_burst: u32,
}

impl RateConfig {
    /// Spec-11 defaults.
    pub const fn default_for_spec_11() -> Self {
        Self {
            rps_sustained: 30,
            rps_burst: 60,
        }
    }
}

#[derive(Debug, Clone)]
struct Bucket {
    tokens: f64,
    last: Instant,
}

/// Token-bucket limiter sharded by caller name.
pub struct RateLimiter {
    buckets: Mutex<HashMap<String, Bucket>>,
    cfg: RateConfig,
}

impl RateLimiter {
    /// Build a limiter from a config.
    pub fn new(cfg: RateConfig) -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
            cfg,
        }
    }

    /// Try to consume one token for `caller`. Refills the bucket up
    /// to `rps_burst` based on elapsed time since the last call.
    pub fn admit(&self, caller: &str) -> RateDecision {
        let mut g = match self.buckets.lock() {
            Ok(g) => g,
            Err(_) => return RateDecision::Admit { remaining: 0 },
        };
        let now = Instant::now();
        let bucket = g.entry(caller.to_string()).or_insert_with(|| Bucket {
            tokens: self.cfg.rps_burst as f64,
            last: now,
        });
        let elapsed = now.duration_since(bucket.last).as_secs_f64();
        let refill = elapsed * self.cfg.rps_sustained as f64;
        bucket.tokens = (bucket.tokens + refill).min(self.cfg.rps_burst as f64);
        bucket.last = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            RateDecision::Admit {
                remaining: bucket.tokens.floor() as u32,
            }
        } else {
            // Time until one full token refills.
            let need = 1.0 - bucket.tokens;
            let secs = need / self.cfg.rps_sustained.max(1) as f64;
            RateDecision::Limit {
                retry_after: Duration::from_secs_f64(secs.max(0.001)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admits_until_burst_capacity_then_throttles() {
        let limiter = RateLimiter::new(RateConfig {
            rps_sustained: 30,
            rps_burst: 5,
        });
        for i in 0..5 {
            match limiter.admit("c") {
                RateDecision::Admit { .. } => (),
                _ => panic!("call {i} must admit"),
            }
        }
        match limiter.admit("c") {
            RateDecision::Limit { retry_after } => {
                assert!(retry_after > Duration::from_millis(0));
            }
            _ => panic!("burst exceeded must throttle"),
        }
    }

    #[test]
    fn refills_over_time() {
        let limiter = RateLimiter::new(RateConfig {
            rps_sustained: 1000,
            rps_burst: 1,
        });
        match limiter.admit("c") {
            RateDecision::Admit { .. } => (),
            _ => panic!(),
        }
        // Wait long enough to refill at least one token.
        std::thread::sleep(Duration::from_millis(20));
        match limiter.admit("c") {
            RateDecision::Admit { .. } => (),
            _ => panic!("refill should let us in again"),
        }
    }

    #[test]
    fn callers_have_independent_buckets() {
        let limiter = RateLimiter::new(RateConfig {
            rps_sustained: 1,
            rps_burst: 1,
        });
        assert!(matches!(limiter.admit("a"), RateDecision::Admit { .. }));
        assert!(matches!(limiter.admit("b"), RateDecision::Admit { .. }));
    }
}
