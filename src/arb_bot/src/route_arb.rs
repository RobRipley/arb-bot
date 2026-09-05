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
