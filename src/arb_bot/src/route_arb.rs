//! Six-asset route-arbitrage primitives.
//!
//! Asset and venue identity is deliberately compiled into the canister. An
//! administrator may later disable one of these pins, but cannot turn an
//! arbitrary principal into a route target through configuration alone.

use candid::{CandidType, Deserialize, Principal};
use candid::Nat;
use futures::StreamExt;
use icrc_ledger_types::icrc1::account::Account;
use num_traits::ToPrimitive;
use serde::Serialize;

#[derive(
    CandidType, Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub enum Asset {
    IcUsd,
    CkUsdt,
    CkUsdc,
    Icp,
    CkBtc,
    CkEth,
}

impl Asset {
    pub const ALL: [Asset; 6] = [
        Asset::IcUsd,
        Asset::CkUsdt,
        Asset::CkUsdc,
        Asset::Icp,
        Asset::CkBtc,
        Asset::CkEth,
    ];

    pub fn is_stable(self) -> bool {
        matches!(self, Asset::IcUsd | Asset::CkUsdt | Asset::CkUsdc)
    }

    pub fn index(self) -> usize {
        match self {
            Asset::IcUsd => 0,
            Asset::CkUsdt => 1,
            Asset::CkUsdc => 2,
            Asset::Icp => 3,
            Asset::CkBtc => 4,
            Asset::CkEth => 5,
        }
    }

    pub fn decimals(self) -> u8 {
        asset_pins()[self.index()].decimals
    }

    pub fn symbol(self) -> &'static str {
        asset_pins()[self.index()].symbol
    }
}

#[derive(
    CandidType, Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord,
)]
pub enum AssetRole {
    StableSettlement,
    IcpPrincipal,
    PassThroughOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetPin {
    pub asset: Asset,
    pub ledger: Principal,
    pub symbol: &'static str,
    pub decimals: u8,
    pub role: AssetRole,
}

#[derive(
    CandidType, Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord,
)]
pub enum VenueKind {
    Rumi3Pool,
    IcpSwap,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolPin {
    pub pool_id: &'static str,
    pub principal: Principal,
    pub venue: VenueKind,
    pub assets: Vec<Asset>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectedEdge {
    pub edge_id: String,
    pub pool_id: &'static str,
    pub pool_principal: Principal,
    pub venue: VenueKind,
    pub from: Asset,
    pub to: Asset,
}

#[derive(
    CandidType, Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord,
)]
pub enum CandidateClass {
    StablePar,
    StableSettledCrossAsset,
    IcpReturning,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub route_id: String,
    pub canonical_cycle_id: Option<String>,
    pub candidate_class: CandidateClass,
    pub asset_path: Vec<Asset>,
    pub edges: Vec<DirectedEdge>,
}

impl Route {
    pub fn start_asset(&self) -> Asset {
        self.asset_path[0]
    }

    pub fn end_asset(&self) -> Asset {
        self.asset_path[self.asset_path.len() - 1]
    }
}

fn principal(text: &str) -> Principal {
    Principal::from_text(text).expect("compile-time route principal must be valid")
}

pub fn asset_pins() -> Vec<AssetPin> {
    vec![
        AssetPin { asset: Asset::IcUsd, ledger: principal("t6bor-paaaa-aaaap-qrd5q-cai"), symbol: "icUSD", decimals: 8, role: AssetRole::StableSettlement },
        AssetPin { asset: Asset::CkUsdt, ledger: principal("cngnf-vqaaa-aaaar-qag4q-cai"), symbol: "ckUSDT", decimals: 6, role: AssetRole::StableSettlement },
        AssetPin { asset: Asset::CkUsdc, ledger: principal("xevnm-gaaaa-aaaar-qafnq-cai"), symbol: "ckUSDC", decimals: 6, role: AssetRole::StableSettlement },
        AssetPin { asset: Asset::Icp, ledger: principal("ryjl3-tyaaa-aaaaa-aaaba-cai"), symbol: "ICP", decimals: 8, role: AssetRole::IcpPrincipal },
        AssetPin { asset: Asset::CkBtc, ledger: principal("mxzaz-hqaaa-aaaar-qaada-cai"), symbol: "ckBTC", decimals: 8, role: AssetRole::PassThroughOnly },
        AssetPin { asset: Asset::CkEth, ledger: principal("ss2fx-dyaaa-aaaar-qacoq-cai"), symbol: "ckETH", decimals: 18, role: AssetRole::PassThroughOnly },
    ]
}

pub fn pool_pins() -> Vec<PoolPin> {
    use Asset::*;
    let definitions: [(&str, &str, VenueKind, &[Asset]); 15] = [
        ("rumi-3pool", "fohh4-yyaaa-aaaap-qtkpa-cai", VenueKind::Rumi3Pool, &[IcUsd, CkUsdt, CkUsdc]),
        ("icpswap-icp-ckusdc", "mohjv-bqaaa-aaaag-qjyia-cai", VenueKind::IcpSwap, &[Icp, CkUsdc]),
        ("icpswap-icp-icusd", "nqxwe-hiaaa-aaaar-qb5yq-cai", VenueKind::IcpSwap, &[Icp, IcUsd]),
        ("icpswap-icp-ckusdt", "hkstf-6iaaa-aaaag-qkcoq-cai", VenueKind::IcpSwap, &[Icp, CkUsdt]),
        ("icpswap-icusd-ckusdt", "jogrm-gqaaa-aaaar-qcg2a-cai", VenueKind::IcpSwap, &[IcUsd, CkUsdt]),
        ("icpswap-icusd-ckusdc", "eb25l-dyaaa-aaaar-qb4lq-cai", VenueKind::IcpSwap, &[IcUsd, CkUsdc]),
        ("icpswap-ckusdt-ckusdc", "heq6n-fyaaa-aaaag-qkcpq-cai", VenueKind::IcpSwap, &[CkUsdt, CkUsdc]),
        ("icpswap-ckbtc-icp", "xmiu5-jqaaa-aaaag-qbz7q-cai", VenueKind::IcpSwap, &[CkBtc, Icp]),
        ("icpswap-icp-cketh", "angxa-baaaa-aaaag-qcvnq-cai", VenueKind::IcpSwap, &[Icp, CkEth]),
        ("icpswap-ckbtc-cketh", "akhru-myaaa-aaaag-qcvna-cai", VenueKind::IcpSwap, &[CkBtc, CkEth]),
        ("icpswap-cketh-ckusdc", "mvcvq-3iaaa-aaaag-qjykq-cai", VenueKind::IcpSwap, &[CkEth, CkUsdc]),
        ("icpswap-ckbtc-ckusdc", "mhecj-xyaaa-aaaag-qjyjq-cai", VenueKind::IcpSwap, &[CkBtc, CkUsdc]),
        ("icpswap-ckbtc-icusd", "jhf2q-qyaaa-aaaar-qcg3q-cai", VenueKind::IcpSwap, &[CkBtc, IcUsd]),
        ("icpswap-ckusdt-ckbtc", "ipfno-pqaaa-aaaag-qkevq-cai", VenueKind::IcpSwap, &[CkUsdt, CkBtc]),
        ("icpswap-cketh-icusd", "jjhxy-liaaa-aaaar-qcg2q-cai", VenueKind::IcpSwap, &[CkEth, IcUsd]),
    ];
    definitions
        .into_iter()
        .map(|(pool_id, principal_text, venue, assets)| PoolPin {
            pool_id,
            principal: principal(principal_text),
            venue,
            assets: assets.to_vec(),
        })
        .collect()
}

pub fn directed_edges() -> Vec<DirectedEdge> {
    let mut edges = Vec::new();
    for pool in pool_pins() {
        for (index, from) in pool.assets.iter().enumerate() {
            for to in pool.assets.iter().skip(index + 1) {
                for (source, destination) in [(*from, *to), (*to, *from)] {
                    edges.push(DirectedEdge {
                        edge_id: format!("{}:{:?}>{:?}", pool.pool_id, source, destination),
                        pool_id: pool.pool_id,
                        pool_principal: pool.principal,
                        venue: pool.venue,
                        from: source,
                        to: destination,
                    });
                }
            }
        }
    }
    edges.sort_by(|left, right| left.edge_id.cmp(&right.edge_id));
    edges
}

pub fn canonical_cycle_id<S: AsRef<str>>(edge_ids: &[S]) -> String {
    if edge_ids.is_empty() {
        return String::new();
    }
    let ids: Vec<&str> = edge_ids.iter().map(AsRef::as_ref).collect();
    (0..ids.len())
        .map(|offset| {
            (0..ids.len())
                .map(|index| ids[(offset + index) % ids.len()])
                .collect::<Vec<_>>()
                .join("|")
        })
        .min()
        .expect("a non-empty cycle has a rotation")
}

fn classify_path(path: &[Asset]) -> Option<CandidateClass> {
    let start = *path.first()?;
    let end = *path.last()?;
    if start.is_stable() && end.is_stable() {
        return Some(if path.iter().all(|asset| asset.is_stable()) {
            CandidateClass::StablePar
        } else {
            CandidateClass::StableSettledCrossAsset
        });
    }
    if start == Asset::Icp
        && end == Asset::Icp
        && path.len() > 2
        && path[1..path.len() - 1].iter().all(|asset| asset.is_stable())
    {
        return Some(CandidateClass::IcpReturning);
    }
    None
}

