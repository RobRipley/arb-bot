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
