use std::{
    collections::BTreeMap,
    env,
    path::{Path as FsPath, PathBuf},
    sync::{Mutex, OnceLock, RwLock},
};

use axum::{
    extract::{Json, Path, State},
    http::{HeaderMap, StatusCode},
    Json as JsonResponse,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    api::resource_registry::active_resource_binding_for_node,
    core::{
        artifact_reconciliation_context::{
            build_artifact_reconciliation_context_with_authority, required_completeness_gaps,
            PlacementAuthority,
        },
        operational_dgraph::{
            authorization_is_valid_for_context, build_operational_slice,
            build_reconciliation_authorization, required_mode_persistence_eligible,
            validate_operational_dgraph, ArtifactBindingV1, OperationalDGraph,
            OperationalDGraphSlice, OperationalReconciliationAuthorization,
            OperationalReconciliationAuthorizationRequest, AUTHORIZATION_OPERATION_RECONCILE,
            AUTHORIZATION_OPERATION_ROLLBACK, AUTHORIZATION_SCOPE_DURABLE,
            AUTHORIZATION_SCOPE_PROCESS, BLOCKED_REQUIRED_PERSISTENCE,
        },
        service_artifact_registry::{pinning_mode_from_state, EXECUTION_PINNING_MODE_REQUIRED},
        operational_dgraph_persistence::{
            authorization_key, persist_operational_dgraph_state, OperationalDGraphPersistedState,
            OPERATIONAL_DGRAPH_PERSISTENCE_VERSION,
        },
        operational_dgraph_recovery::{
            load_configured_operational_dgraph_state, OperationalDGraphRecoveryPolicy,
        },
    },
    AppState,
};

type ApiError = (StatusCode, JsonResponse<Value>);
type AuthorizationMap = BTreeMap<String, OperationalReconciliationAuthorization>;

const DEFAULT_OPERATIONAL_DGRAPH_STATE_PATH: &str = ".datum-operational-dgraph-state.json";
const OPERATIONAL_DGRAPH_STATE_PATH_ENV: &str = "DATUM_OPERATIONAL_DGRAPH_STATE_PATH";

static OPERATIONAL_DGRAPHS: OnceLock<RwLock<BTreeMap<String, OperationalDGraph>>> = OnceLock::new();
static OPERATIONAL_AUTHORIZATIONS: OnceLock<RwLock<AuthorizationMap>> = OnceLock::new();
static PERSISTENCE_PATH: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();
static PERSISTENCE_DB_PATH: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();
static STATE_TRANSACTION: OnceLock<Mutex<()>> = OnceLock::new();
static PERSISTENCE_INITIALIZED: OnceLock<Result<(), String>> = OnceLock::new();

fn graph_store() -> &'static RwLock<BTreeMap<String, OperationalDGraph>> {
    OPERATIONAL_DGRAPHS.get_or_init(|| RwLock::new(BTreeMap::new()))
}
fn authorization_store() -> &'static RwLock<AuthorizationMap> {
    OPERATIONAL_AUTHORIZATIONS.get_or_init(|| RwLock::new(BTreeMap::new()))
}
fn persistence_path_store() -> &'static RwLock<Option<PathBuf>> {
    PERSISTENCE_PATH.get_or_init(|| RwLock::new(None))
}
fn persistence_db_path_store() -> &'static RwLock<Option<PathBuf>> {
    PERSISTENCE_DB_PATH.get_or_init(|| RwLock::new(None))
}
fn state_transaction() -> &'static Mutex<()> {
    STATE_TRANSACTION.get_or_init(|| Mutex::new(()))
}

fn configured_json_path() -> PathBuf {
    env::var(OPERATIONAL_DGRAPH_STATE_PATH_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_OPERATIONAL_DGRAPH_STATE_PATH))
}

/// Startup must call this before binding the HTTP listener. Failure is cached;
/// a subsequent request cannot retry against an unrelated empty state.
pub fn initialize_operational_dgraph_persistence_from_env() -> Result<(), String> {
    ensure_operational_dgraph_persistence_initialized()
}

fn ensure_operational_dgraph_persistence_initialized() -> Result<(), String> {
    PERSISTENCE_INITIALIZED
        .get_or_init(|| {
            let path = configured_json_path();
            let policy = OperationalDGraphRecoveryPolicy::from_env()?;
            let restored = initialize_operational_dgraph_persistence_with_policy(&path, &policy)?;
            if restored {
                println!("restored operational D-Graph state from configured storage");
            } else {
                println!("explicit empty operational D-Graph bootstrap authorized");
            }
            Ok(())
        })
        .clone()
}

