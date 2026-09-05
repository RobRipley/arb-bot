use arb_bot::route_arb::{canonical_cycle_id, enumerate_routes, Asset, CandidateClass};
use std::collections::BTreeSet;

#[test]
fn exact_bounded_route_universe_is_stable() {
    let routes = enumerate_routes(4).expect("valid bound");
    assert_eq!(routes.len(), 696);
    let by_length = (1..=4)
        .map(|length| routes.iter().filter(|route| route.edges.len() == length).count())
        .collect::<Vec<_>>();
    assert_eq!(by_length, vec![12, 44, 190, 450]);
    assert!(routes.windows(2).all(|pair| pair[0].route_id < pair[1].route_id));
    assert_eq!(
        routes.iter().map(|route| route.route_id.as_str()).collect::<BTreeSet<_>>().len(),
        routes.len()
    );
    assert!(enumerate_routes(0).is_err());
    assert!(enumerate_routes(5).is_err());
}

#[test]
fn route_shapes_match_the_three_economic_classes() {
    for route in enumerate_routes(4).unwrap() {
        assert!(route.edges.len() <= 4);
        assert_eq!(route.asset_path.len(), route.edges.len() + 1);
        assert_ne!(route.start_asset(), Asset::CkBtc);
        assert_ne!(route.start_asset(), Asset::CkEth);
        assert_ne!(route.end_asset(), Asset::CkBtc);
        assert_ne!(route.end_asset(), Asset::CkEth);
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
        }
    }
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
