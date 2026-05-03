//! Integration tests for `cortex_workers::classifier::stats`.

use cortex_workers::classifier::stats::PricingTable;

#[test]
fn spend_cents_rounds_up() {
    let p = PricingTable::HAIKU_4_5;
    let c = p.spend_cents(1000, 1000);
    // 1000/1000 * 0.001 + 1000/1000 * 0.005 = 0.006 USD = 0.6 cents -> ceil 1
    assert!(c >= 1);
}
