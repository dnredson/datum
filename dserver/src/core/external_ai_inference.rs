use std::collections::{BTreeMap, BTreeSet};
use std::sync::{OnceLock, RwLock};

use serde_json::{json, Value};

pub const EXTERNAL_AI_INFERENCE_REQUEST_SCHEMA_VERSION: &str = "0.1.0";
pub const EXTERNAL_AI_INFERENCE_REQUEST_KIND: &str = "datum_ai_what_if_inference_request";
pub const EXTERNAL_AI_INFERENCE_RESPONSE_KIND: &str = "datum_ai_what_if_inference_response";
pub const VALIDATED_AI_PROJECTION_KIND: &str = "datum_validated_ai_what_if_projection";

const EXTERNAL_AI_REQUEST_CAPACITY: usize = 256;
type IssuedRequestStore = BTreeMap<String, Value>;

static ISSUED_REQUESTS: OnceLock<RwLock<IssuedRequestStore>> = OnceLock::new();

fn issued_requests() -> &'static RwLock<IssuedRequestStore> {
    ISSUED_REQUESTS.get_or_init(|| RwLock::new(BTreeMap::new()))
}

/// Records the exact request envelope emitted to an external consumer.
pub fn register_external_ai_inference_request(request: &Value) -> Result<(), String> {
    let request_id = required_text(request, "request_id")?;
    let mut guard = issued_requests()
        .write()
        .map_err(|error| format!("external AI request store lock poisoned: {error}"))?;

    if guard.len() >= EXTERNAL_AI_REQUEST_CAPACITY && !guard.contains_key(&request_id) {
        if let Some(oldest) = guard.keys().next().cloned() {
            guard.remove(&oldest);
        }
    }
    guard.insert(request_id, request.clone());
    Ok(())
}

/// Retrieves an exact request previously emitted by this dserver process.
pub fn get_issued_external_ai_inference_request(
    request_id: &str,
) -> Result<Option<Value>, String> {
    let request_id = request_id.trim();
    if request_id.is_empty() {
        return Ok(None);
    }
    let guard = issued_requests()
        .read()
        .map_err(|error| format!("external AI request store lock poisoned: {error}"))?;
    Ok(guard.get(request_id).cloned())
}

/// Wraps the historical what-if context in the provider-neutral request contract
/// consumed by an external AI/GNN system. The dserver does not call a provider.
pub fn build_external_ai_inference_request(context: &Value) -> Result<Value, String> {
    build_external_ai_inference_request_with_id(context, None)
}

pub fn build_external_ai_inference_request_with_id(
    context: &Value,
    requested_request_id: Option<&str>,
) -> Result<Value, String> {
    let project_id = required_text(context, "project_id")?;
    let dgraph_id = required_text(context, "dgraph_id")?;
    let dgraph_revision = required_u64(context, "dgraph_revision")?;
    let context_kind = required_text(context, "contract_kind")?;

    if context_kind != "datum_alternative_path_what_if_context" {
        return Err(format!(
            "external AI request requires datum_alternative_path_what_if_context, got '{}'",
            context_kind
        ));
    }

    let request_id = requested_request_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("what-if-{}", uuid::Uuid::new_v4()));
    Ok(json!({
        "schema_version": EXTERNAL_AI_INFERENCE_REQUEST_SCHEMA_VERSION,
        "contract_kind": EXTERNAL_AI_INFERENCE_REQUEST_KIND,
        "request_id": request_id,
        "task": "project_alternative_paths_from_historical_telemetry",
        "project_id": project_id,
        "dgraph_id": dgraph_id,
        "dgraph_revision": dgraph_revision,
        "input_context": context,
        "constraints": {
            "requested_scope": context.get("requested_scope").cloned().unwrap_or(Value::Null),
            "historical_data_required": true,
            "observed_topology_required": true,
            "max_projection_count": 32,
            "confidence_range": [0.0, 1.0]
        },
        "execution_policy": {
            "mode": "proposal_only",
            "allow_actions": false,
            "allow_topology_changes": false,
            "allow_deployment": false,
            "side_effects_performed": false
        },
        "inference": {
            "provider_mode": "external_ai_required",
            "training_performed": false,
            "prediction_performed": false,
            "projection_available": false
        },
        "side_effects_performed": false
    }))
}