fn walk_routes(
    start: Asset,
    max_legs: usize,
    all_edges: &[DirectedEdge],
    path: &mut Vec<Asset>,
    selected: &mut Vec<DirectedEdge>,
    routes: &mut Vec<Route>,
) {
    if let Some(candidate_class) = classify_path(path).filter(|_| !selected.is_empty()) {
        let route_id = selected.iter().map(|edge| edge.edge_id.as_str()).collect::<Vec<_>>().join("|");
        let is_cycle = path.last().copied() == Some(start);
        let canonical = is_cycle.then(|| {
            let edge_ids = selected.iter().map(|edge| edge.edge_id.as_str()).collect::<Vec<_>>();
            canonical_cycle_id(&edge_ids)
        });
        routes.push(Route {
            route_id,
            canonical_cycle_id: canonical,
            candidate_class,
            asset_path: path.clone(),
            edges: selected.clone(),
        });
    }

    if selected.len() == max_legs || (!selected.is_empty() && path.last().copied() == Some(start)) {
        return;
    }

    let current = *path.last().expect("route path always has its start asset");
    for edge in all_edges.iter().filter(|edge| edge.from == current) {
        if selected.iter().any(|prior| prior.edge_id == edge.edge_id || prior.pool_id == edge.pool_id) {
            continue;
        }
        let closes_cycle = edge.to == start;
        if path.contains(&edge.to) && !closes_cycle {
            continue;
        }
        path.push(edge.to);
        selected.push(edge.clone());
        walk_routes(start, max_legs, all_edges, path, selected, routes);
        selected.pop();
        path.pop();
    }
}

