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
) -> Result<(u64, u64, u64, u64), String> {
    // Returns: (amount_in, amount_out, price_before, price_after)

    let (icpswap_pool, icp_is_token0, stable_ledger, stable_fee, _stable_decimals) = match pool {
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
        // Wired in task V2 (icUSD/BOB ping-pong execution). Unreachable
        // until then — IcusdBob is never enumerated by `run_volume_cycle`.
        VolumePool::IcusdBob => unreachable!("IcusdBob not yet wired into execute_volume_trade"),
    };

    // Fetch price before trade (1 ICP quote)
    let zero_for_one_quote = icp_is_token0;
    let price_before = prices::fetch_icpswap_price(icpswap_pool, zero_for_one_quote)
        .await
        .map_err(|e| format!("Price fetch failed: {}", e))?;

    // Convert trade_size_usd (6-dec) to native token amount
    let (token_in, amount_in_native, zero_for_one, token_in_fee, token_out_fee) = match direction {
        VolumeDirection::BuyIcp => {
            // All volume-bot stables are 8 decimals; USD is 6-dec → multiply by 100
            let amount = trade_size_usd * 100;
            let zfo = !icp_is_token0;
            (stable_ledger, amount, zfo, stable_fee, ICP_FEE)
        },
        VolumeDirection::SellIcp => {
            let icp_amount = if price_before > 0 {
                // All volume-bot stables are 8 decimals; USD is 6-dec → multiply by 100
                let trade_native = trade_size_usd * 100;
                (trade_native as u128 * 100_000_000u128 / price_before as u128) as u64
            } else {
                return Err("Zero price".to_string());
            };
            let zfo = icp_is_token0;
            (config.icp_ledger, icp_amount, zfo, ICP_FEE, stable_fee)
        },
        // Wired in task V2.
        VolumeDirection::BuyBob | VolumeDirection::SellBob => unreachable!("BuyBob/SellBob not yet wired into execute_volume_trade"),
    };

    // Step 1: Transfer tokens from volume subaccount to default account
    // (3USD ledger ignores subaccounts — tokens are already in default account)
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
        0, // min_amount_out = 0 for volume trades
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
        VolumeDirection::SellIcp => stable_ledger,
        // Wired in task V2.
        VolumeDirection::BuyBob | VolumeDirection::SellBob => unreachable!("BuyBob/SellBob not yet wired into execute_volume_trade"),
    };
    let out_fee = match direction {
        VolumeDirection::BuyIcp => ICP_FEE,
        VolumeDirection::SellIcp => stable_fee,
        // Wired in task V2.
        VolumeDirection::BuyBob | VolumeDirection::SellBob => unreachable!("BuyBob/SellBob not yet wired into execute_volume_trade"),
    };
    if amount_out > out_fee && !is_3usd(token_out, config) {
        let transfer_amount = amount_out - out_fee;
        let mut transferred = false;
        for attempt in 0..3 {
            match swaps::transfer_to_subaccount(token_out, transfer_amount, VOLUME_SUBACCOUNT).await {
                Ok(_) => {
                    // Clear any previously stranded amount on success
                    if matches!(direction, VolumeDirection::BuyIcp) {
                        state::mutate_state(|s| { s.volume_stranded_icp = 0; });
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
            // Mark the ICP as stranded so the arb drain doesn't eat it
            if matches!(direction, VolumeDirection::BuyIcp) {
                state::mutate_state(|s| { s.volume_stranded_icp = transfer_amount; });
            }
            return Err(format!("Transfer to subaccount failed after 3 attempts (funds protected from drain)"));
        }
    }

    // Fetch price after trade
    let price_after = prices::fetch_icpswap_price(icpswap_pool, zero_for_one_quote)
        .await
        .unwrap_or(price_before);

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

    let now = ic_cdk::api::time();

    let should_reset_daily = now.saturating_sub(volume_config.daily_spend_reset_ts) >= NANOS_PER_DAY;
    if should_reset_daily {
        state::mutate_state(|s| {
            s.volume.daily_spend_usd = 0;
            s.volume.daily_spend_reset_ts = now;
            s.volume.icusd_icp_state.daily_cost_usd = 0;
            s.volume.three_usd_icp_state.daily_cost_usd = 0;
        });
    }

    for pool in [VolumePool::IcusdIcp, VolumePool::ThreeUsdIcp] {
        let (pool_config, pool_state) = state::read_state(|s| {
            match &pool {
                VolumePool::IcusdIcp => (s.volume.icusd_icp.clone(), s.volume.icusd_icp_state.clone()),
                VolumePool::ThreeUsdIcp => (s.volume.three_usd_icp.clone(), s.volume.three_usd_icp_state.clone()),
                // Wired in task V2.
                VolumePool::IcusdBob => unreachable!("IcusdBob not yet wired into run_volume_cycle"),
            }
        });

        if !pool_config.enabled {
            outcomes.push(format!("{:?}: skipped (pool disabled)", pool));
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

        let (icpswap_pool, icp_is_token0) = state::read_state(|s| match &pool {
            VolumePool::IcusdIcp => (s.config.icpswap_icusd_pool, s.config.icpswap_icusd_icp_is_token0),
            VolumePool::ThreeUsdIcp => (s.config.icpswap_3usd_pool, s.config.icpswap_3usd_icp_is_token0),
            // Wired in task V2.
            VolumePool::IcusdBob => unreachable!("IcusdBob not yet wired into run_volume_cycle"),
        });
        let current_price = match prices::fetch_icpswap_price(icpswap_pool, icp_is_token0).await {
            Ok(p) => p,
            Err(e) => {
                let msg = format!("{:?}: skipped (price fetch failed: {})", pool, e);
                state::log_activity("volume", &msg);
                outcomes.push(msg);
                continue;
            }
        };

        if !is_pool_idle(current_price, pool_state.last_price, pool_config.idle_threshold_bps) {
            state::mutate_state(|s| {
                match &pool {
                    VolumePool::IcusdIcp => s.volume.icusd_icp_state.last_price = Some(current_price),
                    VolumePool::ThreeUsdIcp => s.volume.three_usd_icp_state.last_price = Some(current_price),
                    // Wired in task V2.
                    VolumePool::IcusdBob => unreachable!("IcusdBob not yet wired into run_volume_cycle"),
                }
            });
            outcomes.push(format!(
                "{:?}: skipped (pool not idle; price moved >{}bps)",
                pool, pool_config.idle_threshold_bps
            ));
            continue;
        }

        let input_token = match &pool_state.next_direction {
            VolumeDirection::BuyIcp => match &pool {
                VolumePool::IcusdIcp => bot_config.icusd_ledger,
                VolumePool::ThreeUsdIcp => bot_config.three_usd_ledger,
                // Wired in task V2.
                VolumePool::IcusdBob => unreachable!("IcusdBob not yet wired into run_volume_cycle"),
            },
            VolumeDirection::SellIcp => bot_config.icp_ledger,
            // Wired in task V2.
            VolumeDirection::BuyBob | VolumeDirection::SellBob => unreachable!("BuyBob/SellBob not yet wired into run_volume_cycle"),
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
            (VolumeDirection::SellIcp, _) => {
                if current_price > 0 {
                    let stable_native = match &pool {
                        VolumePool::IcusdIcp => trade_size * 100,
                        VolumePool::ThreeUsdIcp => trade_size * 100, // 3USD is 8 decimals
                        // Wired in task V2.
                        VolumePool::IcusdBob => unreachable!("IcusdBob not yet wired into run_volume_cycle"),
                    };
                    (stable_native as u128 * 100_000_000u128 / current_price as u128) as u64
                } else {
                    outcomes.push(format!("{:?}: skipped (zero price)", pool));
                    continue;
                }
            }
            // Wired in task V2.
            (VolumeDirection::BuyIcp, VolumePool::IcusdBob)
            | (VolumeDirection::BuyBob, _)
            | (VolumeDirection::SellBob, _) => unreachable!("IcusdBob/BuyBob/SellBob not yet wired into run_volume_cycle"),
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

        match execute_volume_trade(pool.clone(), &pool_state.next_direction, trade_size, &bot_config).await {
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
                    // Wired in task V2.
                    (VolumeDirection::BuyIcp, VolumePool::IcusdBob)
                    | (VolumeDirection::SellIcp, VolumePool::IcusdBob)
                    | (VolumeDirection::BuyBob, _)
                    | (VolumeDirection::SellBob, _) => unreachable!("IcusdBob/BuyBob/SellBob not yet wired into run_volume_cycle"),
                };
                let cost = in_usd as i64 - out_usd as i64;

                let leg = VolumeTradeLeg {
                    timestamp: ic_cdk::api::time(),
                    pool: pool.clone(),
                    direction: pool_state.next_direction.clone(),
                    trade_type: VolumeTradeType::PingPong,
                    token_in: input_token,
                    token_out: match &pool_state.next_direction {
                        VolumeDirection::BuyIcp => bot_config.icp_ledger,
                        VolumeDirection::SellIcp => match &pool {
                            VolumePool::IcusdIcp => bot_config.icusd_ledger,
                            VolumePool::ThreeUsdIcp => bot_config.three_usd_ledger,
                            // Wired in task V2.
                            VolumePool::IcusdBob => unreachable!("IcusdBob not yet wired into run_volume_cycle"),
                        },
                        // Wired in task V2.
                        VolumeDirection::BuyBob | VolumeDirection::SellBob => unreachable!("BuyBob/SellBob not yet wired into run_volume_cycle"),
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
                        // Wired in task V2.
                        VolumePool::IcusdBob => unreachable!("IcusdBob not yet wired into run_volume_cycle"),
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

pub async fn run_rebalance(config: &state::BotConfig) {
    let volume = state::read_state(|s| s.volume.clone());
    let drift_threshold = volume.rebalance_drift_pct;

    for pool in [VolumePool::IcusdIcp, VolumePool::ThreeUsdIcp] {
        let pool_config = match &pool {
            VolumePool::IcusdIcp => &volume.icusd_icp,
            VolumePool::ThreeUsdIcp => &volume.three_usd_icp,
            // Wired in task V3 (separate one-hop unwind branch).
            VolumePool::IcusdBob => unreachable!("IcusdBob not yet wired into run_rebalance"),
        };
        if !pool_config.enabled {
            continue;
        }

        let (stable_ledger, _stable_fee) = match &pool {
            VolumePool::IcusdIcp => (config.icusd_ledger, ICUSD_FEE),
            VolumePool::ThreeUsdIcp => (config.three_usd_ledger, THREE_USD_FEE),
            // Wired in task V3.
            VolumePool::IcusdBob => unreachable!("IcusdBob not yet wired into run_rebalance"),
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
            // Wired in task V3.
            VolumePool::IcusdBob => unreachable!("IcusdBob not yet wired into run_rebalance"),
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
                            // Wired in task V3.
                            VolumePool::IcusdBob => unreachable!("IcusdBob not yet wired into run_rebalance"),
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
                // Wired in task V3.
                VolumePool::IcusdBob => unreachable!("IcusdBob not yet wired into run_rebalance"),
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
                    // Wired in task V3.
                    VolumePool::IcusdBob => unreachable!("IcusdBob not yet wired into run_rebalance"),
                }
            }
        }
    }

    state::mutate_state(|s| {
        s.volume.last_rebalance_ts = ic_cdk::api::time();
    });
}