/// Validates an external model response against the exact graph revision and
/// node set used to build the request. It returns a normalized, proposal-only
/// result; it never persists or executes the projections.
pub fn validate_external_ai_inference_response(
    request: &Value,
    response: &Value,
) -> Result<Value, String> {
    let mut errors = Vec::new();

    if !response.is_object() {
        return Err("external AI inference response must be a JSON object".to_string());
    }

    if text(response, "schema_version")
        != Some(EXTERNAL_AI_INFERENCE_REQUEST_SCHEMA_VERSION.to_string())
    {
        errors.push(format!(
            "schema_version must be '{}'",
            EXTERNAL_AI_INFERENCE_REQUEST_SCHEMA_VERSION
        ));
    }
    if text(response, "contract_kind") != Some(EXTERNAL_AI_INFERENCE_RESPONSE_KIND.to_string()) {
        errors.push(format!(
            "contract_kind must be '{}'",
            EXTERNAL_AI_INFERENCE_RESPONSE_KIND
        ));
    }

    for field in ["request_id", "project_id", "dgraph_id"] {
        if text(response, field).is_none() {
            errors.push(format!("response field '{}' must be a non-empty string", field));
        }
    }

    for field in ["project_id", "dgraph_id"] {
        if let (Some(expected), Some(received)) = (text(request, field), text(response, field)) {
            if expected != received {
                errors.push(format!(
                    "response {} '{}' does not match request '{}'",
                    field, received, expected
                ));
            }
        }
    }
    if let (Some(expected), Some(received)) = (
        text(request, "request_id"),
        text(response, "request_id"),
    ) {
        if expected != received {
            errors.push(format!(
                "response request_id '{}' does not match request '{}'",
                received, expected
            ));
        }
    }
    if response.get("dgraph_revision").and_then(Value::as_u64)
        != request.get("dgraph_revision").and_then(Value::as_u64)
    {
        errors.push("response dgraph_revision does not match request".to_string());
    }

    if response
        .get("side_effects_performed")
        .and_then(Value::as_bool)
        != Some(false)
    {
        errors.push("response.side_effects_performed must be false".to_string());
    }

    let node_ids = request_node_ids(request);
    if node_ids.is_empty() {
        errors.push("request input_context must contain at least one observed node".to_string());
    }

    let model = response.get("model").cloned().unwrap_or(Value::Null);
    if !model.is_object() {
        errors.push("response model must be a JSON object".to_string());
    }
    if model
        .get("training_performed")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        errors.push("training_performed must be false for this inference boundary".to_string());
    }

    let execution = response.get("execution").cloned().unwrap_or(Value::Null);
    if !execution.is_object() {
        errors.push("response execution must be a JSON object".to_string());
    } else {
        for field in ["changes_applied", "execution_allowed", "side_effects_performed"] {
            if execution.get(field).and_then(Value::as_bool) != Some(false) {
                errors.push(format!("execution.{} must be false", field));
            }
        }
        if execution.get("mode").and_then(Value::as_str) != Some("proposal_only") {
            errors.push("execution.mode must be 'proposal_only'".to_string());
        }
    }

    let projections = response
        .get("projections")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if response.get("projections").and_then(Value::as_array).is_none() {
        errors.push("response projections must be an array".to_string());
    }
    if projections.len() > 32 {
        errors.push("response projections exceeds max_projection_count=32".to_string());
    }

    let max_hops = request
        .get("input_context")
        .and_then(|value| value.get("requested_scope"))
        .and_then(|value| value.get("max_hops"))
        .and_then(Value::as_u64)
        .unwrap_or(4) as usize;
    let requested_from = request
        .get("input_context")
        .and_then(|value| value.get("requested_scope"))
        .and_then(|value| value.get("from"))
        .and_then(Value::as_str);
    let requested_to = request
        .get("input_context")
        .and_then(|value| value.get("requested_scope"))
        .and_then(|value| value.get("to"))
        .and_then(Value::as_str);

    let allowed_edges = request_observed_edges(request);
    let mut path_ids = BTreeSet::new();
    for (index, projection) in projections.iter().enumerate() {
        validate_projection(
            projection,
            index,
            &node_ids,
            &allowed_edges,
            max_hops,
            requested_from,
            requested_to,
            &mut path_ids,
            &mut errors,
        );
    }

    if !errors.is_empty() {
        return Err(errors.join("; "));
    }

    Ok(json!({
        "schema_version": EXTERNAL_AI_INFERENCE_REQUEST_SCHEMA_VERSION,
        "contract_kind": VALIDATED_AI_PROJECTION_KIND,
        "request_id": request.get("request_id").cloned().unwrap_or(Value::Null),
        "project_id": request.get("project_id").cloned().unwrap_or(Value::Null),
        "dgraph_id": request.get("dgraph_id").cloned().unwrap_or(Value::Null),
        "dgraph_revision": request.get("dgraph_revision").cloned().unwrap_or(Value::Null),
        "model": model,
        "projections": projections,
        "validation": {
            "accepted": true,
            "projection_count": projections.len(),
            "errors": []
        },
        "execution": {
            "mode": "proposal_only",
            "changes_applied": false,
            "execution_allowed": false,
            "side_effects_performed": false
        },
        "side_effects_performed": false
    }))
}

