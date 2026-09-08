use arb_bot::route_arb::{canonical_cycle_id, enumerate_routes, Asset, CandidateClass};
use std::collections::BTreeSet;

#[test]
fn exact_bounded_route_universe_is_stable() {
    let routes = enumerate_routes(4).expect("valid bound");
    assert_eq!(routes.len(), 870);
    let by_length = (1..=4)
        .map(|length| routes.iter().filter(|route| route.edges.len() == length).count())
        .collect::<Vec<_>>();
    assert_eq!(by_length, vec![12, 44, 228, 586]);
    assert!(routes.windows(2).all(|pair| pair[0].route_id < pair[1].route_id));
    assert_eq!(
        routes.iter().map(|route| route.route_id.as_str()).collect::<BTreeSet<_>>().len(),
        routes.len()
    );
    assert!(enumerate_routes(0).is_err());
    assert!(enumerate_routes(5).is_err());
}

#[test]
fn exact_route_universe_breaks_down_by_candidate_class() {
    let routes = enumerate_routes(4).expect("valid bound");
    let count = |class: CandidateClass| routes.iter().filter(|route| route.candidate_class == class).count();
    assert_eq!(count(CandidateClass::StablePar), 66);
    assert_eq!(count(CandidateClass::StableSettledCrossAsset), 600);
    assert_eq!(count(CandidateClass::IcpReturning), 30);
    assert_eq!(count(CandidateClass::CkBtcReturning), 104);
    assert_eq!(count(CandidateClass::CkEthReturning), 70);
}

#[test]
fn route_shapes_match_the_five_economic_classes() {
    for route in enumerate_routes(4).unwrap() {
        assert!(route.edges.len() <= 4);
        assert_eq!(route.asset_path.len(), route.edges.len() + 1);
        match route.candidate_class {
            CandidateClass::StablePar => {
                assert!(route.asset_path.iter().all(|asset| asset.is_stable()));
            }
            CandidateClass::StableSettledCrossAsset => {
                assert!(route.start_asset().is_stable() && route.end_asset().is_stable());
                assert!(route.asset_path.iter().any(|asset| !asset.is_stable()));
            }
            CandidateClass::IcpReturning => {
                assert_eq!(route.start_asset(), Asset::Icp);
                assert_eq!(route.end_asset(), Asset::Icp);
                assert!(route.asset_path[1..route.asset_path.len() - 1]
                    .iter()
                    .all(|asset| asset.is_stable()));
            }
            // Unlike IcpReturning, ckBTC/ckETH-returning admits any simple
            // same-asset cycle over the pinned graph — interior legs are not
            // restricted to stable assets.
            CandidateClass::CkBtcReturning => {
                assert_eq!(route.start_asset(), Asset::CkBtc);
                assert_eq!(route.end_asset(), Asset::CkBtc);
            }
            CandidateClass::CkEthReturning => {
                assert_eq!(route.start_asset(), Asset::CkEth);
                assert_eq!(route.end_asset(), Asset::CkEth);
            }
        }
    }
}

/// Required example shape: ckBTC -> ckETH -> ckUSDC -> ckBTC, a 3-leg
/// CkBtcReturning cycle whose interior passes through another pass-through
/// asset (ckETH) rather than a stable asset only.
#[test]
fn ckbtc_returning_admits_the_ckbtc_cketh_ckusdc_cycle() {
    let routes = enumerate_routes(4).unwrap();
    let found = routes.iter().find(|route| {
        route.candidate_class == CandidateClass::CkBtcReturning
            && route.asset_path == vec![Asset::CkBtc, Asset::CkEth, Asset::CkUsdc, Asset::CkBtc]
    });
    assert!(found.is_some(), "expected a ckBTC->ckETH->ckUSDC->ckBTC route in the universe");
    let route = found.unwrap();
    assert_eq!(route.edges.len(), 3);
    let pool_ids: BTreeSet<_> = route.edges.iter().map(|edge| edge.pool_id).collect();
    assert_eq!(pool_ids.len(), 3, "each leg must use a distinct pinned pool");
}

/// Required example shape: ckETH -> ICP -> ckUSDC -> ckETH, a 3-leg
/// CkEthReturning cycle whose interior passes through ICP rather than a
/// stable asset only.
#[test]
fn cketh_returning_admits_the_cketh_icp_ckusdc_cycle() {
    let routes = enumerate_routes(4).unwrap();
    let found = routes.iter().find(|route| {
        route.candidate_class == CandidateClass::CkEthReturning
            && route.asset_path == vec![Asset::CkEth, Asset::Icp, Asset::CkUsdc, Asset::CkEth]
    });
    assert!(found.is_some(), "expected a ckETH->ICP->ckUSDC->ckETH route in the universe");
    let route = found.unwrap();
    assert_eq!(route.edges.len(), 3);
    let pool_ids: BTreeSet<_> = route.edges.iter().map(|edge| edge.pool_id).collect();
    assert_eq!(pool_ids.len(), 3, "each leg must use a distinct pinned pool");
}

#[test]
fn routes_never_repeat_vertices_edges_or_physical_pools() {
    for route in enumerate_routes(4).unwrap() {
        let is_cycle = route.start_asset() == route.end_asset();
        let vertex_slice = if is_cycle {
            &route.asset_path[..route.asset_path.len() - 1]
        } else {
            route.asset_path.as_slice()
        };
        assert_eq!(vertex_slice.iter().collect::<BTreeSet<_>>().len(), vertex_slice.len());
        assert_eq!(route.edges.iter().map(|edge| &edge.edge_id).collect::<BTreeSet<_>>().len(), route.edges.len());
        assert_eq!(route.edges.iter().map(|edge| edge.pool_id).collect::<BTreeSet<_>>().len(), route.edges.len());
        assert!(route.edges.windows(2).all(|pair| pair[0].pool_id != pair[1].pool_id));
        assert!(route.edges.iter().filter(|edge| edge.pool_id == "rumi-3pool").count() <= 1);
    }
}

#[test]
fn cycle_rotations_canonicalize_but_reversal_stays_distinct() {
    let a = ["pool-a:A>B", "pool-b:B>C", "pool-c:C>A"];
    let rotation = ["pool-b:B>C", "pool-c:C>A", "pool-a:A>B"];
    let reverse = ["pool-c:A>C", "pool-b:C>B", "pool-a:B>A"];
    assert_eq!(canonical_cycle_id(&a), canonical_cycle_id(&rotation));
    assert_ne!(canonical_cycle_id(&a), canonical_cycle_id(&reverse));

    for route in enumerate_routes(4).unwrap() {
        assert_eq!(route.canonical_cycle_id.is_some(), route.start_asset() == route.end_asset());
    }
}