pub fn initialize_operational_dgraph_persistence(path: &FsPath) -> Result<bool, String> {
    let policy = OperationalDGraphRecoveryPolicy::from_env()?;
    initialize_operational_dgraph_persistence_with_policy(path, &policy)
}

fn initialize_operational_dgraph_persistence_with_policy(
    path: &FsPath,
    policy: &OperationalDGraphRecoveryPolicy,
) -> Result<bool, String> {
    let _transaction = state_transaction().lock().map_err(string_lock_error)?;
    let state = load_configured_operational_dgraph_state(path)?;
    policy.validate(state.as_ref())?;
    let restored = state.is_some();
    let state = state.unwrap_or_else(OperationalDGraphPersistedState::empty);
    // Resolve durable leases before changing any in-memory state. A failed
    // registry read must not replace an already loaded graph with an empty one.
    let authorizations = prepare_restored_authorizations(&state)?;
    let mut graphs_guard = graph_store().write().map_err(string_lock_error)?;
    let mut auth_guard = authorization_store().write().map_err(string_lock_error)?;
    let mut path_guard = persistence_path_store().write().map_err(string_lock_error)?;
    let mut db_guard = persistence_db_path_store().write().map_err(string_lock_error)?;
    *graphs_guard = state.graphs;
    *auth_guard = authorizations;
    *path_guard = Some(path.to_path_buf());
    *db_guard = crate::storage::control_state::configured_control_state_db_path();
    Ok(restored)
}

fn prepare_restored_authorizations(state: &OperationalDGraphPersistedState) -> Result<AuthorizationMap, String> {
    let mut restored = BTreeMap::new();
    let now = Utc::now();
    for (key, authorization) in &state.authorizations {
        let Some(graph) = state.graphs.get(&authorization.project_id) else { continue; };
        let Some(binding) = active_resource_binding_for_node(&authorization.project_id, &authorization.node_id)? else { continue; };
        if authorization_is_valid_for_context(authorization, graph, &authorization.node_id, &binding.binding_id, now) {
            restored.insert(key.clone(), authorization.clone());
        }
    }
    Ok(restored)
}

#[derive(Debug, Deserialize)]
pub struct CreateOperationalProjectRequest {
    pub project_id: String,
    #[serde(default)]
    pub dgraph_id: Option<String>,
}

/// Create a new project with an explicit, inert empty operational graph.
///
/// This endpoint never creates nodes, desired services, or host-mutation
/// authorization. The first deployable graph must be published later with a
/// higher revision and explicit reconciliation authorization.
pub async fn post_operational_project(
    Json(request): Json<CreateOperationalProjectRequest>,
) -> Result<JsonResponse<OperationalDGraph>, ApiError> {
    ensure_operational_dgraph_persistence_initialized().map_err(internal_error)?;

    let project_id = request.project_id.trim();
    if project_id.is_empty() {
        return Err(bad_request(
            "project_id must be a non-empty string".to_string(),
        ));
    }
    let dgraph_id = request
        .dgraph_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("dgraph:{project_id}"));

    let _transaction = state_transaction().lock().map_err(lock_error)?;
    if graph_store()
        .read()
        .map_err(lock_error)?
        .contains_key(project_id)
    {
        return Err(conflict(format!(
            "operational D-Graph project '{}' already exists",
            project_id
        )));
    }

    let graph = OperationalDGraph {
        schema_version: crate::core::operational_dgraph::OPERATIONAL_DGRAPH_SCHEMA_VERSION
            .to_string(),
        dgraph_id,
        revision: 0,
        project_id: project_id.to_string(),
        nodes: Vec::new(),
        metadata: Some(json!({
            "bootstrap_mode": crate::core::operational_dgraph::OPERATIONAL_DGRAPH_BOOTSTRAP_MODE_EMPTY,
            "host_mutation_allowed": false,
        })),
    };
    validate_operational_dgraph(&graph).map_err(bad_request)?;

    let mut candidate = persisted_candidate_from_memory().map_err(internal_error)?;
    candidate
        .graphs
        .insert(project_id.to_string(), graph.clone());
    persist_candidate(&candidate).map_err(internal_error)?;
    replace_in_memory_graphs(&candidate).map_err(internal_error)?;
    Ok(JsonResponse(graph))
}

