use datum::agent::service_loop::ServiceLoopOptions;

#[test]
fn monitor_only_is_opt_in_and_preserves_existing_default() {
    let options = ServiceLoopOptions::default();
    assert!(!options.monitor_only);
}

#[test]
fn monitor_only_primary_collection_is_structurally_read_only() {
    let source = include_str!("../src/agent/service_loop.rs");
    let monitor_branch = source
        .split("let primary_snapshot = if options.monitor_only")
        .nth(1)
        .expect("monitor-only primary collection branch must exist")
        .split("} else {")
        .next()
        .expect("monitor-only branch must end before standard branch");

    assert!(
        monitor_branch.contains("run_observation_only_cycle_for_node_identity"),
        "monitor-only mode must source its primary snapshot from the read-only observation pipeline"
    );
    assert!(
        !monitor_branch.contains("run_cycle_for_node_identity("),
        "monitor-only mode must not call the full planning/recovery cycle"
    );
}

#[test]
fn monitor_only_reuses_primary_snapshot_for_dmonitor_projection() {
    let source = include_str!("../src/agent/service_loop.rs");
    assert!(
        source.contains("options.monitor_only.then(|| snapshot.clone())"),
        "monitor-only mode should reuse its primary observation instead of collecting twice"
    );
}
