use std::sync::Arc;

use async_graphql::{
    http::GraphiQLSource, Context, EmptyMutation, EmptySubscription, Json, Object, Schema,
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
    storage::firebase::FirebaseClient,
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
