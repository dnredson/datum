use std::sync::Arc;

use async_graphql::{
    http::GraphiQLSource, Context, EmptyMutation, EmptySubscription, Enum, Json, Object, Schema,
    SimpleObject,
};
use axum::{
    extract::State,
    response::Html,
    Json as AxumJson,
};
use serde_json::Value;

use crate::{
    api::{
        operational_dgraph::operational_dgraph_slice_snapshot,
        resource_registry::{active_resource_binding_for_node, resource_registry_snapshot},
    },
    core::{
        operational_dgraph::{
            OperationalDGraphSlice, OperationalDesiredService,
            OperationalReconciliationAuthorization,
        },
        resource_binding::ResourceRoleBindingRecord,
        resource_registry::ResourceRegistryRecord,
    },
    storage::{
        control_state::{
            configured_control_state_db_path, domain_history_count, list_current_domains,
            load_domain_history, ControlStateHistoryEntry, DOMAIN_OPERATIONAL_DGRAPH,
            DOMAIN_RESOURCE_REGISTRY,
        },
        firebase::FirebaseClient,
    },
    AppState,
};

pub type DatumSchema = Schema<QueryRoot, EmptyMutation, EmptySubscription>;

pub fn build_schema(firebase: Arc<FirebaseClient>) -> DatumSchema {
    Schema::build(QueryRoot, EmptyMutation, EmptySubscription)
        .data(firebase)
        .finish()
}

pub async fn graphql_post(
    State(state): State<AppState>,
    AxumJson(request): AxumJson<async_graphql::Request>,
) -> AxumJson<async_graphql::Response> {
    AxumJson(state.graphql.execute(request).await)
}

pub async fn graphiql() -> Html<String> {
    Html(GraphiQLSource::build().endpoint("/graphql").finish())
}

pub struct QueryRoot;

#[Object]
impl QueryRoot {
    async fn node(
        &self,
        ctx: &Context<'_>,
        project_id: String,
        node_id: String,
    ) -> async_graphql::Result<NodeContextView> {
        let project_id = project_id.trim().to_string();
        let node_id = node_id.trim().to_string();
        if project_id.is_empty() {
            return Err("projectId must not be empty".into());
        }
        if node_id.is_empty() {
            return Err("nodeId must not be empty".into());
        }

        let binding = active_resource_binding_for_node(&project_id, &node_id)
            .map_err(async_graphql::Error::new)?;
        let resources = resource_registry_snapshot(&project_id)
            .map_err(async_graphql::Error::new)?;
        let resource = binding
            .as_ref()
            .and_then(|binding| {
                resources
                    .iter()
                    .find(|resource| resource.resource_id == binding.resource_id)
                    .cloned()
            })
            .or_else(|| {
                resources
                    .into_iter()
                    .find(|resource| resource.assigned_node_id.as_deref() == Some(node_id.as_str()))
            });

        let slice = operational_dgraph_slice_snapshot(&project_id, &node_id)
            .map_err(async_graphql::Error::new)?;

        let firebase = ctx.data::<Arc<FirebaseClient>>()?;
        let (latest_telemetry, telemetry_available, telemetry_error) =
            match firebase.get_latest_snapshot(&project_id, &node_id).await {
                Ok(value) => (Some(Json(value)), true, None),
                Err(error) => (None, false, Some(error.to_string())),
            };

        Ok(NodeContextView {
            project_id,
            node_id,
            resource: resource.map(ResourceView::from),
            binding: binding.map(BindingView::from),
            desired: slice.as_ref().map(DesiredView::from),
            authorization: slice
                .as_ref()
                .and_then(|slice| slice.authorization.as_ref())
                .map(AuthorizationView::from),
            latest_telemetry,
            telemetry_available,
            telemetry_source: "firebase".to_string(),
            telemetry_error,
        })
    }

    async fn control_state_domains(&self) -> async_graphql::Result<Vec<ControlStateDomainView>> {
        let db_path = configured_control_state_db_path().ok_or_else(|| {
            async_graphql::Error::new(
                "DATUM_CONTROL_STATE_DB_PATH is not configured; SQLite control-state history is unavailable",
            )
        })?;
        let domains = list_current_domains(&db_path).map_err(async_graphql::Error::new)?;

        domains
            .into_iter()
            .map(|(domain, updated_at_utc)| {
                let history_count =
                    domain_history_count(&db_path, &domain).map_err(async_graphql::Error::new)?;
                Ok(ControlStateDomainView {
                    domain,
                    updated_at_utc,
                    history_count: history_count.min(i64::MAX as u64) as i64,
                })
            })
            .collect()
    }

    async fn control_state_history(
        &self,
        domain: ControlStateDomain,
        limit: Option<i32>,
    ) -> async_graphql::Result<Vec<ControlStateHistoryView>> {
        let limit = limit.unwrap_or(20);
        if !(1..=100).contains(&limit) {
            return Err("limit must be between 1 and 100".into());
        }
        let db_path = configured_control_state_db_path().ok_or_else(|| {
            async_graphql::Error::new(
                "DATUM_CONTROL_STATE_DB_PATH is not configured; SQLite control-state history is unavailable",
            )
        })?;
        let entries = load_domain_history(&db_path, domain.as_storage_name(), limit as u32)
            .map_err(async_graphql::Error::new)?;
        Ok(entries
            .into_iter()
            .map(ControlStateHistoryView::from)
            .collect())
    }
}