pub async fn get_operational_projects() -> Result<JsonResponse<Value>, ApiError> {
    ensure_operational_dgraph_persistence_initialized().map_err(internal_error)?;
    let _transaction = state_transaction().lock().map_err(lock_error)?;
    let graphs = graph_store().read().map_err(lock_error)?;
    let projects = graphs
        .values()
        .map(|graph| {
            json!({
                "project_id": graph.project_id,
                "dgraph_id": graph.dgraph_id,
                "revision": graph.revision,
                "node_count": graph.nodes.len(),
                "bootstrap_mode": graph.metadata.as_ref()
                    .and_then(|metadata| metadata.get("bootstrap_mode"))
                    .and_then(Value::as_str),
            })
        })
        .collect::<Vec<_>>();
    Ok(JsonResponse(json!({ "projects": projects })))
}

pub async fn put_operational_dgraph(
    headers: HeaderMap,
    Json(graph): Json<OperationalDGraph>,
) -> Result<JsonResponse<OperationalDGraph>, ApiError> {
    ensure_operational_dgraph_persistence_initialized().map_err(internal_error)?;
    let project_id = extract_project_id(&headers)?;
    // Stage 111 (ADR-0018): once a project has an active canonical
    // `datum.dmap/1`-backed deployment, this legacy write path must never
    // independently overwrite placement -- fail closed rather than let a
    // stale write silently clobber (or race) the canonical projection.
    if crate::api::ddeploy::canonical_authority_active(&project_id).map_err(internal_error)? {
        return Err(canonical_authority_conflict(&project_id));
    }
    validate_operational_dgraph(&graph).map_err(bad_request)?;
    crate::core::multinode_dgraph::validate_multinode_metadata(&graph).map_err(bad_request)?;
    if graph.project_id.trim() != project_id {
        return Err(bad_request(format!("operational D-Graph project_id '{}' does not match X-Datum-Project '{}'", graph.project_id, project_id)));
    }
    let _transaction = state_transaction().lock().map_err(lock_error)?;
    let current_graphs = graph_store().read().map_err(lock_error)?.clone();
    if let Some(existing) = current_graphs.get(&project_id) {
        if graph.revision < existing.revision {
            return Err(conflict(format!("operational D-Graph revision regression: active revision is {}, received {}", existing.revision, graph.revision)));
        }
        if graph.revision == existing.revision {
            if &graph == existing { return Ok(JsonResponse(existing.clone())); }
            return Err(conflict(format!("operational D-Graph revision {} is already active with different content; publish a higher revision", graph.revision)));
        }
    }
    let replacing_existing = current_graphs.contains_key(&project_id);
    let mut candidate = persisted_candidate_from_memory().map_err(internal_error)?;
    candidate.graphs.insert(project_id.clone(), graph.clone());
    if replacing_existing {
        let prefix = format!("{}|", project_id);
        candidate.authorizations.retain(|key, _| !key.starts_with(&prefix));
    }
    persist_candidate(&candidate).map_err(internal_error)?;
    replace_in_memory_graphs(&candidate).map_err(internal_error)?;
    if replacing_existing {
        let prefix = format!("{}|", project_id);
        authorization_store().write().map_err(lock_error)?.retain(|key, _| !key.starts_with(&prefix));
    }
    Ok(JsonResponse(graph))
}

pub async fn get_operational_dgraph(headers: HeaderMap) -> Result<JsonResponse<OperationalDGraph>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let graph = operational_dgraph_snapshot(&project_id).map_err(internal_error)?
        .ok_or_else(|| missing_graph(&project_id))?;
    Ok(JsonResponse(graph))
}

pub async fn get_operational_dgraph_slice(
    Path(node_id): Path<String>,
    headers: HeaderMap,
) -> Result<JsonResponse<OperationalDGraphSlice>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let slice = operational_dgraph_slice_snapshot(&project_id, &node_id)
        .map_err(internal_error)?.ok_or_else(|| missing_graph(&project_id))?;
    Ok(JsonResponse(slice))
}

