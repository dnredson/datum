use datum::agent::operational_reconciliation::{
    build_trusted_container_run_args, run_embedded_reconciler, OPERATIONAL_RECONCILER_FUNCTION,
    OPERATIONAL_RECONCILER_MODULE_ID,
};
use datum::config::{DispatchContainerMapping, DispatchContainerPortMapping};

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

#[test]
fn mqtt_trusted_mapping_builds_typed_port_and_restart_arguments() {
    let mapping = DispatchContainerMapping {
        module_id: None,
        service_name: Some("mqtt".to_string()),
        container_name: "datum-mqtt".to_string(),
        artifact: Some("eclipse-mosquitto:2".to_string()),
        ports: vec![DispatchContainerPortMapping {
            host_port: 1883,
            container_port: 1883,
            protocol: "tcp".to_string(),
            host_ip: None,
        }],
        restart_policy: Some("unless-stopped".to_string()),
    };

    let args = build_trusted_container_run_args(&mapping).expect("typed Docker args should build");
    assert_eq!(
        args,
        vec![
            "run",
            "-d",
            "--name",
            "datum-mqtt",
            "--restart",
            "unless-stopped",
            "-p",
            "1883:1883/tcp",
            "eclipse-mosquitto:2",
        ]
    );
}