fn validate_projection(
    projection: &Value,
    index: usize,
    node_ids: &BTreeSet<String>,
    allowed_edges: &BTreeSet<(String, String)>,
    max_hops: usize,
    requested_from: Option<&str>,
    requested_to: Option<&str>,
    path_ids: &mut BTreeSet<String>,
    errors: &mut Vec<String>,
) {
    let label = format!("projections[{}]", index);
    if !projection.is_object() {
        errors.push(format!("{} must be a JSON object", label));
        return;
    }

    let path_id = match projection.get("path_id").and_then(Value::as_str) {
        Some(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => {
            errors.push(format!("{}.path_id must be a non-empty string", label));
            String::new()
        }
    };
    if !path_id.is_empty() && !path_ids.insert(path_id) {
        errors.push(format!("{}.path_id must be unique", label));
    }

    let from = projection.get("from").and_then(Value::as_str);
    let to = projection.get("to").and_then(Value::as_str);
    if from.is_none() || to.is_none() {
        errors.push(format!("{}.from and {}.to are required", label, label));
    }
    for (field, value) in [("from", from), ("to", to)] {
        if let Some(node_id) = value {
            if !node_ids.contains(node_id) {
                errors.push(format!(
                    "{}.{} node '{}' is absent from the request topology",
                    label, field, node_id
                ));
            }
        }
    }

    if let Some(expected) = requested_from {
        if from != Some(expected) {
            errors.push(format!("{}.from must match requested scope '{}'", label, expected));
        }
    }
    if let Some(expected) = requested_to {
        if to != Some(expected) {
            errors.push(format!("{}.to must match requested scope '{}'", label, expected));
        }
    }

    let nodes = match projection.get("nodes").and_then(Value::as_array) {
        Some(nodes) if !nodes.is_empty() => nodes,
        _ => {
            errors.push(format!("{}.nodes must be a non-empty array", label));
            return;
        }
    };
    if from != nodes.first().and_then(Value::as_str) {
        errors.push(format!("{}.nodes[0] must equal from", label));
    }
    if to != nodes.last().and_then(Value::as_str) {
        errors.push(format!("{}.nodes[last] must equal to", label));
    }
    let mut seen_nodes = BTreeSet::new();
    for node in nodes {
        match node.as_str() {
            Some(node_id) if node_ids.contains(node_id) => {
                if !seen_nodes.insert(node_id.to_string()) {
                    errors.push(format!(
                        "{}.nodes must not repeat node '{}'",
                        label, node_id
                    ));
                }
            }
            Some(node_id) => errors.push(format!(
                "{}.nodes contains unknown node '{}'",
                label, node_id
            )),
            None => errors.push(format!("{}.nodes must contain only strings", label)),
        }
    }

    for pair in nodes.windows(2) {
        let from_node = pair.first().and_then(Value::as_str);
        let to_node = pair.get(1).and_then(Value::as_str);
        if let (Some(from_node), Some(to_node)) = (from_node, to_node) {
            if !allowed_edges.contains(&(from_node.to_string(), to_node.to_string())) {
                errors.push(format!(
                    "{} contains hop '{} -> {}' absent from observed topology",
                    label, from_node, to_node
                ));
            }
        }
    }

    let hop_count = nodes.len().saturating_sub(1);
    if hop_count > max_hops {
        errors.push(format!(
            "{} has {} hops, exceeding max_hops={}",
            label, hop_count, max_hops
        ));
    }
    if let Some(declared_hops) = projection.get("hop_count").and_then(Value::as_u64) {
        if declared_hops as usize != hop_count {
            errors.push(format!(
                "{}.hop_count={} does not match nodes length={}",
                label, declared_hops, hop_count
            ));
        }
    }

    match projection.get("confidence").and_then(Value::as_f64) {
        Some(confidence) if confidence.is_finite() && (0.0..=1.0).contains(&confidence) => {}
        _ => errors.push(format!(
            "{}.confidence must be a finite number between 0 and 1",
            label
        )),
    }
    if projection
        .get("reason")
        .and_then(Value::as_str)
        .is_none_or(|value| value.trim().is_empty())
    {
        errors.push(format!("{}.reason must be a non-empty string", label));
    }
}

fn request_observed_edges(request: &Value) -> BTreeSet<(String, String)> {
    let graph = request
        .get("input_context")
        .and_then(|context| context.get("observed_dgraph"));

    let mut edges = BTreeSet::new();
    for field in [
        "configured_relations",
        "observed_relations",
        "measured_peer_edges",
        "measured_connectivity_edges",
    ] {
        if let Some(values) = graph
            .and_then(|graph| graph.get(field))
            .and_then(Value::as_array)
        {
            for edge in values {
                let from = edge.get("from").and_then(Value::as_str);
                let to = edge.get("to").and_then(Value::as_str);
                if let (Some(from), Some(to)) = (from, to) {
                    edges.insert((from.to_string(), to.to_string()));
                }
            }
        }
    }
    edges
}

fn request_node_ids(request: &Value) -> BTreeSet<String> {
    request
        .get("input_context")
        .and_then(|context| context.get("observed_dgraph"))
        .and_then(|graph| graph.get("nodes"))
        .and_then(Value::as_array)
        .map(|nodes| {
            nodes
                .iter()
                .filter_map(|node| node.get("node_id").and_then(Value::as_str))
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn required_text(value: &Value, field: &str) -> Result<String, String> {
    text(value, field).ok_or_else(|| format!("context field '{}' must be a non-empty string", field))
}

fn text(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn required_u64(value: &Value, field: &str) -> Result<u64, String> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("context field '{}' must be a positive integer", field))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> Value {
        json!({
            "schema_version": "0.1.0",
            "contract_kind": "datum_alternative_path_what_if_context",
            "project_id": "monitor-f49c8",
            "dgraph_id": "dgraph:test",
            "dgraph_revision": 5,
            "requested_scope": {
                "from": "fog-01",
                "to": "cloud-01",
                "max_hops": 4
            },
            "observed_dgraph": {
                "nodes": [
                    {"node_id": "fog-01"},
                    {"node_id": "cloud-01"}
                ],
                "measured_peer_edges": [
                    {"from": "fog-01", "to": "cloud-01"}
                ]
            }
        })
    }

    fn response(request: &Value) -> Value {
        json!({
            "schema_version": "0.1.0",
            "contract_kind": "datum_ai_what_if_inference_response",
            "request_id": request["request_id"],
            "project_id": "monitor-f49c8",
            "dgraph_id": "dgraph:test",
            "dgraph_revision": 5,
            "model": {
                "provider": "external-test",
                "model_id": "gnn-test",
                "training_performed": false
            },
            "projections": [{
                "path_id": "fog-01=>cloud-01:0",
                "from": "fog-01",
                "to": "cloud-01",
                "nodes": ["fog-01", "cloud-01"],
                "hop_count": 1,
                "confidence": 0.91,
                "reason": "historical model projection"
            }],
            "execution": {
                "mode": "proposal_only",
                "changes_applied": false,
                "execution_allowed": false,
                "side_effects_performed": false
            },
            "side_effects_performed": false
        })
    }

    #[test]
    fn request_preserves_context_and_declares_external_inference() {
        let request = build_external_ai_inference_request(&context()).unwrap();
        assert_eq!(request["contract_kind"], EXTERNAL_AI_INFERENCE_REQUEST_KIND);
        assert_eq!(request["input_context"]["contract_kind"], "datum_alternative_path_what_if_context");
        assert_eq!(request["inference"]["provider_mode"], "external_ai_required");
        assert_eq!(request["execution_policy"]["allow_actions"], false);
    }

    #[test]
    fn response_is_accepted_only_for_same_revision_and_topology() {
        let request = build_external_ai_inference_request(&context()).unwrap();
        let validated = validate_external_ai_inference_response(&request, &response(&request)).unwrap();
        assert_eq!(validated["contract_kind"], VALIDATED_AI_PROJECTION_KIND);
        assert_eq!(validated["validation"]["accepted"], true);
        assert_eq!(validated["projections"][0]["confidence"], 0.91);
    }

    #[test]
    fn response_with_unknown_node_is_rejected() {
        let request = build_external_ai_inference_request(&context()).unwrap();
        let mut invalid = response(&request);
        invalid["projections"][0]["nodes"][1] = json!("unknown-01");
        assert!(validate_external_ai_inference_response(&request, &invalid)
            .unwrap_err()
            .contains("unknown node"));
    }

    #[test]
    fn response_with_execution_enabled_is_rejected() {
        let request = build_external_ai_inference_request(&context()).unwrap();
        let mut invalid = response(&request);
        invalid["execution"]["execution_allowed"] = json!(true);
        assert!(validate_external_ai_inference_response(&request, &invalid)
            .unwrap_err()
            .contains("execution.execution_allowed must be false"));
    }

    #[test]
    fn response_with_top_level_side_effects_is_rejected() {
        let request = build_external_ai_inference_request(&context()).unwrap();
        let mut invalid = response(&request);
        invalid["side_effects_performed"] = json!(true);
        assert!(validate_external_ai_inference_response(&request, &invalid)
            .unwrap_err()
            .contains("response.side_effects_performed must be false"));
    }

    #[test]
    fn response_with_hop_absent_from_observed_topology_is_rejected() {
        let request = build_external_ai_inference_request(&context()).unwrap();
        let mut invalid = response(&request);
        invalid["projections"][0]["nodes"] = json!(["fog-01", "fog-01", "cloud-01"]);
        invalid["projections"][0]["hop_count"] = json!(2);
        assert!(validate_external_ai_inference_response(&request, &invalid)
            .unwrap_err()
            .contains("absent from observed topology"));
    }

    #[test]
    fn issued_request_can_be_retrieved_by_exact_request_id() {
        let request = build_external_ai_inference_request(&context()).unwrap();
        register_external_ai_inference_request(&request).unwrap();
        let retrieved = get_issued_external_ai_inference_request(
            request["request_id"].as_str().unwrap(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(retrieved, request);
    }
}