/// Capture the accepted graph under the same transaction used by publication.
///
/// Stage 111 (ADR-0018) effective-authority accessor: this is the ONE
/// centralized decision point every read path in this crate uses. If a
/// canonical `datum.dmap/1`-derived deployment is active for `project_id`,
/// that derived graph (`api::ddeploy::effective_canonical_operational_graph`)
/// IS the current operational graph -- never the legacy in-process store,
/// which would otherwise silently go stale the instant canonical authority
/// activates (ADR-0002). Every consumer in this crate reaches the legacy
/// store exclusively through this function or `graph_and_authorizations_snapshot`
/// (audited: `alternative_paths`, `artifact_reconciliation`,
/// `governed_deployment_preflight`, `external_ai_inference`,
/// `operational_dgraph_acquisition`, `operational_analysis_context`,
/// `live_dashboard`, `secret_preflight`, `secret_resolver_contract`,
/// `graphql`, `sentinel_telemetry`), so there is no bypass where one API
/// sees canonical placement and another sees the stale legacy graph.
pub fn operational_dgraph_snapshot(project_id: &str) -> Result<Option<OperationalDGraph>, String> {
    ensure_operational_dgraph_persistence_initialized()?;
    if let Some(graph) = crate::api::ddeploy::effective_canonical_operational_graph(project_id)? {
        return Ok(Some(graph));
    }
    let _transaction = state_transaction().lock().map_err(string_lock_error)?;
    Ok(graph_store().read().map_err(string_lock_error)?.get(project_id.trim()).cloned())
}

fn graph_and_authorizations_snapshot(project_id: &str) -> Result<Option<(OperationalDGraph, AuthorizationMap)>, String> {
    ensure_operational_dgraph_persistence_initialized()?;
    // Resolved BEFORE acquiring `state_transaction()`: the canonical check
    // reads a separate SQLite domain via `core::ddeploy`, never this
    // module's mutex, so there is no lock-ordering or reentrancy hazard.
    let canonical_graph = crate::api::ddeploy::effective_canonical_operational_graph(project_id)?;
    let _transaction = state_transaction().lock().map_err(string_lock_error)?;
    let graph = match canonical_graph {
        Some(graph) => graph,
        None => {
            let graphs = graph_store().read().map_err(string_lock_error)?;
            match graphs.get(project_id.trim()).cloned() {
                Some(graph) => graph,
                None => return Ok(None),
            }
        }
    };
    let authorizations = authorization_store().read().map_err(string_lock_error)?.clone();
    Ok(Some((graph, authorizations)))
}

fn materialize_slices_for_graph<F>(
    graph: &OperationalDGraph,
    authorizations: &AuthorizationMap,
    now: DateTime<Utc>,
    mut binding_for_node: F,
) -> Result<Vec<OperationalDGraphSlice>, String>
where F: FnMut(&str) -> Result<Option<String>, String> {
    validate_operational_dgraph(graph)?;
    let mut slices = Vec::with_capacity(graph.nodes.len());
    for node in &graph.nodes {
        let binding = binding_for_node(&node.node_id)?;
        let authorization = authorizations.get(&authorization_key(&graph.project_id, &node.node_id))
            .filter(|authorization| binding.as_deref().is_some_and(|binding_id| {
                authorization_is_valid_for_context(authorization, graph, &node.node_id, binding_id, now)
            }));
        slices.push(build_operational_slice(graph, &node.node_id, authorization)?);
    }
    Ok(slices)
}

/// All slices are built from one captured graph, even if a newer revision is
/// published while external Resource Registry bindings are being resolved.
pub fn operational_dgraph_acquisition_snapshot(
    project_id: &str,
) -> Result<Option<(OperationalDGraph, Vec<OperationalDGraphSlice>)>, String> {
    let Some((graph, authorizations)) = graph_and_authorizations_snapshot(project_id)? else { return Ok(None); };
    let project_id = graph.project_id.clone();
    let slices = materialize_slices_for_graph(&graph, &authorizations, Utc::now(), |node_id| {
        Ok(active_resource_binding_for_node(&project_id, node_id)?.map(|binding| binding.binding_id))
    })?;
    Ok(Some((graph, slices)))
}

pub fn operational_dgraph_slice_snapshot(
    project_id: &str,
    node_id: &str,
) -> Result<Option<OperationalDGraphSlice>, String> {
    let Some((graph, authorizations)) = graph_and_authorizations_snapshot(project_id)? else { return Ok(None); };
    let node_id = node_id.trim();
    if !graph.nodes.iter().any(|node| node.node_id.trim() == node_id) { return Ok(None); }
    let binding = active_resource_binding_for_node(&graph.project_id, node_id)?;
    let authorization = authorizations.get(&authorization_key(&graph.project_id, node_id))
        .filter(|authorization| binding.as_ref().is_some_and(|binding| {
            authorization_is_valid_for_context(authorization, &graph, node_id, &binding.binding_id, Utc::now())
        }));
    build_operational_slice(&graph, node_id, authorization).map(Some)
}

