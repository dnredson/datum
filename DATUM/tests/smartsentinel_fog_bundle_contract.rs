use datum::agent::service_artifact::load_service_artifact;
use datum::config::load_config;
use std::path::PathBuf;

#[test]
fn fog_core_artifacts_and_local_profile_validate() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let cfg = load_config(root.join("DATUM/config.fog.linux").to_str().unwrap()).unwrap();
    assert_eq!(cfg.agent.node_id, "fog-01");
    let dispatch = cfg.dispatch.unwrap();
    for id in ["mosquitto", "chirpstack-postgres", "chirpstack-redis", "chirpstack"] {
        let artifact = load_service_artifact(&root.join(format!("artifacts/catalog/{id}.json"))).unwrap();
        let c = artifact.runtime.container.unwrap();
        let mapping = dispatch.container_mappings.iter().find(|m| m.container_name == c.container_name).unwrap();
        assert_eq!(mapping.artifact.as_deref(), Some(c.image.as_str()));
        for mount in c.configuration_mounts {
            assert!(mount.read_only);
            assert!(root.join("artifacts/config").join(mount.source).exists());
        }
        if id == "chirpstack" {
            assert_eq!(c.command, vec!["-c", "/etc/chirpstack"]);
        }
    }
    for file in ["chirpstack.toml", "region_eu868.toml"] {
        let text = std::fs::read_to_string(root.join("artifacts/config/chirpstack/server").join(file)).unwrap();
        toml::from_str::<toml::Value>(&text).unwrap();
    }
}
