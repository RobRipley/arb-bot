//! Deterministic tests for Strategy T's pure profit math, fixtured against
//! live mainnet quotes independently audited 2026-09-03 (see the plan's
//! Global Constraints for the four-ledger-movement accounting these encode).
//! No network access — these must never require `dfx` or a running canister.

use arb_bot::strategy_t::{
    all_routes, allowance_status_for, check_inventory_bands, closing_leg_input,
    one_leg_net_profit_usd, par_usd_6dec, two_leg_net_profit_usd, AllowanceStatus,
    ClosingPool, StableToken, TokenAmounts,
};

#[test]
fn all_routes_has_exactly_twelve_candidates_six_stop_six_close() {
    let routes = all_routes();
    assert_eq!(routes.len(), 12);
    assert_eq!(routes.iter().filter(|r| r.closing.is_none()).count(), 6);
    assert_eq!(routes.iter().filter(|r| r.closing.is_some()).count(), 6);
    // No degenerate route (start == rumi_out).
    assert!(routes.iter().all(|r| r.start != r.rumi_out));
}

#[test]
fn closing_pool_covers_every_unordered_pair_exactly_once() {
    use StableToken::*;
    assert_eq!(ClosingPool::for_pair(IcUsd, CkUsdc), Some(ClosingPool::IcusdCkusdc));
    assert_eq!(ClosingPool::for_pair(CkUsdc, IcUsd), Some(ClosingPool::IcusdCkusdc));
    assert_eq!(ClosingPool::for_pair(IcUsd, CkUsdt), Some(ClosingPool::IcusdCkusdt));
    assert_eq!(ClosingPool::for_pair(CkUsdt, CkUsdc), Some(ClosingPool::CkusdtCkusdc));
    assert_eq!(ClosingPool::for_pair(IcUsd, IcUsd), None);
}

#[test]
fn par_usd_6dec_treats_all_three_tokens_as_one_dollar_peg() {
    // 100 ckUSDC (6 dec) == $100.00
    assert_eq!(par_usd_6dec(100_000_000, StableToken::CkUsdc), 100_000_000);
    // 100 icUSD (8 dec) == $100.00
    assert_eq!(par_usd_6dec(10_000_000_000, StableToken::IcUsd), 100_000_000);
}

/// $10 ckUSDC round trip, audited 2026-09-03: Rumi gross 10.40381707 icUSD,
/// closing (eb25l) gross 10.373120 ckUSDC, net profit +$0.353120.
#[test]
fn two_leg_round_trip_ten_dollars_matches_audited_fixture() {
    let start_amount = 10_000_000u64; // $10.00 ckUSDC
    let rumi_gross_icusd = 1_040_381_707u64; // 10.40381707 icUSD, raw 8-dec
    let leg2_input = closing_leg_input(StableToken::IcUsd, rumi_gross_icusd);
    assert_eq!(leg2_input, 1_040_181_707); // matches audited "10.40181707"
    let closing_gross_ckusdc = 10_373_120u64; // audited eb25l quoteForAll output
    let profit = two_leg_net_profit_usd(StableToken::CkUsdc, start_amount, closing_gross_ckusdc);
    assert_eq!(profit, 353_120); // +$0.353120
}

/// $100 ckUSDC round trip, audited 2026-09-03: Rumi gross 102.75379366 icUSD,
/// closing gross 102.429613 ckUSDC, net profit +$2.409613.
#[test]
fn two_leg_round_trip_hundred_dollars_matches_audited_fixture() {
    let start_amount = 100_000_000u64;
    let rumi_gross_icusd = 10_275_379_366u64;
    let closing_gross_ckusdc = 102_429_613u64;
    let profit = two_leg_net_profit_usd(StableToken::CkUsdc, start_amount, closing_gross_ckusdc);
    assert_eq!(profit, 2_409_613);
    let _ = rumi_gross_icusd; // exercised via closing_leg_input in the $10/$300/$450 tests
}

/// $300 ckUSDC round trip, audited 2026-09-03: Rumi gross 304.80381301 icUSD,
/// closing gross 303.595192 ckUSDC, net profit +$3.575192.
#[test]
fn two_leg_round_trip_three_hundred_dollars_matches_audited_fixture() {
    let start_amount = 300_000_000u64;
    let rumi_gross_icusd = 30_480_381_301u64;
    let leg2_input = closing_leg_input(StableToken::IcUsd, rumi_gross_icusd);
    assert_eq!(leg2_input, 30_480_181_301);
    let closing_gross_ckusdc = 303_595_192u64;
    let profit = two_leg_net_profit_usd(StableToken::CkUsdc, start_amount, closing_gross_ckusdc);
    assert_eq!(profit, 3_575_192);
}

/// $450 ckUSDC round trip, audited 2026-09-03 (last size that still fully
/// filled before the eb25l ceiling that hour): Rumi gross 455.44972349 icUSD,
/// closing gross 453.192764 ckUSDC, net profit +$3.172764.
#[test]
fn two_leg_round_trip_four_fifty_dollars_matches_audited_fixture() {
    let start_amount = 450_000_000u64;
    let closing_gross_ckusdc = 453_192_764u64;
    let profit = two_leg_net_profit_usd(StableToken::CkUsdc, start_amount, closing_gross_ckusdc);
    assert_eq!(profit, 3_172_764);
}

