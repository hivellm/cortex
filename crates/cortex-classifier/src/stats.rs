//! Pricing + spend rollup.

use serde::{Deserialize, Serialize};

/// Pricing for a single model.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PricingTable {
    /// USD per 1 000 input tokens.
    pub tokens_in_usd_per_1k: f64,
    /// USD per 1 000 output tokens.
    pub tokens_out_usd_per_1k: f64,
}

impl PricingTable {
    /// Claude Haiku 4.5 default pricing (public rates — reload from config if needed).
    pub const HAIKU_4_5: PricingTable = PricingTable {
        tokens_in_usd_per_1k: 0.001,
        tokens_out_usd_per_1k: 0.005,
    };

    /// Convert a token bucket to US cents (rounded up).
    pub fn spend_cents(&self, tokens_in: u64, tokens_out: u64) -> u64 {
        let usd = (tokens_in as f64 / 1000.0) * self.tokens_in_usd_per_1k
            + (tokens_out as f64 / 1000.0) * self.tokens_out_usd_per_1k;
        (usd * 100.0).ceil() as u64
    }
}

/// Daily rollup row exchanged with the metadata store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifierSpend {
    /// UTC day in `YYYY-MM-DD` form.
    pub day: String,
    /// Number of classifier calls.
    pub calls: u64,
    /// Tokens-in accumulated.
    pub tokens_in: u64,
    /// Tokens-out accumulated.
    pub tokens_out: u64,
    /// Running spend estimate in US cents.
    pub est_usd_cents: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spend_cents_rounds_up() {
        let p = PricingTable::HAIKU_4_5;
        let c = p.spend_cents(1000, 1000);
        // 1000/1000 * 0.001 + 1000/1000 * 0.005 = 0.006 USD = 0.6 cents -> ceil 1
        assert!(c >= 1);
    }
}
