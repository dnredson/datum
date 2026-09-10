use datum::agent::service_artifact::load_service_artifact;
use datum::config::load_config;
use std::path::PathBuf;

#[test]
fn mist_mapping_and_configuration_match_gateway_artifact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let path = root.join("artifacts/catalog/chirpstack-gateway-bridge.json");
    let artifact = load_service_artifact(&path).unwrap();
    let document: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let cfg = load_config(root.join("DATUM/config.mist.linux").to_str().unwrap()).unwrap();
    assert_eq!(cfg.agent.node_id, "mist-01");
    let dispatch = cfg.dispatch.unwrap();
    assert!(dispatch.secret_bindings.is_empty());
    assert!(document["control_plane"]["secret_refs"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(document["control_plane"]["bindings"]
        .as_object()
        .unwrap()
        .is_empty());
    assert_eq!(
        dispatch.configuration_root.as_deref(),
        Some("/opt/datum/governed-config")
    );
    let container = artifact.runtime.container.unwrap();
    let mapping = &dispatch.container_mappings[0];
    assert_eq!(
        mapping.service_name.as_deref(),
        Some(artifact.service_id.as_str())
    );
    assert_eq!(mapping.container_name, container.container_name);
    assert_eq!(mapping.artifact.as_deref(), Some(container.image.as_str()));
    assert_eq!(mapping.ports.len(), container.ports.len());
    for port in &container.ports {
        assert!(mapping
            .ports
            .iter()
            .any(|p| p.host_port == port.host_port.unwrap()
                && p.container_port == port.container_port
                && p.protocol == port.protocol));
    }
    let mount = &container.configuration_mounts[0];
    assert!(mount.read_only);
    assert_eq!(
        mount.target,
        "/etc/chirpstack-gateway-bridge/chirpstack-gateway-bridge.toml"
    );
    let config: toml::Value = toml::from_str(
        &std::fs::read_to_string(
            root.join("artifacts/config")
                .join(&mount.source),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(config["backend"]["type"].as_str(), Some("semtech_udp"));
    assert_eq!(
        config["backend"]["semtech_udp"]["udp_bind"].as_str(),
        Some("0.0.0.0:1700")
    );
    assert_eq!(
        config["integration"]["mqtt"]["auth"]["generic"]["servers"][0].as_str(),
        Some("tcp://192.168.10.103:1883")
    );
    for key in [
        "event_topic_template",
        "state_topic_template",
        "command_topic_template",
    ] {
        assert_eq!(
            config["integration"]["mqtt"][key].as_str(),
            document["metadata"]["configuration"][key].as_str()
        );
    }
    assert_eq!(
        config["metrics"]["prometheus"]["endpoint_enabled"].as_bool(),
        Some(true)
    );
    assert_eq!(
        config["metrics"]["prometheus"]["bind"].as_str(),
        Some("0.0.0.0:9100")
    );
    assert_eq!(
        artifact.probes.health.unwrap().params["url"],
        "http://127.0.0.1:19100/metrics"
    );
    // Do not report broker connectivity as proof of bridge delivery.
    assert!(artifact.probes.availability.is_none());
}