pub async fn post_operational_reconciliation_authorization(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<OperationalReconciliationAuthorizationRequest>,
) -> Result<JsonResponse<OperationalReconciliationAuthorization>, ApiError> {
    if request.operation.trim() != AUTHORIZATION_OPERATION_RECONCILE {
        return Err(bad_request(format!(
            "reconciliation authorization operation must be '{}'",
            AUTHORIZATION_OPERATION_RECONCILE
        )));
    }
    authorize_operational_reconciliation(state, node_id, headers, request).await
}

pub async fn post_operational_reconciliation_rollback_authorization(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<OperationalReconciliationAuthorizationRequest>,
) -> Result<JsonResponse<OperationalReconciliationAuthorization>, ApiError> {
    if request.operation.trim() != AUTHORIZATION_OPERATION_ROLLBACK {
        return Err(bad_request(format!(
            "rollback authorization operation must be '{}'",
            AUTHORIZATION_OPERATION_ROLLBACK
        )));
    }
    authorize_operational_reconciliation(state, node_id, headers, request).await
}

async fn authorize_operational_reconciliation(
    state: AppState,
    node_id: String,
    headers: HeaderMap,
    request: OperationalReconciliationAuthorizationRequest,
) -> Result<JsonResponse<OperationalReconciliationAuthorization>, ApiError> {
    ensure_operational_dgraph_persistence_initialized().map_err(internal_error)?;
    let project_id = extract_project_id(&headers)?;
    let node_id = node_id.trim();

    // P1-4 (v4 review): this endpoint issues authorizations for `reconcile`
    // and `rollback` -- both mean "gate a real host mutation". A
    // `process`-scope authorization NEVER has a finite lease
    // (`build_reconciliation_authorization` rejects `valid_for_seconds` for
    // `process`, so `expires_at_utc` is always `None`), and
    // `authorization_has_finite_lease` / `LeaseGuard::from_authorization`
    // refuse any mutation attempt against a non-finite lease
    // (`blocked_authorization_not_leased`). Issuing one from this endpoint
    // therefore only produces an authorization that is unusable for its own
    // stated purpose and fails opaquely later, at mutation time, instead of
    // clearly, at issuance time. Refuse it outright here.
    let requested_persistence_scope_for_shape_check = request
        .persistence_scope
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(AUTHORIZATION_SCOPE_PROCESS);
    if !reconciliation_authorization_scope_is_acceptable(requested_persistence_scope_for_shape_check) {
        return Err(bad_request(format!(
            "operational reconciliation/rollback authorization requires persistence_scope='{AUTHORIZATION_SCOPE_DURABLE}' with a valid_for_seconds lease; '{requested_persistence_scope_for_shape_check}' can never satisfy a governed host mutation's finite-lease requirement (it would only be discovered later, as blocked_authorization_not_leased, at mutation time)"
        )));
    }

    let binding = active_resource_binding_for_node(&project_id, node_id)
        .map_err(internal_error)?.ok_or_else(|| conflict(format!("cannot authorize operational reconciliation for node '{}': no active Resource Registry binding exists", node_id)))?;
    // Stage 111 (ADR-0018): resolved BEFORE the transaction lock (own SQLite
    // domain, not this module's mutex -- see `graph_and_authorizations_snapshot`).
    // An authorization issued while canonical authority is active must bind
    // the canonical-derived graph's `(dgraph_id, revision)`, never the stale
    // legacy graph's -- otherwise it would authorize a mutation against
    // content that is not, in fact, the current effective deployment.
    let canonical_graph = crate::api::ddeploy::effective_canonical_operational_graph(&project_id).map_err(internal_error)?;
    // Stage 111 (ADR-0018 independent review v2, P0-1): the SAME authority
    // decision that selected `canonical_graph` above must also govern how
    // artifacts are resolved for the authorization snapshot below -- an
    // authorization issued while canonical authority is active must bind
    // artifacts using `CanonicalDMap` semantics (never re-checking
    // `target_nodes` as current placement truth), exactly matching what
    // `get_operational_reconciliation_context` will resolve when it later
    // serves this same node.
    let placement_authority = if canonical_graph.is_some() {
        PlacementAuthority::CanonicalDMap
    } else {
        PlacementAuthority::LegacyTargetNodes
    };
    // Hold the transaction from the graph read through authorization commit.
    // A concurrent publication cannot invalidate the revision between them.
    let _transaction = state_transaction().lock().map_err(lock_error)?;
    let graph = match canonical_graph {
        Some(graph) => graph,
        None => graph_store().read().map_err(lock_error)?.get(&project_id).cloned()
            .ok_or_else(|| missing_graph(&project_id))?,
    };

    let slice_for_binding = build_operational_slice(&graph, node_id, None).map_err(bad_request)?;
    let requested_persistence_scope = request
        .persistence_scope
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(AUTHORIZATION_SCOPE_PROCESS)
        .to_string();
    // P0-2 (v4 review, DC-4): the operational-authorization domain's actual
    // resolved backend path, read once here (this lock is independent of the
    // registry lock taken below -- no ordering interaction).
    let operational_authorization_backend_path =
        persistence_db_path_store().read().map_err(lock_error)?.clone();

    // P0-1 (v4 review): resolve, gap-check, persistence-eligibility-check,
    // snapshot, build, AND commit the authorization all under the SAME held
    // registry read lock (`with_stable_view`). `state_transaction()` (`_transaction`,
    // held above) only serializes this handler against other operational-graph
    // writers -- it does NOT block a concurrent
    // `ServiceArtifactRegistry::create`/`update`/`delete`/`set_pinning_mode`,
    // none of which touch `state_transaction()`. Without holding the registry
    // lock across the whole sequence, a PUT could commit strictly between the
    // artifact snapshot resolved here and the authorization commit below,
    // binding a snapshot that was already stale the instant it was issued.
    // Under `RwLock` semantics, `ServiceArtifactRegistry::mutate` (used by
    // every registry write) cannot acquire its write lock while this closure
    // still holds the read lock, so the resolved artifacts are provably the
    // same artifacts still current when the authorization below is committed.
    let authorization = state
        .service_artifacts
        .with_stable_view(|registry_state| -> Result<OperationalReconciliationAuthorization, ApiError> {
            // Stage 107A (ADR-0011 §2, D-7a): resolve the exact artifact
            // content this authorization will bind, using the SAME join
            // rules `get_operational_reconciliation_context` uses, so the
            // snapshot bound here is exactly what the agent will later be
            // served. A join failure (no matching artifact, multiple
            // matches, missing container runtime, or an unsafe/incomplete
            // assignment) refuses authorization outright -- content that
            // cannot be resolved cannot be bound.
            let artifacts = registry_state
                .artifacts_by_project
                .get(&project_id)
                .cloned()
                .unwrap_or_default();
            let resolved_context = build_artifact_reconciliation_context_with_authority(&slice_for_binding, &artifacts, placement_authority)
                .map_err(|error| bad_request(format!(
                    "cannot authorize: artifact join failed for node '{node_id}': {error}"
                )))?;
            let artifact_bindings: Vec<ArtifactBindingV1> = resolved_context
                .artifacts
                .iter()
                .map(|assignment| ArtifactBindingV1 {
                    service_id: assignment.service_id.clone(),
                    artifact_id: assignment.artifact_id.clone(),
                    execution_generation: assignment.execution_generation,
                    execution_projection_digest: assignment.execution_projection_digest.clone(),
                })
                .collect();
            let pinning_mode = pinning_mode_from_state(registry_state, &project_id);

            // P0-3 (ADR-0011 N-2): a `required` authorization must be
            // resolved against a required-COMPLETE snapshot -- refuse
            // outright otherwise, rather than issuing a "required"
            // authorization the agent can only partially honour.
            // `legacy_partial` proceeds regardless (its gaps are recorded as
            // unpinned inputs downstream, not refused here).
            if pinning_mode == EXECUTION_PINNING_MODE_REQUIRED {
                let incomplete: Vec<(String, Vec<String>)> = resolved_context
                    .artifacts
                    .iter()
                    .filter_map(|assignment| {
                        let gaps = required_completeness_gaps(assignment);
                        (!gaps.is_empty()).then(|| (assignment.service_id.clone(), gaps))
                    })
                    .collect();
                if !incomplete.is_empty() {
                    return Err(bad_request(format!(
                        "blocked_required_incomplete: execution_pinning_mode=required but the resolved snapshot is not required-complete for node '{node_id}': {incomplete:?}"
                    )));
                }
            }

            // P0-2 (DC-4, ADR-0011 §3): `required` refuses a durable
            // authorization unless the artifact registry and the durable
            // operational-authorization state are backed by the exact same
            // control-state SQLite database file.
            if pinning_mode == EXECUTION_PINNING_MODE_REQUIRED
                && !required_mode_persistence_eligible(
                    &requested_persistence_scope,
                    state.service_artifacts.backend_path().as_deref(),
                    operational_authorization_backend_path.as_deref(),
                )
            {
                return Err(bad_request(format!(
                    "{BLOCKED_REQUIRED_PERSISTENCE}: execution_pinning_mode=required refuses a '{requested_persistence_scope}'-scope authorization because the artifact registry and the durable operational-authorization state are not backed by the same control-state SQLite database"
                )));
            }

            // P0-1: the authorization stores the EXACT resolved assignments
            // as the immutable snapshot. A later artifact PUT changes the
            // mutable registry (and future resolutions), but never this
            // already-captured Vec -- and, per the locking above, cannot
            // even commit until this closure returns.
            let snapshot = resolved_context.artifacts.clone();
            let authorization = build_reconciliation_authorization(
                &graph,
                node_id,
                &binding.binding_id,
                &binding.resource_id,
                &request,
                Utc::now(),
                artifact_bindings,
                &pinning_mode,
                snapshot,
            )
                .map_err(bad_request)?;
            let key = authorization_key(&project_id, node_id);
            let existing_durable = authorization_store().read().map_err(lock_error)?.get(&key)
                .is_some_and(|current| current.persistence_scope == AUTHORIZATION_SCOPE_DURABLE);
            if authorization.persistence_scope == AUTHORIZATION_SCOPE_DURABLE || existing_durable {
                let mut candidate = persisted_candidate_from_memory().map_err(internal_error)?;
                if authorization.persistence_scope == AUTHORIZATION_SCOPE_DURABLE {
                    candidate.authorizations.insert(key.clone(), authorization.clone());
                } else {
                    candidate.authorizations.remove(&key);
                }
                persist_candidate(&candidate).map_err(internal_error)?;
            }
            authorization_store().write().map_err(lock_error)?.insert(key, authorization.clone());
            Ok(authorization)
        })
        .map_err(internal_error)??;
    Ok(JsonResponse(authorization))
}

