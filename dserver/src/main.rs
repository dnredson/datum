mod api;
mod config;
mod core;
mod storage;

use std::sync::Arc;

use axum::{
    routing::{get, post},
    Router,
};

use api::action_results::{
    get_latest_action_execution_report, list_latest_action_execution_reports,
    post_action_execution_report,
};
use api::capabilities::{get_node_capabilities, list_capabilities, put_node_capabilities};
use api::dashboard::get_dashboard;
use api::deployment_planning::{get_deployment_planning_for_node, get_latest_deployment_planning};
use api::dispatch::{get_dispatch_for_node, get_materialized_dispatch, publish_latest_dispatch};
use api::dmap::{get_dmap, set_dmap};
use api::graph::{get_latest_graph, get_materialized_graph};
use api::planner::{build_latest_action_plan, get_materialized_action_plan};
use api::topology::{get_topology, set_topology};
use api::{
    analysis::{analyze_latest_graph, get_materialized_analysis},
    events::{build_latest_global_events, get_materialized_global_events},
    health::health,
    ingest::ingest_snapshot,
    middleware::cors::build_cors_layer,
    nodes::{get_latest, list_nodes},
};
use config::AppConfig;
use storage::firebase::FirebaseClient;
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub firebase: Arc<FirebaseClient>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().init();

    let config = Arc::new(AppConfig::from_file("config.toml").expect("failed to load config"));

    let firebase = FirebaseClient::new(config.clone())
        .await
        .expect("failed to initialize firebase");

    let state = AppState {
        config: config.clone(),
        firebase: Arc::new(firebase),
    };

    let bind_addr = state
        .config
        .server
        .bind
        .clone()
        .unwrap_or_else(|| "0.0.0.0:8080".to_string());

    let mut app = Router::new()
        .route("/health", get(health))
        .route("/api/v1/dashboard", get(get_dashboard))
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
