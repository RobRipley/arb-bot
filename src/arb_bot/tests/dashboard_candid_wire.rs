use arb_bot::route_arb::{
    Asset, CandidateClass, ExecutionPhaseV1, ExecutionRecordV1, RouteExecutionDetailV1,
};

fn record(realized_profit: Option<i128>) -> ExecutionRecordV1 {
    ExecutionRecordV1 {
        execution_id: "wire-execution".into(),
        route_id: "wire-route".into(),
        canonical_cycle_id: None,
        candidate_class: CandidateClass::StablePar,
        phase: ExecutionPhaseV1::Completed,
        current_leg_index: 2,
        planned_input_native: 1_000_000,
        required_min_output_native: 999_000,
        quote_timestamp_ns: 10,
        submission_started_at_ns: Some(11),
        adapter_request_fingerprint: None,
        evidence: vec![],
        reconciliation_query_count: 1,
        incident: None,
        updated_at_ns: 12,
        realized_profit,
        start_asset: Some(Asset::CkUsdc),
    }
}

#[test]
fn execution_record_opt_int_round_trips_current_terminal_and_detail_shapes() {
    // The second value is deliberately wider than signed i64. Candid's int
    // must survive the same wire shape used by current, terminal, and detail
    // dashboard queries without truncation.
    for value in [Some(123_i128), Some((1_i128 << 100) + 123), None] {
        let current = record(value);
        let current_wire = candid::encode_one(&current).expect("encode current execution");
        let current_decoded: ExecutionRecordV1 =
            candid::decode_one(&current_wire).expect("decode current execution");
        assert_eq!(current_decoded.realized_profit, value);

        let terminal_wire = candid::encode_one(&current_decoded).expect("encode terminal execution");
        let terminal_decoded: ExecutionRecordV1 =
            candid::decode_one(&terminal_wire).expect("decode terminal execution");
        assert_eq!(terminal_decoded.realized_profit, value);

        let detail = RouteExecutionDetailV1 {
            asset_path: vec![Asset::CkUsdc, Asset::IcUsd, Asset::CkUsdt, Asset::CkUsdc],
            legs: vec![],
            detail_available: true,
            record: terminal_decoded,
        };
        let detail_wire = candid::encode_one(&detail).expect("encode execution detail");
        let detail_decoded: RouteExecutionDetailV1 =
            candid::decode_one(&detail_wire).expect("decode execution detail");
        assert_eq!(detail_decoded.record.realized_profit, value);
    }
}
