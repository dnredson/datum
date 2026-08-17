use std::collections::BTreeMap;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::core::{
    deployment_manifest::{
        build_deployment_artifact_identity, validate_deployment_manifest, DeploymentAssignment,
        DeploymentExecutionBoundary, DeploymentManifest, DeploymentRuntimeLimits,
        DeploymentStrategy, GraphDistribution, DEPLOYMENT_MANIFEST_VERSION,
    },
    dgraph::{
        build_basic_dgraph_relationships, summarize_dgraph, validate_dgraph, DGraph,
        DGraphSummary, DGraphWasmRuntimeReportRef,
    },
};

pub const I34_DEFAULT_FUEL: u64 = 1_000_000;
pub const I34_DEFAULT_TIMEOUT_MS: u64 = 1_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ControlledDGraphActivation {
    pub source_dgraph_id: String,
    pub project_id: String,
    pub manifest: DeploymentManifest,
    pub source_graph_summary: DGraphSummary,
    pub side_effects_performed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ControlledObservedDGraphState {
    pub source_dgraph_id: String,
    pub manifest_id: String,
    pub project_id: String,
    pub materialized_at_utc: String,
    pub report_count: u64,
    pub graph: DGraph,
    pub summary: DGraphSummary,
    pub validation_errors: Vec<String>,
    pub side_effects_performed: bool,
    pub metadata: Option<Value>,
}

pub fn controlled_source_dgraph_id(graph: &DGraph) -> Result<String, String> {
    metadata_string(&graph.metadata, "dgraph_id")
        .ok_or_else(|| "external D-Graph metadata.dgraph_id must be a non-empty string".to_string())
}

pub fn build_controlled_dgraph_activation(
    graph: &DGraph,
) -> Result<ControlledDGraphActivation, String> {
    let source_dgraph_id = controlled_source_dgraph_id(graph)?;
    let manifest = derive_deployment_manifest_from_dgraph(graph)?;
    Ok(ControlledDGraphActivation {
        source_dgraph_id,
        project_id: graph.project_id.clone(),
        manifest,
        source_graph_summary: summarize_dgraph(graph),
        side_effects_performed: false,
    })
}

pub fn derive_deployment_manifest_from_dgraph(graph: &DGraph) -> Result<DeploymentManifest, String> {
    if let Err(errors) = validate_dgraph(graph) {
        return Err(format!("external D-Graph validation failed: {}", errors.join("; ")));
    }
    let source_dgraph_id = controlled_source_dgraph_id(graph)?;
    if graph.wasm_modules.is_empty() {
        return Err("external D-Graph must contain at least one wasm_modules entry".to_string());
    }

    let manifest_id = metadata_string(&graph.metadata, "deployment_manifest_id")
        .unwrap_or_else(|| format!("manifest:{source_dgraph_id}"));
    let mut assignments = Vec::new();

    for wasm in &graph.wasm_modules {
        let module = graph
            .modules
            .iter()
            .find(|item| item.module_id == wasm.module_id)
            .ok_or_else(|| format!("WASM descriptor '{}' references missing module '{}'", wasm.wasm_module_id, wasm.module_id))?;
        let function_name = wasm
            .function_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("WASM descriptor '{}' requires function_name", wasm.wasm_module_id))?;
        let artifact_id = module
            .artifact_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("module '{}' requires artifact_id", module.module_id))?;
        let artifact = graph
            .artifacts
            .iter()
            .find(|item| item.artifact_id == artifact_id)
            .ok_or_else(|| format!("module '{}' references missing artifact '{}'", module.module_id, artifact_id))?;
        let artifact_uri = artifact
            .artifact_ref
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("artifact '{}' requires artifact_ref URI", artifact.artifact_id))?;
        let sha256 = metadata_string(&artifact.metadata, "sha256")
            .ok_or_else(|| format!("artifact '{}' metadata.sha256 is required", artifact.artifact_id))?;
        let identity = build_deployment_artifact_identity(&artifact.artifact_id, artifact_uri, &sha256)?;

        let placements = graph
            .placements
            .iter()
            .filter(|item| item.module_id == module.module_id)
            .collect::<Vec<_>>();
        if placements.len() != 1 {
            return Err(format!(
                "module '{}' requires exactly one placement decision for I34; found {}",
                module.module_id,
                placements.len()
            ));
        }
        let placement = placements[0];
        let target_node = placement.selected_node_id.trim();
        let assignment_id = metadata_string(&wasm.metadata, "assignment_id")
            .unwrap_or_else(|| format!("assignment:{}:{}", wasm.wasm_module_id, target_node));

        assignments.push(DeploymentAssignment {
            assignment_id,
            module_id: module.module_id.clone(),
            function_name: function_name.to_string(),
            artifact: identity,
            target_node: target_node.to_string(),
            limits: DeploymentRuntimeLimits {
                fuel: Some(metadata_u64(&wasm.metadata, "fuel").unwrap_or(I34_DEFAULT_FUEL)),
                timeout_ms: Some(
                    metadata_u64(&wasm.metadata, "timeout_ms").unwrap_or(I34_DEFAULT_TIMEOUT_MS),
                ),
            },
            metadata: Some(json!({
                "derivation_kind": "external_dgraph",
                "source_wasm_module_id": wasm.wasm_module_id,
                "source_placement_id": placement.placement_id,
            })),
        });
    }

    assignments.sort_by(|a, b| a.assignment_id.cmp(&b.assignment_id));
    let manifest = DeploymentManifest {
        manifest_version: DEPLOYMENT_MANIFEST_VERSION.to_string(),
        manifest_id,
        project_id: graph.project_id.clone(),
        strategy: DeploymentStrategy::Distributed,
        graph_distribution: GraphDistribution::NodeSlice,
        source_dgraph_id: Some(source_dgraph_id.clone()),
        assignments,
        execution_boundary: DeploymentExecutionBoundary::default(),
        metadata: Some(json!({
            "derivation_kind": "external_dgraph_to_controlled_wasm_manifest",
            "source_dgraph_id": source_dgraph_id,
        })),
    };
    validate_deployment_manifest(&manifest)?;
    Ok(manifest)
}

