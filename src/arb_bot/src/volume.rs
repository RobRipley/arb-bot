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

    let (icpswap_pool, icp_is_token0, stable_ledger, stable_fee, stable_decimals) = match pool {
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
            18u32,
        ),
    };

    // Fetch price before trade (1 ICP quote)
    let zero_for_one_quote = icp_is_token0;
    let price_before = prices::fetch_icpswap_price(icpswap_pool, zero_for_one_quote)
        .await
        .map_err(|e| format!("Price fetch failed: {}", e))?;

    // Convert trade_size_usd (6-dec) to native token amount
    let (token_in, amount_in_native, zero_for_one, token_in_fee, token_out_fee) = match direction {
        VolumeDirection::BuyIcp => {
            let amount = match stable_decimals {
                8 => trade_size_usd * 100,
                18 => trade_size_usd * 1_000_000_000_000,
                _ => trade_size_usd,
            };
            let zfo = !icp_is_token0;
            (stable_ledger, amount, zfo, stable_fee, ICP_FEE)
        },
        VolumeDirection::SellIcp => {
            let icp_amount = if price_before > 0 {
                let trade_native = match stable_decimals {
                    8 => trade_size_usd * 100,
                    18 => trade_size_usd * 1_000_000_000_000,
                    _ => trade_size_usd,
                };
                (trade_native as u128 * 100_000_000u128 / price_before as u128) as u64
            } else {
                return Err("Zero price".to_string());
            };
            let zfo = icp_is_token0;
            (config.icp_ledger, icp_amount, zfo, ICP_FEE, stable_fee)
        },
    };

    // Step 1: Transfer tokens from volume subaccount to default account
    swaps::transfer_from_subaccount(token_in, amount_in_native, VOLUME_SUBACCOUNT)
        .await
        .map_err(|e| format!("Transfer from subaccount failed: {:?}", e))?;

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
            // Try to return them to the volume subaccount.
            let recovery_amount = amount_in_native.saturating_sub(token_in_fee * 2);
            if recovery_amount > 0 {
                let _ = swaps::transfer_to_subaccount(token_in, recovery_amount, VOLUME_SUBACCOUNT).await;
            }
            return Err(format!("Swap failed (tokens recovered): {:?}", e));
        }
    };

    // Step 3: Transfer output tokens back to volume subaccount
    let token_out = match direction {
        VolumeDirection::BuyIcp => config.icp_ledger,
        VolumeDirection::SellIcp => stable_ledger,
    };
    let out_fee = match direction {
        VolumeDirection::BuyIcp => ICP_FEE,
        VolumeDirection::SellIcp => stable_fee,
    };
    if amount_out > out_fee {
        swaps::transfer_to_subaccount(token_out, amount_out - out_fee, VOLUME_SUBACCOUNT)
            .await
            .map_err(|e| format!("Transfer to subaccount failed: {:?}", e))?;
    }

    // Fetch price after trade
    let price_after = prices::fetch_icpswap_price(icpswap_pool, zero_for_one_quote)
        .await
        .unwrap_or(price_before);

    Ok((amount_in_native, amount_out, price_before, price_after))
}