pub fn enumerate_routes(max_legs: u8) -> Result<Vec<Route>, String> {
    if !(1..=4).contains(&max_legs) {
        return Err("max_route_legs must be between 1 and 4".to_string());
    }
    let all_edges = directed_edges();
    let mut routes = Vec::new();
    for start in [Asset::IcUsd, Asset::CkUsdt, Asset::CkUsdc, Asset::Icp] {
        walk_routes(
            start,
            max_legs as usize,
            &all_edges,
            &mut vec![start],
            &mut Vec::new(),
            &mut routes,
        );
    }
    routes.sort_by(|left, right| left.route_id.cmp(&right.route_id));
    Ok(routes)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetAmounts {
    values: [Option<u128>; 6],
}

impl AssetAmounts {
    pub fn unknown() -> Self {
        Self { values: [None; 6] }
    }

    pub fn zero() -> Self {
        Self { values: [Some(0); 6] }
    }

    pub fn get(&self, asset: Asset) -> Option<u128> {
        self.values[asset.index()]
    }

    pub fn set(&mut self, asset: Asset, value: Option<u128>) {
        self.values[asset.index()] = value;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReservationTotals {
    pub held: AssetAmounts,
    pub active: AssetAmounts,
    pub non_route: AssetAmounts,
    pub whole_asset_frozen: [bool; 6],
}

impl Default for ReservationTotals {
    fn default() -> Self {
        Self {
            held: AssetAmounts::zero(),
            active: AssetAmounts::zero(),
            non_route: AssetAmounts::zero(),
            whole_asset_frozen: [false; 6],
        }
    }
}

impl ReservationTotals {
    pub fn set_whole_asset_frozen(&mut self, asset: Asset, frozen: bool) {
        self.whole_asset_frozen[asset.index()] = frozen;
    }

    pub fn whole_asset_frozen(&self, asset: Asset) -> bool {
        self.whole_asset_frozen[asset.index()]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InventoryBands {
    floors: [u128; 6],
    ceilings: [u128; 6],
}

impl InventoryBands {
    pub fn unbounded() -> Self {
        Self { floors: [0; 6], ceilings: [u128::MAX; 6] }
    }

    pub fn set(&mut self, asset: Asset, floor: u128, ceiling: u128) {
        self.floors[asset.index()] = floor;
        self.ceilings[asset.index()] = ceiling;
    }

    pub fn floor(&self, asset: Asset) -> u128 {
        self.floors[asset.index()]
    }

    pub fn ceiling(&self, asset: Asset) -> u128 {
        self.ceilings[asset.index()]
    }
}

pub fn available_native(
    asset: Asset,
    ledger_balance: Option<u128>,
    reservations: &ReservationTotals,
) -> Result<u128, String> {
    if reservations.whole_asset_frozen(asset) {
        return Err(format!("{:?} is frozen by an unresolved ownership reservation", asset));
    }
    let mut available = ledger_balance.ok_or_else(|| format!("unknown {:?} ledger balance", asset))?;
    for (name, values) in [
        ("held", &reservations.held),
        ("active", &reservations.active),
        ("non-route", &reservations.non_route),
    ] {
        let reserved = values.get(asset).ok_or_else(|| format!("unknown {name} reservation for {:?}", asset))?;
        available = available
            .checked_sub(reserved)
            .ok_or_else(|| format!("{name} reservation exceeds {:?} ledger balance", asset))?;
    }
    Ok(available)
}

pub fn par_usd_6dec_checked(amount_native: u128, decimals: u8) -> Result<i128, String> {
    let amount = if decimals >= 6 {
        let exponent = u32::from(decimals - 6);
        let divisor = 10u128.checked_pow(exponent).ok_or("unsupported decimal exponent")?;
        amount_native / divisor
    } else {
        let exponent = u32::from(6 - decimals);
        let multiplier = 10u128.checked_pow(exponent).ok_or("unsupported decimal exponent")?;
        amount_native.checked_mul(multiplier).ok_or("stable-par multiplication overflow")?
    };
    i128::try_from(amount).map_err(|_| "stable-par result exceeds signed P&L range".to_string())
}

pub fn net_profit_bps_checked(net_profit: i128, principal: u128) -> Result<i64, String> {
    if principal == 0 {
        return Err("principal must be greater than zero".to_string());
    }
    let principal = i128::try_from(principal).map_err(|_| "principal exceeds signed P&L range")?;
    let numerator = net_profit.checked_mul(10_000).ok_or("profit-bps multiplication overflow")?;
    i64::try_from(numerator / principal).map_err(|_| "profit-bps result exceeds report range".to_string())
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub enum ProfitDomain {
    StableParUsd6Dec,
    IcpE8s,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuoteLeg {
    pub edge_id: String,
    pub from: Asset,
    pub to: Asset,
    pub wallet_before: u128,
    pub entry_ledger_fee: u128,
    pub venue_input: u128,
    pub gross_output: u128,
    pub output_ledger_fee: u128,
    pub wallet_after: u128,
    pub dex_fee_native: u128,
    pub full_fill: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteQuote {
    pub route_id: String,
    pub canonical_cycle_id: Option<String>,
    pub start_asset: Asset,
    pub end_asset: Asset,
    pub asset_path: Vec<Asset>,
    pub principal_native: u128,
    pub legs: Vec<QuoteLeg>,
    pub allowance_sufficient: Option<bool>,
    pub quoted_at_ns: u64,
    pub size_ladder_index: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateEvaluation {
    pub route_id: String,
    pub canonical_cycle_id: Option<String>,
    pub start_asset: Asset,
    pub end_asset: Asset,
    pub profit_domain: ProfitDomain,
    pub principal_native: u128,
    pub net_profit_native: i128,
    pub net_profit_bps: i64,
    pub leg_count: u8,
    pub size_ladder_index: u8,
    pub par_assumption: bool,
    pub eligible: bool,
    pub rejection_reason: Option<String>,
}

fn rejected(quote: &RouteQuote, domain: ProfitDomain, reason: impl Into<String>) -> CandidateEvaluation {
    CandidateEvaluation {
        route_id: quote.route_id.clone(),
        canonical_cycle_id: quote.canonical_cycle_id.clone(),
        start_asset: quote.start_asset,
        end_asset: quote.end_asset,
        profit_domain: domain,
        principal_native: quote.principal_native,
        net_profit_native: 0,
        net_profit_bps: 0,
        leg_count: quote.legs.len().try_into().unwrap_or(u8::MAX),
        size_ladder_index: quote.size_ladder_index,
        par_assumption: quote.start_asset != quote.end_asset,
        eligible: false,
        rejection_reason: Some(reason.into()),
    }
}

fn quote_profit(quote: &RouteQuote) -> Result<(ProfitDomain, i128, i64), String> {
    let final_amount = quote.legs.last().ok_or("route has no quoted legs")?.wallet_after;
    let (domain, profit, principal) = if quote.start_asset.is_stable() && quote.end_asset.is_stable() {
        let start = par_usd_6dec_checked(quote.principal_native, quote.start_asset.decimals())?;
        let end = par_usd_6dec_checked(final_amount, quote.end_asset.decimals())?;
        (ProfitDomain::StableParUsd6Dec, end.checked_sub(start).ok_or("stable profit overflow")?, u128::try_from(start).map_err(|_| "invalid stable principal")?)
    } else if quote.start_asset == Asset::Icp && quote.end_asset == Asset::Icp {
        let final_amount = i128::try_from(final_amount).map_err(|_| "ICP output exceeds signed P&L range")?;
        let principal = i128::try_from(quote.principal_native).map_err(|_| "ICP principal exceeds signed P&L range")?;
        (ProfitDomain::IcpE8s, final_amount.checked_sub(principal).ok_or("ICP profit overflow")?, quote.principal_native)
    } else {
        return Err("candidate endpoints do not share an admitted profit domain".to_string());
    };
    let bps = net_profit_bps_checked(profit, principal)?;
    Ok((domain, profit, bps))
}

pub fn evaluate_candidate(
    quote: &RouteQuote,
    balances: &AssetAmounts,
    reservations: &ReservationTotals,
    bands: &InventoryBands,
    min_stable_profit_usd_6dec: i128,
    min_stable_profit_bps: i64,
    min_icp_profit_e8s: i128,
    min_icp_profit_bps: i64,
) -> CandidateEvaluation {
    let fallback_domain = if quote.start_asset == Asset::Icp { ProfitDomain::IcpE8s } else { ProfitDomain::StableParUsd6Dec };
    if quote.principal_native == 0 {
        return rejected(quote, fallback_domain, "principal must be greater than zero");
    }
    if quote.allowance_sufficient != Some(true) {
        return rejected(quote, fallback_domain, "allowance unknown or insufficient");
    }
    let mut expected_before = quote.principal_native;
    let mut expected_from = quote.start_asset;
    for (leg_index, leg) in quote.legs.iter().enumerate() {
        if !leg.full_fill {
            return rejected(quote, fallback_domain, "route contains a non-full-fill leg");
        }
        if leg.from != expected_from || leg.wallet_before != expected_before {
            return rejected(quote, fallback_domain, "quoted legs do not form an exact native-unit chain");
        }
        if leg.wallet_before.checked_sub(leg.entry_ledger_fee) != Some(leg.venue_input)
            || leg.gross_output.checked_sub(leg.output_ledger_fee) != Some(leg.wallet_after)
        {
            return rejected(quote, fallback_domain, "ledger-fee recurrence mismatch");
        }
        let current = match balances.get(leg.to) {
            Some(value) => value,
            None => return rejected(quote, fallback_domain, format!("unknown {:?} ledger balance", leg.to)),
        };
        let current_after_start_debit = if leg_index + 1 == quote.legs.len() && leg.to == quote.start_asset {
            match current.checked_sub(quote.principal_native) {
                Some(value) => value,
                None => return rejected(quote, fallback_domain, "starting debit exceeds ledger balance"),
            }
        } else {
            current
        };
        let exposure = match current_after_start_debit.checked_add(leg.wallet_after) {
            Some(value) => value,
            None => return rejected(quote, fallback_domain, "inventory exposure overflow"),
        };
        if exposure > bands.ceiling(leg.to) {
            return rejected(quote, fallback_domain, format!("{:?} inventory ceiling exceeded", leg.to));
        }
        if leg_index + 1 == quote.legs.len()
            && quote.end_asset != quote.start_asset
            && exposure < bands.floor(leg.to)
        {
            return rejected(quote, fallback_domain, format!("{:?} inventory floor breached", leg.to));
        }
        expected_before = leg.wallet_after;
        expected_from = leg.to;
    }
    if expected_from != quote.end_asset {
        return rejected(quote, fallback_domain, "quoted terminal asset does not match route");
    }
    let available = match available_native(quote.start_asset, balances.get(quote.start_asset), reservations) {
        Ok(value) => value,
        Err(reason) => return rejected(quote, fallback_domain, reason),
    };
    let remaining = match available.checked_sub(quote.principal_native) {
        Some(value) => value,
        None => return rejected(quote, fallback_domain, "insufficient unencumbered starting balance"),
    };
    if remaining < bands.floor(quote.start_asset) {
        return rejected(quote, fallback_domain, "starting asset inventory floor breached");
    }
    let (domain, profit, bps) = match quote_profit(quote) {
        Ok(value) => value,
        Err(reason) => return rejected(quote, fallback_domain, reason),
    };
    let threshold_reason = match domain {
        ProfitDomain::StableParUsd6Dec if profit < min_stable_profit_usd_6dec => Some("below stable absolute-profit threshold"),
        ProfitDomain::StableParUsd6Dec if bps < min_stable_profit_bps => Some("below stable bps threshold"),
        ProfitDomain::IcpE8s if profit < min_icp_profit_e8s => Some("below ICP absolute-profit threshold"),
        ProfitDomain::IcpE8s if bps < min_icp_profit_bps => Some("below ICP bps threshold"),
        _ => None,
    };
    CandidateEvaluation {
        route_id: quote.route_id.clone(), canonical_cycle_id: quote.canonical_cycle_id.clone(),
        start_asset: quote.start_asset, end_asset: quote.end_asset, profit_domain: domain,
        principal_native: quote.principal_native, net_profit_native: profit, net_profit_bps: bps,
        leg_count: quote.legs.len().try_into().unwrap_or(u8::MAX), size_ladder_index: quote.size_ladder_index,
        par_assumption: quote.start_asset != quote.end_asset, eligible: threshold_reason.is_none(),
        rejection_reason: threshold_reason.map(str::to_string),
    }
}

pub fn rank_book(candidates: &mut [CandidateEvaluation]) {
    candidates.sort_by(|left, right| {
        right.net_profit_native.cmp(&left.net_profit_native)
            .then_with(|| right.net_profit_bps.cmp(&left.net_profit_bps))
            .then_with(|| left.leg_count.cmp(&right.leg_count))
            .then_with(|| left.route_id.cmp(&right.route_id))
            .then_with(|| left.size_ladder_index.cmp(&right.size_ladder_index))
            .then_with(|| left.principal_native.cmp(&right.principal_native))
    });
}

pub const HARD_MAX_ROUTE_LEGS: u8 = 4;
pub const HARD_MAX_SIZE_LADDER_ENTRIES: u8 = 16;
pub const HARD_MAX_CONCURRENT_QUOTES: u8 = 16;
pub const HARD_MAX_QUOTE_AGE_NS: u64 = 60_000_000_000;
pub const HARD_MAX_SETTLEMENT_TIMEOUT_NS: u64 = 86_400_000_000_000;
pub const HARD_MAX_RECONCILIATION_QUERIES_PER_CYCLE: u8 = 32;
pub const HARD_MAX_PAGE_SIZE: u16 = 100;
pub const HARD_MAX_OPEN_HELD_POSITIONS: u16 = 256;
pub const HARD_MAX_OPEN_NON_ROUTE_RESERVATIONS: u16 = 256;
pub const HARD_MAX_EXECUTION_RECORD_BYTES: u32 = 65_536;
pub const HARD_MAX_RECONCILIATION_EVIDENCE_ITEMS: u8 = 64;
pub const HARD_MAX_TERMINAL_EXECUTION_RECORDS: u32 = 10_000;

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct AssetControlV1 {
    pub asset: Asset,
    pub enabled: bool,
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct PoolControlV1 {
    pub pool_id: String,
    pub enabled: bool,
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct AssetInventoryBandV1 {
    pub asset: Asset,
    pub floor_native: u64,
    pub ceiling_native: u64,
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct RouteArbConfigV1 {
    pub enabled: bool,
    pub dry_run: bool,
    pub stable_book_enabled: bool,
    pub icp_book_enabled: bool,
    pub asset_controls: Vec<AssetControlV1>,
    pub pool_controls: Vec<PoolControlV1>,
    pub stable_size_ladder: Vec<u64>,
    pub icp_size_ladder: Vec<u64>,
    pub max_route_legs: u8,
    pub max_quote_calls_per_observation: u16,
    pub max_concurrent_quote_calls: u8,
    pub max_stable_principal_usd_6dec: u64,
    pub max_icp_principal_e8s: u64,
    pub min_stable_profit_usd_6dec: u64,
    pub min_stable_profit_bps: u32,
    pub min_icp_profit_e8s: u64,
    pub min_icp_profit_bps: u32,
    pub inventory_bands: Vec<AssetInventoryBandV1>,
    pub quote_max_age_ns: u64,
    pub settlement_timeout_ns: u64,
    pub reconciliation_queries_per_cycle: u8,
    pub max_open_held_positions: u16,
    pub max_open_non_route_reservations: u16,
    pub max_terminal_execution_records: u32,
    pub max_execution_record_bytes: u32,
    pub max_reconciliation_evidence_items: u8,
}

impl Default for RouteArbConfigV1 {
    fn default() -> Self {
        Self {
            enabled: false,
            dry_run: true,
            stable_book_enabled: true,
            icp_book_enabled: true,
            asset_controls: Asset::ALL.into_iter().map(|asset| AssetControlV1 { asset, enabled: true }).collect(),
            pool_controls: pool_pins().into_iter().map(|pool| PoolControlV1 { pool_id: pool.pool_id.to_string(), enabled: true }).collect(),
            stable_size_ladder: vec![1_000_000, 5_000_000, 10_000_000, 40_000_000],
            icp_size_ladder: vec![100_000_000, 500_000_000, 1_000_000_000],
            // Three-leg routes are the measured default universe. Four-leg
            // routes remain supported but require an explicit expansion.
            max_route_legs: 3,
            max_quote_calls_per_observation: 4_096,
            max_concurrent_quote_calls: 8,
            max_stable_principal_usd_6dec: 40_000_000,
            max_icp_principal_e8s: 1_000_000_000,
            min_stable_profit_usd_6dec: 50_000,
            min_stable_profit_bps: 50,
            min_icp_profit_e8s: 10_000,
            min_icp_profit_bps: 50,
            inventory_bands: Asset::ALL.into_iter().map(|asset| AssetInventoryBandV1 {
                asset,
                floor_native: 0,
                ceiling_native: u64::MAX,
            }).collect(),
            quote_max_age_ns: 30_000_000_000,
            settlement_timeout_ns: 3_600_000_000_000,
            reconciliation_queries_per_cycle: 16,
            max_open_held_positions: HARD_MAX_OPEN_HELD_POSITIONS,
            max_open_non_route_reservations: HARD_MAX_OPEN_NON_ROUTE_RESERVATIONS,
            max_terminal_execution_records: HARD_MAX_TERMINAL_EXECUTION_RECORDS,
            max_execution_record_bytes: HARD_MAX_EXECUTION_RECORD_BYTES,
            max_reconciliation_evidence_items: HARD_MAX_RECONCILIATION_EVIDENCE_ITEMS,
        }
    }
}

fn exact_asset_set<T>(items: &[T], asset_of: impl Fn(&T) -> Asset) -> bool {
    let actual: std::collections::BTreeSet<_> = items.iter().map(asset_of).collect();
    actual == Asset::ALL.into_iter().collect() && items.len() == Asset::ALL.len()
}

pub fn validate_route_config(config: &RouteArbConfigV1) -> Result<(), String> {
    if !(1..=HARD_MAX_ROUTE_LEGS).contains(&config.max_route_legs) {
        return Err("max_route_legs must be between 1 and 4".to_string());
    }
    for (name, ladder, maximum) in [
        ("stable", &config.stable_size_ladder, config.max_stable_principal_usd_6dec),
        ("ICP", &config.icp_size_ladder, config.max_icp_principal_e8s),
    ] {
        if ladder.is_empty() || ladder.len() > usize::from(HARD_MAX_SIZE_LADDER_ENTRIES) {
            return Err(format!("{name} size ladder must contain 1..=16 entries"));
        }
        if ladder.iter().any(|value| *value == 0 || *value > maximum)
            || ladder.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(format!("{name} size ladder must be strictly increasing, nonzero, and within its principal cap"));
        }
    }
    if config.max_quote_calls_per_observation == 0 {
        return Err("max_quote_calls_per_observation must be nonzero".to_string());
    }
    if !(1..=HARD_MAX_CONCURRENT_QUOTES).contains(&config.max_concurrent_quote_calls) {
        return Err("max_concurrent_quote_calls outside 1..=16".to_string());
    }
    if !(1..=HARD_MAX_QUOTE_AGE_NS).contains(&config.quote_max_age_ns) {
        return Err("quote_max_age_ns outside immutable bounds".to_string());
    }
    if !(1..=HARD_MAX_SETTLEMENT_TIMEOUT_NS).contains(&config.settlement_timeout_ns) {
        return Err("settlement_timeout_ns outside immutable bounds".to_string());
    }
    if !(1..=HARD_MAX_RECONCILIATION_QUERIES_PER_CYCLE).contains(&config.reconciliation_queries_per_cycle) {
        return Err("reconciliation_queries_per_cycle outside immutable bounds".to_string());
    }
    if config.max_open_held_positions == 0 || config.max_open_held_positions > HARD_MAX_OPEN_HELD_POSITIONS
        || config.max_open_non_route_reservations == 0 || config.max_open_non_route_reservations > HARD_MAX_OPEN_NON_ROUTE_RESERVATIONS
        || config.max_terminal_execution_records == 0 || config.max_terminal_execution_records > HARD_MAX_TERMINAL_EXECUTION_RECORDS
        || config.max_execution_record_bytes == 0 || config.max_execution_record_bytes > HARD_MAX_EXECUTION_RECORD_BYTES
        || config.max_reconciliation_evidence_items == 0 || config.max_reconciliation_evidence_items > HARD_MAX_RECONCILIATION_EVIDENCE_ITEMS
    {
        return Err("configured durable-storage bound exceeds immutable ceiling".to_string());
    }
    if !exact_asset_set(&config.asset_controls, |item| item.asset)
        || !exact_asset_set(&config.inventory_bands, |item| item.asset)
    {
        return Err("asset controls and inventory bands must contain each code-pinned asset exactly once".to_string());
    }
    if config.inventory_bands.iter().any(|band| band.floor_native > band.ceiling_native) {
        return Err("inventory floor cannot exceed ceiling".to_string());
    }
    let expected_pools: std::collections::BTreeSet<_> = pool_pins().into_iter().map(|pool| pool.pool_id.to_string()).collect();
    let actual_pools: std::collections::BTreeSet<_> = config.pool_controls.iter().map(|pool| pool.pool_id.clone()).collect();
    if config.pool_controls.len() != expected_pools.len() || actual_pools != expected_pools {
        return Err("pool controls must contain each code-pinned pool exactly once".to_string());
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedgerReadResult {
    pub symbol: Option<String>,
    pub decimals: Option<u8>,
    pub fee_native: Option<u128>,
    pub balance_native: Option<u128>,
    pub error: Option<String>,
}

impl LedgerReadResult {
    pub fn ok(symbol: &str, decimals: u8, fee_native: u128, balance_native: u128) -> Self {
        Self { symbol: Some(symbol.to_string()), decimals: Some(decimals), fee_native: Some(fee_native), balance_native: Some(balance_native), error: None }
    }

    pub fn failed(error: &str) -> Self {
        Self { symbol: None, decimals: None, fee_native: None, balance_native: None, error: Some(error.to_string()) }
    }
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct WalletAssetBalanceV1 {
    pub asset: Asset,
    pub symbol: String,
    pub ledger: Principal,
    pub expected_decimals: u8,
    pub observed_decimals: Option<u8>,
    pub ledger_fee_native: Option<u128>,
    pub balance_native: Option<u128>,
    pub metadata_valid: bool,
    pub error: Option<String>,
}

pub fn wallet_rows_from_results(reads: [LedgerReadResult; 6]) -> Vec<WalletAssetBalanceV1> {
    asset_pins().into_iter().zip(reads).map(|(pin, read)| {
        let metadata_valid = read.error.is_none()
            && read.symbol.as_deref() == Some(pin.symbol)
            && read.decimals == Some(pin.decimals);
        let error = read.error.or_else(|| (!metadata_valid).then(|| "ledger metadata does not match code-pinned identity".to_string()));
        WalletAssetBalanceV1 {
            asset: pin.asset,
            symbol: pin.symbol.to_string(),
            ledger: pin.ledger,
            expected_decimals: pin.decimals,
            observed_decimals: read.decimals,
            ledger_fee_native: read.fee_native,
            balance_native: read.balance_native,
            metadata_valid,
            error,
        }
    }).collect()
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct RouteArbStatusV1 {
    pub config_valid: bool,
    pub config_incident: Option<String>,
    pub execution_compiled_in: bool,
    pub live_execution_authorized: bool,
    pub route_count: u32,
}

pub fn route_status(config: &RouteArbConfigV1) -> RouteArbStatusV1 {
    let validation = validate_route_config(config);
    RouteArbStatusV1 {
        config_valid: validation.is_ok(),
        config_incident: validation.err(),
        execution_compiled_in: false,
        live_execution_authorized: false,
        route_count: enumerate_routes(config.max_route_legs).map(|routes| routes.len() as u32).unwrap_or(0),
    }
}

async fn read_ledger(pin: &AssetPin, owner: Principal) -> LedgerReadResult {
    let account = Account { owner, subaccount: None };
    let symbol: Result<(String,), _> = ic_cdk::call(pin.ledger, "icrc1_symbol", ()).await;
    let decimals: Result<(u8,), _> = ic_cdk::call(pin.ledger, "icrc1_decimals", ()).await;
    let fee: Result<(Nat,), _> = ic_cdk::call(pin.ledger, "icrc1_fee", ()).await;
    let balance: Result<(Nat,), _> = ic_cdk::call(pin.ledger, "icrc1_balance_of", (account,)).await;
    match (symbol, decimals, fee, balance) {
        (Ok((symbol,)), Ok((decimals,)), Ok((fee,)), Ok((balance,))) => {
            match (fee.0.to_u128(), balance.0.to_u128()) {
                (Some(fee_native), Some(balance_native)) => LedgerReadResult::ok(&symbol, decimals, fee_native, balance_native),
                _ => LedgerReadResult::failed("ledger fee or balance exceeds supported u128 range"),
            }
        }
        (symbol, decimals, fee, balance) => LedgerReadResult::failed(&format!(
            "ledger read failed: symbol={:?}; decimals={:?}; fee={:?}; balance={:?}",
            symbol.err(), decimals.err(), fee.err(), balance.err()
        )),
    }
}

pub async fn read_wallet_balances(owner: Principal) -> Vec<WalletAssetBalanceV1> {
    let pins = asset_pins();
    let reads = futures::future::join_all(pins.iter().map(|pin| read_ledger(pin, owner))).await;
    let reads: [LedgerReadResult; 6] = reads.try_into().expect("asset registry is compile-time fixed at six entries");
    wallet_rows_from_results(reads)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedgerFeeTable {
    values: [Option<u128>; 6],
}

impl LedgerFeeTable {
    pub fn zero() -> Self {
        Self { values: [Some(0); 6] }
    }

    pub fn unknown() -> Self {
        Self { values: [None; 6] }
    }

    pub fn set(&mut self, asset: Asset, fee: u128) {
        self.values[asset.index()] = Some(fee);
    }

    pub fn get(&self, asset: Asset) -> Result<u128, String> {
        self.values[asset.index()].ok_or_else(|| format!("unknown {:?} ledger fee", asset))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteWorkItem {
    pub work_id: String,
    pub route: Route,
    pub size_ladder_index: u8,
    pub principal_native: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkUniverse {
    pub route_count: usize,
    pub required_quote_calls: u64,
    pub items: Vec<RouteWorkItem>,
}

fn native_stable_principal(usd_6dec: u64, asset: Asset) -> Result<u64, String> {
    let decimals = asset.decimals();
    if decimals >= 6 {
        let factor = 10u64.checked_pow(u32::from(decimals - 6)).ok_or("unsupported decimal exponent")?;
        usd_6dec.checked_mul(factor).ok_or_else(|| "stable principal conversion overflow".to_string())
    } else {
        let divisor = 10u64.checked_pow(u32::from(6 - decimals)).ok_or("unsupported decimal exponent")?;
        Ok(usd_6dec / divisor)
    }
}

pub fn build_work_universe(config: &RouteArbConfigV1) -> Result<WorkUniverse, String> {
    validate_route_config(config)?;
    let enabled_assets: std::collections::BTreeSet<_> = config.asset_controls.iter().filter(|item| item.enabled).map(|item| item.asset).collect();
    let enabled_pools: std::collections::BTreeSet<_> = config.pool_controls.iter().filter(|item| item.enabled).map(|item| item.pool_id.as_str()).collect();
    let routes = enumerate_routes(config.max_route_legs)?
        .into_iter()
        .filter(|route| route.asset_path.iter().all(|asset| enabled_assets.contains(asset)))
        .filter(|route| route.edges.iter().all(|edge| enabled_pools.contains(edge.pool_id)))
        .filter(|route| match route.candidate_class {
            CandidateClass::StablePar | CandidateClass::StableSettledCrossAsset => config.stable_book_enabled,
            CandidateClass::IcpReturning => config.icp_book_enabled,
        })
        .collect::<Vec<_>>();
    let route_count = routes.len();
    let mut items = Vec::new();
    for route in routes {
        let ladder = if route.candidate_class == CandidateClass::IcpReturning {
            &config.icp_size_ladder
        } else {
            &config.stable_size_ladder
        };
        for (index, configured_size) in ladder.iter().enumerate() {
            let principal_native = if route.candidate_class == CandidateClass::IcpReturning {
                *configured_size
            } else {
                native_stable_principal(*configured_size, route.start_asset())?
            };
            items.push(RouteWorkItem {
                work_id: format!("{}#s{:02}", route.route_id, index),
                route: route.clone(),
                size_ladder_index: index as u8,
                principal_native,
            });
        }
    }
    items.sort_by(|left, right| left.work_id.cmp(&right.work_id));
    let required_quote_calls = items.iter().map(|item| item.route.edges.len() as u64).sum();
    Ok(WorkUniverse { route_count, required_quote_calls, items })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuoteMethod {
    RumiCalcSwap { coin_in: u8, coin_out: u8 },
    IcpSwapQuoteForAll { zero_for_one: bool },
}

fn rumi_index(asset: Asset) -> Result<u8, String> {
    match asset {
        Asset::IcUsd => Ok(0),
        Asset::CkUsdt => Ok(1),
        Asset::CkUsdc => Ok(2),
        _ => Err("Rumi 3pool edge contains a non-stable asset".to_string()),
    }
}

pub fn quote_method(edge: &DirectedEdge, observed_token0: Option<Asset>) -> Result<QuoteMethod, String> {
    match edge.venue {
        VenueKind::Rumi3Pool => Ok(QuoteMethod::RumiCalcSwap {
            coin_in: rumi_index(edge.from)?,
            coin_out: rumi_index(edge.to)?,
        }),
        VenueKind::IcpSwap => {
            let token0 = observed_token0.ok_or("ICPSwap token ordering is unverified")?;
            if token0 != edge.from && token0 != edge.to {
                return Err("ICPSwap token0 is not an admitted edge asset".to_string());
            }
            Ok(QuoteMethod::IcpSwapQuoteForAll { zero_for_one: token0 == edge.from })
        }
    }
}

pub fn build_quote_from_outputs(
    route: &Route,
    principal_native: u64,
    fees: &LedgerFeeTable,
    gross_outputs: &[u64],
    quoted_at_ns: u64,
    size_ladder_index: u8,
) -> Result<RouteQuote, String> {
    if route.edges.len() != gross_outputs.len() || route.edges.is_empty() {
        return Err("gross output count must match a non-empty route".to_string());
    }
    let mut wallet_before = u128::from(principal_native);
    let mut legs = Vec::with_capacity(route.edges.len());
    for (edge, gross_output) in route.edges.iter().zip(gross_outputs) {
        let entry_ledger_fee = fees.get(edge.from)?;
        let output_ledger_fee = fees.get(edge.to)?;
        let venue_input = wallet_before.checked_sub(entry_ledger_fee).ok_or("entry fee consumes route input")?;
        let gross_output = u128::from(*gross_output);
        let wallet_after = gross_output.checked_sub(output_ledger_fee).ok_or("output fee consumes route output")?;
        legs.push(QuoteLeg {
            edge_id: edge.edge_id.clone(),
            from: edge.from,
            to: edge.to,
            wallet_before,
            entry_ledger_fee,
            venue_input,
            gross_output,
            output_ledger_fee,
            wallet_after,
            dex_fee_native: 0,
            full_fill: true,
        });
        wallet_before = wallet_after;
    }
    Ok(RouteQuote {
        route_id: route.route_id.clone(),
        canonical_cycle_id: route.canonical_cycle_id.clone(),
        start_asset: route.start_asset(),
        end_asset: route.end_asset(),
        asset_path: route.asset_path.clone(),
        principal_native: u128::from(principal_native),
        legs,
        allowance_sufficient: Some(true),
        quoted_at_ns,
        size_ladder_index,
    })
}

pub fn checked_quote_age(now_ns: u64, quoted_at_ns: u64, max_age_ns: u64) -> Result<u64, String> {
    if max_age_ns == 0 || max_age_ns > HARD_MAX_QUOTE_AGE_NS {
        return Err("invalid quote age bound".to_string());
    }
    let age = now_ns.checked_sub(quoted_at_ns).ok_or("clock regression while checking quote age")?;
    if age >= max_age_ns {
        return Err("quote is stale".to_string());
    }
    Ok(age)
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct QuoteLegReportV1 {
    pub edge_id: String,
    pub from: Asset,
    pub to: Asset,
    pub wallet_before: u128,
    pub entry_ledger_fee: u128,
    pub venue_input: u128,
    pub gross_output: u128,
    pub output_ledger_fee: u128,
    pub wallet_after: u128,
    pub full_fill: bool,
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct RouteCandidateReportV1 {
    pub route_id: String,
    pub canonical_cycle_id: Option<String>,
    pub candidate_class: CandidateClass,
    pub asset_path: Vec<Asset>,
    pub venue_edges: Vec<String>,
    pub start_asset: Asset,
    pub end_asset: Asset,
    pub principal_native: u128,
    pub net_profit_native: i64,
    pub net_profit_bps: i64,
    pub size_ladder_index: u8,
    pub par_assumption: bool,
    pub full_fill: bool,
    pub allowance_status: String,
    pub inventory_effect: String,
    pub quote_timestamp_ns: u64,
    pub legs: Vec<QuoteLegReportV1>,
    pub eligible: bool,
    pub rejection_reason: Option<String>,
}

impl RouteCandidateReportV1 {
    /// Deterministic constructor used by pure observation-state tests.
    pub fn fixture(id: &str, class: CandidateClass, profit: i64, eligible: bool) -> Self {
        let (start, end) = if class == CandidateClass::IcpReturning {
            (Asset::Icp, Asset::Icp)
        } else {
            (Asset::CkUsdc, Asset::CkUsdc)
        };
        Self {
            route_id: id.to_string(), canonical_cycle_id: None, candidate_class: class,
            asset_path: vec![start, end], venue_edges: Vec::new(), start_asset: start,
            end_asset: end, principal_native: 1, net_profit_native: profit,
            net_profit_bps: profit, size_ladder_index: 0, par_assumption: false,
            full_fill: true, allowance_status: "sufficient".to_string(),
            inventory_effect: "within bands".to_string(), quote_timestamp_ns: 0,
            legs: Vec::new(), eligible, rejection_reason: None,
        }
    }
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ObservationAccumulatorV1 {
    pub observation_id: String,
    pub started_at_ns: u64,
    pub next_cursor: u64,
    pub total_work_items: u64,
    pub route_count: u32,
    pub required_quote_calls: u64,
    pub quote_call_budget_sufficient: bool,
    pub quote_calls_made: u64,
    pub full_fill_rejections: u64,
    pub candidates_evaluated: u64,
    pub scan_complete: bool,
    pub completed_at_ns: Option<u64>,
    pub provisional_best_stable_candidate: Option<RouteCandidateReportV1>,
    pub provisional_best_icp_candidate: Option<RouteCandidateReportV1>,
    pub best_stable_candidate: Option<RouteCandidateReportV1>,
    pub best_icp_candidate: Option<RouteCandidateReportV1>,
    pub incident: Option<String>,
}

impl ObservationAccumulatorV1 {
    pub fn new(
        observation_id: String,
        started_at_ns: u64,
        next_cursor: u64,
        total_work_items: u64,
        required_quote_calls: u64,
        quote_call_budget_sufficient: bool,
    ) -> Self {
        Self {
            observation_id, started_at_ns, next_cursor, total_work_items,
            route_count: 0, required_quote_calls, quote_call_budget_sufficient,
            quote_calls_made: 0, full_fill_rejections: 0, candidates_evaluated: 0,
            scan_complete: false, completed_at_ns: None,
            provisional_best_stable_candidate: None, provisional_best_icp_candidate: None,
            best_stable_candidate: None, best_icp_candidate: None, incident: None,
        }
    }
}

fn replace_best(slot: &mut Option<RouteCandidateReportV1>, candidate: &RouteCandidateReportV1) {
    if !candidate.eligible {
        return;
    }
    let replace = slot.as_ref().map(|prior| {
        candidate.net_profit_native > prior.net_profit_native
            || (candidate.net_profit_native == prior.net_profit_native
                && (candidate.net_profit_bps > prior.net_profit_bps
                    || (candidate.net_profit_bps == prior.net_profit_bps
                        && (candidate.legs.len() < prior.legs.len()
                            || (candidate.legs.len() == prior.legs.len()
                                && (candidate.route_id < prior.route_id
                                    || (candidate.route_id == prior.route_id
                                        && candidate.size_ladder_index < prior.size_ladder_index)))))))
    }).unwrap_or(true);
    if replace {
        *slot = Some(candidate.clone());
    }
}

pub fn accumulate_observation_batch(
    state: &mut ObservationAccumulatorV1,
    cursor: u64,
    candidates: Vec<RouteCandidateReportV1>,
    quote_calls: u64,
    full_fill_rejections: u64,
) -> Result<(), String> {
    if state.scan_complete {
        return Err("observation is already complete".to_string());
    }
    if cursor != state.next_cursor {
        return Err(format!("cursor mismatch: expected {}, got {}", state.next_cursor, cursor));
    }
    if !state.quote_call_budget_sufficient && quote_calls > 0 {
        return Err("observation exceeds configured quote-call budget; refusing quoted results".to_string());
    }
    if state.quote_calls_made.checked_add(quote_calls).is_none_or(|total| total > state.required_quote_calls) {
        return Err("batch quote-call count exceeds the declared observation universe".to_string());
    }
    let next = cursor.checked_add(candidates.len() as u64).ok_or("observation cursor overflow")?;
    if next > state.total_work_items {
        return Err("batch exceeds observation universe".to_string());
    }
    state.next_cursor = next;
    state.quote_calls_made = state.quote_calls_made.checked_add(quote_calls).ok_or("quote-call counter overflow")?;
    state.full_fill_rejections = state.full_fill_rejections.checked_add(full_fill_rejections).ok_or("rejection counter overflow")?;
    state.candidates_evaluated = state.candidates_evaluated.checked_add(candidates.len() as u64).ok_or("candidate counter overflow")?;
    for candidate in &candidates {
        match candidate.candidate_class {
            CandidateClass::StablePar | CandidateClass::StableSettledCrossAsset => replace_best(&mut state.provisional_best_stable_candidate, candidate),
            CandidateClass::IcpReturning => replace_best(&mut state.provisional_best_icp_candidate, candidate),
        }
    }
    if next == state.total_work_items {
        state.scan_complete = true;
        if state.quote_call_budget_sufficient {
            state.best_stable_candidate = state.provisional_best_stable_candidate.clone();
            state.best_icp_candidate = state.provisional_best_icp_candidate.clone();
        } else {
            state.incident = Some("complete route-and-size universe exceeds configured quote-call budget".to_string());
        }
    }
    Ok(())
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ObservationStartV1 {
    pub observation_id: String,
    pub route_count: u32,
    pub route_size_count: u64,
    pub required_quote_calls: u64,
    pub quote_call_budget_sufficient: bool,
    pub next_cursor: u64,
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ObservationBatchResultV1 {
    pub observation: ObservationAccumulatorV1,
    pub candidates: Vec<RouteCandidateReportV1>,
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct BestRouteCandidatesV1 {
    pub observation_id: Option<String>,
    pub scan_complete: bool,
    pub stable: Option<RouteCandidateReportV1>,
    pub icp: Option<RouteCandidateReportV1>,
}

#[derive(CandidType, Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutationOwnerV1 {
    RouteExecution,
    VolumeOperation,
    GenericWithdrawal,
    RetiredVenueRecovery,
    LegacyMigration,
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct MutationLockV1 {
    pub operation_id: String,
    pub owner: MutationOwnerV1,
    pub acquired_at_ns: u64,
    pub reconciliation_required: bool,
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct MutationLockSlotV1 {
    pub lock: Option<MutationLockV1>,
}

#[derive(CandidType, Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReservationKindV1 {
    ActiveRoute,
    HeldPosition,
    NonRoute,
    LegacyFreeze,
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct OwnershipReservationV1 {
    pub reservation_id: String,
    pub asset: Asset,
    pub amount_native: u64,
    pub whole_asset_freeze: bool,
    pub kind: ReservationKindV1,
    pub owner: MutationOwnerV1,
    pub operation_id: String,
    pub reconciled_at_ns: u64,
    pub active: bool,
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub enum HeldBasisV1 {
    StablePar { start_asset: Asset, principal_native: u64, principal_usd_6dec: u64 },
    IcpNative { principal_icp_e8s: u64 },
    LegacyUnknown { preserved_pending_fields: String },
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct HeldLotV1 {
    pub asset: Asset,
    pub native_amount: u64,
    pub attributable_fees_native: u64,
    pub reserved_native: u64,
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct HeldPositionV1 {
    pub position_id: String,
    pub originating_execution_id: String,
    pub originating_route_id: String,
    pub basis: HeldBasisV1,
    pub lots: Vec<HeldLotV1>,
    pub reason: String,
    pub first_held_timestamp_ns: u64,
    pub last_reconciled_timestamp_ns: u64,
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct AssetReservationStatusV1 {
    pub asset: Option<Asset>,
    pub held: u64,
    pub active_route: u64,
    pub non_route: u64,
    pub whole_asset_frozen: bool,
}

#[derive(CandidType, Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionPhaseV1 {
    Planned,
    LegPrepared,
    LegSubmitted,
    AwaitingSettlement,
    ReconciliationRequired,
    LegSettled,
    RemainingRouteRequoted,
    Completed,
    Aborted,
    HeldInventory,
}

impl ExecutionPhaseV1 {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Aborted | Self::HeldInventory)
    }
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ReconciliationEvidenceV1 {
    pub evidence_kind: String,
    pub source_reference: String,
    pub amount_native: u64,
    pub observed_at_ns: u64,
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ExecutionRecordV1 {
    pub execution_id: String,
    pub route_id: String,
    pub canonical_cycle_id: Option<String>,
    pub candidate_class: CandidateClass,
    pub phase: ExecutionPhaseV1,
    pub current_leg_index: u8,
    pub planned_input_native: u64,
    pub required_min_output_native: u64,
    pub quote_timestamp_ns: u64,
    pub submission_started_at_ns: Option<u64>,
    pub adapter_request_fingerprint: Option<String>,
    pub evidence: Vec<ReconciliationEvidenceV1>,
    pub reconciliation_query_count: u8,
    pub incident: Option<String>,
    pub updated_at_ns: u64,
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct ExecutionSlotV1 {
    pub execution: Option<ExecutionRecordV1>,
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct SettlementProofV1 {
    pub request_fingerprint: String,
    pub planned_input_native: u64,
    pub effective_input_native: u64,
    pub gross_output_native: u64,
    pub refund_native: u64,
    pub source_debit_bound: bool,
    pub venue_execution_bound: bool,
    pub output_credit_bound: bool,
    pub refund_bound: bool,
    pub fully_reconciled: bool,
}

pub fn prepare_execution(
    candidate: &RouteCandidateReportV1,
    execution_id: &str,
    now_ns: u64,
) -> Result<ExecutionRecordV1, String> {
    if execution_id.is_empty() || execution_id.len() > 256 {
        return Err("execution_id must contain 1..=256 bytes".to_string());
    }
    if !candidate.eligible || !candidate.full_fill {
        return Err("candidate must be eligible and fully filled at quote time".to_string());
    }
    let endpoint_valid = match candidate.candidate_class {
        CandidateClass::StablePar | CandidateClass::StableSettledCrossAsset => {
            candidate.start_asset.is_stable() && candidate.end_asset.is_stable()
        }
        CandidateClass::IcpReturning => {
            candidate.start_asset == Asset::Icp && candidate.end_asset == Asset::Icp
        }
    };
    if !endpoint_valid {
        return Err("candidate endpoints do not match its profit domain".to_string());
    }
    Ok(ExecutionRecordV1 {
        execution_id: execution_id.to_string(), route_id: candidate.route_id.clone(),
        canonical_cycle_id: candidate.canonical_cycle_id.clone(),
        candidate_class: candidate.candidate_class, phase: ExecutionPhaseV1::Planned,
        current_leg_index: 0, planned_input_native: 0, required_min_output_native: 0,
        quote_timestamp_ns: candidate.quote_timestamp_ns, submission_started_at_ns: None,
        adapter_request_fingerprint: None, evidence: Vec::new(),
        reconciliation_query_count: 0, incident: None, updated_at_ns: now_ns,
    })
}

pub fn prepare_leg(
    record: &mut ExecutionRecordV1,
    planned_input_native: u64,
    required_min_output_native: u64,
    now_ns: u64,
) -> Result<(), String> {
    if !matches!(record.phase, ExecutionPhaseV1::Planned | ExecutionPhaseV1::RemainingRouteRequoted) {
        return Err("a leg can only be prepared from Planned or RemainingRouteRequoted".to_string());
    }
    if planned_input_native == 0 || required_min_output_native == 0 {
        return Err("planned input and minimum output must be nonzero".to_string());
    }
    record.planned_input_native = planned_input_native;
    record.required_min_output_native = required_min_output_native;
    record.adapter_request_fingerprint = None;
    record.submission_started_at_ns = None;
    record.phase = ExecutionPhaseV1::LegPrepared;
    record.updated_at_ns = now_ns;
    Ok(())
}

pub fn persist_leg_submission(
    record: &mut ExecutionRecordV1,
    request_fingerprint: &str,
    now_ns: u64,
) -> Result<(), String> {
    if record.phase != ExecutionPhaseV1::LegPrepared {
        return Err("submission intent may be persisted exactly once from LegPrepared".to_string());
    }
    if request_fingerprint.is_empty() || request_fingerprint.len() > 256 {
        return Err("request fingerprint must contain 1..=256 bytes".to_string());
    }
    record.adapter_request_fingerprint = Some(request_fingerprint.to_string());
    record.submission_started_at_ns = Some(now_ns);
    record.phase = ExecutionPhaseV1::LegSubmitted;
    record.updated_at_ns = now_ns;
    Ok(())
}

pub fn mark_awaiting_settlement(record: &mut ExecutionRecordV1, now_ns: u64) -> Result<(), String> {
    if record.phase != ExecutionPhaseV1::LegSubmitted {
        return Err("only a persisted LegSubmitted intent can await settlement".to_string());
    }
    record.phase = ExecutionPhaseV1::AwaitingSettlement;
    record.updated_at_ns = now_ns;
    Ok(())
}

pub fn mark_reconciliation_required(
    record: &mut ExecutionRecordV1,
    now_ns: u64,
    settlement_timeout_ns: u64,
) -> Result<(), String> {
    if !matches!(record.phase, ExecutionPhaseV1::LegSubmitted | ExecutionPhaseV1::AwaitingSettlement) {
        return Err("only a submitted unsettled leg can require reconciliation".to_string());
    }
    let submitted = record.submission_started_at_ns.ok_or("submission timestamp unavailable")?;
    let elapsed = now_ns.checked_sub(submitted).ok_or("settlement clock regressed")?;
    if elapsed < settlement_timeout_ns {
        return Err("settlement timeout has not elapsed".to_string());
    }
    record.phase = ExecutionPhaseV1::ReconciliationRequired;
    record.incident = Some("source-bound settlement evidence remains incomplete".to_string());
    record.updated_at_ns = now_ns;
    Ok(())
}

pub fn reconcile_settlement(
    record: &mut ExecutionRecordV1,
    proof: &SettlementProofV1,
    now_ns: u64,
) -> Result<(), String> {
    if !matches!(record.phase, ExecutionPhaseV1::AwaitingSettlement | ExecutionPhaseV1::ReconciliationRequired) {
        return Err("settlement proof is only accepted for a submitted unsettled leg".to_string());
    }
    let expected = record.adapter_request_fingerprint.as_deref().ok_or("request fingerprint unavailable")?;
    if proof.request_fingerprint != expected || proof.planned_input_native != record.planned_input_native {
        return Err("settlement proof is not bound to the persisted request fingerprint".to_string());
    }
    if !proof.fully_reconciled
        || !proof.source_debit_bound
        || !proof.venue_execution_bound
        || !proof.output_credit_bound
        || !proof.refund_bound
    {
        return Err("amount-only or incomplete evidence cannot advance settlement".to_string());
    }
    let conserved = proof.effective_input_native.checked_add(proof.refund_native)
        .ok_or("settlement conservation overflow")?;
    if conserved != proof.planned_input_native {
        return Err("effective input plus refund does not conserve planned input".to_string());
    }
    record.updated_at_ns = now_ns;
    record.incident = None;
    if proof.effective_input_native != proof.planned_input_native {
        record.phase = if proof.effective_input_native == 0 && proof.gross_output_native == 0 {
            ExecutionPhaseV1::Aborted
        } else {
            ExecutionPhaseV1::HeldInventory
        };
    } else if proof.gross_output_native < record.required_min_output_native {
        record.phase = ExecutionPhaseV1::HeldInventory;
    } else {
        record.phase = ExecutionPhaseV1::LegSettled;
    }
    Ok(())
}

pub fn record_remaining_route_requote(
    record: &mut ExecutionRecordV1,
    still_profitable: bool,
    now_ns: u64,
) -> Result<(), String> {
    if record.phase != ExecutionPhaseV1::LegSettled {
        return Err("remaining route may only be evaluated after exact leg settlement".to_string());
    }
    record.phase = if still_profitable {
        ExecutionPhaseV1::RemainingRouteRequoted
    } else {
        ExecutionPhaseV1::HeldInventory
    };
    record.updated_at_ns = now_ns;
    Ok(())
}

pub fn consume_reconciliation_queries(
    record: &mut ExecutionRecordV1,
    requested: u8,
    configured_limit: u8,
) -> Result<(), String> {
    if configured_limit == 0 || configured_limit > HARD_MAX_RECONCILIATION_QUERIES_PER_CYCLE {
        return Err("invalid reconciliation query limit".to_string());
    }
    let next = record.reconciliation_query_count.checked_add(requested)
        .ok_or("reconciliation query counter overflow")?;
    if next > configured_limit || next > HARD_MAX_RECONCILIATION_QUERIES_PER_CYCLE {
        return Err("reconciliation query budget exhausted".to_string());
    }
    record.reconciliation_query_count = next;
    Ok(())
}

fn asset_for_ledger_text(address: &str) -> Option<Asset> {
    asset_pins().into_iter().find(|pin| pin.ledger.to_text() == address).map(|pin| pin.asset)
}

pub fn asset_for_ledger(ledger: Principal) -> Option<Asset> {
    asset_pins().into_iter().find(|pin| pin.ledger == ledger).map(|pin| pin.asset)
}

async fn load_pool_orderings(items: &[RouteWorkItem]) -> std::collections::BTreeMap<String, Result<Option<Asset>, String>> {
    let needed: std::collections::BTreeSet<_> = items.iter()
        .flat_map(|item| item.route.edges.iter().map(|edge| edge.pool_id.to_string()))
        .collect();
    let mut result = std::collections::BTreeMap::new();
    for pool in pool_pins().into_iter().filter(|pool| needed.contains(pool.pool_id)) {
        let admission = match pool.venue {
            VenueKind::Rumi3Pool => {
                match crate::prices::fetch_rumi_pool_status(pool.principal).await {
                    Ok(status) => {
                        let actual: Vec<_> = status.tokens.iter().map(|token| token.ledger_id).collect();
                        let expected: Vec<_> = pool.assets.iter().map(|asset| asset_pins()[asset.index()].ledger).collect();
                        if actual == expected
                            && status.tokens.iter().zip(pool.assets.iter()).all(|(token, asset)| {
                                token.symbol == asset.symbol() && token.decimals == asset.decimals()
                            })
                        {
                            Ok(None)
                        } else {
                            Err("Rumi 3pool token identity or ordering does not match immutable registry".to_string())
                        }
                    }
                    Err(error) => Err(error),
                }
            }
            VenueKind::IcpSwap => {
                match crate::prices::fetch_icpswap_pool_metadata(pool.principal).await {
                    Ok(metadata) => {
                        let token0 = asset_for_ledger_text(&metadata.token0.address);
                        let token1 = asset_for_ledger_text(&metadata.token1.address);
                        match (token0, token1) {
                            (Some(token0), Some(token1))
                                if pool.assets.len() == 2
                                    && ((pool.assets[0] == token0 && pool.assets[1] == token1)
                                        || (pool.assets[0] == token1 && pool.assets[1] == token0)) => Ok(Some(token0)),
                            _ => Err("ICPSwap token identity or pair does not match immutable registry".to_string()),
                        }
                    }
                    Err(error) => Err(error),
                }
            }
        };
        result.insert(pool.pool_id.to_string(), admission);
    }
    result
}

fn tables_from_wallet_rows(rows: &[WalletAssetBalanceV1]) -> (LedgerFeeTable, AssetAmounts) {
    let mut fees = LedgerFeeTable::unknown();
    let mut balances = AssetAmounts::unknown();
    for row in rows {
        if row.metadata_valid {
            if let Some(fee) = row.ledger_fee_native {
                fees.set(row.asset, fee);
            }
            balances.set(row.asset, row.balance_native);
        }
    }
    (fees, balances)
}

fn inventory_bands_from_config(config: &RouteArbConfigV1) -> InventoryBands {
    let mut bands = InventoryBands::unbounded();
    for band in &config.inventory_bands {
        bands.set(band.asset, u128::from(band.floor_native), u128::from(band.ceiling_native));
    }
    bands
}

fn rejection_report(item: &RouteWorkItem, timestamp: u64, reason: String) -> RouteCandidateReportV1 {
    RouteCandidateReportV1 {
        route_id: item.route.route_id.clone(),
        canonical_cycle_id: item.route.canonical_cycle_id.clone(),
        candidate_class: item.route.candidate_class,
        asset_path: item.route.asset_path.clone(),
        venue_edges: item.route.edges.iter().map(|edge| edge.edge_id.clone()).collect(),
        start_asset: item.route.start_asset(), end_asset: item.route.end_asset(),
        principal_native: u128::from(item.principal_native), net_profit_native: 0,
        net_profit_bps: 0, size_ladder_index: item.size_ladder_index,
        par_assumption: item.route.start_asset() != item.route.end_asset(),
        full_fill: false, allowance_status: "unknown".to_string(),
        inventory_effect: "not evaluated".to_string(), quote_timestamp_ns: timestamp,
        legs: Vec::new(), eligible: false, rejection_reason: Some(reason),
    }
}

fn candidate_report(
    item: &RouteWorkItem,
    quote: &RouteQuote,
    evaluation: CandidateEvaluation,
    allowance_sufficient: bool,
) -> RouteCandidateReportV1 {
    let profit = i64::try_from(evaluation.net_profit_native);
    let inventory_blocked = evaluation.rejection_reason.as_deref().is_some_and(|reason| {
        reason.contains("inventory") || reason.contains("balance")
    });
    let mut rejection_reason = evaluation.rejection_reason;
    let eligible = evaluation.eligible && profit.is_ok();
    if profit.is_err() {
        rejection_reason = Some("native profit exceeds report representation".to_string());
    }
    RouteCandidateReportV1 {
        route_id: evaluation.route_id,
        canonical_cycle_id: evaluation.canonical_cycle_id,
        candidate_class: item.route.candidate_class,
        asset_path: item.route.asset_path.clone(),
        venue_edges: item.route.edges.iter().map(|edge| edge.edge_id.clone()).collect(),
        start_asset: evaluation.start_asset,
        end_asset: evaluation.end_asset,
        principal_native: evaluation.principal_native,
        net_profit_native: profit.unwrap_or(0),
        net_profit_bps: evaluation.net_profit_bps,
        size_ladder_index: evaluation.size_ladder_index,
        par_assumption: evaluation.par_assumption,
        full_fill: quote.legs.iter().all(|leg| leg.full_fill),
        allowance_status: if allowance_sufficient { "sufficient" } else { "unknown_or_insufficient" }.to_string(),
        inventory_effect: if inventory_blocked {
            "blocked".to_string()
        } else {
            "within configured bands".to_string()
        },
        quote_timestamp_ns: quote.quoted_at_ns,
        legs: quote.legs.iter().map(|leg| QuoteLegReportV1 {
            edge_id: leg.edge_id.clone(), from: leg.from, to: leg.to,
            wallet_before: leg.wallet_before, entry_ledger_fee: leg.entry_ledger_fee,
            venue_input: leg.venue_input, gross_output: leg.gross_output,
            output_ledger_fee: leg.output_ledger_fee, wallet_after: leg.wallet_after,
            full_fill: leg.full_fill,
        }).collect(),
        eligible,
        rejection_reason,
    }
}

async fn quote_work_item_live(
    item: &RouteWorkItem,
    config: &RouteArbConfigV1,
    fees: &LedgerFeeTable,
    balances: &AssetAmounts,
    reservations: &ReservationTotals,
    admissions: &std::collections::BTreeMap<String, Result<Option<Asset>, String>>,
) -> (RouteCandidateReportV1, u64, bool) {
    let mut wallet_before = item.principal_native;
    let mut gross_outputs = Vec::with_capacity(item.route.edges.len());
    let mut allowance_sufficient = true;
    let mut quote_calls = 0u64;
    for edge in &item.route.edges {
        let ordering = match admissions.get(edge.pool_id) {
            Some(Ok(ordering)) => *ordering,
            Some(Err(error)) => return (rejection_report(item, ic_cdk::api::time(), error.clone()), quote_calls, false),
            None => return (rejection_report(item, ic_cdk::api::time(), "pool admission evidence unavailable".to_string()), quote_calls, false),
        };
        let entry_fee = match fees.get(edge.from).and_then(|fee| u64::try_from(fee).map_err(|_| "ledger fee exceeds u64".to_string())) {
            Ok(fee) => fee,
            Err(error) => return (rejection_report(item, ic_cdk::api::time(), error), quote_calls, false),
        };
        let venue_input = match wallet_before.checked_sub(entry_fee) {
            Some(value) if value > 0 => value,
            _ => return (rejection_report(item, ic_cdk::api::time(), "entry fee consumes route input".to_string()), quote_calls, false),
        };
        let ledger = asset_pins()[edge.from.index()].ledger;
        match crate::swaps::query_allowance(ledger, ic_cdk::id(), edge.pool_principal).await {
            Ok((allowance, expires_at)) => {
                let expired = expires_at.is_some_and(|expiry| expiry <= ic_cdk::api::time());
                if allowance < venue_input || expired { allowance_sufficient = false; }
            }
            Err(_) => allowance_sufficient = false,
        }
        quote_calls += 1;
        let gross = match quote_method(edge, ordering) {
            Ok(QuoteMethod::RumiCalcSwap { coin_in, coin_out }) => {
                crate::swaps::pool_calc_swap(edge.pool_principal, coin_in, coin_out, venue_input).await
            }
            Ok(QuoteMethod::IcpSwapQuoteForAll { zero_for_one }) => {
                crate::prices::fetch_icpswap_quote_for_all(edge.pool_principal, venue_input, zero_for_one).await
            }
            Err(error) => Err(error),
        };
        let gross = match gross {
            Ok(value) => value,
            Err(error) => return (rejection_report(item, ic_cdk::api::time(), error), quote_calls, true),
        };
        let output_fee = match fees.get(edge.to).and_then(|fee| u64::try_from(fee).map_err(|_| "ledger fee exceeds u64".to_string())) {
            Ok(fee) => fee,
            Err(error) => return (rejection_report(item, ic_cdk::api::time(), error), quote_calls, false),
        };
        wallet_before = match gross.checked_sub(output_fee) {
            Some(value) if value > 0 => value,
            _ => return (rejection_report(item, ic_cdk::api::time(), "output fee consumes route output".to_string()), quote_calls, false),
        };
        gross_outputs.push(gross);
    }
    let quoted_at = ic_cdk::api::time();
    let mut quote = match build_quote_from_outputs(&item.route, item.principal_native, fees, &gross_outputs, quoted_at, item.size_ladder_index) {
        Ok(quote) => quote,
        Err(error) => return (rejection_report(item, quoted_at, error), quote_calls, false),
    };
    quote.allowance_sufficient = Some(allowance_sufficient);
    let bands = inventory_bands_from_config(config);
    let evaluation = evaluate_candidate(
        &quote, balances, reservations, &bands,
        i128::from(config.min_stable_profit_usd_6dec), i64::from(config.min_stable_profit_bps),
        i128::from(config.min_icp_profit_e8s), i64::from(config.min_icp_profit_bps),
    );
    (candidate_report(item, &quote, evaluation, allowance_sufficient), quote_calls, false)
}

pub async fn quote_observation_items(
    config: &RouteArbConfigV1,
    items: &[RouteWorkItem],
    reservations: &ReservationTotals,
) -> Vec<(RouteCandidateReportV1, u64, bool)> {
    let wallet_rows = read_wallet_balances(ic_cdk::id()).await;
    let (fees, balances) = tables_from_wallet_rows(&wallet_rows);
    let admissions = load_pool_orderings(items).await;
    futures::stream::iter(items.iter().map(|item| {
        quote_work_item_live(item, config, &fees, &balances, reservations, &admissions)
    }))
    .buffered(usize::from(config.max_concurrent_quote_calls))
    .collect()
    .await
}
