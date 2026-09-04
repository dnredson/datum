use datum::agent::bootstrap::classify_node_lookup_status;
use datum::agent::discovery::BootstrapMode;
use reqwest::StatusCode;

#[test]
fn bootstrap_classifies_known_and_unknown_nodes_explicitly() {
    assert_eq!(
        classify_node_lookup_status(StatusCode::OK).unwrap(),
        BootstrapMode::KnownNode
    );
    assert_eq!(
        classify_node_lookup_status(StatusCode::NOT_FOUND).unwrap(),
        BootstrapMode::Discovery
    );
}

#[test]
fn bootstrap_refuses_to_guess_on_control_plane_failure() {
    assert!(classify_node_lookup_status(StatusCode::INTERNAL_SERVER_ERROR).is_err());
    assert!(classify_node_lookup_status(StatusCode::UNAUTHORIZED).is_err());
}