pub async fn run_volume_cycle() {
    if arb::is_cycle_in_progress() {
        return;
    }

    let already_running = VOLUME_CYCLE_IN_PROGRESS.with(|c| {
        if c.get() { true } else { c.set(true); false }
    });
    if already_running { return; }

    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) { VOLUME_CYCLE_IN_PROGRESS.with(|c| c.set(false)); }
    }
    let _guard = Guard;

    let (volume_config, bot_config) = state::read_state(|s| (s.volume.clone(), s.config.clone()));

    if volume_config.volume_paused {
        return;
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
            }
        });

        if !pool_config.enabled {
            continue;
        }

        // Check per-pool daily cost cap
        if pool_state.daily_cost_usd >= pool_config.daily_cost_cap_usd as i64 {
            continue;
        }

        let (icpswap_pool, icp_is_token0) = state::read_state(|s| match &pool {
            VolumePool::IcusdIcp => (s.config.icpswap_icusd_pool, s.config.icpswap_icusd_icp_is_token0),
            VolumePool::ThreeUsdIcp => (s.config.icpswap_3usd_pool, s.config.icpswap_3usd_icp_is_token0),
        });
        let current_price = match prices::fetch_icpswap_price(icpswap_pool, icp_is_token0).await {
            Ok(p) => p,
            Err(_) => continue,
        };

        if !is_pool_idle(current_price, pool_state.last_price, pool_config.idle_threshold_bps) {
            state::mutate_state(|s| {
                match &pool {
                    VolumePool::IcusdIcp => s.volume.icusd_icp_state.last_price = Some(current_price),
                    VolumePool::ThreeUsdIcp => s.volume.three_usd_icp_state.last_price = Some(current_price),
                }
            });
            continue;
        }

        let input_token = match &pool_state.next_direction {
            VolumeDirection::BuyIcp => match &pool {
                VolumePool::IcusdIcp => bot_config.icusd_ledger,
                VolumePool::ThreeUsdIcp => bot_config.three_usd_ledger,
            },
            VolumeDirection::SellIcp => bot_config.icp_ledger,
        };
        let balance = match swaps::icrc1_balance_of_subaccount(input_token, VOLUME_SUBACCOUNT).await {
            Ok(b) => b,
            Err(_) => continue,
        };

        let trade_size = randomized_trade_size(pool_config.trade_size_usd, pool_config.trade_variance_pct).await;

        let min_native = match (&pool_state.next_direction, &pool) {
            (VolumeDirection::BuyIcp, VolumePool::IcusdIcp) => trade_size * 100,
            (VolumeDirection::BuyIcp, VolumePool::ThreeUsdIcp) => trade_size * 1_000_000_000_000,
            (VolumeDirection::SellIcp, _) => {
                if current_price > 0 {
                    let stable_native = match &pool {
                        VolumePool::IcusdIcp => trade_size * 100,
                        VolumePool::ThreeUsdIcp => trade_size * 1_000_000_000_000,
                    };
                    (stable_native as u128 * 100_000_000u128 / current_price as u128) as u64
                } else {
                    continue;
                }
            }
        };

        if balance < min_native {
            state::log_activity("volume", &format!(
                "skipping {:?} {:?} — insufficient balance ({} < {})",
                pool, pool_state.next_direction, balance, min_native
            ));
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
                        // in: 3USD (18 dec) → 6 dec; out: ICP (8 dec) * price (18 dec 3USD/ICP) → 6 dec
                        let in_6 = (amount_in / 1_000_000_000_000) as u64;
                        let out_6 = (amount_out as u128 * price_before as u128 / 100_000_000u128 / 1_000_000_000_000u128) as u64;
                        (in_6, out_6)
                    },
                    (VolumeDirection::SellIcp, VolumePool::IcusdIcp) => {
                        // in: ICP (8 dec) * price (8 dec icUSD/ICP) → 6 dec; out: icUSD (8 dec) → 6 dec
                        let in_6 = (amount_in as u128 * price_before as u128 / 100_000_000u128 / 100) as u64;
                        let out_6 = amount_out / 100;
                        (in_6, out_6)
                    },
                    (VolumeDirection::SellIcp, VolumePool::ThreeUsdIcp) => {
                        // in: ICP (8 dec) * price (18 dec 3USD/ICP) → 6 dec; out: 3USD (18 dec) → 6 dec
                        let in_6 = (amount_in as u128 * price_before as u128 / 100_000_000u128 / 1_000_000_000_000u128) as u64;
                        let out_6 = (amount_out / 1_000_000_000_000) as u64;
                        (in_6, out_6)
                    },
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
                        },
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
                    };
                    ps.last_price = Some(price_after);
                    ps.next_direction = match ps.next_direction {
                        VolumeDirection::BuyIcp => VolumeDirection::SellIcp,
                        VolumeDirection::SellIcp => VolumeDirection::BuyIcp,
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
            },
            Err(e) => {
                state::log_activity("volume", &format!("{:?} trade failed: {}", pool, e));
            }
        }
    }

    // Daily rebalance check
    let volume = state::read_state(|s| s.volume.clone());
    if now.saturating_sub(volume.last_rebalance_ts) >= NANOS_PER_DAY {
        run_rebalance(&bot_config).await;
    }
}

