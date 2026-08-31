use serde_json::{json, Map, Value};

use crate::core::{
    controlled_dgraph_deployment::ControlledObservedDGraphState,
    deployment_lifecycle::DeploymentAttemptRecord,
    deployment_manifest::DeploymentManifest,
};

pub fn materialize_lifecycle_evidence(
    observed: &mut ControlledObservedDGraphState,
    manifest: &DeploymentManifest,
    attempts: &[DeploymentAttemptRecord],
) -> Result<(), String> {
    let serialized_attempts = serde_json::to_value(attempts)
        .map_err(|error| format!("serialize deployment lifecycle evidence: {error}"))?;

    observed.metadata = Some(merge_metadata(
        observed.metadata.take(),
        json!({
            "deployment_lifecycle_attempt_count": attempts.len(),
            "deployment_attempts": serialized_attempts.clone(),
            "automatic_retry_performed": false,
            "governed_execution_performed": false,
        }),
    ));
    observed.graph.metadata = Some(merge_metadata(
        observed.graph.metadata.take(),
        json!({
            "deployment_lifecycle_attempt_count": attempts.len(),
            "deployment_attempts": serialized_attempts,
        }),
    ));

    for module in &mut observed.graph.modules {
        let assignment_ids = manifest
            .assignments
            .iter()
            .filter(|assignment| assignment.module_id == module.module_id)
            .map(|assignment| assignment.assignment_id.as_str())
            .collect::<Vec<_>>();
        let module_attempts = attempts
            .iter()
            .filter(|attempt| assignment_ids.contains(&attempt.assignment_id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if module_attempts.is_empty() {
            continue;
        }
        let current = module_attempts
            .iter()
            .max_by(|a, b| {
                a.updated_at_utc
                    .cmp(&b.updated_at_utc)
                    .then(a.attempt_number.cmp(&b.attempt_number))
            })
            .expect("non-empty module attempt list has current attempt");
        module.metadata = Some(merge_metadata(
            module.metadata.take(),
            json!({
                "deployment_lifecycle_current_attempt_id": current.attempt_id,
                "deployment_lifecycle_current_status": current.status,
                "deployment_attempts": module_attempts,
            }),
        ));
    }

    Ok(())
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
