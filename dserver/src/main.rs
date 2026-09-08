mod api;
mod config;
mod core;
mod storage;

use std::{env, path::PathBuf, sync::Arc};

use anyhow::Context;

use axum::{
    routing::{delete, get, patch, post, put},
    Router,
};

use api::alternative_paths::get_alternative_paths;
use api::artifact_reconciliation::get_operational_reconciliation_context;
use api::governed_deployment_preflight::get_governed_deployment_preflight;
use api::external_ai_inference::{
    get_external_ai_inference_request, post_external_ai_inference_response,
};
use api::action_results::{
    get_latest_action_execution_report, list_latest_action_execution_reports,
    post_action_execution_report,
};
use api::capabilities::{get_node_capabilities, list_capabilities, put_node_capabilities};
use api::controlled_wasm_deployment::{
    get_controlled_wasm_deployment_for_node, get_controlled_wasm_deployment_manifest,
    get_controlled_wasm_dgraph, get_controlled_wasm_observed_dgraph,
    list_controlled_wasm_live_reports, post_controlled_wasm_deployment_manifest,
    post_controlled_wasm_dgraph, post_controlled_wasm_live_report,
};
use api::dashboard::get_dashboard;
use api::live_dashboard::{get_graph_overview, get_live_dashboard_html, get_live_dashboard_state};
use api::secret_preflight::get_secret_preflight;
use api::service_artifacts::{
    delete_service_artifact, get_service_artifact, list_service_artifacts,
    post_service_artifact, put_service_artifact,
};
use api::deployment_lifecycle::{
    get_deployment_attempt, list_deployment_attempts, post_deployment_attempt,
    post_deployment_attempt_retry, post_deployment_attempt_transition,
};
use api::deployment_planning::{get_deployment_planning_for_node, get_latest_deployment_planning};
use api::dgraph_artifact_integrity::get_dgraph_artifact_integrity_contract;
use api::dgraph_capability_negotiation::get_dgraph_capability_negotiation_contract;
use api::dgraph_execution_allowlist::get_dgraph_execution_allowlist_contract;
use api::dgraph_execution_policy::get_dgraph_execution_policy_contract;
use api::dgraph_execution_prerequisites::get_dgraph_execution_prerequisites_contract;
use api::dgraph_execution_safety::get_dgraph_execution_safety_contract;
use api::dgraph_materialized::{get_materialized_dgraph, get_materialized_dgraph_summary};
use api::dgraph_operator_approval::get_dgraph_operator_approval_contract;
use api::dgraph_planner::{get_dgraph_planner_dry_run, get_dgraph_planner_dry_run_summary};
use api::dgraph_readiness::get_dgraph_readiness_summary;
use api::dgraph_resource_limits::get_dgraph_resource_limits_contract;
use api::dgraph_rollback_audit::get_dgraph_rollback_audit_contract;
use api::dispatch::{get_dispatch_for_node, get_materialized_dispatch, publish_latest_dispatch};
use api::dmap::{get_dmap, set_dmap};
use api::graph::{get_latest_graph, get_materialized_graph};
use api::graphql::{build_schema, graphiql, graphql_post, DatumSchema};
use api::installation_plan::{get_installation_plan_for_node, get_latest_installation_plan};
use api::installation_tasks::{
    get_installation_task, get_installation_task_assisted_execution_state,
    get_installation_task_readiness, get_latest_assisted_execution_attempt,
    get_latest_assisted_execution_claim, get_latest_assisted_execution_request,
    get_latest_installation_task_approval, get_latest_local_mapping_preflight_report,
    list_assisted_execution_attempts_for_node, list_assisted_execution_claims_for_node,
    list_assisted_execution_requests_for_node, list_assisted_execution_state_for_node,
    list_installation_task_approvals_for_node, list_installation_task_readiness_for_node,
    list_installation_tasks, list_installation_tasks_for_node,
    list_latest_local_mapping_preflight_reports, list_local_mapping_patch_proposals,
    list_local_mapping_patch_proposals_for_node, list_local_mapping_recommendations,
    list_local_mapping_recommendations_for_node, materialize_installation_tasks,
    materialize_installation_tasks_for_node, patch_assisted_execution_claim_status,
    patch_installation_task_status, post_assisted_execution_attempt, post_assisted_execution_claim,
    post_assisted_execution_request, post_installation_task_approval,
    post_local_mapping_preflight_report,
};
use api::local_dgraph_snapshot::{
    get_latest_local_dgraph_snapshot, list_latest_local_dgraph_snapshots,
    post_local_dgraph_snapshot,
};
use api::operational_analysis_context::{
    acquire_active_analysis_context, acquire_passive_analysis_context,
};
use api::operational_dgraph::{
    get_operational_dgraph, get_operational_dgraph_slice,
    post_operational_reconciliation_authorization,
    post_operational_reconciliation_rollback_authorization, put_operational_dgraph,
};
use api::operational_evidence::{list_operational_evidence, post_operational_evidence};
use api::planner::{build_latest_action_plan, get_materialized_action_plan};
use api::resource_registry::{
    get_resource, get_resource_binding, list_resource_bindings, list_resources,
    post_resource_binding, post_resource_discovery,
};
use api::sentinel_registration::{
    get_sentinel_registration, list_sentinel_registrations, post_sentinel_heartbeat,
    put_sentinel_registration,
};
use api::sentinel_telemetry::{
    get_observed_dgraph, get_sentinel_telemetry, get_sentinel_telemetry_history,
    list_sentinel_telemetry, post_sentinel_telemetry,
};
use api::topology::{get_topology, set_topology};
use api::wasm_module_registry::{
    get_wasm_module_descriptor, list_wasm_module_descriptors, post_wasm_module_descriptor,
};
use api::wasm_runtime_reports::{
    get_latest_wasm_runtime_report, list_latest_wasm_runtime_reports,
    post_wasm_runtime_report,
};
use api::{
    analysis::{analyze_latest_graph, get_materialized_analysis},
    events::{build_latest_global_events, get_materialized_global_events},
    health::health,
    ingest::ingest_snapshot,
    middleware::cors::build_cors_layer,
    nodes::{get_latest, list_nodes},
};
use config::AppConfig;
use core::service_artifact_registry::ServiceArtifactRegistry;
use storage::firebase::FirebaseClient;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub firebase: Arc<FirebaseClient>,
    pub graphql: DatumSchema,
    pub service_artifacts: Arc<ServiceArtifactRegistry>,
}