/// P1-4 (v4 review): true when `requested_persistence_scope` is acceptable
/// for `authorize_operational_reconciliation` (issuing `reconcile` /
/// `rollback` authorizations). Only `durable` is -- `process`-scope never
/// carries a finite lease (`build_reconciliation_authorization` rejects
/// `valid_for_seconds` for it), and every governed host mutation refuses a
/// non-finite lease anyway (`blocked_authorization_not_leased`), so a
/// `process`-scope authorization from this endpoint can never legally gate a
/// mutation. Extracted as a small pure function so it is directly
/// unit-testable: this handler has no HTTP-level test harness in this crate.
fn reconciliation_authorization_scope_is_acceptable(requested_persistence_scope: &str) -> bool {
    requested_persistence_scope == AUTHORIZATION_SCOPE_DURABLE
}

fn persisted_candidate_from_memory() -> Result<OperationalDGraphPersistedState, String> {
    let graphs = graph_store().read().map_err(string_lock_error)?.clone();
    let authorizations = authorization_store().read().map_err(string_lock_error)?.iter()
        .filter(|(_, authorization)| authorization.persistence_scope == AUTHORIZATION_SCOPE_DURABLE)
        .map(|(key, authorization)| (key.clone(), authorization.clone())).collect();
    Ok(OperationalDGraphPersistedState {
        persistence_version: OPERATIONAL_DGRAPH_PERSISTENCE_VERSION.to_string(), graphs, authorizations,
    })
}

