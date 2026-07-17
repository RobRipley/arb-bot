use candid::Nat;
use std::cell::Cell;
use crate::arb;
use crate::prices;
use crate::state::{self, VolumePool, VolumeDirection, VolumeTradeType, VolumeTradeLeg};
use crate::swaps::{self, VOLUME_SUBACCOUNT};

const ICUSD_FEE: u64 = 100_000;    // 0.001 icUSD (8 dec)
const ICP_FEE: u64 = 10_000;       // 0.0001 ICP (8 dec)
const THREE_USD_FEE: u64 = 0; // 3USD has no transfer fee
const NANOS_PER_DAY: u64 = 86_400_000_000_000;
const RUMI_POOL_ID: &str = "fohh4-yyaaa-aaaap-qtkpa-cai_ryjl3-tyaaa-aaaaa-aaaba-cai";

/// Maximum allowed deviation (bps) between the icUSD/BOB pool's own spot
/// price and the multi-venue reference (`ref_icusd_per_bob`) before the
/// volume bot refuses to size/execute an icUSD/BOB trade this cycle. Both
/// quote sources are individually manipulable (the icUSD/BOB pool is thin;
/// the reference's BOB/ICP leg is a single ICPSwap pool too) — requiring
/// them to agree defeats single-pool manipulation of EITHER side, since an
/// attacker would need to move both independently-sourced prices in lockstep.
const VOLUME_BOB_MAX_DEVIATION_BPS: u64 = 200;

/// The 3USD ledger doesn't support ICRC-1 subaccounts — it ignores the
/// subaccount field and always operates on the owner's total balance.
/// Transfers to/from a subaccount fail with "cannot transfer to self".
fn is_3usd(token: candid::Principal, config: &state::BotConfig) -> bool {
    token == config.three_usd_ledger
}

thread_local! {
    static VOLUME_CYCLE_IN_PROGRESS: Cell<bool> = Cell::new(false);
}

pub fn is_volume_cycle_in_progress() -> bool {
    VOLUME_CYCLE_IN_PROGRESS.with(|c| c.get())
}

async fn randomized_trade_size(base_usd: u64, variance_pct: u64) -> u64 {
    if variance_pct == 0 {
        return base_usd;
    }
    let rand_bytes: Vec<u8> = match ic_cdk::api::management_canister::main::raw_rand().await {
        Ok((bytes,)) => bytes,
        Err(_) => return base_usd,
    };
    let raw = u32::from_le_bytes([rand_bytes[0], rand_bytes[1], rand_bytes[2], rand_bytes[3]]);
    let factor = (raw as f64 / u32::MAX as f64) * 2.0 - 1.0; // [-1.0, 1.0]
    let variance = base_usd as f64 * variance_pct as f64 / 100.0 * factor;
    let result = (base_usd as f64 + variance).round() as u64;
    result.max(1)
}

/// Reference icUSD-per-BOB rate (8 dec): (ICP per 1 BOB, from the BOB/ICP
/// pool) × (USD per 1 ICP, median across the stable/ICP candidate pools) —
/// the same two-step formula `arb::find_optimal_bob` uses for Strategy S's
/// reference price (steps 1-2), reusing `arb::median_stable_usd_per_icp`
/// rather than duplicating the manipulation-hardened median logic. BOB is
/// NOT $1-pegged, so the icUSD/BOB volume pool's BOB leg sizes and marks off
/// this reference in both directions instead of the flat ×100 the ICP-paired
/// pools use for their $1-pegged stable leg.
pub(crate) async fn ref_icusd_per_bob(config: &state::BotConfig) -> Result<u64, String> {
    let usd_per_icp = arb::median_stable_usd_per_icp(config, 100_000_000).await
        .filter(|&r| r > 0)
        .ok_or_else(|| "No stable/ICP reference quote available".to_string())?;
    const BOB_PROBE: u64 = 100_000_000; // 1 BOB (8 dec)
    let ref_icp_per_bob = prices::fetch_icpswap_quote_for_amount(
        config.icpswap_bob_icp_pool, BOB_PROBE, !config.icpswap_bob_icp_icp_is_token0,
    ).await.map_err(|e| format!("BOB/ICP reference quote failed: {}", e))?;
    if ref_icp_per_bob == 0 {
        return Err("Zero BOB/ICP reference quote".to_string());
    }
    // (icp_e8s × usd_6dec / 1e8) is 6-dec USD per BOB; ×100 lifts to 8-dec
    // icUSD (icUSD ≈ $1). Combined: /1e6. Mirrors arb::find_optimal_bob.
    Ok((ref_icp_per_bob as u128 * usd_per_icp as u128 / 1_000_000) as u64)
}

/// The icUSD/BOB pool's OWN spot quote, in the same units as
/// `ref_icusd_per_bob`: 8-dec icUSD received per 1 BOB sold.
///
/// Units proof: `fetch_icpswap_price` hardcodes `amountIn = "100000000"`
/// raw units of whichever token `zero_for_one` sells. BOB is an 8-decimal
/// ledger (see `BOB_PROBE` above and `bob_ledger_fee`'s "0.01 BOB" comment
/// in state.rs), so 100_000_000 raw units is exactly 1 BOB. Passing
/// `zero_for_one = !icusd_is_token0` sells BOB (BOB is token0 iff icUSD is
/// NOT token0 — the pool has exactly two tokens) and receives icUSD out;
/// icUSD is also 8-decimal (`ICUSD_FEE` = "0.001 icUSD (8 dec)"). So the
/// result is 8-dec icUSD per 1 BOB — identical units to
/// `ref_icusd_per_bob`'s "8-dec icUSD per 1 BOB" (see `mark_bob_usd`'s
/// doc comment in arb.rs). This mirrors `find_optimal_bob`'s `pool_res`
/// quote (`fetch_icpswap_quote_for_amount(icusd_bob_pool, BOB_PROBE,
/// !icusd_is_token0)`), just via the fixed-1e8-input `fetch_icpswap_price`
/// entry point instead.
async fn pool_icusd_per_bob(config: &state::BotConfig) -> Result<u64, String> {
    prices::fetch_icpswap_price(
        config.icpswap_icusd_bob_pool,
        !config.icpswap_icusd_bob_icusd_is_token0,
    ).await.map_err(|e| format!("icUSD/BOB pool price fetch failed: {}", e))
}

