//! Six-asset route-arbitrage primitives.
//!
//! Asset and venue identity is deliberately compiled into the canister. An
//! administrator may later disable one of these pins, but cannot turn an
//! arbitrary principal into a route target through configuration alone.

use candid::{CandidType, Deserialize, Principal};
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
}

impl Default for ReservationTotals {
    fn default() -> Self {
        Self {
            held: AssetAmounts::zero(),
            active: AssetAmounts::zero(),
            non_route: AssetAmounts::zero(),
        }
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
