use serde_json::{json, Value};

use super::{
    controlled_dgraph_deployment::{
        derive_deployment_manifest_from_dgraph, materialize_controlled_observed_dgraph,
    },
    deployment_manifest::{DeploymentStrategy, GraphDistribution},
    dgraph::{
        DGraph, DGraphArtifact, DGraphModule, DGraphNode, DGraphPlacementDecision,
        DGraphWasmModuleRef,
    },
};

const SHA: &str =
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn source_graph() -> DGraph {
    DGraph {
        project_id: "i34-live-project".to_string(),
        generated_at_utc: Some("2026-08-17T15:00:00-03:00".to_string()),
        nodes: vec![DGraphNode {
            node_id: "fog-01".to_string(),
            display_name: Some("Fog 01".to_string()),
            node_kind: Some("fog".to_string()),
            status: Some("available".to_string()),
            labels: vec!["i34".to_string()],
            metadata: None,
        }],
        modules: vec![DGraphModule {
            module_id: "fog-scale-module".to_string(),
            module_kind: "wasm".to_string(),
            desired_state: Some("executed".to_string()),
            observed_state: Some("unknown".to_string()),
            runtime: Some("wasmtime".to_string()),
            artifact_id: Some("fog-scale-artifact".to_string()),
            metadata: None,
        }],
        artifacts: vec![DGraphArtifact {
            artifact_id: "fog-scale-artifact".to_string(),
            artifact_kind: "wasm".to_string(),
            artifact_ref: Some(
                "http://192.168.10.107:8090/artifacts/fog-scale.wasm".to_string(),
            ),
            version: Some("i34".to_string()),
            metadata: Some(json!({"sha256": SHA})),
        }],
        placements: vec![DGraphPlacementDecision {
            placement_id: "placement:fog-scale".to_string(),
            module_id: "fog-scale-module".to_string(),
            selected_node_id: "fog-01".to_string(),
            selected_runtime_id: None,
            reason: Some("i34 test".to_string()),
            status: "selected".to_string(),
            metadata: None,
        }],
        wasm_modules: vec![DGraphWasmModuleRef {
            module_id: "fog-scale-module".to_string(),
            wasm_module_id: "fog-scale-wasm".to_string(),
            function_name: Some("fog_scale".to_string()),
            metadata: Some(json!({
                "assignment_id": "fog-scale",
                "fuel": 1_000_000,
                "timeout_ms": 1_000
            })),
        }],
        metadata: Some(json!({"dgraph_id": "i34-external-dgraph"})),
        ..DGraph::default()
    }
}

fn live_report() -> Value {
    json!({
        "execution_id": "exec-fog-1",
        "observed_at_utc": "2026-08-17T18:00:00Z",
        "manifest_id": "manifest:i34-external-dgraph",
        "project_id": "i34-live-project",
        "node_id": "fog-01",
        "assignment_id": "fog-scale",
        "module_id": "fog-scale-module",
        "function_name": "fog_scale",
        "artifact_content_id": format!("sha256:{SHA}"),
        "status": "executed",
        "results": [{"type": "i32", "value": 24}],
        "fuel_consumed": 4,
        "integrity_verified": true,
        "wasm_guest_side_effects": false,
        "governed_execution_performed": false,
        "acquisition": {"host_network_fetch_performed": true}
    })
}

#[test]
fn derives_distributed_manifest_from_external_dgraph() {
    let manifest = derive_deployment_manifest_from_dgraph(&source_graph()).unwrap();
    assert_eq!(manifest.manifest_id, "manifest:i34-external-dgraph");
    assert_eq!(
        manifest.source_dgraph_id.as_deref(),
        Some("i34-external-dgraph")
    );
    assert_eq!(manifest.strategy, DeploymentStrategy::Distributed);
    assert_eq!(manifest.graph_distribution, GraphDistribution::NodeSlice);
    assert_eq!(manifest.assignments.len(), 1);
    let plan = manifest
        .metadata
        .as_ref()
        .and_then(|value| value.get("dependency_plan"))
        .expect("derived manifest should materialize dependency plan");
    assert_eq!(plan["stage_count"].as_u64(), Some(1));
    assert_eq!(plan["dependency_edge_count"].as_u64(), Some(0));
}

