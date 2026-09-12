use datum::agent::service_artifact::ServiceArtifact;
use datum::agent::service_deployment_reconciliation::{
    build_docker_run_args, evaluate_service_deployment_reconciliation_gate,
};
use serde_json::json;

fn mqtt_artifact() -> ServiceArtifact {
    serde_json::from_value(json!({
        "schema_version": "0.2.0",
        "artifact_id": "service:mqtt:mosquitto-2",
        "service_id": "mqtt",
        "runtime": {
            "kind": "container",
            "container": {
                "container_name": "datum-mqtt",
                "image": "eclipse-mosquitto:2",
                "ports": [
                    {
                        "container_port": 1883,
                        "host_port": 1883,
                        "protocol": "tcp"
                    }
                ]
            }
        },
        "probes": {
            "execution": {
                "kind": "docker_container_running",
                "vantage_point": "local_runtime"
            },
            "availability": {
                "kind": "mqtt_connect",
                "params": {
                    "host": "127.0.0.1",
                    "port": 1883
                },
                "vantage_point": "local_host"
            }
        }
    }))
    .unwrap()
}

#[test]
fn deployment_reconciliation_gate_fails_closed_in_order() {
    let not_desired = evaluate_service_deployment_reconciliation_gate(true, false, true, true);
    assert!(!not_desired.allowed);
    assert_eq!(not_desired.status, "blocked_not_desired");

    let dry_run = evaluate_service_deployment_reconciliation_gate(false, true, true, true);
    assert!(!dry_run.allowed);
    assert_eq!(dry_run.status, "suggested");

    let no_control_auth = evaluate_service_deployment_reconciliation_gate(true, true, false, true);
    assert!(!no_control_auth.allowed);
    assert_eq!(no_control_auth.status, "blocked_control_plane");

    let no_local_policy = evaluate_service_deployment_reconciliation_gate(true, true, true, false);
    assert!(!no_local_policy.allowed);
    assert_eq!(no_local_policy.status, "blocked_local_policy");

    let allowed = evaluate_service_deployment_reconciliation_gate(true, true, true, true);
    assert!(allowed.allowed);
    assert_eq!(allowed.status, "authorized");
}

#[test]
fn trusted_container_artifact_builds_typed_docker_run_arguments() {
    let args = build_docker_run_args(&mqtt_artifact()).unwrap();
    assert_eq!(
        args,
        vec![
            "run",
            "-d",
            "--name",
            "datum-mqtt",
            "-p",
            "1883:1883/tcp",
            "eclipse-mosquitto:2"
        ]
    );
}

#[test]
fn container_command_is_appended_as_literal_arguments_after_image() {
    let mut artifact = mqtt_artifact();
    let command = vec!["-c".to_string(), "/etc/chirpstack".to_string(), "$(literal); text".to_string()];
    artifact.runtime.container.as_mut().unwrap().command = command.clone();
    let args = build_docker_run_args(&artifact).unwrap();
    assert_eq!(&args[args.len() - command.len()..], command.as_slice());
    assert_eq!(args[args.len() - command.len() - 1], "eclipse-mosquitto:2");
}

#[test]
fn container_command_rejects_nul_and_excessive_arguments() {
    let mut artifact = mqtt_artifact();
    for command in [vec!["bad\0arg".to_string()], vec!["x".to_string(); 65], vec!["x".repeat(4097)]] {
        artifact.runtime.container.as_mut().unwrap().command = command;
        assert!(build_docker_run_args(&artifact).is_err());
    }
}
