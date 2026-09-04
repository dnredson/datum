use datum::agent::bootstrap::{classify_bootstrap_state, BootstrapState};

#[test]
fn bootstrap_classifies_bound_unbound_and_unknown_resources_explicitly() {
    assert_eq!(
        classify_bootstrap_state(true, true, true),
        BootstrapState::KnownBoundNode
    );
    assert_eq!(
        classify_bootstrap_state(false, true, true),
        BootstrapState::KnownBoundNode
    );
    assert_eq!(
        classify_bootstrap_state(false, true, false),
        BootstrapState::KnownUnboundResource
    );
    assert_eq!(
        classify_bootstrap_state(false, false, false),
        BootstrapState::UnknownResource
    );
}

#[test]
fn resource_presence_without_binding_does_not_become_a_known_node() {
    assert_ne!(
        classify_bootstrap_state(false, true, false),
        BootstrapState::KnownBoundNode
    );
}
