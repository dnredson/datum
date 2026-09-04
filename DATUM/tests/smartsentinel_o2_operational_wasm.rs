use datum::agent::operational_reconciliation::{
    run_embedded_reconciler, OPERATIONAL_RECONCILER_FUNCTION, OPERATIONAL_RECONCILER_MODULE_ID,
};

#[test]
fn missing_desired_service_produces_typed_ensure_intent_in_real_wasmtime() {
    let decision = run_embedded_reconciler(true, false).expect("embedded reconciler should execute");
    assert_eq!(decision.module_id, OPERATIONAL_RECONCILER_MODULE_ID);
    assert_eq!(decision.function_name, OPERATIONAL_RECONCILER_FUNCTION);
    assert_eq!(decision.status, "executed");
    assert_eq!(decision.action_code, 1);
    assert!(!decision.host_imports_allowed);
    assert!(!decision.guest_side_effects_performed);
    assert!(decision.fuel_consumed.is_some());
}

#[test]
fn already_running_desired_service_produces_no_host_intent() {
    let decision = run_embedded_reconciler(true, true).expect("embedded reconciler should execute");
    assert_eq!(decision.action_code, 0);
    assert!(!decision.guest_side_effects_performed);
}