#[derive(SimpleObject)]
pub struct NodeContextView {
    pub project_id: String,
    pub node_id: String,
    pub resource: Option<ResourceView>,
    pub binding: Option<BindingView>,
    pub desired: Option<DesiredView>,
    pub authorization: Option<AuthorizationView>,
    pub latest_telemetry: Option<Json<Value>>,
    pub telemetry_available: bool,
    pub telemetry_source: String,
    pub telemetry_error: Option<String>,
}

#[derive(SimpleObject)]
pub struct ResourceView {
    pub resource_id: String,
    pub state: String,
    pub assigned_node_id: Option<String>,
    pub identity_scope: String,
    pub planning_eligible: bool,
    pub observation_count: i64,
    pub last_seen_utc: String,
    pub hostname: String,
    pub sentinel_version: String,
    pub capacity_profile_id: Option<String>,
}

impl From<ResourceRegistryRecord> for ResourceView {
    fn from(record: ResourceRegistryRecord) -> Self {
        let identity_scope = serde_json::to_value(record.identity_scope)
            .ok()
            .and_then(|value| value.as_str().map(ToString::to_string))
            .unwrap_or_else(|| "unknown".to_string());
        let capacity_profile_id = record
            .discovery
            .resources
            .get("capacity_profile_id")
            .and_then(Value::as_str)
            .map(ToString::to_string);

        Self {
            resource_id: record.resource_id,
            state: record.state,
            assigned_node_id: record.assigned_node_id,
            identity_scope,
            planning_eligible: record.planning_eligible,
            observation_count: record.observation_count.min(i64::MAX as u64) as i64,
            last_seen_utc: record.last_seen_utc,
            hostname: record.discovery.identity.hostname,
            sentinel_version: record.discovery.identity.sentinel_version,
            capacity_profile_id,
        }
    }
}

#[derive(SimpleObject)]
pub struct BindingView {
    pub binding_id: String,
    pub resource_id: String,
    pub node_id: String,
    pub status: String,
    pub bound_at_utc: String,
    pub identity_scope: String,
    pub capacity_profile_id: String,
}

impl From<ResourceRoleBindingRecord> for BindingView {
    fn from(binding: ResourceRoleBindingRecord) -> Self {
        Self {
            binding_id: binding.binding_id,
            resource_id: binding.resource_id,
            node_id: binding.node_id,
            status: binding.status,
            bound_at_utc: binding.bound_at_utc,
            identity_scope: binding.identity_scope,
            capacity_profile_id: binding.capacity_profile_id,
        }
    }
}

#[derive(SimpleObject)]
pub struct DesiredView {
    pub dgraph_id: String,
    pub revision: i64,
    pub services: Vec<DesiredServiceView>,
}

impl From<&OperationalDGraphSlice> for DesiredView {
    fn from(slice: &OperationalDGraphSlice) -> Self {
        Self {
            dgraph_id: slice.dgraph_id.clone(),
            revision: slice.dgraph_revision.min(i64::MAX as u64) as i64,
            services: slice
                .desired_services
                .iter()
                .map(DesiredServiceView::from)
                .collect(),
        }
    }
}

#[derive(SimpleObject)]
pub struct DesiredServiceView {
    pub service_id: String,
    pub runtime: String,
    pub desired_state: String,
}

impl From<&OperationalDesiredService> for DesiredServiceView {
    fn from(service: &OperationalDesiredService) -> Self {
        Self {
            service_id: service.service_id.clone(),
            runtime: service.runtime.clone(),
            desired_state: service.desired_state.clone(),
        }
    }
}

#[derive(SimpleObject)]
pub struct AuthorizationView {
    pub authorization_id: String,
    pub dgraph_revision: i64,
    pub binding_id: Option<String>,
    pub authorized_by: String,
    pub authorized_at_utc: String,
    pub allow_host_mutation: bool,
    pub persistence_scope: String,
    pub expires_at_utc: Option<String>,
}

impl From<&OperationalReconciliationAuthorization> for AuthorizationView {
    fn from(authorization: &OperationalReconciliationAuthorization) -> Self {
        Self {
            authorization_id: authorization.authorization_id.clone(),
            dgraph_revision: authorization.dgraph_revision.min(i64::MAX as u64) as i64,
            binding_id: authorization.binding_id.clone(),
            authorized_by: authorization.authorized_by.clone(),
            authorized_at_utc: authorization.authorized_at_utc.clone(),
            allow_host_mutation: authorization.allow_host_mutation,
            persistence_scope: authorization.persistence_scope.clone(),
            expires_at_utc: authorization.expires_at_utc.clone(),
        }
    }
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum ControlStateDomain {
    #[graphql(name = "RESOURCE_REGISTRY")]
    ResourceRegistry,
    #[graphql(name = "OPERATIONAL_DGRAPH")]
    OperationalDgraph,
}

impl ControlStateDomain {
    fn as_storage_name(self) -> &'static str {
        match self {
            Self::ResourceRegistry => DOMAIN_RESOURCE_REGISTRY,
            Self::OperationalDgraph => DOMAIN_OPERATIONAL_DGRAPH,
        }
    }
}

#[derive(SimpleObject)]
pub struct ControlStateDomainView {
    pub domain: String,
    pub updated_at_utc: String,
    pub history_count: i64,
}

#[derive(SimpleObject)]
pub struct ControlStateHistoryView {
    pub id: i64,
    pub domain: String,
    pub recorded_at_utc: String,
    pub payload: Json<Value>,
}

impl From<ControlStateHistoryEntry> for ControlStateHistoryView {
    fn from(entry: ControlStateHistoryEntry) -> Self {
        Self {
            id: entry.id,
            domain: entry.domain,
            recorded_at_utc: entry.recorded_at_utc,
            payload: Json(entry.payload),
        }
    }
}