fn persist_candidate(state: &OperationalDGraphPersistedState) -> Result<(), String> {
    let path = persistence_path_store().read().map_err(string_lock_error)?.clone()
        .ok_or_else(|| "operational D-Graph persistence path is not initialized".to_string())?;
    let configured_db = crate::storage::control_state::configured_control_state_db_path();
    let initialized_db = persistence_db_path_store().read().map_err(string_lock_error)?.clone();
    if configured_db != initialized_db {
        return Err("operational D-Graph storage configuration changed after initialization; refusing to write to a different backend".to_string());
    }
    persist_operational_dgraph_state(&path, state)
}

fn replace_in_memory_graphs(state: &OperationalDGraphPersistedState) -> Result<(), String> {
    *graph_store().write().map_err(string_lock_error)? = state.graphs.clone();
    Ok(())
}

fn extract_project_id(headers: &HeaderMap) -> Result<String, ApiError> {
    headers.get("x-datum-project").and_then(|value| value.to_str().ok()).map(str::trim)
        .filter(|value| !value.is_empty()).map(ToString::to_string)
        .ok_or_else(|| bad_request("missing or invalid X-Datum-Project header".to_string()))
}
fn missing_graph(project_id: &str) -> ApiError {
    not_found(format!("operational D-Graph not found for project '{}'", project_id))
}
fn bad_request(message: String) -> ApiError { (StatusCode::BAD_REQUEST, JsonResponse(json!({"error": message}))) }
fn conflict(message: String) -> ApiError { (StatusCode::CONFLICT, JsonResponse(json!({"error": message}))) }
fn canonical_authority_conflict(project_id: &str) -> ApiError {
    (StatusCode::CONFLICT, JsonResponse(json!({
        "error": "canonical_dmap_authority_active",
        "message": format!("project '{project_id}' has an active canonical D-Deploy deployment; legacy operational D-Graph writes are refused -- publish a new D-Deploy proposal and accept it instead"),
    })))
}
fn not_found(message: String) -> ApiError { (StatusCode::NOT_FOUND, JsonResponse(json!({"error": message}))) }
fn internal_error(message: String) -> ApiError { (StatusCode::INTERNAL_SERVER_ERROR, JsonResponse(json!({"error": message}))) }
fn string_lock_error<T>(error: std::sync::PoisonError<T>) -> String {
    format!("operational D-Graph state lock poisoned: {error}")
}
fn lock_error<T>(error: std::sync::PoisonError<T>) -> ApiError { internal_error(string_lock_error(error)) }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        operational_dgraph::{OperationalNodeDesiredState, OperationalDesiredService, OPERATIONAL_DGRAPH_SCHEMA_VERSION},
        operational_dgraph_acquisition::build_active_operational_dgraph_acquisition,
    };

    fn graph(revision: u64) -> OperationalDGraph {
        OperationalDGraph {
            schema_version: OPERATIONAL_DGRAPH_SCHEMA_VERSION.to_string(),
            dgraph_id: "dgraph:test".to_string(), revision,
            project_id: "project:test".to_string(),
            nodes: vec![
                OperationalNodeDesiredState { node_id: "fog-01".to_string(), desired_services: vec![OperationalDesiredService { service_id: "mqtt".to_string(), runtime: "container".to_string(), desired_state: "running".to_string(), application_id: String::new(), dmap_ref: None }] },
                OperationalNodeDesiredState { node_id: "cloud-01".to_string(), desired_services: Vec::new() },
            ],
            metadata: Some(json!({"relations":[{"from":"fog-01","to":"cloud-01","rel":"connected_to","latency_ms":12.5}]})),
        }
    }

    #[test]
    fn c68_captured_revision_produces_complete_compatible_slices() {
        let captured = graph(4);
        let mut accepted = captured.clone();
        let slices = materialize_slices_for_graph(&captured, &BTreeMap::new(), Utc::now(), |_| Ok(None)).unwrap();
        accepted.revision = 5;
        assert_eq!(slices.len(), 2);
        assert!(slices.iter().all(|slice| slice.dgraph_revision == 4));
        let package = build_active_operational_dgraph_acquisition(captured.clone(), slices, Utc::now()).unwrap();
        assert_eq!(package.operational_dgraph, captured);
        assert_eq!(package.topology.relations.len(), 1);
        assert!(package.slice_compatibility.compatible);
        assert!(!package.side_effects_performed);
        assert_eq!(accepted.revision, 5);
    }

    #[test]
    fn c68_binding_lookup_failure_does_not_produce_partial_acquisition() {
        let graph = graph(4);
        let result = materialize_slices_for_graph(&graph, &BTreeMap::new(), Utc::now(), |node| {
            if node == "cloud-01" { Err("registry unavailable".to_string()) } else { Ok(None) }
        });
        assert!(result.unwrap_err().contains("registry unavailable"));
    }

    // ---- P1-4: reconcile/rollback authorization endpoint refuses --------
    // ---- `process` scope outright ---------------------------------------

    #[test]
    fn reconciliation_authorization_scope_accepts_durable() {
        assert!(reconciliation_authorization_scope_is_acceptable(
            AUTHORIZATION_SCOPE_DURABLE
        ));
    }

    #[test]
    fn reconciliation_authorization_scope_rejects_process() {
        assert!(!reconciliation_authorization_scope_is_acceptable(
            AUTHORIZATION_SCOPE_PROCESS
        ));
    }

    #[test]
    fn reconciliation_authorization_scope_rejects_unknown_values() {
        assert!(!reconciliation_authorization_scope_is_acceptable("bogus"));
        assert!(!reconciliation_authorization_scope_is_acceptable(""));
    }
}
