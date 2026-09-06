use arb_bot::route_arb::{
    Asset, CandidateClass, ExecutionPhaseV1, ExecutionRecordV1, ReconciliationEvidenceV1,
    RouteExecutionDetailV1, RouteExecutionLegStatusV1, RouteExecutionLegV1,
};

fn three_leg_execution_detail() -> RouteExecutionDetailV1 {
    let edges = arb_bot::route_arb::directed_edges();
    let edge = |from, to| {
        edges
            .iter()
            .find(|edge| {
                edge.pool_id == "rumi-3pool" && edge.from == from && edge.to == to
            })
            .expect("fixture edge")
    };
    let route_edges = [
        edge(Asset::CkUsdc, Asset::IcUsd),
        edge(Asset::IcUsd, Asset::CkUsdt),
        edge(Asset::CkUsdt, Asset::CkUsdc),
    ];
    let path = vec![Asset::CkUsdc, Asset::IcUsd, Asset::CkUsdt, Asset::CkUsdc];
    let legs = route_edges
        .iter()
        .enumerate()
        .map(|(index, edge)| RouteExecutionLegV1 {
            leg_index: index as u8,
            status: if index == 1 {
                RouteExecutionLegStatusV1::Settled
            } else {
                RouteExecutionLegStatusV1::Quoted
            },
            edge_id: edge.edge_id.clone(),
            pool_id: edge.pool_id.to_string(),
            pool_principal: edge.pool_principal,
            venue: edge.venue,
            from: edge.from,
            to: edge.to,
            quoted_input_native: [1_000_000, 1_000_000, 1_247_000][index],
            requested_input_native: (index == 1).then_some(999_500),
            quoted_output_native: [Some(999_000), Some(1_250_000), Some(1_245_000)][index],
            minimum_output_native: [998_000, 1_240_000, 1_240_000][index],
            input_fee_native: 1_000,
            output_fee_native: 1_000,
            actual_input_debit_native: (index == 1).then_some(1_001_000),
            actual_effective_input_native: (index == 1).then_some(1_000_000),
            actual_output_credit_native: (index == 1).then_some(1_247_000),
            refund_credit_native: None,
            prepared_at_ns: Some(10 + index as u64),
            submitted_at_ns: Some(20 + index as u64),
            settled_at_ns: (index == 1).then_some(30),
            reconciled_at_ns: (index == 1).then_some(31),
            evidence: if index == 1 {
                vec![ReconciliationEvidenceV1 {
                    evidence_kind: "ledger_block".into(),
                    source_reference: "block-1".into(),
                    amount_native: 1_247_000,
                    observed_at_ns: 31,
                }]
            } else {
                vec![]
            },
            incident: None,
        })
        .collect();
    RouteExecutionDetailV1 {
        record: ExecutionRecordV1 {
            execution_id: "execution-detail-fixture".into(),
            route_id: "route-detail-fixture".into(),
            canonical_cycle_id: None,
            candidate_class: CandidateClass::StablePar,
            phase: ExecutionPhaseV1::Completed,
            current_leg_index: 2,
            planned_input_native: 1_000_000,
            required_min_output_native: 1_240_000,
            quote_timestamp_ns: 1,
            submission_started_at_ns: Some(20),
            adapter_request_fingerprint: None,
            evidence: vec![],
            reconciliation_query_count: 3,
            incident: None,
            updated_at_ns: 32,
            realized_profit: Some(2_000),
            start_asset: Some(Asset::CkUsdc),
        },
        asset_path: path,
        legs,
        detail_available: true,
    }
}

#[test]
fn detail_keeps_variable_legs_in_route_order() {
    let detail = three_leg_execution_detail();
    assert_eq!(detail.legs.len(), 3);
    assert_eq!(
        detail.legs.iter().map(|leg| leg.leg_index).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert_eq!(detail.legs[0].edge_id, "rumi-3pool:CkUsdc>IcUsd");
    assert_eq!(detail.legs[1].quoted_output_native, Some(1_250_000));
    assert_eq!(detail.legs[1].requested_input_native, Some(999_500));
    assert_eq!(detail.legs[1].actual_output_credit_native, Some(1_247_000));
    assert_ne!(
        detail.legs[1].quoted_output_native,
        detail.legs[1].actual_output_credit_native
    );
    assert_eq!(detail.record.realized_profit, Some(2_000));
    assert_eq!(detail.record.start_asset, Some(Asset::CkUsdc));
}

#[test]
fn resolver_rejects_unknown_edges_and_reversed_transitions() {
    let path = vec![Asset::CkUsdc, Asset::IcUsd];
    assert!(arb_bot::route_arb::resolve_route_edges(&["missing-edge".into()], &path).is_err());

    let edge = arb_bot::route_arb::directed_edges()
        .into_iter()
        .find(|edge| edge.from == Asset::CkUsdc && edge.to == Asset::IcUsd)
        .expect("fixture edge");
    let reversed = vec![Asset::IcUsd, Asset::CkUsdc];
    assert!(arb_bot::route_arb::resolve_route_edges(&[edge.edge_id], &reversed).is_err());
}
