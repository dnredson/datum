use axum::{
    http::{HeaderMap, StatusCode},
    Json,
};

use crate::core::dgraph_rollback_audit::{
    build_fail_closed_rollback_audit_contract, ProjectDGraphRollbackAuditResponse,
};

fn extract_project_id(headers: &HeaderMap) -> Result<String, (StatusCode, String)> {
    headers
        .get("x-datum-project")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|project_id| !project_id.is_empty())
        .map(str::to_string)
        .ok_or((
            StatusCode::BAD_REQUEST,
            "missing or invalid X-Datum-Project header".to_string(),
        ))
}

pub async fn get_dgraph_rollback_audit_contract(
    headers: HeaderMap,
) -> Result<Json<ProjectDGraphRollbackAuditResponse>, (StatusCode, String)> {
    let project_id = extract_project_id(&headers)?;
    Ok(Json(build_fail_closed_rollback_audit_contract(
        &project_id,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_header_extraction_requires_x_datum_project() {
        let error = extract_project_id(&HeaderMap::new()).expect_err("header should be required");
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn source_guard_is_read_only_and_does_not_rollback_write_audit_or_execute() {
        let source = include_str!("dgraph_rollback_audit.rs");
        let implementation = source.split("#[cfg(test)]").next().unwrap_or(source);

        for forbidden in [
            "State<",
            "store_",
            "post(",
            "patch(",
            "fs::read",
            "fs::write",
            "File::open",
            "OpenOptions",
            "reqwest",
            "wasmtime",
            "std::process::Command",
            "Command::new",
            "run_embedded_wasm",
            "dispatch --execute",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "rollback and audit endpoint must not contain forbidden pattern: {forbidden}"
            );
        }
    }
}