pub async fn run_rebalance(config: &state::BotConfig) {
    let volume = state::read_state(|s| s.volume.clone());
    let drift_threshold = volume.rebalance_drift_pct;

    for pool in [VolumePool::IcusdIcp, VolumePool::ThreeUsdIcp] {
        let pool_config = match &pool {
            VolumePool::IcusdIcp => &volume.icusd_icp,
            VolumePool::ThreeUsdIcp => &volume.three_usd_icp,
        };
        if !pool_config.enabled {
            continue;
        }

        let (stable_ledger, _stable_fee) = match &pool {
            VolumePool::IcusdIcp => (config.icusd_ledger, ICUSD_FEE),
            VolumePool::ThreeUsdIcp => (config.three_usd_ledger, THREE_USD_FEE),
        };

        let icp_bal = swaps::icrc1_balance_of_subaccount(config.icp_ledger, VOLUME_SUBACCOUNT)
            .await.unwrap_or(0);
        let stable_bal = swaps::icrc1_balance_of_subaccount(stable_ledger, VOLUME_SUBACCOUNT)
            .await.unwrap_or(0);

        let (icpswap_pool, icp_is_token0) = match &pool {
            VolumePool::IcusdIcp => (config.icpswap_icusd_pool, config.icpswap_icusd_icp_is_token0),
            VolumePool::ThreeUsdIcp => (config.icpswap_3usd_pool, config.icpswap_3usd_icp_is_token0),
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

        if icp_pct > drift_threshold {
            // Too much ICP — sell some via Rumi AMM
            let excess_icp = icp_bal / 2;
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
                                let _ = swaps::transfer_to_subaccount(config.three_usd_ledger, three_usd_out.saturating_sub(THREE_USD_FEE), VOLUME_SUBACCOUNT).await;
                            },
                            VolumePool::IcusdIcp => {
                                match swaps::pool_remove_one_coin(config.rumi_3pool, three_usd_out, 0, 0).await {
                                    Ok(icusd_out) => {
                                        let _ = swaps::transfer_to_subaccount(config.icusd_ledger, icusd_out.saturating_sub(ICUSD_FEE), VOLUME_SUBACCOUNT).await;
                                    },
                                    Err(e) => {
                                        // 3pool redeem failed — 3USD is stranded in default account.
                                        // Return it to subaccount so it's not lost.
                                        let _ = swaps::transfer_to_subaccount(config.three_usd_ledger, three_usd_out, VOLUME_SUBACCOUNT).await;
                                        state::log_activity("volume", &format!("rebalance: 3pool redeem failed (3USD recovered): {:?}", e));
                                    }
                                }
                            }
                        }
                        state::log_activity("volume", &format!("rebalance: sold {} ICP via Rumi for {:?}", excess_icp, pool));
                    },
                    Err(e) => {
                        state::log_activity("volume", &format!("rebalance: Rumi swap failed: {:?}", e));
                        let _ = swaps::transfer_to_subaccount(config.icp_ledger, excess_icp - ICP_FEE * 2, VOLUME_SUBACCOUNT).await;
                    }
                }
            }
        } else if (100 - icp_pct) > drift_threshold {
            // Too much stable — buy ICP via Rumi AMM
            let excess_stable = stable_bal / 2;
            let min_amount = match &pool {
                VolumePool::IcusdIcp => ICUSD_FEE * 3,
                VolumePool::ThreeUsdIcp => THREE_USD_FEE * 3,
            };
            if excess_stable > min_amount {
                match &pool {
                    VolumePool::ThreeUsdIcp => {
                        match swaps::transfer_from_subaccount(config.three_usd_ledger, excess_stable, VOLUME_SUBACCOUNT).await {
                            Ok(_) => {
                                let rumi_pool_id = RUMI_POOL_ID;
                                match swaps::rumi_swap(config.rumi_amm, rumi_pool_id, config.three_usd_ledger, excess_stable - THREE_USD_FEE, 0).await {
                                    Ok(icp_out) => {
                                        let _ = swaps::transfer_to_subaccount(config.icp_ledger, icp_out.saturating_sub(ICP_FEE), VOLUME_SUBACCOUNT).await;
                                        state::log_activity("volume", &format!("rebalance: bought {} ICP with 3USD", icp_out));
                                    },
                                    Err(e) => {
                                        // Rumi swap failed — 3USD is stranded. Return to subaccount.
                                        let _ = swaps::transfer_to_subaccount(config.three_usd_ledger, excess_stable, VOLUME_SUBACCOUNT).await;
                                        state::log_activity("volume", &format!("rebalance: Rumi swap failed (3USD recovered): {:?}", e));
                                    },
                                }
                            },
                            Err(e) => state::log_activity("volume", &format!("rebalance: transfer failed: {:?}", e)),
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
                                                // Rumi swap failed — 3USD LP is stranded. Return to subaccount.
                                                let _ = swaps::transfer_to_subaccount(config.three_usd_ledger, lp_out, VOLUME_SUBACCOUNT).await;
                                                state::log_activity("volume", &format!("rebalance: Rumi swap failed (3USD recovered): {:?}", e));
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
                }
            }
        }
    }

    state::mutate_state(|s| {
        s.volume.last_rebalance_ts = ic_cdk::api::time();
    });
}