/// One-leg stop (inventory conversion, no closing leg): derived from the
/// audited $100 Rumi-leg quote above, not independently cross-checked by a
/// live one-leg audit — this pins the arithmetic, not a second live fact.
#[test]
fn one_leg_conversion_hundred_dollars_derived_from_audited_rumi_quote() {
    let start_amount = 100_000_000u64; // $100.00 ckUSDC
    let rumi_gross_icusd = 10_275_379_366u64;
    let profit = one_leg_net_profit_usd(StableToken::CkUsdc, start_amount, StableToken::IcUsd, rumi_gross_icusd);
    assert_eq!(profit, 2_742_793); // +$2.742793
}

#[test]
fn zero_for_one_matches_live_verified_token_ordering() {
    use StableToken::*;
    // eb25l: token0=icUSD, token1=ckUSDC
    assert!(ClosingPool::IcusdCkusdc.zero_for_one_from(IcUsd));
    assert!(!ClosingPool::IcusdCkusdc.zero_for_one_from(CkUsdc));
    // jogrm: token0=ckUSDT, token1=icUSD
    assert!(ClosingPool::IcusdCkusdt.zero_for_one_from(CkUsdt));
    assert!(!ClosingPool::IcusdCkusdt.zero_for_one_from(IcUsd));
    // heq6n: token0=ckUSDT, token1=ckUSDC
    assert!(ClosingPool::CkusdtCkusdc.zero_for_one_from(CkUsdt));
    assert!(!ClosingPool::CkusdtCkusdc.zero_for_one_from(CkUsdc));
}

#[test]
fn inventory_bands_block_start_leg_below_floor() {
    let balances = TokenAmounts { icusd: 0, ckusdt: 0, ckusdc: 10_000_000 }; // $10 ckUSDC
    let floors = TokenAmounts { icusd: 0, ckusdt: 0, ckusdc: 5_000_000 };    // $5 floor
    let ceilings = TokenAmounts { icusd: 200_000_000_000, ckusdt: 2_000_000_000, ckusdc: 2_000_000_000 };
    // Spending $8 would leave $2 balance, below the $5 floor.
    let check = check_inventory_bands(
        StableToken::CkUsdc, 8_000_000, StableToken::IcUsd, 0,
        balances, floors, ceilings,
    );
    assert!(!check.start_ok);
    assert!(!check.eligible());
}

#[test]
fn inventory_bands_block_end_leg_above_ceiling() {
    let balances = TokenAmounts { icusd: 199_000_000_000, ckusdt: 0, ckusdc: 100_000_000 };
    let floors = TokenAmounts { icusd: 0, ckusdt: 0, ckusdc: 5_000_000 };
    let ceilings = TokenAmounts { icusd: 200_000_000_000, ckusdt: 2_000_000_000, ckusdc: 2_000_000_000 };
    // Receiving 2000 icUSD would push balance to 199_002B + ... over the 200_000_000_000 ceiling.
    let check = check_inventory_bands(
        StableToken::CkUsdc, 10_000_000, StableToken::IcUsd, 2_000_000_000,
        balances, floors, ceilings,
    );
    assert!(check.start_ok);
    assert!(!check.end_ok);
    assert!(!check.eligible());
}

#[test]
fn inventory_bands_pass_within_range() {
    let balances = TokenAmounts { icusd: 771_230_051_57, ckusdt: 402_313_538, ckusdc: 73_465_211 };
    let floors = TokenAmounts { icusd: 500_000_000, ckusdt: 5_000_000, ckusdc: 5_000_000 };
    let ceilings = TokenAmounts { icusd: 200_000_000_000, ckusdt: 2_000_000_000, ckusdc: 2_000_000_000 };
    let check = check_inventory_bands(
        StableToken::CkUsdc, 10_000_000, StableToken::IcUsd, 10_400_000_00,
        balances, floors, ceilings,
    );
    assert!(check.eligible());
}

#[test]
fn allowance_status_sufficient_when_allowance_covers_required() {
    let result = allowance_status_for(50u64, Ok((100u64, None)));
    assert_eq!(result, AllowanceStatus::Sufficient);
}

#[test]
fn allowance_status_insufficient_when_allowance_below_required() {
    let result = allowance_status_for(50u64, Ok((30u64, None)));
    assert_eq!(result, AllowanceStatus::Insufficient { allowance: 30, required: 50 });
}

#[test]
fn allowance_status_insufficient_on_query_error() {
    let result = allowance_status_for(50u64, Err("query failed".to_string()));
    assert_eq!(result, AllowanceStatus::Insufficient { allowance: 0, required: 50 });
}

#[test]
fn allowance_status_sufficient_at_exact_boundary() {
    let result = allowance_status_for(50u64, Ok((50u64, None)));
    assert_eq!(result, AllowanceStatus::Sufficient);
}