pub fn materialize_controlled_observed_dgraph(
    source: &DGraph,
    manifest: &DeploymentManifest,
    reports: &[Value],
) -> Result<ControlledObservedDGraphState, String> {
    let source_dgraph_id = controlled_source_dgraph_id(source)?;
    if source.project_id != manifest.project_id
        || manifest.source_dgraph_id.as_deref() != Some(source_dgraph_id.as_str())
    {
        return Err("source D-Graph identity does not match active manifest".to_string());
    }

    let now = Utc::now().to_rfc3339();
    let mut graph = source.clone();
    for report in reports {
        let execution_id = report_string(report, "execution_id")?;
        let node_id = report_string(report, "node_id")?;
        let assignment_id = report_string(report, "assignment_id")?;
        let module_id = report_string(report, "module_id")?;
        let function_name = report_string(report, "function_name")?;
        let status = report_string(report, "status")?;
        let content_id = report_string(report, "artifact_content_id")?;

        let assignment = manifest
            .assignments
            .iter()
            .find(|item| item.assignment_id == assignment_id)
            .ok_or_else(|| format!("report references assignment '{assignment_id}' outside manifest"))?;
        if assignment.module_id != module_id
            || assignment.function_name != function_name
            || assignment.target_node != node_id
            || assignment.artifact.content_id != content_id
        {
            return Err(format!("report execution '{execution_id}' does not match its assignment"));
        }

        let wasm_module_id = graph
            .wasm_modules
            .iter()
            .find(|item| item.module_id == module_id && item.function_name.as_deref() == Some(function_name))
            .map(|item| item.wasm_module_id.clone())
            .ok_or_else(|| format!("report execution '{execution_id}' has no matching WASM descriptor"))?;

        graph.wasm_reports.push(DGraphWasmRuntimeReportRef {
            report_id: execution_id.to_string(),
            node_id: node_id.to_string(),
            wasm_module_id: Some(wasm_module_id),
            status: status.to_string(),
            metadata: Some(json!({
                "source": "controlled_live_wasm_execution",
                "manifest_id": manifest.manifest_id,
                "assignment_id": assignment_id,
                "module_id": module_id,
                "function_name": function_name,
                "artifact_content_id": content_id,
                "observed_at_utc": report.get("observed_at_utc").cloned().unwrap_or(Value::Null),
                "results": report.get("results").cloned().unwrap_or(Value::Null),
                "fuel_consumed": report.get("fuel_consumed").cloned().unwrap_or(Value::Null),
                "integrity_verified": report.get("integrity_verified").cloned().unwrap_or(Value::Null),
            })),
        });

        let module = graph
            .modules
            .iter_mut()
            .find(|item| item.module_id == module_id)
            .ok_or_else(|| format!("report references missing D-Graph module '{module_id}'"))?;
        module.observed_state = Some(status.to_string());
        module.metadata = Some(merge_metadata(
            module.metadata.take(),
            json!({
                "last_wasm_execution_id": execution_id,
                "last_wasm_execution_node_id": node_id,
                "last_artifact_content_id": content_id,
            }),
        ));
    }

    graph.generated_at_utc = Some(now.clone());
    graph.metadata = Some(merge_metadata(
        graph.metadata.take(),
        json!({
            "materialization_kind": "controlled_wasm_observed_dgraph",
            "source_dgraph_id": source_dgraph_id,
            "manifest_id": manifest.manifest_id,
            "report_count": reports.len(),
        }),
    ));

    let mut relationships = BTreeMap::new();
    for rel in graph.relationships.drain(..) {
        relationships.insert(rel.relationship_id.clone(), rel);
    }
    for rel in build_basic_dgraph_relationships(&graph) {
        relationships.insert(rel.relationship_id.clone(), rel);
    }
    graph.relationships = relationships.into_values().collect();
    graph.wasm_reports.sort_by(|a, b| a.report_id.cmp(&b.report_id));

    let validation_errors = validate_dgraph(&graph).err().unwrap_or_default();
    let summary = summarize_dgraph(&graph);
    Ok(ControlledObservedDGraphState {
        source_dgraph_id,
        manifest_id: manifest.manifest_id.clone(),
        project_id: manifest.project_id.clone(),
        materialized_at_utc: now,
        report_count: reports.len() as u64,
        graph,
        summary,
        validation_errors,
        side_effects_performed: false,
        metadata: Some(json!({
            "observation_source": "controlled_live_wasm_reports",
            "governed_execution_performed": false,
            "safe_to_auto_execute": false,
        })),
    })
}

fn metadata_string(metadata: &Option<Value>, key: &str) -> Option<String> {
    metadata.as_ref()?.as_object()?.get(key)?.as_str().map(str::trim)
        .filter(|value| !value.is_empty()).map(ToString::to_string)
}

fn metadata_u64(metadata: &Option<Value>, key: &str) -> Option<u64> {
    metadata.as_ref()?.as_object()?.get(key)?.as_u64()
}

fn report_string<'a>(report: &'a Value, key: &str) -> Result<&'a str, String> {
    report.get(key).and_then(Value::as_str).map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("controlled live report field '{key}' must be a non-empty string"))
}

fn merge_metadata(existing: Option<Value>, extra: Value) -> Value {
    let mut object = match existing {
        Some(Value::Object(object)) => object,
        Some(value) => {
            let mut object = Map::new();
            object.insert("source_metadata".to_string(), value);
            object
        }
        None => Map::new(),
    };
    if let Value::Object(extra) = extra {
        object.extend(extra);
    }
    Value::Object(object)
}
