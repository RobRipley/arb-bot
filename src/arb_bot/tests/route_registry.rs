use arb_bot::route_arb::{asset_pins, directed_edges, pool_pins, Asset, AssetRole, VenueKind};
use candid::Principal;
use std::collections::BTreeSet;

fn p(text: &str) -> Principal {
    Principal::from_text(text).expect("valid principal fixture")
}

#[test]
fn six_asset_registry_is_exact_and_code_pinned() {
    let pins = asset_pins();
    assert_eq!(pins.len(), 6);
    let actual: Vec<_> = pins
        .iter()
        .map(|pin| (pin.asset, pin.ledger, pin.symbol, pin.decimals, pin.role))
        .collect();
    assert_eq!(
        actual,
        vec![
            (Asset::IcUsd, p("t6bor-paaaa-aaaap-qrd5q-cai"), "icUSD", 8, AssetRole::StableSettlement),
            (Asset::CkUsdt, p("cngnf-vqaaa-aaaar-qag4q-cai"), "ckUSDT", 6, AssetRole::StableSettlement),
            (Asset::CkUsdc, p("xevnm-gaaaa-aaaar-qafnq-cai"), "ckUSDC", 6, AssetRole::StableSettlement),
            (Asset::Icp, p("ryjl3-tyaaa-aaaaa-aaaba-cai"), "ICP", 8, AssetRole::IcpPrincipal),
            (Asset::CkBtc, p("mxzaz-hqaaa-aaaar-qaada-cai"), "ckBTC", 8, AssetRole::PassThroughOnly),
            (Asset::CkEth, p("ss2fx-dyaaa-aaaar-qacoq-cai"), "ckETH", 18, AssetRole::PassThroughOnly),
        ]
    );
}

#[test]
fn active_pool_registry_is_exact_and_excludes_retired_venues() {
    let expected = [
        ("rumi-3pool", "fohh4-yyaaa-aaaap-qtkpa-cai", VenueKind::Rumi3Pool, Asset::IcUsd, Asset::CkUsdt),
        ("icpswap-icp-ckusdc", "mohjv-bqaaa-aaaag-qjyia-cai", VenueKind::IcpSwap, Asset::Icp, Asset::CkUsdc),
        ("icpswap-icp-icusd", "nqxwe-hiaaa-aaaar-qb5yq-cai", VenueKind::IcpSwap, Asset::Icp, Asset::IcUsd),
        ("icpswap-icp-ckusdt", "hkstf-6iaaa-aaaag-qkcoq-cai", VenueKind::IcpSwap, Asset::Icp, Asset::CkUsdt),
        ("icpswap-icusd-ckusdt", "jogrm-gqaaa-aaaar-qcg2a-cai", VenueKind::IcpSwap, Asset::IcUsd, Asset::CkUsdt),
        ("icpswap-icusd-ckusdc", "eb25l-dyaaa-aaaar-qb4lq-cai", VenueKind::IcpSwap, Asset::IcUsd, Asset::CkUsdc),
        ("icpswap-ckusdt-ckusdc", "heq6n-fyaaa-aaaag-qkcpq-cai", VenueKind::IcpSwap, Asset::CkUsdt, Asset::CkUsdc),
        ("icpswap-ckbtc-icp", "xmiu5-jqaaa-aaaag-qbz7q-cai", VenueKind::IcpSwap, Asset::CkBtc, Asset::Icp),
        ("icpswap-icp-cketh", "angxa-baaaa-aaaag-qcvnq-cai", VenueKind::IcpSwap, Asset::Icp, Asset::CkEth),
        ("icpswap-ckbtc-cketh", "akhru-myaaa-aaaag-qcvna-cai", VenueKind::IcpSwap, Asset::CkBtc, Asset::CkEth),
        ("icpswap-cketh-ckusdc", "mvcvq-3iaaa-aaaag-qjykq-cai", VenueKind::IcpSwap, Asset::CkEth, Asset::CkUsdc),
        ("icpswap-ckbtc-ckusdc", "mhecj-xyaaa-aaaag-qjyjq-cai", VenueKind::IcpSwap, Asset::CkBtc, Asset::CkUsdc),
        ("icpswap-ckbtc-icusd", "jhf2q-qyaaa-aaaar-qcg3q-cai", VenueKind::IcpSwap, Asset::CkBtc, Asset::IcUsd),
        ("icpswap-ckusdt-ckbtc", "ipfno-pqaaa-aaaag-qkevq-cai", VenueKind::IcpSwap, Asset::CkUsdt, Asset::CkBtc),
        ("icpswap-cketh-icusd", "jjhxy-liaaa-aaaar-qcg2q-cai", VenueKind::IcpSwap, Asset::CkEth, Asset::IcUsd),
    ];
    let pins = pool_pins();
    assert_eq!(pins.len(), expected.len());
    for (pin, (id, principal, venue, a, b)) in pins.iter().zip(expected) {
        assert_eq!((pin.pool_id, pin.principal, pin.venue, pin.assets[0], pin.assets[1]), (id, p(principal), venue, a, b));
    }
    let retired = [
        p("ijlzs-2yaaa-aaaap-quaaq-cai"),
        p("xjiq2-fiaaa-aaaan-q52ra-cai"),
        p("6b2bo-kyaaa-aaaao-qpira-cai"),
        p("ybilh-nqaaa-aaaag-qkhzq-cai"),
        p("gxvvw-aiaaa-aaaar-qcada-cai"),
    ];
    assert!(pins.iter().all(|pin| !retired.contains(&pin.principal)));
}

#[test]
fn directed_edges_are_complete_deterministic_and_uniquely_identified() {
    let edges = directed_edges();
    assert_eq!(edges.len(), 34, "six Rumi pair directions plus 28 ICPSwap directions");
    let ids: BTreeSet<_> = edges.iter().map(|edge| edge.edge_id.clone()).collect();
    assert_eq!(ids.len(), edges.len());
    assert!(edges.iter().all(|edge| edge.from != edge.to));
    assert!(edges.iter().all(|edge| Asset::ALL.contains(&edge.from) && Asset::ALL.contains(&edge.to)));
    assert!(edges.windows(2).all(|pair| pair[0].edge_id < pair[1].edge_id));
    assert_eq!(
        edges.iter().filter(|edge| edge.venue == VenueKind::Rumi3Pool).count(),
        6
    );
}
