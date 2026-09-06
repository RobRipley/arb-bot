use arb_bot::route_runtime::RuntimeStatus;
use arb_bot::route_scheduler::{next_action,TickAction};
use arb_bot::route_arb::{ObservationAccumulatorV1,RouteCandidateReportV1,CandidateClass};
fn status()->RuntimeStatus{RuntimeStatus{compiled_support:true,live_authorized:true,enabled:true,dry_run:false,last_error:None,last_tick_ns:0,scheduler_in_flight_since_ns:None,last_realized_profit:None,last_profit_class:None}}
fn observation()->ObservationAccumulatorV1{ObservationAccumulatorV1::new("obs".into(),1,0,972,2644,true)}
#[test]
fn disabled_trading_stays_idle_but_existing_attempt_always_reconciles(){
    for s in [RuntimeStatus{live_authorized:false,..status()},RuntimeStatus{enabled:false,..status()},RuntimeStatus{dry_run:true,..status()}] {
        assert_eq!(next_action(&s,false,None),TickAction::Idle);
        assert_eq!(next_action(&s,true,None),TickAction::ServiceExecution);
    }
}
#[test]
fn scheduler_scans_whole_universe_before_selecting_and_restarts_no_winner_scan(){
    let s=status(); assert_eq!(next_action(&s,false,None),TickAction::StartObservation);
    let mut o=observation();o.next_cursor=100;
    assert_eq!(next_action(&s,false,Some(&o)),TickAction::QuoteBatch(100));
    o.scan_complete=true;
    assert_eq!(next_action(&s,false,Some(&o)),TickAction::StartObservation);
    o.best_stable_candidate=Some(RouteCandidateReportV1::fixture("route",CandidateClass::StablePar,10,true));
    assert_eq!(next_action(&s,false,Some(&o)),TickAction::SelectRoute);
    assert_eq!(next_action(&s,true,Some(&o)),TickAction::ServiceExecution);
}