/// Cross-checks the icUSD/BOB pool's own spot price against the multi-venue
/// reference. Returns `(ref_price, pool_price)` — both 8-dec icUSD per 1
/// BOB — only when they agree within `VOLUME_BOB_MAX_DEVIATION_BPS`. Any
/// quote failure, a zero price, or an excessive deviation is surfaced as
/// `Err` with a human-readable reason so the caller can skip the cycle and
/// cede to strategy S, which has its own spread-based profitability check
/// rather than blindly sizing off a single (possibly manipulated) quote.
async fn checked_bob_prices(config: &state::BotConfig) -> Result<(u64, u64), String> {
    let ref_price = ref_icusd_per_bob(config).await?;
    let pool_price = pool_icusd_per_bob(config).await?;
    if ref_price == 0 || pool_price == 0 {
        return Err("zero price (reference or pool)".to_string());
    }
    let diff = if pool_price > ref_price { pool_price - ref_price } else { ref_price - pool_price };
    // u128 so an extreme (self-funded) pool_price can't overflow the *10_000
    // and wrap the deviation down past the guard threshold.
    let deviation_bps = (diff as u128 * 10_000 / ref_price as u128) as u64;
    if deviation_bps > VOLUME_BOB_MAX_DEVIATION_BPS {
        return Err(format!(
            "icUSD/BOB pool price deviates {}bps from reference; deferring to arb strategy S",
            deviation_bps
        ));
    }
    Ok((ref_price, pool_price))
}

fn is_pool_idle(current_price: u64, last_price: Option<u64>, threshold_bps: u64) -> bool {
    match last_price {
        None => true,
        Some(prev) => {
            if prev == 0 { return true; }
            let diff = if current_price > prev {
                current_price - prev
            } else {
                prev - current_price
            };
            let movement_bps = diff * 10_000 / prev;
            movement_bps <= threshold_bps
        }
    }
}