#[test]
fn derived_assignment_preserves_target_function_and_integrity() {
    let manifest = derive_deployment_manifest_from_dgraph(&source_graph()).unwrap();
    let assignment = &manifest.assignments[0];
    assert_eq!(assignment.assignment_id, "fog-scale");
    assert_eq!(assignment.target_node, "fog-01");
    assert_eq!(assignment.function_name, "fog_scale");
    assert_eq!(assignment.artifact.sha256, SHA);
    assert_eq!(
        assignment.artifact.content_id,
        format!("sha256:{SHA}")
    );
    assert_eq!(
        assignment
            .metadata
            .as_ref()
            .and_then(|value| value.get("deployment_stage"))
            .and_then(Value::as_u64),
        Some(0)
    );
}

#[test]
fn derived_manifest_keeps_fail_closed_execution_boundary() {
    let manifest = derive_deployment_manifest_from_dgraph(&source_graph()).unwrap();
    assert!(manifest.execution_boundary.explicit_invocation_required);
    assert!(!manifest.execution_boundary.automatic_execution_allowed);
    assert!(!manifest.execution_boundary.governed_execution_authorized);
}

#[test]
fn missing_dgraph_id_is_rejected() {
    let mut graph = source_graph();
    graph.metadata = None;
    let error = derive_deployment_manifest_from_dgraph(&graph).unwrap_err();
    assert!(error.contains("metadata.dgraph_id"));
}

#[test]
fn missing_artifact_sha256_is_rejected() {
    let mut graph = source_graph();
    graph.artifacts[0].metadata = None;
    let error = derive_deployment_manifest_from_dgraph(&graph).unwrap_err();
    assert!(error.contains("metadata.sha256"));
}

#[test]
fn ambiguous_placement_is_rejected() {
    let mut graph = source_graph();
    graph.placements.push(graph.placements[0].clone());
    graph.placements[1].placement_id = "placement:duplicate".to_string();
    let error = derive_deployment_manifest_from_dgraph(&graph).unwrap_err();
    assert!(error.contains("exactly one placement"));
}

#[test]
fn live_report_materializes_observed_wasm_state() {
    let graph = source_graph();
    let manifest = derive_deployment_manifest_from_dgraph(&graph).unwrap();
    let observed =
        materialize_controlled_observed_dgraph(&graph, &manifest, &[live_report()]).unwrap();

    assert_eq!(observed.report_count, 1);
    assert!(observed.validation_errors.is_empty());
    assert_eq!(observed.graph.wasm_reports.len(), 1);
    assert_eq!(observed.graph.wasm_reports[0].report_id, "exec-fog-1");
    assert_eq!(
        observed.graph.modules[0].observed_state.as_deref(),
        Some("executed")
    );
    assert_eq!(
        observed.graph.modules[0]
            .metadata
            .as_ref()
            .and_then(|value| value.get("deployment_stage"))
            .and_then(Value::as_u64),
        Some(0)
    );
    assert!(observed
        .metadata
        .as_ref()
        .and_then(|value| value.get("dependency_plan"))
        .is_some());
    assert!(!observed.side_effects_performed);
}

#[test]
fn observed_dgraph_links_report_to_node_and_wasm_descriptor() {
    let graph = source_graph();
    let manifest = derive_deployment_manifest_from_dgraph(&graph).unwrap();
    let observed =
        materialize_controlled_observed_dgraph(&graph, &manifest, &[live_report()]).unwrap();

    let links = observed
        .graph
        .relationships
        .iter()
        .filter(|rel| {
            rel.from_kind == "WasmRuntimeReport"
                && rel.from_id == "exec-fog-1"
                && rel.relation == "OBSERVES"
        })
        .count();
    assert_eq!(links, 2);
}