const DSERVER_CONFIG_PATH_ENV: &str = "DATUM_DSERVER_CONFIG_PATH";

fn load_server_config() -> anyhow::Result<AppConfig> {
    if let Ok(configured_path) = env::var(DSERVER_CONFIG_PATH_ENV) {
        let path = PathBuf::from(configured_path);
        return AppConfig::from_file(&path)
            .with_context(|| format!("load dserver config '{}'", path.display()));
    }

    let mut candidates = vec![
        PathBuf::from("config.toml"),
        PathBuf::from("dserver/config.toml"),
    ];
    if let Ok(executable) = env::current_exe() {
        if let Some(crate_dir) = executable
            .parent()
            .and_then(|dir| dir.parent())
            .and_then(|dir| dir.parent())
        {
            candidates.push(crate_dir.join("config.toml"));
        }
    }

    for path in &candidates {
        if path.is_file() {
            return AppConfig::from_file(path)
                .with_context(|| format!("load dserver config '{}'", path.display()));
        }
    }

    let searched = candidates
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(anyhow::anyhow!(
        "dserver config not found; searched: {searched}. Set {DSERVER_CONFIG_PATH_ENV} to an explicit config.toml path"
    ))
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().init();

    // Restore the accepted control-plane state before opening any listener or
    // initializing external clients. A failed recovery is a startup failure,
    // never an invitation to publish a replacement graph.
    api::operational_dgraph::initialize_operational_dgraph_persistence_from_env()
        .expect("failed to recover operational D-Graph control-plane state");

    let config = Arc::new(load_server_config().expect("failed to load config"));

    let firebase = Arc::new(
        FirebaseClient::new(config.clone())
            .await
            .expect("failed to initialize firebase"),
    );
    let graphql = build_schema(firebase.clone());
    let service_artifacts = Arc::new(
        ServiceArtifactRegistry::load().expect("failed to load service artifact registry"),
    );

    let state = AppState {
        config: config.clone(),
        firebase,
        graphql,
        service_artifacts,
    };

    let bind_addr = state
        .config
        .server
        .bind
        .clone()
        .unwrap_or_else(|| "0.0.0.0:8080".to_string());

    let mut app = Router::new()
        .route("/graphql", post(graphql_post))
        .route("/graphiql", get(graphiql))
        .route("/health", get(health))
        .route("/api/v1/dashboard", get(get_dashboard))
        .route("/api/v1/dashboard/live", get(get_live_dashboard_state))
        .route("/api/v1/graph-overview", get(get_graph_overview))
        .route("/api/v1/alternative-paths", get(get_alternative_paths))
        .route(
            "/api/v1/alternative-paths/inference-request",
            get(get_external_ai_inference_request),
        )
        .route(
            "/api/v1/alternative-paths/inference-response",
            post(post_external_ai_inference_response),
        )
        .route("/dashboard/live", get(get_live_dashboard_html))
        .route(
            "/api/v1/nodes/:node_id/operational-reconciliation-context",
            get(get_operational_reconciliation_context),
        )
        .route(
            "/api/v1/nodes/:node_id/governed-deployment-preflight",
            get(get_governed_deployment_preflight),
        )
        .route(
            "/api/v1/nodes/:node_id/deployment-secrets-preflight",
            get(get_secret_preflight),
        )
        .route(
            "/api/v1/service-artifacts",
            post(post_service_artifact).get(list_service_artifacts),
        )
        .route(
            "/api/v1/service-artifacts/:artifact_id",
            get(get_service_artifact)
                .put(put_service_artifact)
                .delete(delete_service_artifact),
        )
        .route("/api/v1/resources", get(list_resources))
        .route("/api/v1/resource-bindings", get(list_resource_bindings))
        .route("/api/v1/resources/discovery", post(post_resource_discovery))
        .route("/api/v1/resources/:resource_id", get(get_resource))
        .route(
            "/api/v1/resources/:resource_id/binding",
            post(post_resource_binding).get(get_resource_binding),
        )
        .route(
            "/api/v1/operational-dgraph",
            put(put_operational_dgraph).get(get_operational_dgraph),
        )
        .route(
            "/api/v1/nodes/:node_id/operational-dgraph-slice",
            get(get_operational_dgraph_slice),
        )
        .route(
            "/api/v1/nodes/:node_id/operational-reconciliation/authorize",
            post(post_operational_reconciliation_authorization),
        )
        .route(
            "/api/v1/nodes/:node_id/operational-reconciliation/rollback-authorize",
            post(post_operational_reconciliation_rollback_authorization),
        )
        .route(
            "/api/v1/operational-evidence",
            post(post_operational_evidence).get(list_operational_evidence),
        )
        .route(
            "/api/v1/analysis-context/passive/:node_id",
            post(acquire_passive_analysis_context),
        )
        .route(
            "/api/v1/analysis-context/active",
            post(acquire_active_analysis_context),
        )
        .route(
            "/api/v1/observed-dgraph",
            get(get_observed_dgraph),
        )
        .route(
            "/api/v1/sentinels/telemetry",
            get(list_sentinel_telemetry),
        )
        .route(
            "/api/v1/sentinels/:node_id/telemetry",
            post(post_sentinel_telemetry).get(get_sentinel_telemetry),
        )
        .route(
            "/api/v1/sentinels/:node_id/telemetry/history",
            get(get_sentinel_telemetry_history),
        )
        .route("/api/v1/sentinels", get(list_sentinel_registrations))
        .route(
            "/api/v1/sentinels/:node_id",
            get(get_sentinel_registration),
        )
        .route(
            "/api/v1/sentinels/:node_id/registration",
            put(put_sentinel_registration),
        )
        .route(
            "/api/v1/sentinels/:node_id/heartbeat",
            post(post_sentinel_heartbeat),
        )
        .route(
            "/api/v1/controlled-wasm/dgraph",
            post(post_controlled_wasm_dgraph).get(get_controlled_wasm_dgraph),
        )
        .route(
            "/api/v1/controlled-wasm/observed-dgraph",
            get(get_controlled_wasm_observed_dgraph),
        )
        .route(
            "/api/v1/controlled-wasm/deployment-manifest",
            post(post_controlled_wasm_deployment_manifest)
                .get(get_controlled_wasm_deployment_manifest),
        )
        .route(
            "/api/v1/controlled-wasm/deployment/for-node/:node_id",
            get(get_controlled_wasm_deployment_for_node),
        )
        .route(
            "/api/v1/controlled-wasm/live-reports",
            post(post_controlled_wasm_live_report).get(list_controlled_wasm_live_reports),
        )
        .route(
            "/api/v1/controlled-wasm/deployment-attempts",
            post(post_deployment_attempt).get(list_deployment_attempts),
        )
        .route(
            "/api/v1/controlled-wasm/deployment-attempts/:attempt_id",
            get(get_deployment_attempt),
        )
        .route(
            "/api/v1/controlled-wasm/deployment-attempts/:attempt_id/transition",
            post(post_deployment_attempt_transition),
        )
        .route(
            "/api/v1/controlled-wasm/deployment-attempts/:attempt_id/retry",
            post(post_deployment_attempt_retry),
        )
        .route("/api/v1/snapshots", post(ingest_snapshot))
        .route("/api/v1/nodes", get(list_nodes))
        .route("/api/v1/nodes/:node_id/latest", get(get_latest))
        .route(
            "/api/v1/nodes/:node_id/capabilities",
            post(put_node_capabilities).get(get_node_capabilities),
        )
        .route(
            "/api/v1/nodes/:node_id/action-results",
            post(post_action_execution_report),
        )
        .route(
            "/api/v1/nodes/:node_id/action-results/latest",
            get(get_latest_action_execution_report),
        )
        .route("/api/v1/capabilities", get(list_capabilities))
        .route(
            "/api/v1/action-results/latest",
            get(list_latest_action_execution_reports),
        )
        .route(
            "/api/v1/nodes/:node_id/wasm-runtime-reports",
            post(post_wasm_runtime_report),
        )
        .route(
            "/api/v1/nodes/:node_id/wasm-runtime-reports/latest",
            get(get_latest_wasm_runtime_report),
        )
        .route(
            "/api/v1/wasm-runtime-reports/latest",
            get(list_latest_wasm_runtime_reports),
        )
        .route(
            "/api/v1/wasm-modules",
            post(post_wasm_module_descriptor).get(list_wasm_module_descriptors),
        )
        .route(
            "/api/v1/wasm-modules/:module_id",
            get(get_wasm_module_descriptor),
        )
        .route("/api/v1/graph/latest", get(get_latest_graph))
        .route("/api/v1/graph/materialized", get(get_materialized_graph))
        .route("/api/v1/analysis/latest", get(analyze_latest_graph))
        .route("/api/v1/topology", post(set_topology))
        .route("/api/v1/topology", get(get_topology))
        .route("/api/v1/dmap", post(set_dmap))
        .route("/api/v1/dmap", get(get_dmap))
        .route("/api/v1/action-plan/latest", get(build_latest_action_plan))
        .route(
            "/api/v1/action-plan/materialized",
            get(get_materialized_action_plan),
        )
        .route(
            "/api/v1/deployment-planning/latest",
            get(get_latest_deployment_planning),
        )
        .route(
            "/api/v1/deployment-planning/for-node/:node_id",
            get(get_deployment_planning_for_node),
        )
        .route(
            "/api/v1/installation-plan/latest",
            get(get_latest_installation_plan),
        )
        .route(
            "/api/v1/installation-plan/for-node/:node_id",
            get(get_installation_plan_for_node),
        )
        .route(
            "/api/v1/installation-tasks/materialize",
            post(materialize_installation_tasks),
        )
        .route(
            "/api/v1/installation-tasks/materialize/for-node/:node_id",
            post(materialize_installation_tasks_for_node),
        )
        .route("/api/v1/installation-tasks", get(list_installation_tasks))
        .route(
            "/api/v1/local-mapping-recommendations",
            get(list_local_mapping_recommendations),
        )
        .route(
            "/api/v1/local-mapping-recommendations/for-node/:node_id",
            get(list_local_mapping_recommendations_for_node),
        )
        .route(
            "/api/v1/local-mapping-patch-proposals",
            get(list_local_mapping_patch_proposals),
        )
        .route(
            "/api/v1/local-mapping-patch-proposals/for-node/:node_id",
            get(list_local_mapping_patch_proposals_for_node),
        )
        .route(
            "/api/v1/local-mapping-preflight/latest",
            get(list_latest_local_mapping_preflight_reports),
        )
        .route(
            "/api/v1/nodes/:node_id/local-mapping-preflight",
            post(post_local_mapping_preflight_report),
        )
        .route(
            "/api/v1/nodes/:node_id/local-mapping-preflight/latest",
            get(get_latest_local_mapping_preflight_report),
        )
        .route(            "/api/v1/nodes/:node_id/dgraph-snapshot",
            post(post_local_dgraph_snapshot),
        )
        .route(
            "/api/v1/nodes/:node_id/dgraph-snapshot/latest",
            get(get_latest_local_dgraph_snapshot),
        )
        .route(
            "/api/v1/dgraph-snapshots/latest",
            get(list_latest_local_dgraph_snapshots),
        )
        .route("/api/v1/dgraph/materialized", get(get_materialized_dgraph))
        .route(
            "/api/v1/dgraph/materialized/summary",
            get(get_materialized_dgraph_summary),
        )
        .route(
            "/api/v1/dgraph/planner/dry-run",
            get(get_dgraph_planner_dry_run),
        )
        .route(
            "/api/v1/dgraph/planner/dry-run/summary",
            get(get_dgraph_planner_dry_run_summary),
        )
        .route(
            "/api/v1/dgraph/readiness/summary",
            get(get_dgraph_readiness_summary),
        )
        .route(
            "/api/v1/dgraph/artifact-integrity",
            get(get_dgraph_artifact_integrity_contract),
        )
        .route(
            "/api/v1/dgraph/capability-negotiation",
            get(get_dgraph_capability_negotiation_contract),
        )
        .route(
            "/api/v1/dgraph/execution-allowlists",
            get(get_dgraph_execution_allowlist_contract),
        )
        .route(
            "/api/v1/dgraph/resource-limits",
            get(get_dgraph_resource_limits_contract),
        )
        .route(
            "/api/v1/dgraph/rollback-audit",
            get(get_dgraph_rollback_audit_contract),
        )
        .route(
            "/api/v1/dgraph/execution-prerequisites",
            get(get_dgraph_execution_prerequisites_contract),
        )
        .route(
            "/api/v1/dgraph/execution-safety",
            get(get_dgraph_execution_safety_contract),
        )
        .route(
            "/api/v1/dgraph/execution-policy",
            get(get_dgraph_execution_policy_contract),
        )
        .route(
            "/api/v1/dgraph/operator-approval",
            get(get_dgraph_operator_approval_contract),
        )
        .route(
            "/api/v1/installation-tasks/for-node/:node_id",
            get(list_installation_tasks_for_node),
        )
        .route(
            "/api/v1/installation-tasks/for-node/:node_id/:task_id",
            get(get_installation_task),
        )
        .route(
            "/api/v1/installation-tasks/for-node/:node_id/:task_id/status",
            patch(patch_installation_task_status),
        )
        .route(
            "/api/v1/installation-tasks/for-node/:node_id/:task_id/approval",
            post(post_installation_task_approval),
        )
        .route(
            "/api/v1/installation-tasks/for-node/:node_id/:task_id/approval/latest",
            get(get_latest_installation_task_approval),
        )
        .route(
            "/api/v1/installation-tasks/for-node/:node_id/:task_id/readiness",
            get(get_installation_task_readiness),
        )
        .route(
            "/api/v1/installation-tasks/for-node/:node_id/:task_id/assisted-execution-state",
            get(get_installation_task_assisted_execution_state),
        )
        .route(
            "/api/v1/installation-tasks/for-node/:node_id/:task_id/assisted-execution-request",
            post(post_assisted_execution_request),
        )
        .route(
            "/api/v1/installation-tasks/for-node/:node_id/:task_id/assisted-execution-request/latest",
            get(get_latest_assisted_execution_request),
        )
        .route(
            "/api/v1/installation-tasks/for-node/:node_id/:task_id/assisted-execution-request/:request_id/claim",
            post(post_assisted_execution_claim),
        )
        .route(
            "/api/v1/installation-tasks/for-node/:node_id/:task_id/assisted-execution-request/:request_id/claim/latest",
            get(get_latest_assisted_execution_claim),
        )
        .route(
            "/api/v1/installation-tasks/for-node/:node_id/:task_id/assisted-execution-request/:request_id/claim/status",
            patch(patch_assisted_execution_claim_status),
        )
        .route(
            "/api/v1/installation-tasks/for-node/:node_id/:task_id/assisted-execution-request/:request_id/claim/:claim_id/attempt",
            post(post_assisted_execution_attempt),
        )
        .route(
            "/api/v1/installation-tasks/for-node/:node_id/:task_id/assisted-execution-request/:request_id/claim/:claim_id/attempt/latest",
            get(get_latest_assisted_execution_attempt),
        )
        .route(
            "/api/v1/installation-task-approvals/for-node/:node_id",
            get(list_installation_task_approvals_for_node),
        )
        .route(
            "/api/v1/installation-task-readiness/for-node/:node_id",
            get(list_installation_task_readiness_for_node),
        )
        .route(
            "/api/v1/assisted-execution-requests/for-node/:node_id",
            get(list_assisted_execution_requests_for_node),
        )
        .route(
            "/api/v1/assisted-execution-claims/for-node/:node_id",
            get(list_assisted_execution_claims_for_node),
        )
        .route(
            "/api/v1/assisted-execution-attempts/for-node/:node_id",
            get(list_assisted_execution_attempts_for_node),
        )
        .route(
            "/api/v1/assisted-execution-state/for-node/:node_id",
            get(list_assisted_execution_state_for_node),
        )
        .route("/api/v1/dispatch/latest", get(publish_latest_dispatch))
        .route(
            "/api/v1/dispatch/for-node/:node_id",
            get(get_dispatch_for_node),
        )
        .route(
            "/api/v1/dispatch/materialized",
            get(get_materialized_dispatch),
        )
        .route(
            "/api/v1/analysis/materialized",
            get(get_materialized_analysis),
        )
        .route("/api/v1/events/latest", get(build_latest_global_events))
        .route(
            "/api/v1/events/materialized",
            get(get_materialized_global_events),
        )
        .with_state(state);

    if let Some(cors_layer) = build_cors_layer(config.as_ref()) {
        app = app.layer(cors_layer);
    }

    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .expect("failed to bind address");

    println!("🚀 Datum Server running on {}", bind_addr);

    axum::serve(listener, app).await.expect("server error");
}