async fn execute_volume_trade(
    pool: VolumePool,
    direction: &VolumeDirection,
    trade_size_usd: u64,
    config: &state::BotConfig,
    // icUSD/BOB pool's own spot price (8-dec icUSD per 1 BOB) — only
    // meaningful for `VolumePool::IcusdBob`. The caller has already run the
    // deviation guard (`checked_bob_prices`) before calling this, so this is
    // a guard-checked price, not a fresh unguarded fetch. Ignored (pass 0)
    // for the ICP-paired pools.
    bob_pool_price: u64,
) -> Result<(u64, u64, u64, u64), String> {
    // Returns: (amount_in, amount_out, price_before, price_after)
    //
    // The tuple below is generalized so the "other" leg isn't assumed to be
    // ICP: `base_is_token0` marks whichever leg is available elsewhere on
    // `config` without needing to flow through this tuple (ICP for the
    // ICP-paired pools, icUSD for the icUSD/BOB pool); `other_ledger`/
    // `other_fee` carry the leg that varies per pool (the $1-pegged stable
    // for ICP pools, BOB for the icUSD/BOB pool — NOT $1-pegged).
    let (icpswap_pool, base_is_token0, other_ledger, other_fee, _other_decimals) = match pool {
        VolumePool::IcusdIcp => (
            config.icpswap_icusd_pool,
            config.icpswap_icusd_icp_is_token0,
            config.icusd_ledger,
            ICUSD_FEE,
            8u32,
        ),
        VolumePool::ThreeUsdIcp => (
            config.icpswap_3usd_pool,
            config.icpswap_3usd_icp_is_token0,
            config.three_usd_ledger,
            THREE_USD_FEE,
            8u32,
        ),
        VolumePool::IcusdBob => (
            config.icpswap_icusd_bob_pool,
            config.icpswap_icusd_bob_icusd_is_token0,
            config.bob_ledger,
            config.bob_ledger_fee,
            8u32,
        ),
    };

    // Fetch price before trade. For the ICP-paired pools this is the pool's
    // own 1-ICP quote. For the icUSD/BOB pool it is the external reference
    // (`ref_icusd_per_bob`) instead — BOB is NOT $1-pegged, so both sizing
    // and marking for its leg go through the reference in both directions
    // rather than the pool's own (thin, manipulable) quote.
    let zero_for_one_quote = base_is_token0;
    let price_before = match pool {
        // Reuse the caller's guard-checked pool price (within
        // VOLUME_BOB_MAX_DEVIATION_BPS of the reference) for both sizing and
        // marking, rather than a fresh unguarded ref re-fetch — avoids a TOCTOU
        // where the reference could shift between the guard and the mark.
        VolumePool::IcusdBob => bob_pool_price,
        _ => prices::fetch_icpswap_price(icpswap_pool, zero_for_one_quote)
            .await
            .map_err(|e| format!("Price fetch failed: {}", e))?,
    };

    // Convert trade_size_usd (6-dec) to native token amount
    let (token_in, amount_in_native, zero_for_one, token_in_fee, token_out_fee) = match direction {
        VolumeDirection::BuyIcp => {
            // All volume-bot stables are 8 decimals; USD is 6-dec → multiply by 100
            let amount = trade_size_usd * 100;
            let zfo = !base_is_token0;
            (other_ledger, amount, zfo, other_fee, ICP_FEE)
        },
        VolumeDirection::SellIcp => {
            let icp_amount = if price_before > 0 {
                // All volume-bot stables are 8 decimals; USD is 6-dec → multiply by 100
                let trade_native = trade_size_usd * 100;
                (trade_native as u128 * 100_000_000u128 / price_before as u128) as u64
            } else {
                return Err("Zero price".to_string());
            };
            let zfo = base_is_token0;
            (config.icp_ledger, icp_amount, zfo, ICP_FEE, other_fee)
        },
        VolumeDirection::BuyBob => {
            // Spend icUSD (the implicit $1-pegged leg) — flat ×100, same as
            // BuyIcp's stable leg. No reference price needed on this side.
            let amount = trade_size_usd * 100;
            let zfo = base_is_token0;
            (config.icusd_ledger, amount, zfo, ICUSD_FEE, other_fee)
        },
        VolumeDirection::SellBob => {
            // Spend BOB (NOT $1-pegged) — size via the pool's OWN spot
            // price (`bob_pool_price`), not the reference: the caller has
            // already guard-checked pool_price against the reference
            // (`checked_bob_prices`), so sizing off the pool actually being
            // executed against is now trustworthy, and matches what the
            // pool will actually quote at execution time.
            let bob_amount = if bob_pool_price > 0 {
                let trade_native = trade_size_usd * 100; // icUSD-equivalent target, 8 dec
                (trade_native as u128 * 100_000_000u128 / bob_pool_price as u128) as u64
            } else {
                return Err("Zero pool price".to_string());
            };
            let zfo = !base_is_token0;
            (other_ledger, bob_amount, zfo, other_fee, ICUSD_FEE)
        },
    };

    // min_amount_out for the icUSD/BOB leg, derived from bob_pool_price
    // (the same guard-checked pool price used for sizing above) and
    // slippage_bps — closes the min_amount_out=0 gap that let a malicious
    // pool return an arbitrarily bad amount_out. The ICP-paired pools are
    // deliberately untouched: this always evaluates to 0 for them.
    let min_amount_out: u64 = if matches!(pool, VolumePool::IcusdBob) && bob_pool_price > 0 {
        let slippage = config.slippage_bps.min(10_000);
        let expected_out: u64 = match direction {
            // icUSD in → BOB out: amount_in_native is the icUSD amount (8 dec).
            VolumeDirection::BuyBob => (amount_in_native as u128 * 100_000_000u128 / bob_pool_price as u128) as u64,
            // BOB in → icUSD out: amount_in_native is the BOB amount (8 dec).
            VolumeDirection::SellBob => (amount_in_native as u128 * bob_pool_price as u128 / 100_000_000u128) as u64,
            VolumeDirection::BuyIcp | VolumeDirection::SellIcp => 0,
        };
        (expected_out as u128 * (10_000 - slippage) as u128 / 10_000) as u64
    } else {
        0
    };

    // Step 1: Transfer tokens from volume subaccount to default account
    // (3USD ledger ignores subaccounts — tokens are already in default account;
    // BOB and icUSD are normal ICRC ledgers and follow this subaccount path)
    if !is_3usd(token_in, config) {
        swaps::transfer_from_subaccount(token_in, amount_in_native, VOLUME_SUBACCOUNT)
            .await
            .map_err(|e| format!("Transfer from subaccount failed: {:?}", e))?;
    }

    // Step 2: Execute the swap on ICPSwap (from default account)
    let amount_out = match swaps::icpswap_swap(
        icpswap_pool,
        amount_in_native - token_in_fee,
        zero_for_one,
        min_amount_out, // 0 for the ICP-paired pools (unchanged); slippage-derived for IcusdBob
        token_in_fee,
        token_out_fee,
    ).await {
        Ok(out) => out,
        Err(e) => {
            // Swap failed — tokens are stranded in default account.
            // Try to return them to the volume subaccount (skip for 3USD — already there).
            if !is_3usd(token_in, config) {
                let recovery_amount = amount_in_native.saturating_sub(token_in_fee * 2);
                if recovery_amount > 0 {
                    let _ = swaps::transfer_to_subaccount(token_in, recovery_amount, VOLUME_SUBACCOUNT).await;
                }
            }
            return Err(format!("Swap failed (tokens recovered): {:?}", e));
        }
    };

    // Step 3: Transfer output tokens back to volume subaccount (with retry)
    // (3USD ledger ignores subaccounts — output stays in default account)
    let token_out = match direction {
        VolumeDirection::BuyIcp => config.icp_ledger,
        VolumeDirection::SellIcp => other_ledger,
        VolumeDirection::BuyBob => other_ledger,
        VolumeDirection::SellBob => config.icusd_ledger,
    };
    let out_fee = match direction {
        VolumeDirection::BuyIcp => ICP_FEE,
        VolumeDirection::SellIcp => other_fee,
        VolumeDirection::BuyBob => other_fee,
        VolumeDirection::SellBob => ICUSD_FEE,
    };
    if amount_out > out_fee && !is_3usd(token_out, config) {
        let transfer_amount = amount_out - out_fee;
        let mut transferred = false;
        for attempt in 0..3 {
            match swaps::transfer_to_subaccount(token_out, transfer_amount, VOLUME_SUBACCOUNT).await {
                Ok(_) => {
                    // Clear any previously stranded amount on success. Only
                    // BuyIcp (receives ICP) and BuyBob (receives BOB) are
                    // tracked — SellIcp/SellBob receive icUSD/3USD/icUSD,
                    // none of which the arb drain (ICP or BOB) ever sweeps,
                    // so there's nothing on those legs to protect.
                    if matches!(direction, VolumeDirection::BuyIcp) {
                        state::mutate_state(|s| { s.volume_stranded_icp = 0; });
                    } else if matches!(direction, VolumeDirection::BuyBob) {
                        state::mutate_state(|s| { s.volume_stranded_bob = 0; });
                    }
                    transferred = true;
                    break;
                }
                Err(e) => {
                    state::log_activity("volume", &format!(
                        "Transfer to subaccount attempt {}/3 failed: {:?}", attempt + 1, e
                    ));
                }
            }
        }
        if !transferred {
            // Mark the ICP/BOB as stranded so the arb drain doesn't eat it
            if matches!(direction, VolumeDirection::BuyIcp) {
                state::mutate_state(|s| { s.volume_stranded_icp = transfer_amount; });
            } else if matches!(direction, VolumeDirection::BuyBob) {
                state::mutate_state(|s| { s.volume_stranded_bob = transfer_amount; });
            }
            return Err(format!("Transfer to subaccount failed after 3 attempts (funds protected from drain)"));
        }
    }

    // Fetch price after trade
    let price_after = match pool {
        VolumePool::IcusdBob => ref_icusd_per_bob(config).await.unwrap_or(price_before),
        _ => prices::fetch_icpswap_price(icpswap_pool, zero_for_one_quote)
            .await
            .unwrap_or(price_before),
    };

    Ok((amount_in_native, amount_out, price_before, price_after))
}

