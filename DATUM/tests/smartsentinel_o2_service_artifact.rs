use std::collections::BTreeMap;
use std::path::PathBuf;

use datum::agent::service_artifact::{
    load_service_artifact, validate_service_artifact, ContainerDeploymentSpec, ContainerPortMapping,
    ServiceArtifact, ServiceProbeSet, ServiceProbeSpec, ServiceRuntimeSpec,
    SERVICE_ARTIFACT_SCHEMA_VERSION,
};

#[test]
fn mqtt_service_artifact_fixture_is_valid_and_declares_host_exposure() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../scripts/fixtures/o2-service-mqtt-artifact.json");
    let artifact = load_service_artifact(&path).expect("MQTT service artifact fixture should load");
    assert_eq!(artifact.service_id, "mqtt");
    let container = artifact
        .runtime
        .container
        .as_ref()
        .expect("MQTT artifact should use a container runtime");
    assert_eq!(container.container_name, "datum-mqtt");
    assert_eq!(container.ports.len(), 1);
    assert_eq!(container.ports[0].container_port, 1883);
    assert_eq!(container.ports[0].host_port, Some(1883));
    assert_eq!(artifact.probes.execution.kind, "docker_container_running");
    assert_eq!(
        artifact.probes.availability.as_ref().unwrap().kind,
        "mqtt_connect"
    );
}

#[test]
fn unsupported_execution_probe_is_rejected() {
    let artifact = ServiceArtifact {
        schema_version: SERVICE_ARTIFACT_SCHEMA_VERSION.to_string(),
        artifact_id: "service:test".to_string(),
        service_id: "test".to_string(),
        runtime: ServiceRuntimeSpec {
            kind: "container".to_string(),
            container: Some(ContainerDeploymentSpec {
                container_name: "test".to_string(),
                image: "example:test".to_string(),
                ports: vec![ContainerPortMapping {
                    container_port: 1234,
                    host_port: None,
                    protocol: "tcp".to_string(),
                }],
                network: None,
                volumes: Vec::new(),
            configuration_mounts: Vec::new(),
            command: Vec::new(),
            }),
        },
        probes: ServiceProbeSet {
            execution: ServiceProbeSpec {
                kind: "arbitrary_shell".to_string(),
                params: BTreeMap::new(),
                vantage_point: None,
            },
            health: None,
            availability: None,
        },
        metadata: None,
    };
    assert!(validate_service_artifact(&artifact).is_err());
}


#[test]
fn magistrala_bootstrap_db_catalog_contract_is_valid() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../artifacts/catalog/magistrala-bootstrap-db.json");
    let raw = std::fs::read_to_string(&path).expect("Bootstrap DB artifact should be readable");
    let document: serde_json::Value =
        serde_json::from_str(&raw).expect("Bootstrap DB artifact should be valid JSON");

    assert_eq!(
        document["artifact_id"].as_str(),
        Some("magistrala-bootstrap-db")
    );
    // Independent review v3 (F6 provenance, second instance found during the
    // final full-suite run): the already-committed foundation-closure work
    // (`95349e1`) correctly moved this catalog entry from a mutable tag to a
    // digest-pinned identity; this golden-fixture assertion was not updated
    // alongside it. Not caused by, and unrelated to, the D-Code work in this
    // pass -- fixed here as the same baseline-regression hygiene as F6.
    assert_eq!(
        document["runtime"]["container"]["image"].as_str(),
        Some("docker.io/library/postgres@sha256:d2ae11f7207eb2c726b1678d7f98df2210759b8e5014d77afa9f77d014e33a9e")
    );
    assert_eq!(
        document["runtime"]["container"]["network"].as_str(),
        Some("datum-magistrala-net")
    );
    assert_eq!(
        document["runtime"]["container"]["volumes"][0]["name"].as_str(),
        Some("datum-magistrala-bootstrap-db-volume")
    );
    assert_eq!(
        document["control_plane"]["bindings"]["POSTGRES_DB"].as_str(),
        Some("bootstrap")
    );
    assert_eq!(
        document["control_plane"]["bindings"]["POSTGRES_USER"].as_str(),
        Some("magistrala")
    );
    assert_eq!(
        document["control_plane"]["secret_refs"][0].as_str(),
        Some("magistrala_bootstrap_db_password")
    );

    let artifact = load_service_artifact(&path)
        .expect("Bootstrap DB artifact should satisfy the SmartSentinel artifact schema");
    validate_service_artifact(&artifact)
        .expect("Bootstrap DB artifact should pass SmartSentinel validation");
}


#[test]
fn magistrala_bootstrap_db_uses_container_health_contract() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../artifacts/catalog/magistrala-bootstrap-db.json");
    let raw = std::fs::read_to_string(&path).expect("Bootstrap DB artifact should be readable");
    let document: serde_json::Value =
        serde_json::from_str(&raw).expect("Bootstrap DB artifact should be valid JSON");

    assert!(document["probes"]["health"].is_null());
    assert_eq!(
        document["metadata"]["configuration"]["health_contract"].as_str(),
        Some("docker_exec_pg_isready")
    );
}
#[test]
fn magistrala_atom_uses_mounted_bootstrap_configuration() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../artifacts/catalog/magistrala-atom.json");
    let artifact = load_service_artifact(&path)
        .expect("Atom artifact should satisfy the SmartSentinel artifact schema");
    let container = artifact.runtime.container.as_ref().expect("Atom should use a container runtime");
    assert_eq!(container.configuration_mounts.len(), 1);
    assert_eq!(container.configuration_mounts[0].source, "magistrala/atom-bootstrap.yaml");
    assert_eq!(artifact.metadata.unwrap()["configuration"]["note"], "v1.0.0 loads action and policy bootstrap directly at Atom startup; no atom-bootstrap job container is used.");
}