/// Returns a list of per-pool outcome strings describing what happened this
/// cycle (or why the cycle didn't proceed at all). The timer callsite ignores
/// the return value; `trigger_volume_cycle` surfaces it to the admin so a
/// manual run shows exactly which gate fired.
pub async fn run_volume_cycle() -> Vec<String> {
    // Note: we no longer block on arb cycle. The volume bot uses the VOLUME_SUBACCOUNT
    // for ICP/icUSD (separate from arb), and 3USD operates on the default account
    // (shared with arb, but accepted trade-off). Blocking here caused the volume bot
    // to miss its 30-min window whenever the arb timer (every 3 min) happened to overlap.
    if arb::is_cycle_in_progress() {
        state::log_activity("volume", "note: arb cycle in progress, proceeding anyway");
    }

    let already_running = VOLUME_CYCLE_IN_PROGRESS.with(|c| {
        if c.get() { true } else { c.set(true); false }
    });
    if already_running {
        return vec!["skipped: volume cycle already in progress".to_string()];
    }

    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) { VOLUME_CYCLE_IN_PROGRESS.with(|c| c.set(false)); }
    }
    let _guard = Guard;

    let (volume_config, bot_config) = state::read_state(|s| (s.volume.clone(), s.config.clone()));

    if volume_config.volume_paused {
        return vec!["skipped: volume_paused=true".to_string()];
    }

    let mut outcomes: Vec<String> = Vec::new();

    // Recover any ICP stranded in the default account from a prior failure
    let stranded = state::read_state(|s| s.volume_stranded_icp);
    if stranded > 0 {
        match swaps::transfer_to_subaccount(bot_config.icp_ledger, stranded, VOLUME_SUBACCOUNT).await {
            Ok(_) => {
                state::mutate_state(|s| { s.volume_stranded_icp = 0; });
                state::log_activity("volume", &format!("Recovered {} stranded ICP to subaccount", stranded));
                outcomes.push(format!("recovered {} stranded ICP", stranded));
            }
            Err(e) => {
                let msg = format!("Stranded ICP recovery failed (will retry): {:?}", e);
                state::log_activity("volume", &msg);
                // Don't proceed with trades while ICP is stranded
                return vec![format!("blocked: {}", msg)];
            }
        }
    }

    // Recover any BOB stranded in the default account from a prior failure
    // (mirrors the ICP recovery block above, including its don't-proceed-
    // on-failure semantics — a stranded BOB recovery failure blocks the
    // whole cycle, not just the icUSD/BOB pool).
    let stranded_bob = state::read_state(|s| s.volume_stranded_bob);
    if stranded_bob > 0 {
        match swaps::transfer_to_subaccount(bot_config.bob_ledger, stranded_bob, VOLUME_SUBACCOUNT).await {
            Ok(_) => {
                state::mutate_state(|s| { s.volume_stranded_bob = 0; });
                state::log_activity("volume", &format!("Recovered {} stranded BOB to subaccount", stranded_bob));
                outcomes.push(format!("recovered {} stranded BOB", stranded_bob));
            }
            Err(e) => {
                let msg = format!("Stranded BOB recovery failed (will retry): {:?}", e);
                state::log_activity("volume", &msg);
                // Don't proceed with trades while BOB is stranded
                return vec![format!("blocked: {}", msg)];
            }
        }
    }

    let now = ic_cdk::api::time();

    let should_reset_daily = now.saturating_sub(volume_config.daily_spend_reset_ts) >= NANOS_PER_DAY;
    if should_reset_daily {
        state::mutate_state(|s| {
            s.volume.daily_spend_usd = 0;
            s.volume.daily_spend_reset_ts = now;
            s.volume.icusd_icp_state.daily_cost_usd = 0;
            s.volume.three_usd_icp_state.daily_cost_usd = 0;
            s.volume.icusd_bob_state.daily_cost_usd = 0;
        });
    }

    for pool in [VolumePool::IcusdIcp, VolumePool::ThreeUsdIcp, VolumePool::IcusdBob] {
        let (pool_config, pool_state) = state::read_state(|s| {
            match &pool {
                VolumePool::IcusdIcp => (s.volume.icusd_icp.clone(), s.volume.icusd_icp_state.clone()),
                VolumePool::ThreeUsdIcp => (s.volume.three_usd_icp.clone(), s.volume.three_usd_icp_state.clone()),
                VolumePool::IcusdBob => (s.volume.icusd_bob.clone(), s.volume.icusd_bob_state.clone()),
            }
        });

        if !pool_config.enabled {
            outcomes.push(format!("{:?}: skipped (pool disabled)", pool));
            continue;
        }

        // The icUSD/BOB pool ships inert: `icusd_bob.enabled` alone isn't
        // sufficient — the pool principal itself defaults anonymous until an
        // admin sets it (see VolumePoolConfig::default() / BotState::default()).
        // Trading against Principal::anonymous() would panic/fail downstream,
        // so gate on it explicitly, mirroring setup_approvals' same check.
        if matches!(pool, VolumePool::IcusdBob) && bot_config.icpswap_icusd_bob_pool == candid::Principal::anonymous() {
            outcomes.push(format!("{:?}: skipped (pool not configured)", pool));
            continue;
        }

        // Check per-pool daily cost cap
        if pool_state.daily_cost_usd >= pool_config.daily_cost_cap_usd as i64 {
            outcomes.push(format!(
                "{:?}: skipped (daily cost cap hit: {} >= {})",
                pool, pool_state.daily_cost_usd, pool_config.daily_cost_cap_usd
            ));
            continue;
        }

        // For the ICP-paired pools, "current_price" is the pool's own 1-ICP
        // quote. For icUSD/BOB it is the external reference (ref_icusd_per_bob)
        // instead — BOB is NOT $1-pegged and its own thin pool is manipulable,
        // so idle-check anchors to the multi-venue reference (also what
        // get_bot_health surfaces as "current_price" for this pool). Actual
        // trade sizing/execution for icUSD/BOB uses `bob_pool_price` below
        // (the pool's own guard-checked quote), set only inside the IcusdBob arm.
        let mut bob_pool_price: u64 = 0;
        let current_price = match &pool {
            VolumePool::IcusdIcp | VolumePool::ThreeUsdIcp => {
                let (icpswap_pool, icp_is_token0) = state::read_state(|s| match &pool {
                    VolumePool::IcusdIcp => (s.config.icpswap_icusd_pool, s.config.icpswap_icusd_icp_is_token0),
                    VolumePool::ThreeUsdIcp => (s.config.icpswap_3usd_pool, s.config.icpswap_3usd_icp_is_token0),
                    VolumePool::IcusdBob => unreachable!("handled by the outer match arm"),
                });
                match prices::fetch_icpswap_price(icpswap_pool, icp_is_token0).await {
                    Ok(p) => p,
                    Err(e) => {
                        let msg = format!("{:?}: skipped (price fetch failed: {})", pool, e);
                        state::log_activity("volume", &msg);
                        outcomes.push(msg);
                        continue;
                    }
                }
            }
            // Cross-pool deviation guard (FIX 2): the pool's own spot price
            // must agree with the multi-venue reference within
            // VOLUME_BOB_MAX_DEVIATION_BPS before this cycle sizes/executes
            // anything against it — see `checked_bob_prices` for the
            // manipulation rationale.
            VolumePool::IcusdBob => match checked_bob_prices(&bot_config).await {
                Ok((ref_price, pool_price)) => {
                    bob_pool_price = pool_price;
                    ref_price
                }
                Err(e) => {
                    let msg = format!("{:?}: skipped ({})", pool, e);
                    state::log_activity("volume", &msg);
                    outcomes.push(msg);
                    continue;
                }
            },
        };

        if !is_pool_idle(current_price, pool_state.last_price, pool_config.idle_threshold_bps) {
            state::mutate_state(|s| {
                match &pool {
                    VolumePool::IcusdIcp => s.volume.icusd_icp_state.last_price = Some(current_price),
                    VolumePool::ThreeUsdIcp => s.volume.three_usd_icp_state.last_price = Some(current_price),
                    VolumePool::IcusdBob => s.volume.icusd_bob_state.last_price = Some(current_price),
                }
            });
            outcomes.push(format!(
                "{:?}: skipped (pool not idle; price moved >{}bps)",
                pool, pool_config.idle_threshold_bps
            ));
            continue;
        }

        let input_token = match (&pool_state.next_direction, &pool) {
            (VolumeDirection::BuyIcp, VolumePool::IcusdIcp) => bot_config.icusd_ledger,
            (VolumeDirection::BuyIcp, VolumePool::ThreeUsdIcp) => bot_config.three_usd_ledger,
            (VolumeDirection::SellIcp, _) => bot_config.icp_ledger,
            (VolumeDirection::BuyBob, _) => bot_config.icusd_ledger,
            (VolumeDirection::SellBob, _) => bot_config.bob_ledger,
            // BuyIcp/IcusdBob never pair — `next_direction` only ever holds
            // BuyIcp/SellIcp for the ICP-paired pools and BuyBob/SellBob for
            // icUSD/BOB (see `default_icusd_bob_state` and the toggle below).
            (VolumeDirection::BuyIcp, VolumePool::IcusdBob) => unreachable!("BuyIcp never paired with IcusdBob"),
        };
        // 3USD ledger ignores subaccounts — check default account instead
        let balance = if is_3usd(input_token, &bot_config) {
            swaps::icrc1_balance_of_default(input_token).await
        } else {
            swaps::icrc1_balance_of_subaccount(input_token, VOLUME_SUBACCOUNT).await
        };
        let balance = match balance {
            Ok(b) => b,
            Err(e) => {
                let msg = format!("{:?}: skipped (balance fetch failed: {:?})", pool, e);
                state::log_activity("volume", &msg);
                outcomes.push(msg);
                continue;
            }
        };

        let trade_size = randomized_trade_size(pool_config.trade_size_usd, pool_config.trade_variance_pct).await;

        let min_native = match (&pool_state.next_direction, &pool) {
            (VolumeDirection::BuyIcp, VolumePool::IcusdIcp) => trade_size * 100,
            (VolumeDirection::BuyIcp, VolumePool::ThreeUsdIcp) => trade_size * 100, // 3USD is 8 decimals
            (VolumeDirection::SellIcp, VolumePool::IcusdIcp) | (VolumeDirection::SellIcp, VolumePool::ThreeUsdIcp) => {
                if current_price > 0 {
                    let stable_native = trade_size * 100; // icUSD/3USD are 8 decimals
                    (stable_native as u128 * 100_000_000u128 / current_price as u128) as u64
                } else {
                    outcomes.push(format!("{:?}: skipped (zero price)", pool));
                    continue;
                }
            }
            // BuyBob spends icUSD ($1-pegged) — flat ×100, same as BuyIcp.
            (VolumeDirection::BuyBob, _) => trade_size * 100,
            // SellBob spends BOB (NOT $1-pegged) — size via the pool's own
            // guard-checked price (`bob_pool_price`), matching what
            // execute_volume_trade will actually size the swap with (FIX 2).
            (VolumeDirection::SellBob, _) => {
                if bob_pool_price > 0 {
                    let icusd_target_native = trade_size * 100;
                    (icusd_target_native as u128 * 100_000_000u128 / bob_pool_price as u128) as u64
                } else {
                    outcomes.push(format!("{:?}: skipped (zero pool price)", pool));
                    continue;
                }
            }
            // Unreachable: BuyIcp/SellIcp never pair with IcusdBob (see input_token above).
            (VolumeDirection::BuyIcp, VolumePool::IcusdBob)
            | (VolumeDirection::SellIcp, VolumePool::IcusdBob) => unreachable!("BuyIcp/SellIcp never paired with IcusdBob"),
        };

        if balance < min_native {
            let msg = format!(
                "skipping {:?} {:?} — insufficient balance ({} < {})",
                pool, pool_state.next_direction, balance, min_native
            );
            state::log_activity("volume", &msg);
            outcomes.push(msg);
            continue;
        }

        match execute_volume_trade(pool.clone(), &pool_state.next_direction, trade_size, &bot_config, bob_pool_price).await {
            Ok((amount_in, amount_out, price_before, price_after)) => {
                let (in_usd, out_usd) = match (&pool_state.next_direction, &pool) {
                    (VolumeDirection::BuyIcp, VolumePool::IcusdIcp) => {
                        // in: icUSD (8 dec) → 6 dec; out: ICP (8 dec) * price (8 dec icUSD/ICP) → 6 dec
                        let in_6 = amount_in / 100;
                        let out_6 = (amount_out as u128 * price_before as u128 / 100_000_000u128 / 100) as u64;
                        (in_6, out_6)
                    },
                    (VolumeDirection::BuyIcp, VolumePool::ThreeUsdIcp) => {
                        // in: 3USD (8 dec) → 6 dec; out: ICP (8 dec) * price (8 dec 3USD/ICP) → 6 dec
                        let in_6 = amount_in / 100;
                        let out_6 = (amount_out as u128 * price_before as u128 / 100_000_000u128 / 100) as u64;
                        (in_6, out_6)
                    },
                    (VolumeDirection::SellIcp, VolumePool::IcusdIcp) => {
                        // in: ICP (8 dec) * price (8 dec icUSD/ICP) → 6 dec; out: icUSD (8 dec) → 6 dec
                        let in_6 = (amount_in as u128 * price_before as u128 / 100_000_000u128 / 100) as u64;
                        let out_6 = amount_out / 100;
                        (in_6, out_6)
                    },
                    (VolumeDirection::SellIcp, VolumePool::ThreeUsdIcp) => {
                        // in: ICP (8 dec) * price (8 dec 3USD/ICP) → 6 dec; out: 3USD (8 dec) → 6 dec
                        let in_6 = (amount_in as u128 * price_before as u128 / 100_000_000u128 / 100) as u64;
                        let out_6 = amount_out / 100;
                        (in_6, out_6)
                    },
                    (VolumeDirection::BuyBob, _) => {
                        // in: icUSD (8 dec, $1) → 6 dec flat; out: BOB (8 dec) marked
                        // at the reference price (arb::mark_bob_usd's exact scaling:
                        // amount × ref_icusd_per_bob / 1e8 / 100).
                        let in_6 = amount_in / 100;
                        let out_6 = arb::mark_bob_usd(amount_out, price_before).max(0) as u64;
                        (in_6, out_6)
                    },
                    (VolumeDirection::SellBob, _) => {
                        // in: BOB (8 dec) marked at the reference price; out: icUSD (8 dec, $1) → 6 dec flat.
                        let in_6 = arb::mark_bob_usd(amount_in, price_before).max(0) as u64;
                        let out_6 = amount_out / 100;
                        (in_6, out_6)
                    },
                    // Unreachable: BuyIcp/SellIcp never pair with IcusdBob (see input_token above).
                    (VolumeDirection::BuyIcp, VolumePool::IcusdBob)
                    | (VolumeDirection::SellIcp, VolumePool::IcusdBob) => unreachable!("BuyIcp/SellIcp never paired with IcusdBob"),
                };
                let cost = in_usd as i64 - out_usd as i64;

                let leg = VolumeTradeLeg {
                    timestamp: ic_cdk::api::time(),
                    pool: pool.clone(),
                    direction: pool_state.next_direction.clone(),
                    trade_type: VolumeTradeType::PingPong,
                    token_in: input_token,
                    token_out: match (&pool_state.next_direction, &pool) {
                        (VolumeDirection::BuyIcp, _) => bot_config.icp_ledger,
                        (VolumeDirection::SellIcp, VolumePool::IcusdIcp) => bot_config.icusd_ledger,
                        (VolumeDirection::SellIcp, VolumePool::ThreeUsdIcp) => bot_config.three_usd_ledger,
                        (VolumeDirection::BuyBob, _) => bot_config.bob_ledger,
                        (VolumeDirection::SellBob, _) => bot_config.icusd_ledger,
                        (VolumeDirection::SellIcp, VolumePool::IcusdBob) => unreachable!("SellIcp never paired with IcusdBob"),
                    },
                    amount_in,
                    amount_out,
                    cost_usd: cost,
                    price_before,
                    price_after,
                };
                state::append_volume_trade(leg);

                state::mutate_state(|s| {
                    let ps = match &pool {
                        VolumePool::IcusdIcp => &mut s.volume.icusd_icp_state,
                        VolumePool::ThreeUsdIcp => &mut s.volume.three_usd_icp_state,
                        VolumePool::IcusdBob => &mut s.volume.icusd_bob_state,
                    };
                    ps.last_price = Some(price_after);
                    ps.next_direction = match ps.next_direction {
                        VolumeDirection::BuyIcp => VolumeDirection::SellIcp,
                        VolumeDirection::SellIcp => VolumeDirection::BuyIcp,
                        VolumeDirection::BuyBob => VolumeDirection::SellBob,
                        VolumeDirection::SellBob => VolumeDirection::BuyBob,
                    };
                    ps.trade_count += 1;
                    ps.total_volume_usd += trade_size;
                    ps.total_cost_usd += cost;
                    ps.daily_cost_usd += cost;
                    s.volume.daily_spend_usd += cost;
                });

                state::log_activity("volume", &format!(
                    "{:?} {:?} on {:?} — in: {}, out: {}, cost: {} USD",
                    pool_state.next_direction, pool, pool, amount_in, amount_out, cost
                ));
                outcomes.push(format!(
                    "{:?}: traded (in: {}, out: {}, cost: {} USD)",
                    pool, amount_in, amount_out, cost
                ));
            },
            Err(e) => {
                state::log_activity("volume", &format!("{:?} trade failed: {}", pool, e));
                outcomes.push(format!("{:?}: trade failed: {}", pool, e));
            }
        }
    }

    // Auto-rebalance disabled — use manual trigger_volume_rebalance() if needed
    outcomes
}

/// icUSD/BOB rebalance: a DIRECT one-hop unwind through the icUSD/BOB pool
/// itself (BOB<->icUSD via `icpswap_swap` on `config.icpswap_icusd_bob_pool`)
/// — deliberately NOT the ICP/Rumi multi-hop route the ICP-paired pools use
/// (`run_rebalance`'s other arms), since BOB has no relationship to ICP or
/// the Rumi 3pool. The 50/50 drift valuation is marked at the reference price
/// (`ref_icusd_per_bob`, same rationale as `execute_volume_trade`), but the
/// actual BOB leg that gets swapped is sized off the pool's own guard-checked
/// spot price (`pool_price`) once `checked_bob_prices` has proven it's sane
/// (FIX 2) — trustworthy sizing against the pool actually being executed.
async fn rebalance_icusd_bob(config: &state::BotConfig, pool_config: &state::VolumePoolConfig, drift_threshold: u64) {
    if !pool_config.enabled {
        return;
    }
    // Ships inert: the pool principal defaults anonymous until an admin
    // sets it, even if `enabled` were somehow flipped first.
    if config.icpswap_icusd_bob_pool == candid::Principal::anonymous() {
        return;
    }

    let bob_bal = swaps::icrc1_balance_of_subaccount(config.bob_ledger, VOLUME_SUBACCOUNT)
        .await.unwrap_or(0);
    // icUSD is a normal ICRC ledger — check the subaccount (unlike 3USD).
    let icusd_bal = swaps::icrc1_balance_of_subaccount(config.icusd_ledger, VOLUME_SUBACCOUNT)
        .await.unwrap_or(0);

    // Cross-pool deviation guard (FIX 2) — see `checked_bob_prices`. Defers
    // to strategy S on failure/deviation exactly like run_volume_cycle does.
    let (ref_price, pool_price) = match checked_bob_prices(config).await {
        Ok(p) => p,
        Err(e) => { state::log_activity("volume", &format!("rebalance IcusdBob: {}", e)); return; }
    };
    let slippage = config.slippage_bps.min(10_000);

    let bob_as_icusd = (bob_bal as u128 * ref_price as u128 / 100_000_000u128) as u64;
    let total = bob_as_icusd + icusd_bal;
    if total == 0 {
        return;
    }
    let bob_pct = bob_as_icusd * 100 / total;

    // BOB is token0 in the pool iff icUSD is NOT (the two-token pool has
    // exactly one token0/token1 split).
    let bob_is_token0 = !config.icpswap_icusd_bob_icusd_is_token0;

    if bob_pct > (50 + drift_threshold) {
        // Too much BOB — sell some for icUSD to bring back toward 50/50.
        let target_bob_icusd = total / 2;
        let excess_icusd_equiv = bob_as_icusd.saturating_sub(target_bob_icusd);
        // Size the BOB leg off the pool's own (guard-checked) price, not the
        // reference — matches what the pool will actually quote (FIX 2).
        let excess_bob = (excess_icusd_equiv as u128 * 100_000_000u128 / pool_price as u128) as u64;
        if excess_bob <= config.bob_ledger_fee * 2 {
            return;
        }
        match swaps::transfer_from_subaccount(config.bob_ledger, excess_bob, VOLUME_SUBACCOUNT).await {
            Ok(_) => {},
            Err(e) => {
                state::log_activity("volume", &format!("rebalance IcusdBob: BOB transfer failed: {:?}", e));
                return;
            }
        }
        let sell_amount = excess_bob - config.bob_ledger_fee;
        // Expected icUSD out at pool_price, minus slippage tolerance —
        // closes the min_amount_out=0 gap (FIX 2).
        let expected_icusd = (sell_amount as u128 * pool_price as u128 / 100_000_000u128) as u64;
        let min_out = (expected_icusd as u128 * (10_000 - slippage) as u128 / 10_000) as u64;
        match swaps::icpswap_swap(
            config.icpswap_icusd_bob_pool,
            sell_amount,
            bob_is_token0, // selling BOB: zero_for_one = BOB is token0
            min_out,
            config.bob_ledger_fee,
            ICUSD_FEE,
        ).await {
            Ok(icusd_out) => {
                let _ = swaps::transfer_to_subaccount(config.icusd_ledger, icusd_out.saturating_sub(ICUSD_FEE), VOLUME_SUBACCOUNT).await;
                state::log_activity("volume", &format!("rebalance: sold {} BOB via icUSD/BOB pool for {} icUSD", excess_bob, icusd_out));
            },
            Err(e) => {
                state::log_activity("volume", &format!("rebalance IcusdBob: swap failed: {:?}", e));
                let recovery = excess_bob.saturating_sub(config.bob_ledger_fee * 2);
                if recovery > 0 {
                    let _ = swaps::transfer_to_subaccount(config.bob_ledger, recovery, VOLUME_SUBACCOUNT).await;
                }
            }
        }
    } else if bob_pct < 50u64.saturating_sub(drift_threshold) {
        // Too much icUSD — buy BOB to bring back toward 50/50.
        let target_icusd = total / 2;
        let excess_icusd = icusd_bal.saturating_sub(target_icusd);
        if excess_icusd <= ICUSD_FEE * 3 {
            return;
        }
        match swaps::transfer_from_subaccount(config.icusd_ledger, excess_icusd, VOLUME_SUBACCOUNT).await {
            Ok(_) => {},
            Err(e) => {
                state::log_activity("volume", &format!("rebalance IcusdBob: icUSD transfer failed: {:?}", e));
                return;
            }
        }
        let sell_amount = excess_icusd - ICUSD_FEE;
        // Expected BOB out at pool_price, minus slippage tolerance (FIX 2).
        let expected_bob = (sell_amount as u128 * 100_000_000u128 / pool_price as u128) as u64;
        let min_out = (expected_bob as u128 * (10_000 - slippage) as u128 / 10_000) as u64;
        match swaps::icpswap_swap(
            config.icpswap_icusd_bob_pool,
            sell_amount,
            !bob_is_token0, // selling icUSD: zero_for_one = icUSD is token0
            min_out,
            ICUSD_FEE,
            config.bob_ledger_fee,
        ).await {
            Ok(bob_out) => {
                let _ = swaps::transfer_to_subaccount(config.bob_ledger, bob_out.saturating_sub(config.bob_ledger_fee), VOLUME_SUBACCOUNT).await;
                state::log_activity("volume", &format!("rebalance: bought {} BOB with icUSD via icUSD/BOB pool", bob_out));
            },
            Err(e) => {
                state::log_activity("volume", &format!("rebalance IcusdBob: swap failed: {:?}", e));
                let recovery = excess_icusd.saturating_sub(ICUSD_FEE * 2);
                if recovery > 0 {
                    let _ = swaps::transfer_to_subaccount(config.icusd_ledger, recovery, VOLUME_SUBACCOUNT).await;
                }
            }
        }
    }
}

pub async fn run_rebalance(config: &state::BotConfig) {
    let volume = state::read_state(|s| s.volume.clone());
    let drift_threshold = volume.rebalance_drift_pct;

    for pool in [VolumePool::IcusdIcp, VolumePool::ThreeUsdIcp, VolumePool::IcusdBob] {
        // icUSD/BOB is a separate one-hop unwind through its own pool — not
        // the ICP/Rumi multi-hop route the arms below implement.
        if matches!(pool, VolumePool::IcusdBob) {
            rebalance_icusd_bob(config, &volume.icusd_bob, drift_threshold).await;
            continue;
        }

        let pool_config = match &pool {
            VolumePool::IcusdIcp => &volume.icusd_icp,
            VolumePool::ThreeUsdIcp => &volume.three_usd_icp,
            VolumePool::IcusdBob => unreachable!("handled above"),
        };
        if !pool_config.enabled {
            continue;
        }

        let (stable_ledger, _stable_fee) = match &pool {
            VolumePool::IcusdIcp => (config.icusd_ledger, ICUSD_FEE),
            VolumePool::ThreeUsdIcp => (config.three_usd_ledger, THREE_USD_FEE),
            VolumePool::IcusdBob => unreachable!("handled above"),
        };

        let icp_bal = swaps::icrc1_balance_of_subaccount(config.icp_ledger, VOLUME_SUBACCOUNT)
            .await.unwrap_or(0);
        // 3USD ledger ignores subaccounts — check default account
        let stable_bal = if is_3usd(stable_ledger, config) {
            swaps::icrc1_balance_of_default(stable_ledger).await.unwrap_or(0)
        } else {
            swaps::icrc1_balance_of_subaccount(stable_ledger, VOLUME_SUBACCOUNT).await.unwrap_or(0)
        };

        let (icpswap_pool, icp_is_token0) = match &pool {
            VolumePool::IcusdIcp => (config.icpswap_icusd_pool, config.icpswap_icusd_icp_is_token0),
            VolumePool::ThreeUsdIcp => (config.icpswap_3usd_pool, config.icpswap_3usd_icp_is_token0),
            VolumePool::IcusdBob => unreachable!("handled above"),
        };
        let price = match prices::fetch_icpswap_price(icpswap_pool, icp_is_token0).await {
            Ok(p) => p,
            Err(_) => continue,
        };

        if price == 0 { continue; }

        let icp_as_stable = (icp_bal as u128 * price as u128 / 100_000_000u128) as u64;
        let total = icp_as_stable + stable_bal;
        if total == 0 { continue; }

        let icp_pct = icp_as_stable * 100 / total;

        // Rebalance when ICP share drifts beyond 50% ± drift_threshold
        if icp_pct > (50 + drift_threshold) {
            // Too much ICP — sell some to bring back toward 50/50
            let target_icp_stable = total / 2;
            let excess_stable = icp_as_stable.saturating_sub(target_icp_stable);
            let excess_icp = (excess_stable as u128 * 100_000_000u128 / price as u128) as u64;
            if excess_icp > ICP_FEE * 2 {
                match swaps::transfer_from_subaccount(config.icp_ledger, excess_icp, VOLUME_SUBACCOUNT).await {
                    Ok(_) => {},
                    Err(e) => {
                        state::log_activity("volume", &format!("rebalance: ICP transfer failed: {:?}", e));
                        continue;
                    }
                }
                let rumi_pool_id = RUMI_POOL_ID;
                match swaps::rumi_swap(config.rumi_amm, rumi_pool_id, config.icp_ledger, excess_icp - ICP_FEE, 0).await {
                    Ok(three_usd_out) => {
                        match &pool {
                            VolumePool::ThreeUsdIcp => {
                                // 3USD stays in default account (no subaccount support)
                            },
                            VolumePool::IcusdIcp => {
                                match swaps::pool_remove_one_coin(config.rumi_3pool, three_usd_out, 0, 0).await {
                                    Ok(icusd_out) => {
                                        let _ = swaps::transfer_to_subaccount(config.icusd_ledger, icusd_out.saturating_sub(ICUSD_FEE), VOLUME_SUBACCOUNT).await;
                                    },
                                    Err(e) => {
                                        // 3pool redeem failed — 3USD stays in default account (no subaccount support)
                                        state::log_activity("volume", &format!("rebalance: 3pool redeem failed (3USD in default account): {:?}", e));
                                    }
                                }
                            }
                            VolumePool::IcusdBob => unreachable!("handled above"),
                        }
                        state::log_activity("volume", &format!("rebalance: sold {} ICP via Rumi for {:?}", excess_icp, pool));
                    },
                    Err(e) => {
                        state::log_activity("volume", &format!("rebalance: Rumi swap failed: {:?}", e));
                        let _ = swaps::transfer_to_subaccount(config.icp_ledger, excess_icp - ICP_FEE * 2, VOLUME_SUBACCOUNT).await;
                    }
                }
            }
        } else if icp_pct < 50u64.saturating_sub(drift_threshold) {
            // Too much stable — buy ICP to bring back toward 50/50
            let target_stable = total / 2;
            let excess_stable = stable_bal.saturating_sub(target_stable);
            let min_amount = match &pool {
                VolumePool::IcusdIcp => ICUSD_FEE * 3,
                VolumePool::ThreeUsdIcp => THREE_USD_FEE * 3,
                VolumePool::IcusdBob => unreachable!("handled above"),
            };
            if excess_stable > min_amount {
                match &pool {
                    VolumePool::ThreeUsdIcp => {
                        // 3USD is already in default account (no subaccount support) — swap directly
                        let rumi_pool_id = RUMI_POOL_ID;
                        match swaps::rumi_swap(config.rumi_amm, rumi_pool_id, config.three_usd_ledger, excess_stable - THREE_USD_FEE, 0).await {
                            Ok(icp_out) => {
                                let _ = swaps::transfer_to_subaccount(config.icp_ledger, icp_out.saturating_sub(ICP_FEE), VOLUME_SUBACCOUNT).await;
                                state::log_activity("volume", &format!("rebalance: bought {} ICP with 3USD", icp_out));
                            },
                            Err(e) => {
                                // Rumi swap failed — 3USD stays in default account
                                state::log_activity("volume", &format!("rebalance: Rumi swap failed: {:?}", e));
                            },
                        }
                    },
                    VolumePool::IcusdIcp => {
                        match swaps::transfer_from_subaccount(config.icusd_ledger, excess_stable, VOLUME_SUBACCOUNT).await {
                            Ok(_) => {
                                // Build amounts vec for 3pool add_liquidity: [icusd_amount, 0, 0]
                                let amounts = vec![
                                    Nat::from(excess_stable - ICUSD_FEE),
                                    Nat::from(0u64),
                                    Nat::from(0u64),
                                ];
                                match swaps::pool_add_liquidity(config.rumi_3pool, amounts, 0).await {
                                    Ok(lp_out) => {
                                        let rumi_pool_id = RUMI_POOL_ID;
                                        match swaps::rumi_swap(config.rumi_amm, rumi_pool_id, config.three_usd_ledger, lp_out, 0).await {
                                            Ok(icp_out) => {
                                                let _ = swaps::transfer_to_subaccount(config.icp_ledger, icp_out.saturating_sub(ICP_FEE), VOLUME_SUBACCOUNT).await;
                                                state::log_activity("volume", &format!("rebalance: bought {} ICP with icUSD via 3pool+Rumi", icp_out));
                                            },
                                            Err(e) => {
                                                // Rumi swap failed — 3USD stays in default account (no subaccount support)
                                                state::log_activity("volume", &format!("rebalance: Rumi swap failed (3USD in default account): {:?}", e));
                                            },
                                        }
                                    },
                                    Err(e) => {
                                        // 3pool deposit failed — icUSD is stranded. Return to subaccount.
                                        let recovery = excess_stable.saturating_sub(ICUSD_FEE * 2);
                                        if recovery > 0 {
                                            let _ = swaps::transfer_to_subaccount(config.icusd_ledger, recovery, VOLUME_SUBACCOUNT).await;
                                        }
                                        state::log_activity("volume", &format!("rebalance: 3pool deposit failed (icUSD recovered): {:?}", e));
                                    },
                                }
                            },
                            Err(e) => state::log_activity("volume", &format!("rebalance: transfer failed: {:?}", e)),
                        }
                    },
                    VolumePool::IcusdBob => unreachable!("handled above"),
                }
            }
        }
    }

    state::mutate_state(|s| {
        s.volume.last_rebalance_ts = ic_cdk::api::time();
    });
}
