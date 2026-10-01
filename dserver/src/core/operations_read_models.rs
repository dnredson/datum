//! Phase 127: read-only, bounded operational projections, never authority.
//! SQLite connections are READ_ONLY, never initialize schemas, and use one read
//! transaction per request. Other sources are cloned, validated snapshots.
use super::{
    active_monitoring::{MonitoringCheckStatus, MonitoringLifecycle, MonitoringSchedulerStatus},
    canonical_dmonitor::{DMonitorHealthV1, DMonitorSubjectV1},
    datum_alert::{DatumAlertActorV1, DatumAlertStatusV1, DatumAlertV1},
    datum_event::{
        DatumEventCategoryV1, DatumEventEvidenceRefV1, DatumEventSeverityV1, DatumEventSourceV1,
        DatumEventSubjectV1,
    },
    ddeploy::{self, DDeployStateV1},
    dmonitor_freshness::{self, DMonitorFreshnessPolicyV1},
    dmonitor_state::{self, DMonitorObservationRecordV1, DMonitorStateV1},
    dnode_registry::{self, DNodeRegistryStateV1},
    operational_dgraph::{self, OperationalDGraph},
};
use crate::storage::{
    datum_alert_store::{self, DatumAlertStoreError},
    datum_event_store::{self, DatumEventStoreError, StoredDatumEvent},
};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{params_from_iter, types::Value as SqlValue, Connection, OpenFlags};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::Duration,
};

pub const DEFAULT_LIMIT: u32 = 100;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProjectRuntimeGraph {
    pub project_id: String,
    pub logical: ProjectGraphOverview,
    pub runtime_source_kind: &'static str,
    pub runtime: SourceSection<super::runtime_graph_read_model::RuntimeContent>,
    pub temporal_quality: Vec<TemporalQualityView>,
}
impl OperationsSources {
    pub fn project_runtime_graph(
        &self,
        project: &str,
        at: DateTime<Utc>,
    ) -> Result<ProjectRuntimeGraph, OperationsReadError> {
        let logical = self.project_graph_overview(project, at)?;
        let mut temporal_quality = vec![];
        let runtime = match &self.dmonitor {
            SourceSection::Available(state) => SourceSection::from_result((|| {
                let graph = match &logical.graph {
                    SourceSection::Available(g) => Some(g),
                    _ => None,
                };
                let content = super::runtime_graph_read_model::build(project, state, graph)
                    .map_err(|e| error(ReadErrorKind::SourceCorrupt, "dmonitor", e))?;
                for evidence in &content.evidence {
                    let record = state.by_observation_id(&evidence.observation_id).ok_or_else(|| {
                        error(ReadErrorKind::SourceCorrupt, "dmonitor", "missing evidence")
                    })?;
                    temporal_quality.push(temporal(record, &self.policy, at)?);
                }
                Ok(content)
            })()),
            _ => self.dmonitor.map(|_| unreachable!()),
        }
        .scoped(Some(project), None);
        if !matches!(runtime, SourceSection::Available(_)) {
            temporal_quality.clear();
        }
        Ok(ProjectRuntimeGraph {
            project_id: project.into(),
            logical,
            runtime_source_kind: "admitted_dserv_realization_evidence",
            runtime,
            temporal_quality,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProjectGraphOverview {
    pub project_id: String,
    pub generated_at_utc: String,
    pub source_kind: &'static str,
    pub graph: SourceSection<super::project_graph_read_model::GraphContent>,
}
impl OperationsSources {
    pub fn project_graph_overview(
        &self,
        project: &str,
        at: DateTime<Utc>,
    ) -> Result<ProjectGraphOverview, OperationsReadError> {
        if project.is_empty()
            || project.len() > 256
            || project.trim() != project
            || project.chars().any(char::is_control)
        {
            return Err(invalid("invalid project scope"));
        }
        let graph = match &self.deployment {
            SourceSection::Available(state) => SourceSection::from_result((|| {
                let mut applications = BTreeSet::new();
                for a in state.active_by_application.values() {
                    if a.active_dmap.project_id == project {
                        applications.insert(a.application_id.clone());
                    }
                }
                for a in state.phase113_v2.active.values() {
                    if a.map.project_id == project {
                        applications.insert(a.map.application_id.clone());
                    }
                }
                let mut snapshots = vec![];
                for application in applications {
                    let a = ddeploy::active_placement_authority(state, project, &application)
                        .map_err(|e| error(ReadErrorKind::SourceCorrupt, "ddeploy", e))?
                        .ok_or_else(|| {
                            error(
                                ReadErrorKind::SourceCorrupt,
                                "ddeploy",
                                "missing active authority",
                            )
                        })?;
                    let snapshot = a.dgraph_snapshot.ok_or_else(|| {
                        error(
                            ReadErrorKind::SourceCorrupt,
                            "ddeploy",
                            "missing accepted graph snapshot",
                        )
                    })?;
                    snapshots.push((
                        snapshot,
                        a.dgraph_digest.ok_or_else(|| {
                            error(
                                ReadErrorKind::SourceCorrupt,
                                "ddeploy",
                                "missing accepted graph digest",
                            )
                        })?,
                        a.acceptance_id,
                    ));
                }
                super::project_graph_read_model::build(snapshots)
                    .map_err(|e| error(ReadErrorKind::SourceCorrupt, "ddeploy", e))
            })()),
            _ => self.deployment.map(|_| unreachable!()),
        }
        .scoped(Some(project), None);
        Ok(ProjectGraphOverview {
            project_id: project.into(),
            generated_at_utc: canonical_at(at),
            source_kind: "accepted_application_dgraph_snapshots",
            graph,
        })
    }
}
pub const MAX_LIMIT: u32 = 1000;
pub const MAX_SAFE_MESSAGE_BYTES: usize = 512;
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadErrorKind {
    InvalidQuery,
    SourceUnavailable,
    SourceCorrupt,
    NotConfigured,
    Storage,
    InternalProjection,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OperationsReadError {
    pub kind: ReadErrorKind,
    source: String,
    scope: Option<ReadIssueScope>,
    code: String,
    safe_message: String,
    #[serde(skip)]
    diagnostic: String,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReadIssueScope {
    pub project_id: Option<String>,
    pub application_id: Option<String>,
    pub subject: Option<DatumEventSubjectV1>,
}
impl std::fmt::Display for OperationsReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}: {}", self.source, self.code, self.safe_message)
    }
}
impl std::error::Error for OperationsReadError {}
fn error(kind: ReadErrorKind, source: &str, message: impl Into<String>) -> OperationsReadError {
    let (code, safe) = match kind {
        ReadErrorKind::InvalidQuery => ("invalid_query", "The read query is invalid."),
        ReadErrorKind::SourceUnavailable => ("source_unavailable", "The source could not be read."),
        ReadErrorKind::SourceCorrupt => ("source_corrupt", "Source integrity validation failed."),
        ReadErrorKind::NotConfigured => ("not_configured", "The source is not configured."),
        ReadErrorKind::Storage => ("storage_failed", "The storage read failed."),
        ReadErrorKind::InternalProjection => {
            ("projection_failed", "The projection could not be produced.")
        }
    };
    // Only fixed classifications reach serialized diagnostics. Raw paths, SQL,
    // values and injected acquisition diagnostics stay in nonserialized storage.
    let source = match source {
        "query"
        | "event_alert_db"
        | "event_alert_configuration"
        | "datum_events"
        | "datum_alerts"
        | "operational_graphs"
        | "ddeploy"
        | "dmonitor"
        | "dnode_registry"
        | "scheduler"
        | "timestamp"
        | "lifecycle_reference"
        | "temporal_metadata" => source,
        _ => "source",
    };
    OperationsReadError {
        kind,
        source: source.into(),
        scope: None,
        code: code.into(),
        safe_message: bounded_text(safe, MAX_SAFE_MESSAGE_BYTES),
        diagnostic: message.into(),
    }
}
fn bounded_text(value: &str, max: usize) -> String {
    let mut end = value.len().min(max);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].into()
}
impl OperationsReadError {
    pub fn source(&self) -> &str {
        &self.source
    }
    pub fn code(&self) -> &str {
        &self.code
    }
    pub fn safe_message(&self) -> &str {
        &self.safe_message
    }
    pub fn scope(&self) -> Option<&ReadIssueScope> {
        self.scope.as_ref()
    }
    pub fn source_failure(
        kind: ReadErrorKind,
        source: &str,
        diagnostic: impl Into<String>,
    ) -> Self {
        error(kind, source, diagnostic)
    }
    /// Internal-only; callers must not forward this to Console/API consumers.
    pub fn internal_diagnostic(&self) -> &str {
        &self.diagnostic
    }
    fn scoped(mut self, project: Option<&str>, application: Option<&str>) -> Self {
        if project.is_some() || application.is_some() {
            self.scope = Some(ReadIssueScope {
                project_id: project
                    .filter(|s| s.len() <= MAX_SAFE_MESSAGE_BYTES)
                    .map(str::to_string),
                application_id: application
                    .filter(|s| s.len() <= MAX_SAFE_MESSAGE_BYTES)
                    .map(str::to_string),
                subject: None,
            });
        }
        self
    }
    fn with_subject(mut self, subject: &DatumEventSubjectV1) -> Self {
        if subject.validate().is_empty()
            && serde_json::to_vec(subject).is_ok_and(|s| s.len() <= 2048)
        {
            self.scope
                .get_or_insert(ReadIssueScope {
                    project_id: None,
                    application_id: None,
                    subject: None,
                })
                .subject = Some(subject.clone());
        }
        self
    }
}
fn invalid(message: impl Into<String>) -> OperationsReadError {
    error(ReadErrorKind::InvalidQuery, "query", message)
}
fn sql(error_value: rusqlite::Error) -> OperationsReadError {
    if matches!(
        &error_value,
        rusqlite::Error::InvalidColumnType(..)
            | rusqlite::Error::IntegralValueOutOfRange(..)
            | rusqlite::Error::FromSqlConversionFailure(..)
    ) || matches!(&error_value, rusqlite::Error::SqliteFailure(code, _) if matches!(code.code,
        rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase))
    {
        return error(
            ReadErrorKind::SourceCorrupt,
            "event_alert_db",
            error_value.to_string(),
        );
    }
    error(
        ReadErrorKind::Storage,
        "event_alert_db",
        error_value.to_string(),
    )
}
fn event_error(e: DatumEventStoreError) -> OperationsReadError {
    let kind = match &e {
        DatumEventStoreError::CorruptStoredEvent { .. } => ReadErrorKind::SourceCorrupt,
        DatumEventStoreError::InvalidQuery(_) => ReadErrorKind::InvalidQuery,
        _ => ReadErrorKind::Storage,
    };
    error(kind, "datum_events", e.to_string())
}
fn alert_error(e: DatumAlertStoreError) -> OperationsReadError {
    let kind = match &e {
        DatumAlertStoreError::CorruptStoredAlert { .. } => ReadErrorKind::SourceCorrupt,
        _ => ReadErrorKind::Storage,
    };
    error(kind, "datum_alerts", e.to_string())
}
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "availability", content = "value", rename_all = "snake_case")]
pub enum SourceSection<T> {
    Available(T),
    NotConfigured,
    Unavailable(OperationsReadError),
    Corrupt(OperationsReadError),
    Indeterminate(String),
}
impl<T> SourceSection<T> {
    fn scoped(self, project: Option<&str>, application: Option<&str>) -> Self {
        match self {
            Self::Unavailable(e) => Self::Unavailable(e.scoped(project, application)),
            Self::Corrupt(e) => Self::Corrupt(e.scoped(project, application)),
            other => other,
        }
    }
    fn from_result(result: Result<T, OperationsReadError>) -> Self {
        match result {
            Ok(v) => Self::Available(v),
            Err(e) if e.kind == ReadErrorKind::NotConfigured => Self::NotConfigured,
            Err(e) if e.kind == ReadErrorKind::SourceCorrupt => Self::Corrupt(e),
            Err(e) => Self::Unavailable(e),
        }
    }
    fn as_ref(&self) -> SourceSection<&T> {
        match self {
            Self::Available(v) => SourceSection::Available(v),
            Self::NotConfigured => SourceSection::NotConfigured,
            Self::Unavailable(e) => SourceSection::Unavailable(e.clone()),
            Self::Corrupt(e) => SourceSection::Corrupt(e.clone()),
            Self::Indeterminate(m) => SourceSection::Indeterminate(m.clone()),
        }
    }
    fn map<U>(&self, f: impl FnOnce(&T) -> U) -> SourceSection<U> {
        match self.as_ref() {
            SourceSection::Available(v) => SourceSection::Available(f(v)),
            SourceSection::NotConfigured => SourceSection::NotConfigured,
            SourceSection::Unavailable(e) => SourceSection::Unavailable(e),
            SourceSection::Corrupt(e) => SourceSection::Corrupt(e),
            SourceSection::Indeterminate(m) => SourceSection::Indeterminate(m),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BoundedPage<T> {
    pub items: Vec<T>,
    pub limit: u32,
    pub has_more: bool,
}
fn page<T>(mut items: Vec<T>, limit: u32) -> BoundedPage<T> {
    let has_more = items.len() > limit as usize;
    items.truncate(limit as usize);
    BoundedPage {
        items,
        limit,
        has_more,
    }
}
fn limit_valid(limit: u32) -> Result<(), OperationsReadError> {
    if !(1..=MAX_LIMIT).contains(&limit) {
        Err(invalid("limit must be 1..=1000"))
    } else {
        Ok(())
    }
}
fn id_valid(id: &str) -> Result<(), OperationsReadError> {
    if id.trim().is_empty() {
        Err(invalid(
            "identity must be nonblank; exact canonical value is preserved",
        ))
    } else {
        Ok(())
    }
}
fn time_key(value: &str) -> Result<(i64, u32), OperationsReadError> {
    datum_event_store::canonicalize_and_derive_sortable(value).map_err(invalid)
}
fn canonical_at(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}
fn parsed(value: &str) -> Result<DateTime<Utc>, OperationsReadError> {
    DateTime::parse_from_rfc3339(value)
        .map(|v| v.with_timezone(&Utc))
        .map_err(|e| {
            error(
                ReadErrorKind::InternalProjection,
                "timestamp",
                e.to_string(),
            )
        })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventPresentation {
    Historical,
    AttentionRelated,
    Recovery,
    ControlPlane,
    Monitoring,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OperationalEventView {
    pub event_id: String,
    pub project_id: Option<String>,
    pub application_id: Option<String>,
    pub category: DatumEventCategoryV1,
    pub kind: String,
    pub severity: DatumEventSeverityV1,
    pub source: DatumEventSourceV1,
    pub subject: DatumEventSubjectV1,
    pub occurred_at_utc: String,
    pub admitted_at_utc: String,
    pub evidence_refs: Vec<DatumEventEvidenceRefV1>,
    pub temporal_context: TemporalMetadata,
    pub associated_alerts: BoundedPage<String>,
    /// Lifecycle evidence is NOT a condition-confirmation association.
    pub lifecycle_alert_references: BoundedPage<String>,
    pub classification: EventPresentation,
    pub is_monitoring: bool,
    pub is_control_plane: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertPresentationGroup {
    OperationallyActive,
    PendingClosure,
    HistoricalClosed,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LifecycleCheckpoint {
    pub actor: Option<ActorProvenanceView>,
    pub at_utc: Option<String>,
    pub reason: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActorProvenanceView {
    pub actor_type: String,
    pub actor_id: Option<String>,
    pub model_ref: Option<String>,
}
fn actor_view(actor: &Option<DatumAlertActorV1>) -> Option<ActorProvenanceView> {
    actor.as_ref().map(|a| ActorProvenanceView {
        actor_type: serde_json::to_value(&a.actor_type)
            .expect("actor type serialization")
            .as_str()
            .expect("actor type token")
            .into(),
        actor_id: a.actor_id.clone(),
        model_ref: a.model_ref.clone(),
    })
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OperationalAlertView {
    pub generated_at_utc: String,
    pub alert_id: String,
    pub project_id: Option<String>,
    pub application_id: Option<String>,
    pub category: DatumEventCategoryV1,
    pub kind: String,
    pub subject: DatumEventSubjectV1,
    pub lifecycle_status: DatumAlertStatusV1,
    pub group: AlertPresentationGroup,
    pub severity: DatumEventSeverityV1,
    pub peak_severity: DatumEventSeverityV1,
    pub first_observed_at_utc: String,
    pub last_observed_at_utc: String,
    pub condition_confirmation_count: u64,
    pub count_semantics: String,
    pub acknowledged: LifecycleCheckpoint,
    pub in_progress: LifecycleCheckpoint,
    pub resolution: LifecycleCheckpoint,
    pub closure: LifecycleCheckpoint,
    pub latest_condition_event: OperationalEventView,
    pub recovery_event: Option<OperationalEventView>,
    pub condition_cleared_at_utc: Option<String>,
    pub condition_duration_seconds: Option<i64>,
    pub duration_semantics: String,
    pub is_operationally_active: bool,
    pub is_resolved_pending_close: bool,
    pub is_historical_closed: bool,
}
fn group(status: DatumAlertStatusV1) -> AlertPresentationGroup {
    match status {
        DatumAlertStatusV1::Open
        | DatumAlertStatusV1::Acknowledged
        | DatumAlertStatusV1::InProgress => AlertPresentationGroup::OperationallyActive,
        DatumAlertStatusV1::Resolved => AlertPresentationGroup::PendingClosure,
        DatumAlertStatusV1::Closed => AlertPresentationGroup::HistoricalClosed,
    }
}

/// Fixed filters, keyset pagination, inclusive lower/exclusive upper time range.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventCursor {
    pub occurred_at_utc: String,
    pub event_id: String,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "availability", content = "value", rename_all = "snake_case")]
pub enum TemporalMetadata {
    Absent,
    Available(OperationalTemporalContext),
    Invalid(OperationsReadError),
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OperationalTemporalContext {
    pub server_evaluated_at_utc: String,
    pub evidence: Vec<OperationalTemporalEvidence>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OperationalTemporalEvidence {
    pub observation_id: Option<String>,
    pub source_observed_at_utc: String,
    pub server_received_at_utc: String,
    pub observed_receipt_delta_seconds: i64,
    pub source_age_seconds: i64,
    pub receipt_age_seconds: i64,
}
fn temporal_metadata(event: &super::datum_event::DatumEventV1) -> TemporalMetadata {
    let Some(raw) = event.attributes.get("temporal_context") else {
        return TemporalMetadata::Absent;
    };
    let attempt = (|| -> Result<OperationalTemporalContext, &'static str> {
        let object = raw
            .as_object()
            .ok_or("temporal context must be an object")?;
        let timestamp = |v: Option<&Value>| -> Result<DateTime<Utc>, &'static str> {
            let text = v.and_then(Value::as_str).ok_or("missing timestamp")?;
            let at = DateTime::parse_from_rfc3339(text)
                .map_err(|_| "invalid timestamp")?
                .with_timezone(&Utc);
            if canonical_at(at) != text {
                return Err("timestamp is not canonical UTC");
            }
            Ok(at)
        };
        let evaluated = timestamp(object.get("server_evaluated_at_utc"))?;
        if time_key(&event.occurred_at_utc).map_err(|_| "invalid occurrence time")?
            != (evaluated.timestamp(), evaluated.timestamp_subsec_nanos())
        {
            return Err("evaluation differs from occurrence");
        }
        let evidence = object
            .get("evidence")
            .and_then(Value::as_array)
            .ok_or("missing evidence array")?;
        if evidence.len() > 16 {
            return Err("too much temporal evidence");
        }
        let mut records = Vec::new();
        for value in evidence {
            let value = value.as_object().ok_or("invalid evidence object")?;
            let observed = timestamp(value.get("source_observed_at_utc"))?;
            let received = timestamp(value.get("server_received_at_utc"))?;
            let integer = |key| {
                value
                    .get(key)
                    .and_then(Value::as_i64)
                    .ok_or("invalid integer")
            };
            let delta = integer("observed_receipt_delta_seconds")?;
            let source_age = integer("source_age_seconds")?;
            let receipt_age = integer("receipt_age_seconds")?;
            if delta != (observed - received).num_seconds()
                || source_age != (evaluated - observed).num_seconds()
                || receipt_age != (evaluated - received).num_seconds()
            {
                return Err("inconsistent temporal arithmetic");
            }
            // Signed ages are intentional: skew and before-receipt findings
            // preserve negative values rather than fabricating zero.
            let observation_id = match value.get("observation_id") {
                None => None,
                Some(Value::String(id)) if !id.trim().is_empty() => Some(id.clone()),
                _ => return Err("invalid observation identity"),
            };
            records.push(OperationalTemporalEvidence {
                observation_id,
                source_observed_at_utc: canonical_at(observed),
                server_received_at_utc: canonical_at(received),
                observed_receipt_delta_seconds: delta,
                source_age_seconds: source_age,
                receipt_age_seconds: receipt_age,
            });
        }
        Ok(OperationalTemporalContext {
            server_evaluated_at_utc: canonical_at(evaluated),
            evidence: records,
        })
    })();
    match attempt {
        Ok(context) => TemporalMetadata::Available(context),
        Err(detail) => {
            let mut issue = error(
                ReadErrorKind::InternalProjection,
                "temporal_metadata",
                detail,
            )
            .scoped(event.project_id.as_deref(), event.application_id.as_deref())
            .with_subject(&event.subject);
            issue.code = "invalid_temporal_metadata".into();
            issue.safe_message = "Producer temporal metadata is invalid.".into();
            TemporalMetadata::Invalid(issue)
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AlertTimelineCursor {
    pub occurred_epoch_seconds: i64,
    pub occurred_subsec_nanos: u32,
    pub event_id: String,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertTimelineRelationship {
    ConditionObservation,
    Recovery,
    LifecycleReference,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AlertTimelineEntry {
    pub event: OperationalEventView,
    pub relationships: Vec<AlertTimelineRelationship>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OperationalAlertTimeline {
    pub generated_at_utc: String,
    pub alert_id: String,
    pub entries: BoundedPage<AlertTimelineEntry>,
    pub next_cursor: Option<AlertTimelineCursor>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OperationalEventHistory {
    pub generated_at_utc: String,
    pub events: BoundedPage<OperationalEventView>,
    pub next_cursor: Option<EventCursor>,
}
#[derive(Debug, Clone)]
pub struct EventHistoryQuery {
    pub project_id: Option<String>,
    pub global_only: bool,
    pub application_id: Option<String>,
    pub subject: Option<DatumEventSubjectV1>,
    pub alert_id: Option<String>,
    pub kind: Option<String>,
    pub category: Option<DatumEventCategoryV1>,
    pub source: Option<DatumEventSourceV1>,
    pub severity: Option<DatumEventSeverityV1>,
    pub occurred_from_utc: Option<String>,
    pub occurred_to_utc: Option<String>,
    pub before: Option<EventCursor>,
    pub limit: u32,
}
impl Default for EventHistoryQuery {
    fn default() -> Self {
        Self {
            project_id: None,
            global_only: false,
            application_id: None,
            subject: None,
            alert_id: None,
            kind: None,
            category: None,
            source: None,
            severity: None,
            occurred_from_utc: None,
            occurred_to_utc: None,
            before: None,
            limit: DEFAULT_LIMIT,
        }
    }
}
impl EventHistoryQuery {
    fn predicate(&self) -> Result<(String, Vec<SqlValue>), OperationsReadError> {
        limit_valid(self.limit)?;
        if self.global_only && self.project_id.is_some() {
            return Err(invalid("global_only conflicts with project_id"));
        }
        let mut predicates = vec!["1=1".to_string()];
        let mut values = vec![];
        for (column, value) in [
            ("project_id", &self.project_id),
            ("application_id", &self.application_id),
            ("kind", &self.kind),
        ] {
            if let Some(value) = value {
                id_valid(value)?;
                predicates.push(format!("e.{column} = ?"));
                values.push(SqlValue::Text(value.clone()));
            }
        }
        if self.global_only {
            predicates.push("e.project_id IS NULL".into());
        }
        for (column, value) in [
            (
                "category",
                self.category.map(|v| serde_json::to_value(v).unwrap()),
            ),
            (
                "source",
                self.source
                    .as_ref()
                    .map(|v| serde_json::to_value(v).unwrap()),
            ),
            (
                "severity",
                self.severity.map(|v| serde_json::to_value(v).unwrap()),
            ),
        ] {
            if let Some(v) = value {
                predicates.push(format!("e.{column} = ?"));
                values.push(SqlValue::Text(v.as_str().unwrap().into()));
            }
        }
        if let Some(subject) = &self.subject {
            let findings = subject.validate();
            if !findings.is_empty() {
                return Err(invalid("invalid canonical subject"));
            }
            // Compare JSON member sets, not textual object-key ordering.
            let subject_json =
                serde_json::to_string(subject).map_err(|e| invalid(e.to_string()))?;
            predicates.push("NOT EXISTS (SELECT fullkey,type,atom FROM json_tree(json_extract(e.event_json,'$.subject')) EXCEPT SELECT fullkey,type,atom FROM json_tree(?)) AND NOT EXISTS (SELECT fullkey,type,atom FROM json_tree(?) EXCEPT SELECT fullkey,type,atom FROM json_tree(json_extract(e.event_json,'$.subject')))".into());
            values.extend([
                SqlValue::Text(subject_json.clone()),
                SqlValue::Text(subject_json),
            ]);
        }
        if let Some(id) = &self.alert_id {
            id_valid(id)?;
            predicates.push("EXISTS (SELECT 1 FROM datum_alert_events a WHERE a.event_id=e.event_id AND a.alert_id=?)".into());
            values.push(SqlValue::Text(id.clone()));
        }
        let from = self
            .occurred_from_utc
            .as_deref()
            .map(time_key)
            .transpose()?;
        let to = self.occurred_to_utc.as_deref().map(time_key).transpose()?;
        if matches!((from,to),(Some(f),Some(t)) if f>t) {
            return Err(invalid("reversed time range"));
        }
        for (bound, op) in [(from, ">="), (to, "<")] {
            if let Some((sec, nanos)) = bound {
                predicates.push(format!(
                    "(e.occurred_at_epoch_seconds,e.occurred_at_subsec_nanos) {op} (?,?)"
                ));
                values.extend([SqlValue::Integer(sec), SqlValue::Integer(nanos.into())]);
            }
        }
        if let Some(c) = &self.before {
            let (s, n) = time_key(&c.occurred_at_utc)?;
            id_valid(&c.event_id)?;
            predicates.push(
                "(e.occurred_at_epoch_seconds,e.occurred_at_subsec_nanos,e.event_id)<(?,?,?)"
                    .into(),
            );
            values.extend([
                SqlValue::Integer(s),
                SqlValue::Integer(n.into()),
                SqlValue::Text(c.event_id.clone()),
            ]);
        }
        Ok((predicates.join(" AND "), values))
    }
}
/// Query adapter: opens only existing files, never writes or initializes schema.
pub struct OperationsRepository {
    db_path: PathBuf,
}
impl OperationsRepository {
    pub fn from_runtime(
        runtime: &super::event_producers::EventProducerRuntime,
    ) -> Result<Self, OperationsReadError> {
        Self::new(
            runtime
                .database_path()
                .map_err(|e| {
                    error(
                        ReadErrorKind::SourceUnavailable,
                        "event_alert_configuration",
                        e.to_string(),
                    )
                })?
                .to_path_buf(),
        )
    }
    /// Presentation snapshot wrapper; no hidden wall-clock reads.
    pub fn event_history_at(
        &self,
        query: &EventHistoryQuery,
        at: DateTime<Utc>,
    ) -> Result<OperationalEventHistory, OperationsReadError> {
        let events = self.event_history(query)?;
        let next_cursor = if events.has_more {
            events.items.last().map(|e| EventCursor {
                occurred_at_utc: e.occurred_at_utc.clone(),
                event_id: e.event_id.clone(),
            })
        } else {
            None
        };
        Ok(OperationalEventHistory {
            generated_at_utc: canonical_at(at),
            events,
            next_cursor,
        })
    }
    pub fn new(db_path: PathBuf) -> Result<Self, OperationsReadError> {
        if !db_path.is_absolute() {
            return Err(invalid("read database path must be absolute"));
        }
        Ok(Self { db_path })
    }
    fn connection(&self) -> Result<Connection, OperationsReadError> {
        let connection = Connection::open_with_flags(
            &self.db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| {
            error(
                ReadErrorKind::SourceUnavailable,
                "event_alert_db",
                e.to_string(),
            )
        })?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(sql)?;
        Ok(connection)
    }
    pub fn event_history(
        &self,
        query: &EventHistoryQuery,
    ) -> Result<BoundedPage<OperationalEventView>, OperationsReadError> {
        query.predicate()?;
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(sql)?;
        history(&tx, query)
    }
    pub fn alert_view(
        &self,
        id: &str,
        at: DateTime<Utc>,
    ) -> Result<Option<OperationalAlertView>, OperationsReadError> {
        id_valid(id)?;
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(sql)?;
        datum_alert_store::fetch_alert_by_id(&tx, id)
            .map_err(alert_error)?
            .map(|a| project_alert(&tx, &a, at))
            .transpose()
    }
    pub fn alert_timeline(
        &self,
        id: &str,
        limit: u32,
    ) -> Result<BoundedPage<OperationalEventView>, OperationsReadError> {
        let timeline = self.alert_timeline_page(id, None, limit, DateTime::<Utc>::UNIX_EPOCH)?;
        Ok(BoundedPage {
            items: timeline
                .entries
                .items
                .into_iter()
                .map(|e| e.event)
                .collect(),
            limit,
            has_more: timeline.entries.has_more,
        })
    }
    /// Oldest-first keyset continuation over the entire episode evidence union.
    pub fn alert_timeline_page(
        &self,
        id: &str,
        after: Option<&AlertTimelineCursor>,
        limit: u32,
        at: DateTime<Utc>,
    ) -> Result<OperationalAlertTimeline, OperationsReadError> {
        id_valid(id)?;
        limit_valid(limit)?;
        if let Some(cursor) = after {
            id_valid(&cursor.event_id)?;
            if cursor.occurred_subsec_nanos >= 1_000_000_000
                || DateTime::from_timestamp(
                    cursor.occurred_epoch_seconds,
                    cursor.occurred_subsec_nanos,
                )
                .is_none()
            {
                return Err(invalid("invalid timeline cursor"));
            }
        }
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(sql)?;
        let alert = datum_alert_store::fetch_alert_by_id(&tx, id)
            .map_err(alert_error)?
            .ok_or_else(|| invalid("alert not found"))?;
        let mut values = vec![
            SqlValue::Text(alert.alert_id.clone()),
            SqlValue::Text(alert.alert_id.clone()),
        ];
        let mut predicate = "(EXISTS (SELECT 1 FROM datum_alert_events a WHERE a.alert_id=? AND a.event_id=e.event_id) OR EXISTS (SELECT 1 FROM datum_alert_transitions t WHERE t.alert_id=? AND t.related_event_id=e.event_id))".to_string();
        if let Some(cursor) = after {
            predicate.push_str(
                " AND (e.occurred_at_epoch_seconds,e.occurred_at_subsec_nanos,e.event_id)>(?,?,?)",
            );
            values.extend([
                SqlValue::Integer(cursor.occurred_epoch_seconds),
                SqlValue::Integer(cursor.occurred_subsec_nanos.into()),
                SqlValue::Text(cursor.event_id.clone()),
            ]);
        }
        values.push(SqlValue::Integer(limit as i64 + 1));
        // EXISTS selects each event once even if both relationships are present.
        let statement = format!("SELECT e.event_id FROM datum_events e WHERE {predicate} ORDER BY e.occurred_at_epoch_seconds,e.occurred_at_subsec_nanos,e.event_id LIMIT ?");
        let ids = read_ids(&tx, &statement, values)?;
        let items = ids
            .into_iter()
            .map(|event_id| {
                let event = stored_view(&tx, &event_id, limit)?;
                let associated: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM datum_alert_events WHERE alert_id=? AND event_id=?)", [&id, &event_id.as_str()], |r| r.get(0)).map_err(sql)?;
                let related: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM datum_alert_transitions WHERE alert_id=? AND related_event_id=?)", [&id, &event_id.as_str()], |r| r.get(0)).map_err(sql)?;
                let mut relationships = vec![];
                if associated { relationships.push(AlertTimelineRelationship::ConditionObservation); }
                if related {
                    relationships.push(if event.classification == EventPresentation::Recovery { AlertTimelineRelationship::Recovery } else { AlertTimelineRelationship::LifecycleReference });
                }
                Ok(AlertTimelineEntry { event, relationships })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let entries = page(items, limit);
        let next_cursor = if entries.has_more {
            entries
                .items
                .last()
                .map(|entry| {
                    let (seconds, nanos) = time_key(&entry.event.occurred_at_utc)?;
                    Ok(AlertTimelineCursor {
                        occurred_epoch_seconds: seconds,
                        occurred_subsec_nanos: nanos,
                        event_id: entry.event.event_id.clone(),
                    })
                })
                .transpose()?
        } else {
            None
        };
        Ok(OperationalAlertTimeline {
            generated_at_utc: canonical_at(at),
            alert_id: id.into(),
            entries,
            next_cursor,
        })
    }
}
fn read_ids(
    c: &Connection,
    statement: &str,
    values: Vec<SqlValue>,
) -> Result<Vec<String>, OperationsReadError> {
    let mut stmt = c.prepare(statement).map_err(sql)?;
    let rows = stmt
        .query_map(params_from_iter(values), |r| r.get(0))
        .map_err(sql)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(sql)
}
fn stored_view(
    c: &Connection,
    id: &str,
    limit: u32,
) -> Result<OperationalEventView, OperationsReadError> {
    let stored = datum_event_store::fetch_by_id(c, id)
        .map_err(event_error)?
        .ok_or_else(|| {
            error(
                ReadErrorKind::SourceCorrupt,
                "datum_events",
                "dangling evidence event",
            )
        })?;
    event_view(c, stored, limit)
}
fn event_view(
    c: &Connection,
    stored: StoredDatumEvent,
    limit: u32,
) -> Result<OperationalEventView, OperationsReadError> {
    let event = stored.event;
    let temporal_context = temporal_metadata(&event);
    let ids = read_ids(
        c,
        "SELECT alert_id FROM datum_alert_events WHERE event_id=? ORDER BY alert_id LIMIT ?",
        vec![
            SqlValue::Text(event.event_id.clone()),
            SqlValue::Integer(limit as i64 + 1),
        ],
    )?;
    for id in &ids {
        datum_alert_store::fetch_alert_by_id(c, id)
            .map_err(alert_error)?
            .ok_or_else(|| {
                error(
                    ReadErrorKind::SourceCorrupt,
                    "datum_alerts",
                    "dangling association",
                )
            })?;
    }
    let recovery = event.source == DatumEventSourceV1::ActiveMonitor
        && event.category == DatumEventCategoryV1::Observation
        && matches!(
            event.kind.as_str(),
            "observation_stale_cleared"
                | "observation_missing_cleared"
                | "observation_clock_skew_cleared"
                | "evaluation_before_receipt_cleared"
        );
    let lifecycle_ids = read_ids(c,"SELECT DISTINCT alert_id FROM datum_alert_transitions WHERE related_event_id=? ORDER BY alert_id LIMIT ?",vec![SqlValue::Text(event.event_id.clone()),SqlValue::Integer(limit as i64+1)])?;
    for id in &lifecycle_ids {
        let alert = datum_alert_store::fetch_alert_by_id(c, id)
            .map_err(alert_error)?
            .ok_or_else(|| {
                error(
                    ReadErrorKind::SourceCorrupt,
                    "datum_alerts",
                    "dangling lifecycle reference",
                )
            })?;
        let mut original = event.clone();
        if recovery {
            original.kind = event.kind.strip_suffix("_cleared").unwrap().into();
        }
        let key = super::datum_alert::derive_correlation_key(&original)
            .map_err(|e| error(ReadErrorKind::SourceCorrupt, "lifecycle_reference", e))?;
        if key != alert.correlation_key
            || (recovery
                && (event.attributes["condition_kind"] != alert.kind
                    || event.attributes["condition_correlation_key"] != key))
        {
            return Err(error(
                ReadErrorKind::SourceCorrupt,
                "lifecycle_reference",
                "transition evidence references a different condition",
            ));
        }
    }
    let is_monitoring = event.source == DatumEventSourceV1::ActiveMonitor;
    let is_control_plane =
        event.kind == "dmap_accepted" && event.source == DatumEventSourceV1::DDeploy;
    let classification = if recovery {
        EventPresentation::Recovery
    } else if is_control_plane {
        EventPresentation::ControlPlane
    } else if !ids.is_empty() {
        EventPresentation::AttentionRelated
    } else if is_monitoring {
        EventPresentation::Monitoring
    } else {
        EventPresentation::Historical
    };
    Ok(OperationalEventView {
        event_id: event.event_id,
        project_id: event.project_id,
        application_id: event.application_id,
        category: event.category,
        kind: event.kind,
        severity: event.severity,
        source: event.source,
        subject: event.subject,
        occurred_at_utc: event.occurred_at_utc,
        admitted_at_utc: stored.admitted_at_utc,
        evidence_refs: event.evidence_refs,
        temporal_context,
        associated_alerts: page(ids, limit),
        lifecycle_alert_references: page(lifecycle_ids, limit),
        classification,
        is_monitoring,
        is_control_plane,
    })
}
fn history(
    c: &Connection,
    q: &EventHistoryQuery,
) -> Result<BoundedPage<OperationalEventView>, OperationsReadError> {
    let (predicate, mut values) = q.predicate()?;
    values.push(SqlValue::Integer(q.limit as i64 + 1));
    let ids=read_ids(c,&format!("SELECT e.event_id FROM datum_events e WHERE {predicate} ORDER BY e.occurred_at_epoch_seconds DESC,e.occurred_at_subsec_nanos DESC,e.event_id DESC LIMIT ?"),values)?;
    let items = ids
        .into_iter()
        .map(|id| stored_view(c, &id, q.limit))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(page(items, q.limit))
}
fn project_alert(
    c: &Connection,
    a: &DatumAlertV1,
    at: DateTime<Utc>,
) -> Result<OperationalAlertView, OperationsReadError> {
    let group = group(a.status);
    let latest = stored_view(c, &a.latest_event_id, DEFAULT_LIMIT)?;
    let recovery_ids=read_ids(c,"SELECT related_event_id FROM datum_alert_transitions WHERE alert_id=? AND to_status='resolved' AND related_event_id IS NOT NULL ORDER BY id DESC LIMIT 1",vec![SqlValue::Text(a.alert_id.clone())])?;
    // Old resolve/reopen history is not the current resolution.
    let recovery = if a.resolved_at_utc.is_some() {
        recovery_ids
            .first()
            .map(|id| stored_view(c, id, DEFAULT_LIMIT))
            .transpose()?
    } else {
        None
    };
    let clear_at = recovery
        .as_ref()
        .filter(|e| e.classification == EventPresentation::Recovery)
        .map(|e| e.occurred_at_utc.clone());
    let end = if let Some(t) = &clear_at {
        parsed(t)?
    } else if let Some(t) = &a.resolved_at_utc {
        parsed(t)?
    } else {
        at
    };
    let duration = (end - parsed(&a.first_observed_at_utc)?).num_seconds();
    Ok(OperationalAlertView {
        generated_at_utc: canonical_at(at),
        alert_id: a.alert_id.clone(),
        project_id: a.project_id.clone(),
        application_id: a.application_id.clone(),
        category: a.category,
        kind: a.kind.clone(),
        subject: a.subject.clone(),
        lifecycle_status: a.status,
        is_operationally_active: group == AlertPresentationGroup::OperationallyActive,
        is_resolved_pending_close: group == AlertPresentationGroup::PendingClosure,
        is_historical_closed: group == AlertPresentationGroup::HistoricalClosed,
        group,
        severity: a.severity,
        peak_severity: a.peak_severity,
        first_observed_at_utc: a.first_observed_at_utc.clone(),
        last_observed_at_utc: a.last_observed_at_utc.clone(),
        condition_confirmation_count: a.occurrence_count,
        count_semantics: "condition observations/confirmations; not independent outages".into(),
        acknowledged: LifecycleCheckpoint {
            actor: actor_view(&a.acknowledged_by),
            at_utc: a.acknowledged_at_utc.clone(),
            reason: None,
        },
        in_progress: LifecycleCheckpoint {
            actor: actor_view(&a.in_progress_by),
            at_utc: a.in_progress_at_utc.clone(),
            reason: None,
        },
        resolution: LifecycleCheckpoint {
            actor: actor_view(&a.resolved_by),
            at_utc: a.resolved_at_utc.clone(),
            reason: a.resolution_reason.clone(),
        },
        closure: LifecycleCheckpoint {
            actor: actor_view(&a.closed_by),
            at_utc: a.closed_at_utc.clone(),
            reason: a.close_reason.clone(),
        },
        latest_condition_event: latest,
        recovery_event: recovery,
        condition_cleared_at_utc: clear_at,
        condition_duration_seconds: (duration >= 0).then_some(duration),
        duration_semantics: "elapsed episode span; not continuous outage duration".into(),
    })
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct AlertCounts {
    pub open: u64,
    pub acknowledged: u64,
    pub in_progress: u64,
    pub resolved_pending_close: u64,
    pub historical_closed: u64,
    pub critical_attention: u64,
    pub warning_attention: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AttentionItem {
    pub project_id: Option<String>,
    pub application_id: Option<String>,
    pub severity: DatumEventSeverityV1,
    pub reason_code: String,
    pub source: String,
    pub reference: String,
    pub since_utc: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AlertSection {
    pub counts: AlertCounts,
    pub attention_ordered: BoundedPage<OperationalAlertView>,
    pub attention: BoundedPage<AttentionItem>,
}
fn alert_section(
    c: &Connection,
    project: Option<&str>,
    application: Option<&str>,
    limit: u32,
    at: DateTime<Utc>,
) -> Result<AlertSection, OperationsReadError> {
    let dangling: i64 = c.query_row("SELECT count(*) FROM datum_alerts a LEFT JOIN datum_events e ON e.event_id=a.latest_event_id WHERE e.event_id IS NULL", [], |r|r.get(0)).map_err(sql)?;
    if dangling != 0 {
        return Err(error(
            ReadErrorKind::SourceCorrupt,
            "datum_alerts",
            "dangling latest event reference",
        ));
    }
    let mut predicate = "1=1".to_string();
    let mut values = vec![];
    for (column, id) in [("project_id", project), ("application_id", application)] {
        if let Some(id) = id {
            predicate.push_str(&format!(" AND {column}=?"));
            values.push(SqlValue::Text(id.into()));
        }
    }
    let mut counts = AlertCounts::default();
    let mut stmt=c.prepare(&format!("SELECT status,severity,count(*) FROM datum_alerts WHERE {predicate} GROUP BY status,severity")).map_err(sql)?;
    let rows = stmt
        .query_map(params_from_iter(values.clone()), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)? as u64,
            ))
        })
        .map_err(sql)?;
    for row in rows {
        let (status, severity, n) = row.map_err(sql)?;
        let active = match status.as_str() {
            "open" => {
                counts.open += n;
                true
            }
            "acknowledged" => {
                counts.acknowledged += n;
                true
            }
            "in_progress" => {
                counts.in_progress += n;
                true
            }
            "resolved" => {
                counts.resolved_pending_close += n;
                false
            }
            "closed" => {
                counts.historical_closed += n;
                false
            }
            _ => {
                return Err(error(
                    ReadErrorKind::SourceCorrupt,
                    "datum_alerts",
                    "invalid lifecycle status",
                ))
            }
        };
        if !matches!(severity.as_str(), "info" | "warning" | "high" | "critical") {
            return Err(error(
                ReadErrorKind::SourceCorrupt,
                "datum_alerts",
                "invalid severity",
            ));
        }
        if active && severity == "critical" {
            counts.critical_attention += n
        }
        if active && severity == "warning" {
            counts.warning_attention += n
        }
    }
    values.push(SqlValue::Integer(limit as i64 + 1));
    // Phase 123's safe seconds/nanos come from the canonical latest-event join.
    let statement=format!("SELECT a.alert_id FROM datum_alerts a JOIN datum_events e ON e.event_id=a.latest_event_id WHERE {} ORDER BY CASE a.status WHEN 'open' THEN 0 WHEN 'acknowledged' THEN 0 WHEN 'in_progress' THEN 0 WHEN 'resolved' THEN 1 ELSE 2 END, CASE a.severity WHEN 'critical' THEN 3 WHEN 'high' THEN 2 WHEN 'warning' THEN 1 ELSE 0 END DESC,e.occurred_at_epoch_seconds DESC,e.occurred_at_subsec_nanos DESC,a.alert_id LIMIT ?",predicate.replace("project_id","a.project_id").replace("application_id","a.application_id"));
    let ids = read_ids(c, &statement, values)?;
    let items = ids
        .into_iter()
        .map(|id| {
            let a = datum_alert_store::fetch_alert_by_id(c, &id)
                .map_err(alert_error)?
                .ok_or_else(|| {
                    error(
                        ReadErrorKind::SourceCorrupt,
                        "datum_alerts",
                        "missing alert",
                    )
                })?;
            project_alert(c, &a, at)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let attention = items
        .iter()
        .filter(|a| a.is_operationally_active)
        .map(|a| AttentionItem {
            project_id: a.project_id.clone(),
            application_id: a.application_id.clone(),
            severity: a.severity,
            reason_code: a.kind.clone(),
            source: "datum_alert".into(),
            reference: a.alert_id.clone(),
            since_utc: Some(a.first_observed_at_utc.clone()),
        })
        .collect();
    Ok(AlertSection {
        counts,
        attention: page(attention, limit),
        attention_ordered: page(items, limit),
    })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunicationFreshness {
    Fresh,
    Stale,
    MissingEvidence,
    Indeterminate,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceTimeQuality {
    AlignedOrNoDiscrepancy,
    SourceAhead,
    BehindOrDelayed,
    EvaluationOrderingAnomaly,
    MissingEvidence,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TemporalQualityView {
    pub subject: DatumEventSubjectV1,
    pub project_id: String,
    pub application_id: Option<String>,
    pub node_id: String,
    pub source_observed_at_utc: Option<String>,
    pub server_received_at_utc: Option<String>,
    pub evaluated_at_utc: String,
    pub observed_receipt_delta_seconds: Option<i64>,
    pub observed_age_seconds: Option<i64>,
    pub received_age_seconds: Option<i64>,
    pub communication_freshness: CommunicationFreshness,
    pub observation_freshness: CommunicationFreshness,
    pub source_time_quality: SourceTimeQuality,
    pub existing_dmonitor_freshness: Option<dmonitor_freshness::DMonitorFreshnessV1>,
    pub observation_id: Option<String>,
}
fn subject_scope(subject: &DMonitorSubjectV1) -> Option<(&str, &str)> {
    match subject {
        DMonitorSubjectV1::DServRealization {
            project_id,
            application_id,
            ..
        }
        | DMonitorSubjectV1::DCodeExecution {
            project_id,
            application_id,
            ..
        }
        | DMonitorSubjectV1::DCallDelivery {
            project_id,
            application_id,
            ..
        }
        | DMonitorSubjectV1::DIoTRealization {
            project_id,
            application_id,
            ..
        } => Some((project_id, application_id)),
        _ => None,
    }
}
fn temporal(
    record: &DMonitorObservationRecordV1,
    policy: &DMonitorFreshnessPolicyV1,
    at: DateTime<Utc>,
) -> Result<TemporalQualityView, OperationsReadError> {
    let observed = parsed(&record.observation.observed_at_utc)?;
    let received = parsed(&record.received_at_utc)?;
    let at_text = canonical_at(at);
    let ordering = at < received;
    let interpreted = if ordering {
        None
    } else {
        Some(
            dmonitor_freshness::evaluate_freshness(record, policy, &at_text)
                .map_err(|e| error(ReadErrorKind::SourceCorrupt, "dmonitor", e))?,
        )
    };
    let communication = if ordering {
        CommunicationFreshness::Indeterminate
    } else if interpreted.as_ref().unwrap().received_age_seconds
        > policy.max_received_age_seconds as i64
    {
        CommunicationFreshness::Stale
    } else {
        CommunicationFreshness::Fresh
    };
    let quality = if ordering {
        SourceTimeQuality::EvaluationOrderingAnomaly
    } else if interpreted.as_ref().unwrap().freshness
        == dmonitor_freshness::DMonitorFreshnessV1::ClockSkew
    {
        SourceTimeQuality::SourceAhead
    } else if (received - observed).num_seconds() > policy.max_observed_age_seconds as i64 {
        SourceTimeQuality::BehindOrDelayed
    } else {
        SourceTimeQuality::AlignedOrNoDiscrepancy
    };
    let node = record.observation.node_id.clone();
    let stream = dmonitor_state::stream_key(
        &record.server_project_scope,
        &node,
        &record.observation.monitor_instance_id,
    );
    Ok(TemporalQualityView {
        subject: DatumEventSubjectV1::Other {
            project_id: Some(record.server_project_scope.clone()),
            entity_kind: "dmonitor_stream".into(),
            entity_id: stream,
        },
        project_id: record.server_project_scope.clone(),
        application_id: subject_scope(&record.observation.subject).map(|(_, a)| a.into()),
        node_id: node,
        source_observed_at_utc: Some(canonical_at(observed)),
        server_received_at_utc: Some(canonical_at(received)),
        evaluated_at_utc: at_text,
        observed_receipt_delta_seconds: Some((observed - received).num_seconds()),
        observed_age_seconds: Some((at - observed).num_seconds()),
        received_age_seconds: Some((at - received).num_seconds()),
        communication_freshness: communication,
        observation_freshness: if ordering {
            CommunicationFreshness::Indeterminate
        } else if interpreted.as_ref().unwrap().observed_age_seconds
            > policy.max_observed_age_seconds as i64
        {
            CommunicationFreshness::Stale
        } else {
            CommunicationFreshness::Fresh
        },
        source_time_quality: quality,
        existing_dmonitor_freshness: interpreted.map(|v| v.freshness),
        observation_id: Some(record.observation.observation_id.clone()),
    })
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ObservedRuntimeView {
    pub observation_id: String,
    pub subject: DMonitorSubjectV1,
    pub observed_at_utc: String,
    pub received_at_utc: String,
    pub reported_health: DMonitorHealthV1,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MonitoringSchedulerView {
    pub generated_at_utc: String,
    pub configured: bool,
    pub lifecycle: Option<MonitoringLifecycle>,
    pub driver_activity: SchedulerDriverActivity,
    pub checks: BoundedPage<MonitoringCheckStatus>,
    pub completed_count: Option<u64>,
    pub dropped_output_full: Option<u64>,
    pub dropped_output_closed: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SchedulerDriverActivity {
    Unwired,
    Stopped,
    Stopping,
    Indeterminate,
}
fn scheduler_view(
    status: &SourceSection<MonitoringSchedulerStatus>,
    at: DateTime<Utc>,
    limit: u32,
) -> SourceSection<MonitoringSchedulerView> {
    match status {
        SourceSection::NotConfigured => SourceSection::Available(MonitoringSchedulerView {
            generated_at_utc: canonical_at(at),
            configured: false,
            lifecycle: None,
            driver_activity: SchedulerDriverActivity::Unwired,
            checks: page(vec![], limit),
            completed_count: None,
            dropped_output_full: None,
            dropped_output_closed: None,
        }),
        _ => status.map(|s| {
            let mut checks = s.checks.clone();
            for check in &mut checks {
                if let Some(super::active_monitoring::MonitoringLastEvaluation::Failed { error }) =
                    &mut check.last_evaluation_result
                {
                    use super::active_monitoring::MonitoringCheckError::*;
                    *error = match error {
                        EvidenceUnavailable(_) => {
                            EvidenceUnavailable("Check evidence is unavailable.".into())
                        }
                        InvalidEvidence(_) => {
                            InvalidEvidence("Check evidence validation failed.".into())
                        }
                        EvaluationFailed(_) => EvaluationFailed("Check evaluation failed.".into()),
                        InvalidResult(_) => InvalidResult("Check result validation failed.".into()),
                    };
                }
            }
            checks.sort_by(|a, b| a.check_id.cmp(&b.check_id));
            MonitoringSchedulerView {
                generated_at_utc: canonical_at(at),
                configured: true,
                lifecycle: Some(s.lifecycle),
                driver_activity: match s.lifecycle {
                    MonitoringLifecycle::Stopped => SchedulerDriverActivity::Stopped,
                    MonitoringLifecycle::Stopping => SchedulerDriverActivity::Stopping,
                    MonitoringLifecycle::Running => SchedulerDriverActivity::Indeterminate,
                },
                checks: page(checks, limit),
                completed_count: Some(s.completed_count),
                dropped_output_full: Some(s.dropped_output_full),
                dropped_output_closed: Some(s.dropped_output_closed),
            }
        }),
    }
}

/// Values owned by the request/component; no mutable global internals.
/// Builders validate snapshots with their existing source validators.
pub struct OperationsSources {
    db: Option<OperationsRepository>,
    graphs: SourceSection<BTreeMap<String, OperationalDGraph>>,
    deployment: SourceSection<DDeployStateV1>,
    dmonitor: SourceSection<DMonitorStateV1>,
    nodes: SourceSection<DNodeRegistryStateV1>,
    scheduler: SourceSection<MonitoringSchedulerStatus>,
    expected_streams: Vec<super::active_monitoring::DMonitorStreamSelector>,
    policy: DMonitorFreshnessPolicyV1,
}
#[derive(Debug, Clone, Copy)]
pub enum OperationsSource {
    ProjectGraphs,
    Deployment,
    DMonitor,
    Nodes,
    Scheduler,
}
impl Default for OperationsSources {
    fn default() -> Self {
        Self {
            db: None,
            graphs: SourceSection::NotConfigured,
            deployment: SourceSection::NotConfigured,
            dmonitor: SourceSection::NotConfigured,
            nodes: SourceSection::NotConfigured,
            scheduler: SourceSection::NotConfigured,
            expected_streams: vec![],
            policy: Default::default(),
        }
    }
}
impl OperationsSources {
    /// Caller-side acquisition errors stay explicit; absent configuration is
    /// different from a configured source whose snapshot could not be acquired.
    pub fn with_source_error(
        mut self,
        source: OperationsSource,
        issue: OperationsReadError,
    ) -> Self {
        match source {
            OperationsSource::ProjectGraphs => self.graphs = SourceSection::from_result(Err(issue)),
            OperationsSource::Deployment => {
                self.deployment = SourceSection::from_result(Err(issue))
            }
            OperationsSource::DMonitor => self.dmonitor = SourceSection::from_result(Err(issue)),
            OperationsSource::Nodes => self.nodes = SourceSection::from_result(Err(issue)),
            OperationsSource::Scheduler => self.scheduler = SourceSection::from_result(Err(issue)),
        }
        self
    }
    pub fn with_repository(mut self, repository: OperationsRepository) -> Self {
        self.db = Some(repository);
        self
    }
    pub fn with_project_graphs(mut self, graphs: BTreeMap<String, OperationalDGraph>) -> Self {
        let result = (|| {
            for (key, g) in &graphs {
                if key != &g.project_id {
                    return Err("project graph key mismatch".into());
                }
                operational_dgraph::validate_operational_dgraph(g)?;
            }
            Ok(graphs)
        })();
        self.graphs = SourceSection::from_result(
            result
                .map_err(|e: String| error(ReadErrorKind::SourceCorrupt, "operational_graphs", e)),
        );
        self
    }
    pub fn with_deployment(mut self, state: DDeployStateV1) -> Self {
        if state.state_version != ddeploy::DDEPLOY_STATE_SCHEMA_VERSION {
            self.deployment = SourceSection::Corrupt(error(
                ReadErrorKind::SourceCorrupt,
                "ddeploy",
                "unsupported state version",
            ));
            return self;
        }
        self.deployment = SourceSection::from_result(
            ddeploy::validate_persisted_state_integrity(&state)
                .map(|_| state)
                .map_err(|e| error(ReadErrorKind::SourceCorrupt, "ddeploy", e)),
        );
        self
    }
    pub fn with_dmonitor(
        mut self,
        state: DMonitorStateV1,
        policy: DMonitorFreshnessPolicyV1,
        expected: Vec<super::active_monitoring::DMonitorStreamSelector>,
    ) -> Result<Self, OperationsReadError> {
        dmonitor_freshness::validate_freshness_policy(&policy).map_err(invalid)?;
        if expected.len() > MAX_LIMIT as usize {
            return Err(invalid("too many expected stream selectors"));
        }
        let mut ids = BTreeSet::new();
        for s in &expected {
            let id = s.check_id().map_err(|e| invalid(e.to_string()))?;
            if !ids.insert(id) {
                return Err(invalid("duplicate expected selector"));
            }
        }
        let scopes: BTreeSet<_> = state
            .streams
            .values()
            .map(|s| s.project_scope.clone())
            .collect();
        let scope = if scopes.len() == 1 {
            scopes.iter().next().map(String::as_str)
        } else {
            None
        };
        self.dmonitor = SourceSection::from_result(
            dmonitor_state::validate_persisted_dmonitor_state_integrity(&state)
                .map(|_| state)
                .map_err(|e| {
                    error(ReadErrorKind::SourceCorrupt, "dmonitor", e).scoped(scope, None)
                }),
        );
        self.policy = policy;
        self.expected_streams = expected;
        self.expected_streams.sort_by_key(|s| {
            dmonitor_state::stream_key(&s.project_scope, &s.node_id, &s.monitor_instance_id)
        });
        Ok(self)
    }
    pub fn with_nodes(mut self, state: DNodeRegistryStateV1) -> Self {
        if state.state_version != dnode_registry::DNODE_REGISTRY_STATE_VERSION {
            self.nodes = SourceSection::Corrupt(error(
                ReadErrorKind::SourceCorrupt,
                "dnode_registry",
                "unsupported state version",
            ));
            return self;
        }
        self.nodes = SourceSection::from_result(
            dnode_registry::validate_persisted_dnode_registry_integrity(&state)
                .map(|_| state)
                .map_err(|e| error(ReadErrorKind::SourceCorrupt, "dnode_registry", e)),
        );
        self
    }
    pub fn with_scheduler(
        mut self,
        status: Result<MonitoringSchedulerStatus, OperationsReadError>,
    ) -> Self {
        self.scheduler = SourceSection::from_result(status);
        self
    }
    fn temporal_views(
        &self,
        project: Option<&str>,
        application: Option<&str>,
        at: DateTime<Utc>,
        limit: u32,
    ) -> SourceSection<BoundedPage<TemporalQualityView>> {
        match &self.dmonitor {
            SourceSection::Available(state) => SourceSection::from_result((|| {
                let mut items = BTreeMap::new();
                let mut records: BTreeMap<String, &DMonitorObservationRecordV1> = BTreeMap::new();
                if let Some(application) = application {
                    // Streams can interleave applications. Reuse accepted subject
                    // heads instead of pretending the global stream head belongs
                    // to every application it has ever observed.
                    for per_stream in state.subject_index.values() {
                        for (key, seq) in per_stream {
                            let record = &state.streams[key].observations[seq];
                            if subject_scope(&record.observation.subject).is_some_and(|(p, a)| {
                                a == application && project.is_none_or(|wanted| wanted == p)
                            }) && records.get(key).is_none_or(|prior| {
                                prior.observation.sequence < record.observation.sequence
                            }) {
                                records.insert(key.clone(), record);
                            }
                        }
                    }
                } else {
                    for (key, stream) in &state.streams {
                        if project.is_none_or(|p| p == stream.project_scope) {
                            records
                                .insert(key.clone(), &stream.observations[&stream.head_sequence]);
                        }
                    }
                }
                for (key, record) in records {
                    items.insert(key, temporal(record, &self.policy, at)?);
                }
                if application.is_none() {
                    for s in &self.expected_streams {
                        if project.is_some_and(|p| p != s.project_scope) {
                            continue;
                        }
                        let key = dmonitor_state::stream_key(
                            &s.project_scope,
                            &s.node_id,
                            &s.monitor_instance_id,
                        );
                        if !state.streams.contains_key(&key) {
                            items.insert(
                                key.clone(),
                                TemporalQualityView {
                                    subject: s.subject(),
                                    project_id: s.project_scope.clone(),
                                    application_id: None,
                                    node_id: s.node_id.clone(),
                                    source_observed_at_utc: None,
                                    server_received_at_utc: None,
                                    evaluated_at_utc: canonical_at(at),
                                    observed_receipt_delta_seconds: None,
                                    observed_age_seconds: None,
                                    received_age_seconds: None,
                                    communication_freshness:
                                        CommunicationFreshness::MissingEvidence,
                                    observation_freshness: CommunicationFreshness::MissingEvidence,
                                    source_time_quality: SourceTimeQuality::MissingEvidence,
                                    existing_dmonitor_freshness: None,
                                    observation_id: None,
                                },
                            );
                        }
                    }
                }
                Ok(page(items.into_values().collect(), limit))
            })()),
            _ => self.dmonitor.map(|_| unreachable!()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KnownApplicationReference {
    pub project_id: String,
    pub application_id: String,
    pub provenance: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryCompleteness {
    Complete,
    Partial,
    Indeterminate,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", content = "issue", rename_all = "snake_case")]
pub enum DiscoverySourceCoverage {
    FullyScanned,
    Truncated,
    Unavailable(OperationsReadError),
    Corrupt(OperationsReadError),
    NotConfigured,
    NotApplicable,
    Indeterminate,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiscoveryCoverage {
    pub completeness: DiscoveryCompleteness,
    /// Coverage of identity-key discovery, not certification of every stored row.
    pub source_coverage: BTreeMap<String, DiscoverySourceCoverage>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiscoveryPage<T> {
    pub page: BoundedPage<T>,
    pub coverage: DiscoveryCoverage,
}
fn discovery<T>(
    items: Vec<T>,
    limit: u32,
    sources: BTreeMap<String, DiscoverySourceCoverage>,
) -> DiscoveryPage<T> {
    let complete = sources.values().all(|s| {
        matches!(
            s,
            DiscoverySourceCoverage::FullyScanned | DiscoverySourceCoverage::NotApplicable
        )
    });
    let scanned = sources.values().any(|s| {
        matches!(
            s,
            DiscoverySourceCoverage::FullyScanned | DiscoverySourceCoverage::Truncated
        )
    });
    DiscoveryPage {
        page: page(items, limit),
        coverage: DiscoveryCoverage {
            completeness: if complete {
                DiscoveryCompleteness::Complete
            } else if scanned {
                DiscoveryCompleteness::Partial
            } else {
                DiscoveryCompleteness::Indeterminate
            },
            source_coverage: sources,
        },
    }
}
fn source_coverage<T>(source: &SourceSection<T>) -> DiscoverySourceCoverage {
    match source {
        SourceSection::Available(_) => DiscoverySourceCoverage::FullyScanned,
        SourceSection::NotConfigured => DiscoverySourceCoverage::NotConfigured,
        SourceSection::Unavailable(e) => DiscoverySourceCoverage::Unavailable(e.clone()),
        SourceSection::Corrupt(e) => DiscoverySourceCoverage::Corrupt(e.clone()),
        SourceSection::Indeterminate(_) => DiscoverySourceCoverage::Indeterminate,
    }
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AcceptedPlacementView {
    pub project_id: String,
    pub application_id: String,
    pub dmap_id: String,
    pub revision: u64,
    pub digest: String,
    pub acceptance_id: Option<String>,
    pub dgraph_id: Option<String>,
    pub dgraph_revision: Option<u64>,
    pub dgraph_digest: Option<String>,
    pub service_nodes: BTreeMap<String, String>,
    pub total_service_placements: usize,
    pub placements_truncated: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApplicationConvergenceView {
    pub evaluated_at_utc: String,
    pub convergence: super::dmonitor_interpretation::DMonitorConvergenceV1,
    pub desired_service_count: usize,
    pub unexpected_realization_count: usize,
    pub findings: BoundedPage<super::dmonitor_interpretation::DMonitorConvergenceFindingV1>,
}
fn convergence_view(
    mut v: super::dmonitor_interpretation::DMonitorApplicationConvergenceV1,
    limit: u32,
) -> ApplicationConvergenceView {
    v.findings
        .sort_by(|a, b| a.code.cmp(b.code).then(a.message.cmp(&b.message)));
    ApplicationConvergenceView {
        evaluated_at_utc: v.evaluated_at_utc,
        convergence: v.convergence,
        desired_service_count: v.services.len(),
        unexpected_realization_count: v.unexpected_realizations.len(),
        findings: page(v.findings, limit),
    }
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlannedPlacementView {
    pub proposal_id: String,
    pub created_at_utc: String,
    pub lifecycle: String,
    pub schema: String,
    pub dgraph_id: String,
    pub service_nodes: BTreeMap<String, String>,
    pub total_service_placements: usize,
    pub placements_truncated: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DesiredRuntimeView {
    pub service_id: String,
    pub runtime: String,
    pub desired_state: String,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApplicationOperationsOverview {
    pub generated_at_utc: String,
    pub project_id: String,
    pub application_id: String,
    pub canonical_application_registry: SourceSection<String>,
    pub accepted: SourceSection<Option<AcceptedPlacementView>>,
    pub planned: SourceSection<BoundedPage<PlannedPlacementView>>,
    pub desired_runtime: SourceSection<BoundedPage<DesiredRuntimeView>>,
    pub observed_runtime: SourceSection<BoundedPage<ObservedRuntimeView>>,
    pub temporal_quality: SourceSection<BoundedPage<TemporalQualityView>>,
    pub convergence: SourceSection<ApplicationConvergenceView>,
    pub reconciliation_execution: SourceSection<String>,
    pub alerts: SourceSection<AlertSection>,
    pub recent_events: SourceSection<BoundedPage<OperationalEventView>>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProjectOperationsOverview {
    pub generated_at_utc: String,
    pub project_id: String,
    pub operational_graph: SourceSection<OperationalGraphSummary>,
    pub known_applications: DiscoveryPage<KnownApplicationReference>,
    pub canonical_application_count: SourceSection<u64>,
    pub alerts: SourceSection<AlertSection>,
    pub recent_events: SourceSection<BoundedPage<OperationalEventView>>,
    pub temporal_quality: SourceSection<BoundedPage<TemporalQualityView>>,
    pub accepted_deployments: SourceSection<BoundedPage<AcceptedPlacementView>>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OperationalGraphSummary {
    pub dgraph_id: String,
    pub revision: u64,
    pub node_count: usize,
    pub desired_service_assignment_count: usize,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProjectAttentionSummary {
    pub project_id: String,
    pub active_alert_count: u64,
    pub critical_count: u64,
    pub warning_count: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GlobalOperationsOverview {
    pub generated_at_utc: String,
    pub managed_operational_project_count: SourceSection<usize>,
    pub canonical_application_count: SourceSection<u64>,
    pub structural_node_count: SourceSection<usize>,
    pub desired_service_assignment_count: SourceSection<usize>,
    pub known_projects: DiscoveryPage<String>,
    pub known_applications: DiscoveryPage<KnownApplicationReference>,
    pub alerts: SourceSection<AlertSection>,
    pub recent_event_count: SourceSection<u64>,
    pub recent_events: SourceSection<BoundedPage<OperationalEventView>>,
    pub projects_with_attention: SourceSection<BoundedPage<ProjectAttentionSummary>>,
    pub temporal_quality: SourceSection<BoundedPage<TemporalQualityView>>,
    pub scheduler: SourceSection<MonitoringSchedulerView>,
    pub source_issues: Vec<OperationsReadError>,
    pub consistency: String,
}
#[derive(Debug, Clone)]
pub struct OverviewQuery {
    pub limit: u32,
    pub recent_from_utc: String,
}
impl OverviewQuery {
    pub fn at(at: DateTime<Utc>) -> Self {
        Self {
            limit: DEFAULT_LIMIT,
            recent_from_utc: canonical_at(
                at.checked_sub_signed(chrono::Duration::hours(24))
                    .unwrap_or(DateTime::<Utc>::MIN_UTC),
            ),
        }
    }
    fn validate(&self, at: DateTime<Utc>) -> Result<(), OperationsReadError> {
        limit_valid(self.limit)?;
        if time_key(&self.recent_from_utc)? > time_key(&canonical_at(at))? {
            return Err(invalid("recent lower bound is after generation time"));
        }
        Ok(())
    }
}
struct DatabaseProjection {
    alerts: SourceSection<AlertSection>,
    events: SourceSection<BoundedPage<OperationalEventView>>,
    recent_count: SourceSection<u64>,
    applications: Vec<KnownApplicationReference>,
    projects: Vec<String>,
    project_attention: SourceSection<BoundedPage<ProjectAttentionSummary>>,
    application_coverage: BTreeMap<String, DiscoverySourceCoverage>,
    project_coverage: BTreeMap<String, DiscoverySourceCoverage>,
}
/// Dedicated bounded distinct identity queries, independent of event time windows
/// and presentation ordering. Representative rows use existing canonical checks.
fn database_identities(
    c: &Connection,
    alerts: bool,
    project: Option<&str>,
    applications: bool,
    limit: u32,
) -> Result<
    (
        Vec<KnownApplicationReference>,
        Vec<String>,
        DiscoverySourceCoverage,
    ),
    OperationsReadError,
> {
    let (table, identity) = if alerts {
        ("datum_alerts", "alert_id")
    } else {
        ("datum_events", "event_id")
    };
    let mut values = vec![];
    let mut predicate = "project_id IS NOT NULL".to_string();
    if applications {
        predicate.push_str(" AND application_id IS NOT NULL");
    }
    if let Some(p) = project {
        predicate.push_str(" AND project_id=?");
        values.push(SqlValue::Text(p.into()));
    }
    values.push(SqlValue::Integer(limit as i64 + 1));
    let grouping = if applications {
        "project_id,application_id"
    } else {
        "project_id"
    };
    let ids = read_ids(c, &format!("SELECT min({identity}) FROM {table} WHERE {predicate} GROUP BY {grouping} ORDER BY {grouping} LIMIT ?"), values)?;
    let truncated = ids.len() > limit as usize;
    let mut refs = vec![];
    let mut projects = BTreeSet::new();
    for id in ids.into_iter().take(limit as usize) {
        let (p, a) = if alerts {
            let a = datum_alert_store::fetch_alert_by_id(c, &id)
                .map_err(alert_error)?
                .ok_or_else(|| {
                    error(
                        ReadErrorKind::SourceCorrupt,
                        "datum_alerts",
                        "missing discovery row",
                    )
                })?;
            (a.project_id, a.application_id)
        } else {
            let e = datum_event_store::fetch_by_id(c, &id)
                .map_err(event_error)?
                .ok_or_else(|| {
                    error(
                        ReadErrorKind::SourceCorrupt,
                        "datum_events",
                        "missing discovery row",
                    )
                })?
                .event;
            (e.project_id, e.application_id)
        };
        if let Some(p) = p {
            projects.insert(p.clone());
            if applications {
                if let Some(a) = a {
                    refs.push(KnownApplicationReference {
                        project_id: p,
                        application_id: a,
                        provenance: vec![if alerts { "alert" } else { "event" }.into()],
                    });
                }
            }
        }
    }
    Ok((
        refs,
        projects.into_iter().collect(),
        if truncated {
            DiscoverySourceCoverage::Truncated
        } else {
            DiscoverySourceCoverage::FullyScanned
        },
    ))
}
fn discovery_failure(e: OperationsReadError) -> DiscoverySourceCoverage {
    if e.kind == ReadErrorKind::SourceCorrupt {
        DiscoverySourceCoverage::Corrupt(e)
    } else {
        DiscoverySourceCoverage::Unavailable(e)
    }
}
impl OperationsSources {
    fn database_projection(
        &self,
        project: Option<&str>,
        application: Option<&str>,
        at: DateTime<Utc>,
        q: &OverviewQuery,
    ) -> DatabaseProjection {
        let absent = || DatabaseProjection {
            alerts: SourceSection::NotConfigured,
            events: SourceSection::NotConfigured,
            recent_count: SourceSection::NotConfigured,
            applications: vec![],
            projects: vec![],
            project_attention: SourceSection::NotConfigured,
            application_coverage: BTreeMap::from([
                ("events".into(), DiscoverySourceCoverage::NotConfigured),
                ("alerts".into(), DiscoverySourceCoverage::NotConfigured),
            ]),
            project_coverage: BTreeMap::from([
                ("events".into(), DiscoverySourceCoverage::NotConfigured),
                ("alerts".into(), DiscoverySourceCoverage::NotConfigured),
            ]),
        };
        let Some(repo) = &self.db else {
            return absent();
        };
        let attempt: Result<DatabaseProjection, OperationsReadError> = (|| {
            let mut c = repo.connection()?;
            let tx = c.transaction().map_err(sql)?;
            let alerts =
                SourceSection::from_result(alert_section(&tx, project, application, q.limit, at));
            let history_query = EventHistoryQuery {
                project_id: project.map(str::to_string),
                application_id: application.map(str::to_string),
                occurred_from_utc: Some(q.recent_from_utc.clone()),
                occurred_to_utc: None,
                limit: q.limit,
                ..Default::default()
            };
            // Include only events at/before the common generation instant (not future evidence).
            let (predicate, mut values) = history_query.predicate()?;
            let (s, n) = time_key(&canonical_at(at))?;
            let predicate = format!(
                "{predicate} AND (e.occurred_at_epoch_seconds,e.occurred_at_subsec_nanos)<=(?,?)"
            );
            values.extend([SqlValue::Integer(s), SqlValue::Integer(n.into())]);
            let count: Result<u64, OperationsReadError> = tx
                .query_row(
                    &format!("SELECT count(*) FROM datum_events e WHERE {predicate}"),
                    params_from_iter(values.clone()),
                    |r| r.get::<_, i64>(0).map(|v| v as u64),
                )
                .map_err(sql);
            values.push(SqlValue::Integer(q.limit as i64 + 1));
            let event_result=read_ids(&tx,&format!("SELECT e.event_id FROM datum_events e WHERE {predicate} ORDER BY e.occurred_at_epoch_seconds DESC,e.occurred_at_subsec_nanos DESC,e.event_id DESC LIMIT ?"),values).and_then(|ids|ids.into_iter().map(|id|stored_view(&tx,&id,q.limit)).collect::<Result<Vec<_>,_>>()).map(|v|page(v,q.limit));
            // A corrupt returned page makes its aggregate unavailable, never zero.
            let recent_count = match &event_result {
                Err(e) => SourceSection::from_result(Err(e.clone())),
                Ok(_) => SourceSection::from_result(count),
            };
            let events = SourceSection::from_result(event_result);
            let mut applications = vec![];
            let mut projects = BTreeSet::new();
            let mut application_coverage = BTreeMap::new();
            let mut project_coverage = BTreeMap::new();
            for (name, is_alert) in [("events", false), ("alerts", true)] {
                match database_identities(&tx, is_alert, project, true, MAX_LIMIT) {
                    Ok((refs, _, coverage)) => {
                        applications.extend(refs);
                        application_coverage.insert(name.into(), coverage);
                    }
                    Err(e) => {
                        application_coverage.insert(
                            name.into(),
                            discovery_failure(e.scoped(project, application)),
                        );
                    }
                }
                match database_identities(&tx, is_alert, project, false, MAX_LIMIT) {
                    Ok((_, ids, coverage)) => {
                        projects.extend(ids);
                        project_coverage.insert(name.into(), coverage);
                    }
                    Err(e) => {
                        project_coverage.insert(
                            name.into(),
                            discovery_failure(e.scoped(project, application)),
                        );
                    }
                }
            }
            let project_attention = match &alerts {
                SourceSection::Available(_) => SourceSection::from_result((|| {
                    let mut stmt=tx.prepare("SELECT project_id,count(*),sum(CASE severity WHEN 'critical' THEN 1 ELSE 0 END),sum(CASE severity WHEN 'warning' THEN 1 ELSE 0 END) FROM datum_alerts WHERE project_id IS NOT NULL AND status IN ('open','acknowledged','in_progress') GROUP BY project_id ORDER BY project_id LIMIT ?").map_err(sql)?;
                    let rows = stmt
                        .query_map([q.limit as i64 + 1], |r| {
                            Ok(ProjectAttentionSummary {
                                project_id: r.get(0)?,
                                active_alert_count: r.get::<_, i64>(1)? as u64,
                                critical_count: r.get::<_, i64>(2)? as u64,
                                warning_count: r.get::<_, i64>(3)? as u64,
                            })
                        })
                        .map_err(sql)?;
                    Ok(page(
                        rows.collect::<Result<Vec<_>, _>>().map_err(sql)?,
                        q.limit,
                    ))
                })()),
                _ => alerts.map(|_| unreachable!()),
            };
            Ok(DatabaseProjection {
                alerts: alerts.scoped(project, application),
                events: events.scoped(project, application),
                recent_count: recent_count.scoped(project, application),
                applications,
                projects: projects.into_iter().collect(),
                project_attention,
                application_coverage,
                project_coverage,
            })
        })();
        match attempt {
            Ok(v) => v,
            Err(e) => {
                let e = e.scoped(project, application);
                DatabaseProjection {
                    alerts: SourceSection::from_result(Err(e.clone())),
                    events: SourceSection::from_result(Err(e.clone())),
                    recent_count: SourceSection::from_result(Err(e.clone())),
                    project_attention: SourceSection::from_result(Err(e.clone())),
                    application_coverage: BTreeMap::from([
                        ("events".into(), discovery_failure(e.clone())),
                        ("alerts".into(), discovery_failure(e.clone())),
                    ]),
                    project_coverage: BTreeMap::from([
                        ("events".into(), discovery_failure(e.clone())),
                        ("alerts".into(), discovery_failure(e.clone())),
                    ]),
                    ..absent()
                }
            }
        }
    }
    fn references(
        &self,
        db: &DatabaseProjection,
        project: Option<&str>,
        limit: u32,
    ) -> DiscoveryPage<KnownApplicationReference> {
        let mut map: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
        let mut add = |p: &str, a: &str, source: &str| {
            if project.is_none_or(|wanted| wanted == p) {
                map.entry((p.into(), a.into()))
                    .or_default()
                    .insert(source.into());
            }
        };
        for a in &db.applications {
            for source in &a.provenance {
                add(&a.project_id, &a.application_id, source)
            }
        }
        if let SourceSection::Available(s) = &self.deployment {
            for a in s.active_by_application.values() {
                add(
                    &a.active_dmap.project_id,
                    &a.application_id,
                    "accepted_dmap",
                );
            }
            for a in s.phase113_v2.active.values() {
                add(&a.map.project_id, &a.map.application_id, "accepted_dmap");
            }
            for p in s.proposals.values() {
                add(
                    &p.proposal.project_id,
                    &p.proposal.application_id,
                    "ddeploy_proposal",
                )
            }
            for p in s.phase113_v2.proposals.values() {
                add(
                    &p.proposal.project_id,
                    &p.proposal.application_id,
                    "ddeploy_proposal",
                )
            }
        }
        if let SourceSection::Available(graphs) = &self.graphs {
            for graph in graphs.values() {
                for node in &graph.nodes {
                    for service in &node.desired_services {
                        if !service.application_id.is_empty() {
                            add(
                                &graph.project_id,
                                &service.application_id,
                                "operational_graph",
                            );
                        }
                    }
                }
            }
        }
        if let SourceSection::Available(s) = &self.dmonitor {
            for streams in s.subject_index.values() {
                for (key, seq) in streams {
                    let record = &s.streams[key].observations[seq];
                    if let Some((p, a)) = subject_scope(&record.observation.subject) {
                        add(p, a, "dmonitor")
                    }
                }
            }
        }
        let mut coverage = db.application_coverage.clone();
        coverage.extend(self.snapshot_discovery_coverage());
        discovery(
            map.into_iter()
                .map(|((p, a), v)| KnownApplicationReference {
                    project_id: p,
                    application_id: a,
                    provenance: v.into_iter().collect(),
                })
                .collect(),
            limit,
            coverage,
        )
    }
    fn snapshot_discovery_coverage(&self) -> BTreeMap<String, DiscoverySourceCoverage> {
        BTreeMap::from([
            ("ddeploy".into(), source_coverage(&self.deployment)),
            ("dmonitor".into(), source_coverage(&self.dmonitor)),
            ("operational_graph".into(), source_coverage(&self.graphs)),
        ])
    }
    pub fn global_overview(
        &self,
        at: DateTime<Utc>,
        q: &OverviewQuery,
    ) -> Result<GlobalOperationsOverview, OperationsReadError> {
        q.validate(at)?;
        let db = self.database_projection(None, None, at, q);
        let applications = self.references(&db, None, q.limit);
        let mut projects: BTreeSet<String> = db.projects.iter().cloned().collect();
        // Project keys are discovered independently, never from a truncated
        // application page (one project may contain thousands of applications).
        if let SourceSection::Available(d) = &self.deployment {
            projects.extend(
                d.active_by_application
                    .values()
                    .map(|a| a.active_dmap.project_id.clone()),
            );
            projects.extend(
                d.phase113_v2
                    .active
                    .values()
                    .map(|a| a.map.project_id.clone()),
            );
            projects.extend(d.proposals.values().map(|p| p.proposal.project_id.clone()));
            projects.extend(
                d.phase113_v2
                    .proposals
                    .values()
                    .map(|p| p.proposal.project_id.clone()),
            );
        }
        if let SourceSection::Available(graphs) = &self.graphs {
            projects.extend(graphs.keys().cloned())
        }
        if let SourceSection::Available(d) = &self.dmonitor {
            projects.extend(d.streams.values().map(|s| s.project_scope.clone()))
        }
        let temporal = self.temporal_views(None, None, at, q.limit);
        let scheduler = scheduler_view(&self.scheduler, at, q.limit);
        let mut issues = vec![];
        for section in [
            self.graphs.map(|_| ()),
            self.deployment.map(|_| ()),
            self.dmonitor.map(|_| ()),
            self.nodes.map(|_| ()),
            db.alerts.map(|_| ()),
            db.events.map(|_| ()),
            scheduler.map(|_| ()),
        ] {
            match section {
                SourceSection::Unavailable(e) | SourceSection::Corrupt(e) => issues.push(e),
                _ => {}
            }
        }
        issues.sort_by(|a, b| {
            a.source
                .cmp(&b.source)
                .then(a.code.cmp(&b.code))
                .then(a.safe_message.cmp(&b.safe_message))
        });
        for coverage in [&applications.coverage.source_coverage, &db.project_coverage] {
            for source in coverage.values() {
                if let DiscoverySourceCoverage::Unavailable(e)
                | DiscoverySourceCoverage::Corrupt(e) = source
                {
                    issues.push(e.clone());
                }
            }
        }
        issues.sort_by(|a, b| {
            a.source
                .cmp(&b.source)
                .then(a.code.cmp(&b.code))
                .then(a.safe_message.cmp(&b.safe_message))
        });
        issues.dedup();
        let mut project_coverage = db.project_coverage.clone();
        project_coverage.extend(self.snapshot_discovery_coverage());
        Ok(GlobalOperationsOverview {
generated_at_utc:canonical_at(at),
            managed_operational_project_count:self.graphs.map(BTreeMap::len),
            canonical_application_count:SourceSection::NotConfigured,
            structural_node_count:self.nodes.map(|s|s.nodes.len()),
            desired_service_assignment_count:self.graphs.map(|graphs|graphs.values().map(|g|g.nodes.iter().map(|n|n.desired_services.len()).sum::<usize>()).sum()),
            known_projects:discovery(projects.into_iter().collect(),q.limit, project_coverage),
            known_applications:applications,
            alerts:db.alerts,
            recent_event_count:db.recent_count,
            recent_events:db.events,
            projects_with_attention:db.project_attention,
            temporal_quality:temporal,scheduler,
            source_issues:issues,
            consistency:"one Event/Alert SQLite read transaction; independently validated immutable subsystem snapshots; not globally atomic; aggregate row counts do not certify unreturned history".into()
        })
    }
    pub fn project_overview(
        &self,
        project: &str,
        at: DateTime<Utc>,
        q: &OverviewQuery,
    ) -> Result<ProjectOperationsOverview, OperationsReadError> {
        id_valid(project)?;
        q.validate(at)?;
        let db = self.database_projection(Some(project), None, at, q);
        let applications = self.references(&db, Some(project), q.limit);
        let graph = match self.graphs.as_ref() {
            SourceSection::Available(g) => match g.get(project) {
                Some(g) => SourceSection::Available(OperationalGraphSummary {
                    dgraph_id: g.dgraph_id.clone(),
                    revision: g.revision,
                    node_count: g.nodes.len(),
                    desired_service_assignment_count: g
                        .nodes
                        .iter()
                        .map(|n| n.desired_services.len())
                        .sum(),
                }),
                None => {
                    SourceSection::Indeterminate("no operational graph for this project".into())
                }
            },
            _ => self.graphs.map(|_| unreachable!()),
        };
        let accepted = match &self.deployment {
            SourceSection::Available(d) => SourceSection::from_result((|| {
                let mut views = vec![];
                let mut active_apps = BTreeSet::new();
                for a in d.active_by_application.values() {
                    if a.active_dmap.project_id == project {
                        active_apps.insert(a.application_id.clone());
                    }
                }
                for a in d.phase113_v2.active.values() {
                    if a.map.project_id == project {
                        active_apps.insert(a.map.application_id.clone());
                    }
                }
                for application_id in active_apps {
                    if let Some(authority) =
                        ddeploy::active_placement_authority(d, project, &application_id)
                            .map_err(|e| error(ReadErrorKind::SourceCorrupt, "ddeploy", e))?
                    {
                        views.push(accepted_view(&authority, q.limit));
                    }
                }
                Ok(page(views, q.limit))
            })()),
            _ => self.deployment.map(|_| unreachable!()),
        };
        Ok(ProjectOperationsOverview {
            generated_at_utc: canonical_at(at),
            project_id: project.into(),
            operational_graph: graph.scoped(Some(project), None),
            known_applications: applications,
            canonical_application_count: SourceSection::NotConfigured,
            alerts: db.alerts,
            recent_events: db.events,
            temporal_quality: self
                .temporal_views(Some(project), None, at, q.limit)
                .scoped(Some(project), None),
            accepted_deployments: accepted.scoped(Some(project), None),
        })
    }
    pub fn application_overview(
        &self,
        project: &str,
        application: &str,
        at: DateTime<Utc>,
        q: &OverviewQuery,
    ) -> Result<ApplicationOperationsOverview, OperationsReadError> {
        id_valid(project)?;
        id_valid(application)?;
        q.validate(at)?;
        let db = self.database_projection(Some(project), Some(application), at, q);
        let authority = match &self.deployment {
            SourceSection::Available(d) => SourceSection::from_result(
                ddeploy::active_placement_authority(d, project, application)
                    .map_err(|e| error(ReadErrorKind::SourceCorrupt, "ddeploy", e)),
            ),
            _ => self.deployment.map(|_| unreachable!()),
        };
        let planned = self.deployment.map(|d| {
            let mut plans = vec![];
            for p in d.proposals.values() {
                if p.proposal.project_id == project
                    && p.proposal.application_id == application
                    && p.state == ddeploy::ProposalLifecycleState::Pending
                {
                    plans.push(PlannedPlacementView {
                        proposal_id: p.proposal.proposal_id.clone(),
                        created_at_utc: p.proposal.created_at_utc.clone(),
                        lifecycle: format!("{:?}", p.state),
                        schema: p.proposal.schema.clone(),
                        dgraph_id: p.proposal.dgraph_ref.id.clone(),
                        service_nodes: p
                            .proposal
                            .placements
                            .iter()
                            .map(|p| (p.service_id.clone(), p.node_id.clone()))
                            .collect::<BTreeMap<_, _>>()
                            .into_iter()
                            .take(q.limit as usize)
                            .collect(),
                        total_service_placements: p.proposal.placements.len(),
                        placements_truncated: p.proposal.placements.len() > q.limit as usize,
                    })
                }
            }
            for p in d.phase113_v2.proposals.values() {
                if p.proposal.project_id == project
                    && p.proposal.application_id == application
                    && p.state == super::ddeploy_v2::ProposalV2State::Pending
                {
                    plans.push(PlannedPlacementView {
                        proposal_id: p.proposal.proposal_id.clone(),
                        created_at_utc: p.proposal.created_at_utc.clone(),
                        lifecycle: format!("{:?}", p.state),
                        schema: p.proposal.schema.clone(),
                        dgraph_id: p.proposal.dgraph_ref.id.clone(),
                        service_nodes: p
                            .proposal
                            .placements
                            .iter()
                            .map(|p| (p.service_id.clone(), p.node_id.clone()))
                            .collect::<BTreeMap<_, _>>()
                            .into_iter()
                            .take(q.limit as usize)
                            .collect(),
                        total_service_placements: p.proposal.placements.len(),
                        placements_truncated: p.proposal.placements.len() > q.limit as usize,
                    })
                }
            }
            plans.sort_by(|a, b| a.proposal_id.cmp(&b.proposal_id));
            page(plans, q.limit)
        });
        let desired = match &self.deployment {
            SourceSection::Available(d) => {
                if let Some(a) = d.active_for(project, application) {
                    SourceSection::Available(page(
                        a.operational_services
                            .values()
                            .map(|s| DesiredRuntimeView {
                                service_id: s.service_id.clone(),
                                runtime: s.runtime.clone(),
                                desired_state: s.desired_state.clone(),
                            })
                            .collect(),
                        q.limit,
                    ))
                } else {
                    SourceSection::Indeterminate(
                        "no accepted v1 runtime payload; accepted v2 placements remain separate"
                            .into(),
                    )
                }
            }
            _ => self.deployment.map(|_| unreachable!()),
        };
        let observed = self.dmonitor.map(|d| {
            let mut records = BTreeMap::new();
            for streams in d.subject_index.values() {
                for (key, seq) in streams {
                    let r = &d.streams[key].observations[seq];
                    if subject_scope(&r.observation.subject) == Some((project, application)) {
                        records.insert(
                            r.observation.observation_id.clone(),
                            ObservedRuntimeView {
                                observation_id: r.observation.observation_id.clone(),
                                subject: r.observation.subject.clone(),
                                observed_at_utc: r.observation.observed_at_utc.clone(),
                                received_at_utc: r.received_at_utc.clone(),
                                reported_health: r.observation.health,
                            },
                        );
                    }
                }
            }
            page(records.into_values().collect(), q.limit)
        });
        let convergence = match (&authority, &self.dmonitor) {
            (SourceSection::Available(authority), SourceSection::Available(d)) => {
                SourceSection::Available(convergence_view(
                    super::dmonitor_interpretation::evaluate_application_dserv_convergence(
                        project,
                        application,
                        project,
                        authority.as_ref(),
                        d,
                        &self.policy,
                        &canonical_at(at),
                    ),
                    q.limit,
                ))
            }
            (_, SourceSection::Corrupt(e)) => SourceSection::Corrupt(e.clone()),
            (_, SourceSection::Unavailable(e)) => SourceSection::Unavailable(e.clone()),
            (SourceSection::Corrupt(e), _) => SourceSection::Corrupt(e.clone()),
            (SourceSection::Unavailable(e), _) => SourceSection::Unavailable(e.clone()),
            _ => SourceSection::NotConfigured,
        };
        Ok(ApplicationOperationsOverview {
            generated_at_utc: canonical_at(at),
            project_id: project.into(),
            application_id: application.into(),
            canonical_application_registry: SourceSection::NotConfigured,
            accepted: authority
                .map(|a| a.as_ref().map(|a| accepted_view(a, q.limit)))
                .scoped(Some(project), Some(application)),
            planned: planned.scoped(Some(project), Some(application)),
            desired_runtime: desired.scoped(Some(project), Some(application)),
            observed_runtime: observed.scoped(Some(project), Some(application)),
            temporal_quality: self
                .temporal_views(Some(project), Some(application), at, q.limit)
                .scoped(Some(project), Some(application)),
            convergence: convergence.scoped(Some(project), Some(application)),
            reconciliation_execution: SourceSection::NotConfigured,
            alerts: db.alerts,
            recent_events: db.events,
        })
    }
}
fn accepted_view(
    a: &super::ddeploy_v2::PlacementAuthorityView,
    limit: u32,
) -> AcceptedPlacementView {
    AcceptedPlacementView {
        project_id: a.project_id.clone(),
        application_id: a.application_id.clone(),
        dmap_id: a.dmap_id.clone(),
        revision: a.revision,
        digest: a.digest.clone(),
        acceptance_id: a.acceptance_id.clone(),
        dgraph_id: a.dgraph_id.clone(),
        dgraph_revision: a.dgraph_revision,
        dgraph_digest: a.dgraph_digest.clone(),
        service_nodes: a
            .placements
            .iter()
            .map(|p| (p.service_id.clone(), p.node_id.clone()))
            .collect::<BTreeMap<_, _>>()
            .into_iter()
            .take(limit as usize)
            .collect(),
        total_service_placements: a.placements.len(),
        placements_truncated: a.placements.len() > limit as usize,
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        active_monitoring::{
            DMonitorFreshnessCheck, DMonitorStreamSelector, MonitoringCheckId,
            MonitoringCompletion, MonitoringCompletionResult, MonitoringEvaluation,
            MonitoringEvidenceSnapshot, MonitoringLastEvaluation, MonitoringOutcome,
            MonitoringSchedulerAction,
        },
        canonical_dmonitor::{
            CanonicalDMonitorObservationV1, CANONICAL_DMONITOR_OBSERVATION_SCHEMA,
        },
        datum_event::{DatumEventV1, DATUM_EVENT_SCHEMA},
        event_producers::{DMonitorProducerBinding, EventProducerPipeline, MonitoringOccurrence},
    };
    use super::*;
    use rusqlite::params;
    use serde_json::json;
    struct Db(PathBuf);
    impl Db {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("phase127-{}.db", uuid::Uuid::new_v4()));
            datum_alert_store::initialize_database(&path).unwrap();
            Self(path)
        }
        fn repo(&self) -> OperationsRepository {
            OperationsRepository::new(self.0.clone()).unwrap()
        }
        fn sources(&self) -> OperationsSources {
            OperationsSources::default().with_repository(self.repo())
        }
        fn alert(&self, e: DatumEventV1) -> DatumAlertV1 {
            let s = self.insert(e);
            datum_alert_store::process_event(&self.0, &s)
                .unwrap()
                .alert()
                .clone()
        }
        fn insert(&self, e: DatumEventV1) -> StoredDatumEvent {
            match datum_event_store::admit_event(&self.0, &e).unwrap() {
                datum_event_store::DatumEventAdmission::Inserted(s)
                | datum_event_store::DatumEventAdmission::IdempotentReplay(s) => s,
            }
        }
        fn global(&self) -> GlobalOperationsOverview {
            self.sources()
                .global_overview(at(60), &OverviewQuery::at(at(60)))
                .unwrap()
        }
    }
    impl Drop for Db {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-29T15:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
            + chrono::Duration::seconds(seconds)
    }
    fn text(seconds: i64) -> String {
        canonical_at(at(seconds))
    }
    fn available<T>(s: SourceSection<T>) -> T {
        match s {
            SourceSection::Available(v) => v,
            _ => panic!("expected available section"),
        }
    }
    fn event(
        id: &str,
        p: &str,
        app: &str,
        seconds: i64,
        severity: DatumEventSeverityV1,
    ) -> DatumEventV1 {
        DatumEventV1 {
            schema: DATUM_EVENT_SCHEMA.into(),
            event_id: id.into(),
            project_id: Some(p.into()),
            application_id: Some(app.into()),
            category: DatumEventCategoryV1::Observation,
            kind: "observation_stale".into(),
            severity,
            source: DatumEventSourceV1::ActiveMonitor,
            subject: DatumEventSubjectV1::DServ {
                project_id: p.into(),
                application_id: app.into(),
                service_id: "service".into(),
                node_id: Some("node".into()),
            },
            occurred_at_utc: text(seconds),
            evidence_refs: vec![],
            attributes: json!({}),
            correlation_hint: None,
            confidence_permille: None,
            model_ref: None,
        }
    }
    fn state(observed: i64, received: i64, application: bool) -> DMonitorStateV1 {
        let mut s = DMonitorStateV1::empty();
        dmonitor_state::ingest_observation(
            &mut s,
            "p",
            CanonicalDMonitorObservationV1 {
                schema: CANONICAL_DMONITOR_OBSERVATION_SCHEMA.into(),
                observation_id: "obs".into(),
                project_id: application.then(|| "p".into()),
                node_id: "node".into(),
                monitor_instance_id: "monitor".into(),
                sequence: 1,
                observed_at_utc: text(observed),
                cycle_id: None,
                collector: "test_collector".into(),
                subject: if application {
                    DMonitorSubjectV1::DServRealization {
                        project_id: "p".into(),
                        application_id: "app".into(),
                        service_id: "service".into(),
                        node_id: "node".into(),
                        dmap_ref: None,
                    }
                } else {
                    DMonitorSubjectV1::DNode {
                        node_id: "node".into(),
                    }
                },
                health: DMonitorHealthV1::Healthy,
                facts: json!({}),
            },
            &text(received),
        )
        .unwrap();
        s
    }
    fn expected() -> Vec<DMonitorStreamSelector> {
        vec![DMonitorStreamSelector::new("p", "node", "monitor").unwrap()]
    }
    fn temporal_one(s: DMonitorStateV1, time: i64) -> TemporalQualityView {
        let sources = OperationsSources::default()
            .with_dmonitor(s, Default::default(), expected())
            .unwrap();
        available(sources.temporal_views(None, None, at(time), 100))
            .items
            .remove(0)
    }
    fn graph(p: &str) -> OperationalDGraph {
        OperationalDGraph {
            schema_version: operational_dgraph::OPERATIONAL_DGRAPH_SCHEMA_VERSION.into(),
            dgraph_id: format!("graph-{p}"),
            revision: 1,
            project_id: p.into(),
            nodes: vec![operational_dgraph::OperationalNodeDesiredState {
                node_id: "node".into(),
                desired_services: vec![],
            }],
            metadata: None,
        }
    }
    fn deployment(accept: bool) -> DDeployStateV1 {
        use super::super::service_artifact_registry::{
            ServiceArtifact, ServiceArtifactControlPlane, ServiceProbeSet, ServiceProbeSpec,
            ServiceRuntimeSpec, SERVICE_ARTIFACT_SCHEMA_VERSION,
        };
        let dc: super::super::canonical_dcontinuum::CanonicalDContinuumV1 = serde_json::from_value(json!({
            "schema":"datum.dcontinuum/1","project_id":"p","dcontinuum_id":"continuum","revision":1,
            "nodes":[{"node_id":"node","platform":{"os":"linux","architecture":"amd64"},
                "capacity":{"cpu_cores":4,"memory_bytes":4294967296u64},
                "capabilities":{"dnode_abi_versions":["datum-dnode/0"]}}],"links":[]
        }))
        .unwrap();
        let artifact = ServiceArtifact {
            schema_version: SERVICE_ARTIFACT_SCHEMA_VERSION.into(),
            artifact_id: "artifact".into(),
            service_id: "service".into(),
            runtime: ServiceRuntimeSpec {
                kind: "container".into(),
                container: None,
                native_process: None,
            },
            probes: ServiceProbeSet {
                execution: ServiceProbeSpec {
                    kind: "none".into(),
                    params: Default::default(),
                    vantage_point: None,
                },
                health: None,
                availability: None,
            },
            metadata: None,
            control_plane: ServiceArtifactControlPlane {
                target_nodes: vec!["node".into()],
                ..Default::default()
            },
        };
        let mut s = DDeployStateV1::empty();
        ddeploy::create_proposal(&mut s,ddeploy::CreateProposalInput {
            project_id:"p".into(),application_id:"app".into(),source:ddeploy::ProposalSource::Operator,
            dgraph:serde_json::from_value(json!({"schema":"datum.dgraph/1","application_id":"app","dgraph_id":"graph","revision":1,
                "services":[{"service_id":"service","execution_requirement":{"dnode_abi":"datum-dnode/0"},
                "resource_requirement":{"cpu_cores":1,"memory_bytes":134217728}}],"calls":[]})).unwrap(),
            dcontinuum:dc.clone(),placements:serde_json::from_value(json!([{"service_id":"service","node_id":"node"}])).unwrap(),
            planner_provenance:None,dcode_artifact_authority:BTreeMap::new()
        },"proposal".into(),text(0),std::slice::from_ref(&artifact)).unwrap();
        if accept {
            ddeploy::accept_proposal(
                &mut s,
                "proposal",
                "operator",
                None,
                &[artifact],
                Some(&dc),
                &text(0),
                "acceptance".into(),
            )
            .unwrap();
        }
        s
    }
    fn app(sources: &OperationsSources) -> ApplicationOperationsOverview {
        sources
            .application_overview("p", "app", at(60), &OverviewQuery::at(at(60)))
            .unwrap()
    }
    #[test]
    fn project_graph_uses_only_accepted_snapshots_not_proposals_operational_nodes_or_events() {
        let pending = OperationsSources::default().with_deployment(deployment(false));
        let pending_view = pending.project_graph_overview("p", at(60)).unwrap();
        assert!(
            matches!(pending_view.graph, SourceSection::Available(ref g) if g.nodes.is_empty())
        );
        let state = deployment(true);
        let before = serde_json::to_value(&state).unwrap();
        let sources = OperationsSources::default()
            .with_deployment(state.clone())
            .with_project_graphs(BTreeMap::from([("p".into(), graph("p"))]));
        let view = sources.project_graph_overview("p", at(60)).unwrap();
        assert_eq!(view.source_kind, "accepted_application_dgraph_snapshots");
        assert_eq!(view.generated_at_utc, text(60));
        let SourceSection::Available(content) = view.graph else {
            panic!("expected accepted graph");
        };
        assert_eq!(content.applications[0].application_id, "app");
        assert_eq!(content.applications[0].dgraph_id, "graph");
        assert_eq!(content.nodes.len(), 2);
        assert!(content.nodes.iter().all(|n| n.canonical_id != "node"));
        assert_eq!(serde_json::to_value(&state).unwrap(), before);
        assert!(
            matches!(sources.project_graph_overview("other",at(60)).unwrap().graph,SourceSection::Available(ref g) if g.nodes.is_empty())
        );
    }
    #[test]
    fn project_graph_never_falls_back_to_operational_document_and_preserves_source_failure() {
        let sources = OperationsSources::default()
            .with_project_graphs(BTreeMap::from([("p".into(), graph("p"))]));
        assert!(matches!(
            sources.project_graph_overview("p", at(60)).unwrap().graph,
            SourceSection::NotConfigured
        ));
        let sources = sources.with_source_error(
            OperationsSource::Deployment,
            error(
                ReadErrorKind::SourceCorrupt,
                "ddeploy",
                "private SQL C:\\secrets.db",
            ),
        );
        let json =
            serde_json::to_string(&sources.project_graph_overview("p", at(60)).unwrap()).unwrap();
        assert!(json.contains("corrupt"));
        assert!(!json.contains("secrets.db"));
    }
    fn scheduler() -> MonitoringSchedulerStatus {
        let check = |id: &str| MonitoringCheckStatus {
            check_id: MonitoringCheckId::new(id).unwrap(),
            enabled: true,
            cadence: Duration::from_secs(30),
            running: false,
            last_started_at_utc: Some(text(0)),
            last_completed_at_utc: Some(text(1)),
            last_evaluation_result: Some(MonitoringLastEvaluation::Completed {
                outcome: MonitoringOutcome::Satisfied,
                finding_count: 0,
            }),
            last_scheduler_action: Some(MonitoringSchedulerAction::DeferredAtCapacity),
            next_due_schedule_time: Some(Duration::from_secs(30)),
        };
        MonitoringSchedulerStatus {
            lifecycle: MonitoringLifecycle::Stopped,
            checks: vec![check("z_check"), check("a_check")],
            completed_count: 2,
            dropped_output_full: 3,
            dropped_output_closed: 4,
            driver_polls: 5,
        }
    }
    fn monitoring(
        pipeline: &EventProducerPipeline,
        s: DMonitorStateV1,
        seconds: i64,
        id: &str,
    ) -> super::super::event_producers::MonitoringProduction {
        let snapshot = MonitoringEvidenceSnapshot::from_state(s).unwrap();
        let selector = expected().remove(0);
        let id_check = selector.check_id().unwrap();
        let check = DMonitorFreshnessCheck::new(
            std::sync::Arc::new(snapshot.clone()),
            selector.clone(),
            Default::default(),
        )
        .unwrap();
        let result = check.evaluate_snapshot(&snapshot, at(seconds)).unwrap();
        pipeline
            .monitoring(
                &MonitoringOccurrence {
                    occurrence_id: id.into(),
                    completion: MonitoringCompletion {
                        check_id: id_check.clone(),
                        evaluated_at_utc: text(seconds),
                        completed_at_utc: text(seconds),
                        result: MonitoringCompletionResult::Evaluation(MonitoringEvaluation {
                            check_id: id_check,
                            evaluated_at_utc: text(seconds),
                            outcome: result.outcome,
                            findings: result.findings,
                        }),
                    },
                },
                &snapshot,
                &DMonitorProducerBinding::new(selector, Default::default()).unwrap(),
            )
            .unwrap()
    }
    fn recovered() -> (Db, String) {
        let db = Db::new();
        let pipeline = EventProducerPipeline::open(db.0.clone()).unwrap();
        let opened = monitoring(&pipeline, state(-301, -301, false), 0, "active");
        let id = opened.events[0]
            .alert
            .as_ref()
            .unwrap()
            .alert()
            .alert_id
            .clone();
        monitoring(&pipeline, state(0, 0, false), 1, "clear");
        (db, id)
    }

    #[test]
    fn empty_dserver_has_explicit_not_configured_sections() {
        let v = OperationsSources::default()
            .global_overview(at(0), &OverviewQuery::at(at(0)))
            .unwrap();
        assert!(v.known_projects.page.items.is_empty());
        assert!(matches!(v.alerts, SourceSection::NotConfigured));
        assert!(matches!(
            v.structural_node_count,
            SourceSection::NotConfigured
        ));
        assert!(!available(v.scheduler).configured);
    }
    #[test]
    fn initialized_empty_store_counts_are_real_zeros() {
        let db = Db::new();
        let v = db.global();
        assert_eq!(available(v.recent_event_count), 0);
        assert_eq!(available(v.alerts).counts, AlertCounts::default());
    }
    #[test]
    fn multiple_projects_are_deterministic() {
        let sources = OperationsSources::default().with_project_graphs(BTreeMap::from([
            ("z".into(), graph("z")),
            ("a".into(), graph("a")),
        ]));
        let v = sources
            .global_overview(at(0), &OverviewQuery::at(at(0)))
            .unwrap();
        assert_eq!(v.known_projects.page.items, vec!["a", "z"]);
        assert_eq!(available(v.managed_operational_project_count), 2);
    }
    #[test]
    fn active_resolved_closed_counts_are_distinct() {
        let db = Db::new();
        let a = db.alert(event("a", "p", "one", 0, DatumEventSeverityV1::Critical));
        let b = db.alert(event("b", "p", "two", 0, DatumEventSeverityV1::Warning));
        let c = db.alert(event("c", "p", "three", 0, DatumEventSeverityV1::Info));
        let d = db.alert(event("d", "p", "four", 0, DatumEventSeverityV1::Info));
        let e = db.alert(event("e", "p", "five", 0, DatumEventSeverityV1::Info));
        datum_alert_store::acknowledge_alert(&db.0, &d.alert_id, DatumAlertActorV1::system(), None)
            .unwrap();
        datum_alert_store::acknowledge_alert(&db.0, &e.alert_id, DatumAlertActorV1::system(), None)
            .unwrap();
        datum_alert_store::start_alert_work(&db.0, &e.alert_id, DatumAlertActorV1::system(), None)
            .unwrap();
        for id in [&b.alert_id, &c.alert_id] {
            datum_alert_store::resolve_alert(&db.0, id, DatumAlertActorV1::system(), None).unwrap();
        }
        datum_alert_store::close_alert(&db.0, &c.alert_id, DatumAlertActorV1::system(), None)
            .unwrap();
        let v = available(db.global().alerts);
        assert_eq!(
            (
                v.counts.open,
                v.counts.acknowledged,
                v.counts.in_progress,
                v.counts.resolved_pending_close,
                v.counts.historical_closed
            ),
            (1, 1, 1, 1, 1)
        );
        assert!(v.attention.items.iter().any(|i| i.reference == a.alert_id));
    }
    #[test]
    fn critical_attention_references_existing_alert() {
        let db = Db::new();
        let a = db.alert(event("a", "p", "app", 0, DatumEventSeverityV1::Critical));
        let v = available(db.global().alerts);
        assert_eq!(v.counts.critical_attention, 1);
        assert_eq!(v.attention.items[0].reference, a.alert_id);
        assert_eq!(v.attention.items[0].source, "datum_alert");
    }
    #[test]
    fn resolved_is_not_operationally_active() {
        let db = Db::new();
        let a = db.alert(event("a", "p", "app", 0, DatumEventSeverityV1::Warning));
        datum_alert_store::resolve_alert(&db.0, &a.alert_id, DatumAlertActorV1::system(), None)
            .unwrap();
        let v = db.repo().alert_view(&a.alert_id, at(1)).unwrap().unwrap();
        assert!(!v.is_operationally_active);
        assert!(v.is_resolved_pending_close);
        assert!(available(db.global().alerts).attention.items.is_empty());
    }
    #[test]
    fn closed_is_historical_only() {
        let db = Db::new();
        let a = db.alert(event("a", "p", "app", 0, DatumEventSeverityV1::Warning));
        datum_alert_store::resolve_alert(&db.0, &a.alert_id, DatumAlertActorV1::system(), None)
            .unwrap();
        datum_alert_store::close_alert(&db.0, &a.alert_id, DatumAlertActorV1::system(), None)
            .unwrap();
        let v = db.repo().alert_view(&a.alert_id, at(1)).unwrap().unwrap();
        assert!(v.is_historical_closed);
        assert!(!v.is_resolved_pending_close);
        assert!(!v.is_operationally_active);
    }
    #[test]
    fn unavailable_store_is_not_zero_or_created() {
        let path =
            std::env::temp_dir().join(format!("missing-phase127-{}.db", uuid::Uuid::new_v4()));
        let sources = OperationsSources::default()
            .with_repository(OperationsRepository::new(path.clone()).unwrap());
        let v = sources
            .global_overview(at(0), &OverviewQuery::at(at(0)))
            .unwrap();
        assert!(matches!(
            v.recent_event_count,
            SourceSection::Unavailable(_)
        ));
        assert!(!path.exists());
    }
    #[test]
    fn corrupt_dmonitor_is_explicit_but_other_sources_render() {
        let db = Db::new();
        let mut s = state(0, 0, false);
        s.observation_index.clear();
        let sources = db
            .sources()
            .with_dmonitor(s, Default::default(), expected())
            .unwrap();
        let v = sources
            .global_overview(at(1), &OverviewQuery::at(at(1)))
            .unwrap();
        assert!(matches!(v.temporal_quality, SourceSection::Corrupt(_)));
        assert!(matches!(v.alerts, SourceSection::Available(_)));
        assert!(v
            .source_issues
            .iter()
            .any(|e| e.kind == ReadErrorKind::SourceCorrupt));
    }
    #[test]
    fn attention_order_is_lifecycle_severity_recency_identity() {
        let db = Db::new();
        let resolved = db.alert(event("r", "p", "r", 20, DatumEventSeverityV1::Critical));
        datum_alert_store::resolve_alert(
            &db.0,
            &resolved.alert_id,
            DatumAlertActorV1::system(),
            None,
        )
        .unwrap();
        let warn = db.alert(event("w", "p", "w", 30, DatumEventSeverityV1::Warning));
        let old = db.alert(event("o", "p", "o", 0, DatumEventSeverityV1::Critical));
        let new = db.alert(event("n", "p", "n", 10, DatumEventSeverityV1::Critical));
        let v = available(db.global().alerts);
        assert_eq!(
            v.attention_ordered
                .items
                .iter()
                .map(|a| a.alert_id.clone())
                .collect::<Vec<_>>(),
            vec![new.alert_id, old.alert_id, warn.alert_id, resolved.alert_id]
        );
    }
    #[test]
    fn confirmation_count_is_not_outages() {
        let db = Db::new();
        let a = db.alert(event("a", "p", "app", 0, DatumEventSeverityV1::Warning));
        db.alert(event("b", "p", "app", 1, DatumEventSeverityV1::Warning));
        let v = db.repo().alert_view(&a.alert_id, at(2)).unwrap().unwrap();
        assert_eq!(v.condition_confirmation_count, 2);
        assert!(v.count_semantics.contains("not independent outages"));
    }
    #[test]
    fn recovery_event_is_visible_in_timeline() {
        let (db, id) = recovered();
        let timeline = db.repo().alert_timeline(&id, 100).unwrap();
        assert_eq!(timeline.items.len(), 2);
        assert_eq!(
            timeline.items[1].classification,
            EventPresentation::Recovery
        );
        let v = db.repo().alert_view(&id, at(2)).unwrap().unwrap();
        assert_eq!(v.condition_cleared_at_utc, Some(text(1)));
        assert_eq!(v.condition_duration_seconds, Some(1));
        assert!(v.recovery_event.is_some());
    }
    #[test]
    fn timeline_is_occurrence_not_association_order() {
        let db = Db::new();
        let a = db.alert(event("late", "p", "app", 2, DatumEventSeverityV1::Critical));
        db.alert(event("early", "p", "app", 0, DatumEventSeverityV1::Warning));
        let mut fractional = event("fraction", "p", "app", 0, DatumEventSeverityV1::Warning);
        fractional.occurred_at_utc = "2026-09-29T15:00:00.100Z".into();
        db.alert(fractional);
        let v = db.repo().alert_timeline(&a.alert_id, 100).unwrap();
        assert_eq!(
            v.items
                .iter()
                .map(|e| e.event_id.as_str())
                .collect::<Vec<_>>(),
            vec!["early", "fraction", "late"]
        );
    }
    #[test]
    fn equal_times_have_event_id_tie_break() {
        let db = Db::new();
        let a = db.alert(event("z", "p", "app", 0, DatumEventSeverityV1::Warning));
        db.alert(event("a", "p", "app", 0, DatumEventSeverityV1::Warning));
        let v = db.repo().alert_timeline(&a.alert_id, 100).unwrap();
        assert_eq!(
            v.items
                .iter()
                .map(|e| e.event_id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "z"]
        );
        let recent = db.repo().event_history(&Default::default()).unwrap();
        assert_eq!(recent.items[0].event_id, "z");
    }
    #[test]
    fn current_and_peak_severity_remain_distinct() {
        let db = Db::new();
        let a = db.alert(event(
            "early-critical",
            "p",
            "app",
            0,
            DatumEventSeverityV1::Critical,
        ));
        db.alert(event(
            "late-warning",
            "p",
            "app",
            2,
            DatumEventSeverityV1::Warning,
        ));
        let v = db.repo().alert_view(&a.alert_id, at(3)).unwrap().unwrap();
        assert_eq!(v.severity, DatumEventSeverityV1::Warning);
        assert_eq!(v.peak_severity, DatumEventSeverityV1::Critical);
    }
    #[test]
    fn automatic_resolution_is_pending_closure_not_active() {
        let (db, id) = recovered();
        let v = db.repo().alert_view(&id, at(2)).unwrap().unwrap();
        assert_eq!(v.group, AlertPresentationGroup::PendingClosure);
        assert!(v.is_resolved_pending_close);
        assert!(!v.is_historical_closed);
        assert_eq!(v.resolution.actor.unwrap().actor_type, "system");
    }
    #[test]
    fn recent_receipt_old_source_is_fresh_communication_not_down() {
        let v = temporal_one(state(-3600, 0, false), 1);
        assert_eq!(v.communication_freshness, CommunicationFreshness::Fresh);
        assert_eq!(v.source_time_quality, SourceTimeQuality::BehindOrDelayed);
        assert_eq!(v.observed_receipt_delta_seconds, Some(-3600));
        let json = serde_json::to_string(&v).unwrap();
        assert!(!json.contains("down"));
        assert!(!json.contains("offline"));
    }
    #[test]
    fn stale_receipt_is_communication_stale() {
        let v = temporal_one(state(-301, -301, false), 0);
        assert_eq!(v.communication_freshness, CommunicationFreshness::Stale);
    }
    #[test]
    fn source_ahead_is_clock_quality_not_communication_failure() {
        let v = temporal_one(state(3600, 0, false), 1);
        assert_eq!(v.communication_freshness, CommunicationFreshness::Fresh);
        assert_eq!(v.source_time_quality, SourceTimeQuality::SourceAhead);
        assert_eq!(v.observed_receipt_delta_seconds, Some(3600));
    }
    #[test]
    fn expected_missing_stream_is_missing_not_down() {
        let v = temporal_one(DMonitorStateV1::empty(), 0);
        assert_eq!(
            v.communication_freshness,
            CommunicationFreshness::MissingEvidence
        );
        assert_eq!(v.source_time_quality, SourceTimeQuality::MissingEvidence);
        assert!(v.observation_id.is_none());
    }
    #[test]
    fn evaluation_before_receipt_is_ordering_anomaly() {
        let v = temporal_one(state(1, 1, false), 0);
        assert_eq!(
            v.communication_freshness,
            CommunicationFreshness::Indeterminate
        );
        assert_eq!(
            v.source_time_quality,
            SourceTimeQuality::EvaluationOrderingAnomaly
        );
    }
    #[test]
    fn recovery_updates_current_temporal_projection() {
        let (db, _) = recovered();
        let sources = db
            .sources()
            .with_dmonitor(state(0, 0, false), Default::default(), expected())
            .unwrap();
        let v = sources
            .global_overview(at(1), &OverviewQuery::at(at(1)))
            .unwrap();
        let temporal = available(v.temporal_quality);
        assert_eq!(
            temporal.items[0].source_time_quality,
            SourceTimeQuality::AlignedOrNoDiscrepancy
        );
        assert_eq!(
            temporal.items[0].communication_freshness,
            CommunicationFreshness::Fresh
        );
        assert_eq!(available(v.alerts).counts.resolved_pending_close, 1);
    }
    #[test]
    fn event_project_filter_is_scoped() {
        let db = Db::new();
        db.insert(event("p", "p", "app", 0, DatumEventSeverityV1::Info));
        db.insert(event("q", "q", "app", 0, DatumEventSeverityV1::Info));
        let q = EventHistoryQuery {
            project_id: Some("p".into()),
            ..Default::default()
        };
        let v = db.repo().event_history(&q).unwrap();
        assert_eq!(v.items.len(), 1);
        assert_eq!(v.items[0].event_id, "p");
        // Canonical entity identities are not the stricter check-ID token type.
        db.insert(event(
            "padded-scope",
            " p ",
            "app",
            0,
            DatumEventSeverityV1::Info,
        ));
        let exact = db
            .repo()
            .event_history(&EventHistoryQuery {
                project_id: Some(" p ".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(exact.items.len(), 1);
        assert_eq!(exact.items[0].event_id, "padded-scope");
    }
    #[test]
    fn event_application_filter_is_scoped() {
        let db = Db::new();
        db.insert(event("a", "p", "a", 0, DatumEventSeverityV1::Info));
        db.insert(event("b", "p", "b", 0, DatumEventSeverityV1::Info));
        let q = EventHistoryQuery {
            project_id: Some("p".into()),
            application_id: Some("b".into()),
            ..Default::default()
        };
        assert_eq!(db.repo().event_history(&q).unwrap().items[0].event_id, "b");
    }
    #[test]
    fn event_kind_category_source_severity_filters_intersect() {
        let db = Db::new();
        db.insert(event("yes", "p", "app", 0, DatumEventSeverityV1::Warning));
        db.insert(event("no", "p", "app", 0, DatumEventSeverityV1::Info));
        let q = EventHistoryQuery {
            kind: Some("observation_stale".into()),
            category: Some(DatumEventCategoryV1::Observation),
            source: Some(DatumEventSourceV1::ActiveMonitor),
            severity: Some(DatumEventSeverityV1::Warning),
            ..Default::default()
        };
        assert_eq!(
            db.repo().event_history(&q).unwrap().items[0].event_id,
            "yes"
        );
    }
    #[test]
    fn every_query_limit_is_strictly_bounded() {
        let db = Db::new();
        for limit in [0, 1001, u32::MAX] {
            assert_eq!(
                db.repo()
                    .event_history(&EventHistoryQuery {
                        limit,
                        ..Default::default()
                    })
                    .unwrap_err()
                    .kind,
                ReadErrorKind::InvalidQuery
            );
            assert!(db
                .sources()
                .global_overview(
                    at(0),
                    &OverviewQuery {
                        limit,
                        recent_from_utc: text(-1)
                    }
                )
                .is_err());
        }
        db.insert(event("a", "p", "app", 0, DatumEventSeverityV1::Info));
        db.insert(event("b", "p", "app", 1, DatumEventSeverityV1::Info));
        let v = db
            .repo()
            .event_history(&EventHistoryQuery {
                limit: 1,
                ..Default::default()
            })
            .unwrap();
        assert!(v.has_more);
        assert_eq!(v.items.len(), 1);
    }
    #[test]
    fn time_ranges_use_numeric_chronology_and_half_open_bounds() {
        let db = Db::new();
        db.insert(event("whole", "p", "app", 0, DatumEventSeverityV1::Info));
        let mut fraction = event("fraction", "p", "app", 0, DatumEventSeverityV1::Info);
        fraction.occurred_at_utc = "2026-09-29T15:00:00.100Z".into();
        db.insert(fraction);
        let q = EventHistoryQuery {
            occurred_from_utc: Some(text(0)),
            occurred_to_utc: Some("2026-09-29T15:00:00.100Z".into()),
            ..Default::default()
        };
        let v = db.repo().event_history(&q).unwrap();
        assert_eq!(v.items.len(), 1);
        assert_eq!(v.items[0].event_id, "whole");
    }
    #[test]
    fn control_plane_event_is_historical_only() {
        let db = Db::new();
        let s = deployment(true);
        let a = s.acceptances.values().next().unwrap();
        let e = super::super::event_producers::acceptance_event_v1(a).unwrap();
        EventProducerPipeline::open(db.0.clone())
            .unwrap()
            .record(e)
            .unwrap();
        let v = db.repo().event_history(&Default::default()).unwrap();
        assert_eq!(v.items[0].classification, EventPresentation::ControlPlane);
        assert!(v.items[0].associated_alerts.items.is_empty());
        assert_eq!(available(db.global().alerts).counts.open, 0);
    }
    #[test]
    fn recovery_event_has_separate_presentation_and_admission_time() {
        let (db, _) = recovered();
        let v = db
            .repo()
            .event_history(&EventHistoryQuery {
                kind: Some("observation_stale_cleared".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(v.items[0].classification, EventPresentation::Recovery);
        assert!(v.items[0].associated_alerts.items.is_empty());
        assert!(parsed(&v.items[0].admitted_at_utc).is_ok());
        assert!(matches!(
            v.items[0].temporal_context,
            TemporalMetadata::Available(_)
        ));
    }
    #[test]
    fn one_event_can_reference_multiple_valid_alert_episodes() {
        let db = Db::new();
        let a = db.alert(event(
            "shared",
            "p",
            "app",
            0,
            DatumEventSeverityV1::Warning,
        ));
        datum_alert_store::resolve_alert(&db.0, &a.alert_id, DatumAlertActorV1::system(), None)
            .unwrap();
        datum_alert_store::close_alert(&db.0, &a.alert_id, DatumAlertActorV1::system(), None)
            .unwrap();
        let c = Connection::open(&db.0).unwrap();
        let columns = |table: &str| {
            let mut stmt = c.prepare(&format!("PRAGMA table_info({table})")).unwrap();
            stmt.query_map([], |r| r.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
                .into_iter()
                .filter(|n| n != "id")
                .collect::<Vec<_>>()
        };
        for table in ["datum_alerts", "datum_alert_transitions"] {
            let cols = columns(table);
            let selects = cols
                .iter()
                .map(|n| match n.as_str() {
                    "alert_id" => "?1".into(),
                    "transition_id" => "'copy-' || transition_id".into(),
                    _ => n.clone(),
                })
                .collect::<Vec<String>>();
            let statement = format!(
                "INSERT INTO {table} ({}) SELECT {} FROM {table} WHERE alert_id=?2",
                cols.join(","),
                selects.join(",")
            );
            c.execute(&statement, params!["copied-closed-episode", a.alert_id])
                .unwrap();
        }
        c.execute(
            "INSERT INTO datum_alert_events(alert_id,event_id,associated_at_utc) VALUES(?1,?2,?3)",
            params!["copied-closed-episode", "shared", text(0)],
        )
        .unwrap();
        let v = db.repo().event_history(&Default::default()).unwrap();
        assert_eq!(v.items[0].associated_alerts.items.len(), 2);
        assert!(v.items[0]
            .associated_alerts
            .items
            .windows(2)
            .all(|w| w[0] < w[1]));
    }
    #[test]
    fn unwired_scheduler_is_not_reported_failed() {
        let v = available(scheduler_view(&SourceSection::NotConfigured, at(0), 100));
        assert!(!v.configured);
        assert!(v.lifecycle.is_none());
        assert!(v.dropped_output_full.is_none());
    }
    #[test]
    fn scheduler_check_status_is_sorted() {
        let v = available(scheduler_view(
            &SourceSection::Available(scheduler()),
            at(0),
            100,
        ));
        assert_eq!(
            v.checks.items[0].check_id,
            MonitoringCheckId::new("a_check").unwrap()
        );
        assert_eq!(v.lifecycle, Some(MonitoringLifecycle::Stopped));
    }
    #[test]
    fn scheduler_output_loss_is_preserved() {
        let v = available(scheduler_view(
            &SourceSection::Available(scheduler()),
            at(0),
            100,
        ));
        assert_eq!(
            (v.dropped_output_full, v.dropped_output_closed),
            (Some(3), Some(4))
        );
    }
    #[test]
    fn scheduler_action_does_not_erase_last_evaluation() {
        let v = available(scheduler_view(
            &SourceSection::Available(scheduler()),
            at(0),
            100,
        ));
        assert_eq!(
            v.checks.items[0].last_scheduler_action,
            Some(MonitoringSchedulerAction::DeferredAtCapacity)
        );
        assert!(matches!(
            v.checks.items[0].last_evaluation_result,
            Some(MonitoringLastEvaluation::Completed {
                outcome: MonitoringOutcome::Satisfied,
                ..
            })
        ));
    }
    #[test]
    fn pending_proposal_is_never_accepted_placement() {
        let s = OperationsSources::default().with_deployment(deployment(false));
        let v = app(&s);
        assert!(available(v.accepted).is_none());
        assert_eq!(available(v.planned).items.len(), 1);
        assert!(matches!(v.desired_runtime, SourceSection::Indeterminate(_)));
    }
    #[test]
    fn desired_and_observed_runtime_are_separate() {
        let s = OperationsSources::default()
            .with_deployment(deployment(true))
            .with_dmonitor(state(0, 0, true), Default::default(), expected())
            .unwrap();
        let v = app(&s);
        assert!(available(v.accepted).is_some());
        assert_eq!(available(v.desired_runtime).items.len(), 1);
        let observed = available(v.observed_runtime);
        assert_eq!(observed.items.len(), 1);
        assert_eq!(observed.items[0].reported_health, DMonitorHealthV1::Healthy);
        assert!(matches!(
            v.reconciliation_execution,
            SourceSection::NotConfigured
        ));
    }
    #[test]
    fn alerts_cannot_become_application_runtime_authority() {
        let db = Db::new();
        db.alert(event(
            "critical",
            "p",
            "app",
            0,
            DatumEventSeverityV1::Critical,
        ));
        let v = app(&db.sources());
        assert!(matches!(v.accepted, SourceSection::NotConfigured));
        assert!(matches!(v.desired_runtime, SourceSection::NotConfigured));
        assert!(matches!(v.observed_runtime, SourceSection::NotConfigured));
        assert_eq!(available(v.alerts).counts.critical_attention, 1);
    }
    #[test]
    fn discovered_applications_are_not_canonical_registry() {
        let db = Db::new();
        db.insert(event(
            "reference",
            "p",
            "app",
            0,
            DatumEventSeverityV1::Info,
        ));
        let v = db.global();
        assert!(matches!(
            v.canonical_application_count,
            SourceSection::NotConfigured
        ));
        assert_eq!(v.known_applications.page.items[0].provenance, vec!["event"]);
        assert!(matches!(
            app(&db.sources()).canonical_application_registry,
            SourceSection::NotConfigured
        ));
    }
    #[test]
    fn common_generation_time_reaches_all_top_level_views() {
        let db = Db::new();
        let s = db
            .sources()
            .with_scheduler(Ok(scheduler()))
            .with_dmonitor(state(0, 0, false), Default::default(), expected())
            .unwrap();
        let q = OverviewQuery::at(at(60));
        let v = s.global_overview(at(60), &q).unwrap();
        assert_eq!(v.generated_at_utc, text(60));
        assert_eq!(available(v.scheduler).generated_at_utc, text(60));
        assert_eq!(
            available(v.temporal_quality).items[0].evaluated_at_utc,
            text(60)
        );
        assert_eq!(
            s.project_overview("p", at(60), &q)
                .unwrap()
                .generated_at_utc,
            text(60)
        );
        assert_eq!(
            s.application_overview("p", "app", at(60), &q)
                .unwrap()
                .generated_at_utc,
            text(60)
        );
    }
    #[test]
    fn fixed_time_and_snapshots_are_field_equivalent() {
        let db = Db::new();
        db.alert(event("a", "p", "app", 0, DatumEventSeverityV1::Warning));
        let s = db
            .sources()
            .with_dmonitor(state(0, 0, false), Default::default(), expected())
            .unwrap();
        let q = OverviewQuery::at(at(60));
        let first = s.global_overview(at(60), &q).unwrap();
        let second = s.global_overview(at(60), &q).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            serde_json::to_vec(&first).unwrap(),
            serde_json::to_vec(&second).unwrap()
        );
    }
    #[test]
    fn source_age_uses_one_supplied_generation_time() {
        let v = temporal_one(state(-10, -5, false), 60);
        assert_eq!(v.observed_age_seconds, Some(70));
        assert_eq!(v.received_age_seconds, Some(65));
        assert_eq!(v.evaluated_at_utc, text(60));
    }

    #[test]
    fn keyset_pagination_has_no_equal_time_duplicates() {
        let db = Db::new();
        for id in ["a", "b", "c"] {
            db.insert(event(id, "p", "app", 0, DatumEventSeverityV1::Info));
        }
        let first = db
            .repo()
            .event_history(&EventHistoryQuery {
                limit: 2,
                ..Default::default()
            })
            .unwrap();
        let last = first.items.last().unwrap();
        let second = db
            .repo()
            .event_history(&EventHistoryQuery {
                limit: 2,
                before: Some(EventCursor {
                    occurred_at_utc: last.occurred_at_utc.clone(),
                    event_id: last.event_id.clone(),
                }),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            first
                .items
                .iter()
                .map(|e| e.event_id.as_str())
                .collect::<Vec<_>>(),
            vec!["c", "b"]
        );
        assert_eq!(second.items[0].event_id, "a");
        assert!(!second.has_more);
    }
    #[test]
    fn subject_filter_is_semantic_not_json_field_order() {
        let db = Db::new();
        let e = event("match", "p", "app", 0, DatumEventSeverityV1::Info);
        let subject = e.subject.clone();
        db.insert(e);
        let c = Connection::open(&db.0).unwrap();
        let mut raw: Value = serde_json::from_str(
            &c.query_row(
                "SELECT event_json FROM datum_events WHERE event_id='match'",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap(),
        )
        .unwrap();
        let subject_json = serde_json::to_string(&raw["subject"]).unwrap();
        let changed = subject_json
            .replace("\"application_id\":\"app\",", "")
            .replace("}", ",\"application_id\":\"app\"}");
        raw["subject"] = serde_json::from_str(&changed).unwrap();
        c.execute(
            "UPDATE datum_events SET event_json=? WHERE event_id='match'",
            [raw.to_string()],
        )
        .unwrap();
        let v = db
            .repo()
            .event_history(&EventHistoryQuery {
                subject: Some(subject),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(v.items.len(), 1);
    }
    #[test]
    fn corrupt_materialized_alert_fails_closed_not_zero() {
        let db = Db::new();
        let a = db.alert(event("a", "p", "app", 0, DatumEventSeverityV1::Warning));
        Connection::open(&db.0)
            .unwrap()
            .execute(
                "UPDATE datum_alerts SET occurrence_count=99 WHERE alert_id=?",
                [a.alert_id],
            )
            .unwrap();
        let v = db.global();
        assert!(matches!(v.alerts, SourceSection::Corrupt(_)));
        assert!(!v.source_issues.is_empty());
    }
    #[test]
    fn corrupted_event_is_not_hidden_by_counts() {
        let db = Db::new();
        db.insert(event("a", "p", "app", 0, DatumEventSeverityV1::Info));
        Connection::open(&db.0)
            .unwrap()
            .execute(
                "UPDATE datum_events SET event_digest='bad' WHERE event_id='a'",
                [],
            )
            .unwrap();
        let v = db.global();
        assert!(matches!(v.recent_events, SourceSection::Corrupt(_)));
        assert!(matches!(v.recent_event_count, SourceSection::Corrupt(_)));
    }
    #[test]
    fn invalid_source_snapshot_never_becomes_accepted() {
        let mut d = DDeployStateV1::empty();
        d.state_version = "invalid".into();
        let v = app(&OperationsSources::default().with_deployment(d));
        assert!(matches!(v.accepted, SourceSection::Corrupt(_)));
    }
    #[test]
    fn projection_does_not_mutate_evidence_or_authority() {
        let db = Db::new();
        let d = deployment(true);
        let evidence = state(0, 0, true);
        let original_d = d.clone();
        let original_e = evidence.clone();
        let s = db
            .sources()
            .with_deployment(d.clone())
            .with_dmonitor(evidence.clone(), Default::default(), expected())
            .unwrap();
        let _ = app(&s);
        assert_eq!(d, original_d);
        assert_eq!(evidence, original_e);
        assert_eq!(
            datum_event_store::list_events(&db.0, &Default::default())
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            datum_alert_store::list_alerts(&db.0, &Default::default())
                .unwrap()
                .len(),
            0
        );
    }
    #[test]
    fn reverse_association_index_is_used() {
        let db = Db::new();
        let c = Connection::open(&db.0).unwrap();
        let plan:String=c.query_row("EXPLAIN QUERY PLAN SELECT alert_id FROM datum_alert_events WHERE event_id=? ORDER BY alert_id LIMIT ?",params!["event",100],|r|r.get(3)).unwrap();
        assert!(plan.contains("idx_datum_alert_events_event"), "{plan}");
    }
    #[test]
    fn exact_freshness_boundary_reuses_existing_policy() {
        let equality = temporal_one(state(-300, -300, false), 0);
        let beyond = temporal_one(state(-301, -301, false), 0);
        assert_eq!(
            equality.communication_freshness,
            CommunicationFreshness::Fresh
        );
        assert_eq!(
            beyond.communication_freshness,
            CommunicationFreshness::Stale
        );
        assert_eq!(
            equality.existing_dmonitor_freshness,
            Some(dmonitor_freshness::DMonitorFreshnessV1::Fresh)
        );
    }
    #[test]
    fn application_temporal_uses_subject_head_when_global_head_is_another_app() {
        let mut evidence = state(0, 0, true);
        let mut next = evidence
            .stream_head("p", "node", "monitor")
            .unwrap()
            .observation
            .clone();
        next.observation_id = "other-observation".into();
        next.sequence = 2;
        next.observed_at_utc = text(1);
        if let DMonitorSubjectV1::DServRealization { application_id, .. } = &mut next.subject {
            *application_id = "other-app".into();
        }
        dmonitor_state::ingest_observation(&mut evidence, "p", next, &text(1)).unwrap();
        let sources = OperationsSources::default()
            .with_dmonitor(evidence, Default::default(), expected())
            .unwrap();
        let v = app(&sources);
        let temporal = available(v.temporal_quality);
        assert_eq!(temporal.items.len(), 1);
        assert_eq!(temporal.items[0].observation_id, Some("obs".into()));
        assert_eq!(available(v.observed_runtime).items[0].observation_id, "obs");
    }
    #[test]
    fn acquisition_failure_is_unavailable_not_unconfigured() {
        let sources = OperationsSources::default().with_source_error(
            OperationsSource::DMonitor,
            error(
                ReadErrorKind::SourceUnavailable,
                "dmonitor",
                "snapshot acquisition failed",
            ),
        );
        let v = sources
            .global_overview(at(0), &OverviewQuery::at(at(0)))
            .unwrap();
        assert!(matches!(v.temporal_quality, SourceSection::Unavailable(_)));
        assert_eq!(v.source_issues.len(), 1);
        let sources = OperationsSources::default()
            .with_source_error(
                OperationsSource::Deployment,
                error(
                    ReadErrorKind::SourceUnavailable,
                    "ddeploy",
                    "snapshot acquisition failed",
                ),
            )
            .with_dmonitor(state(0, 0, true), Default::default(), expected())
            .unwrap();
        let application = app(&sources);
        assert!(matches!(
            application.accepted,
            SourceSection::Unavailable(_)
        ));
        assert!(matches!(
            application.convergence,
            SourceSection::Unavailable(_)
        ));
    }
    #[test]
    fn presentation_history_has_generation_and_cursor() {
        let db = Db::new();
        db.insert(event("a", "p", "app", 0, DatumEventSeverityV1::Info));
        db.insert(event("b", "p", "app", 1, DatumEventSeverityV1::Info));
        let v = db
            .repo()
            .event_history_at(
                &EventHistoryQuery {
                    limit: 1,
                    ..Default::default()
                },
                at(60),
            )
            .unwrap();
        assert_eq!(v.generated_at_utc, text(60));
        assert_eq!(v.next_cursor.unwrap().event_id, "b");
    }
    #[test]
    fn accepted_proposals_are_not_presented_as_pending_plans() {
        let sources = OperationsSources::default().with_deployment(deployment(true));
        let v = app(&sources);
        assert!(available(v.planned).items.is_empty());
        let accepted = available(v.accepted).unwrap();
        assert_eq!(accepted.service_nodes.get("service").unwrap(), "node");
        assert_eq!(accepted.total_service_placements, 1);
        let project = sources
            .project_overview("p", at(60), &OverviewQuery::at(at(60)))
            .unwrap();
        let accepted = available(project.accepted_deployments);
        assert_eq!(accepted.items[0].project_id, "p");
        assert_eq!(accepted.items[0].application_id, "app");
        assert!(accepted.items[0].dgraph_digest.is_some());
    }
    #[test]
    fn recovery_event_reference_is_not_a_confirmation_association() {
        let (db, id) = recovered();
        let v = db
            .repo()
            .event_history(&EventHistoryQuery {
                kind: Some("observation_stale_cleared".into()),
                ..Default::default()
            })
            .unwrap();
        assert!(v.items[0].associated_alerts.items.is_empty());
        assert_eq!(v.items[0].lifecycle_alert_references.items, vec![id]);
    }
    #[test]
    fn related_event_index_supports_recovery_links() {
        let db = Db::new();
        let c = Connection::open(&db.0).unwrap();
        let plan:String=c.query_row("EXPLAIN QUERY PLAN SELECT DISTINCT alert_id FROM datum_alert_transitions WHERE related_event_id=? ORDER BY alert_id LIMIT ?",params!["event",100],|r|r.get(3)).unwrap();
        assert!(
            plan.contains("idx_datum_alert_transitions_related_event"),
            "{plan}"
        );
    }
    #[test]
    fn read_repository_reuses_retained_runtime_path_without_initializing_it() {
        let path =
            std::env::temp_dir().join(format!("phase127-not-created-{}.db", uuid::Uuid::new_v4()));
        let runtime = super::super::event_producers::EventProducerRuntime::from_path(
            path.to_str().unwrap(),
            &std::env::temp_dir(),
        )
        .unwrap();
        let repository = OperationsRepository::from_runtime(&runtime).unwrap();
        assert_eq!(repository.db_path, path);
        assert!(!path.exists());
        assert!(repository.event_history(&Default::default()).is_err());
        assert!(!path.exists());
    }
    #[test]
    fn contradictory_recovery_reference_fails_closed() {
        let (db, id) = recovered();
        let other = db.alert(event(
            "other-condition",
            "p",
            "another-app",
            0,
            DatumEventSeverityV1::Warning,
        ));
        let c = Connection::open(&db.0).unwrap();
        c.execute("UPDATE datum_alert_transitions SET related_event_id='other-condition' WHERE alert_id=? AND to_status='resolved'",[id.clone()]).unwrap();
        assert_eq!(
            db.repo().alert_view(&id, at(2)).unwrap_err().kind,
            ReadErrorKind::SourceCorrupt
        );
        assert_eq!(
            datum_alert_store::get_alert(&db.0, &other.alert_id)
                .unwrap()
                .unwrap()
                .status,
            DatumAlertStatusV1::Open
        );
    }

    #[test]
    fn critical_temporal_aligned_stale_is_not_clock_discrepancy() {
        let view = temporal_one(state(-3600, -3600, false), 0);
        assert_eq!(view.communication_freshness, CommunicationFreshness::Stale);
        assert_eq!(view.observation_freshness, CommunicationFreshness::Stale);
        assert_eq!(
            view.source_time_quality,
            SourceTimeQuality::AlignedOrNoDiscrepancy
        );
        assert_eq!(view.observed_receipt_delta_seconds, Some(0));
    }
    #[test]
    fn critical_temporal_recent_receipt_old_observation_is_independent() {
        let view = temporal_one(state(-3600, -5, false), 0);
        assert_eq!(view.communication_freshness, CommunicationFreshness::Fresh);
        assert_eq!(view.observation_freshness, CommunicationFreshness::Stale);
        assert_eq!(view.source_time_quality, SourceTimeQuality::BehindOrDelayed);
        assert_eq!(view.observed_receipt_delta_seconds, Some(-3595));
    }
    #[test]
    fn critical_temporal_stale_receipt_can_also_have_delayed_source() {
        let view = temporal_one(state(-7200, -3600, false), 0);
        assert_eq!(view.communication_freshness, CommunicationFreshness::Stale);
        assert_eq!(view.observation_freshness, CommunicationFreshness::Stale);
        assert_eq!(view.source_time_quality, SourceTimeQuality::BehindOrDelayed);
    }
    #[test]
    fn critical_temporal_ahead_boundary_uses_existing_skew_policy() {
        let tolerance = DMonitorFreshnessPolicyV1::default().max_future_clock_skew_seconds as i64;
        let within = temporal_one(state(tolerance, 0, false), 0);
        let beyond = temporal_one(state(tolerance + 1, 0, false), 0);
        assert_eq!(
            within.source_time_quality,
            SourceTimeQuality::AlignedOrNoDiscrepancy
        );
        assert_eq!(beyond.source_time_quality, SourceTimeQuality::SourceAhead);
        assert_eq!(
            beyond.communication_freshness,
            CommunicationFreshness::Fresh
        );
        assert_eq!(beyond.observation_freshness, CommunicationFreshness::Fresh);
    }
    #[test]
    fn critical_temporal_ordering_and_missing_dimensions_are_explicit() {
        let ordering = temporal_one(state(1, 1, false), 0);
        assert_eq!(
            ordering.observation_freshness,
            CommunicationFreshness::Indeterminate
        );
        assert_eq!(
            ordering.source_time_quality,
            SourceTimeQuality::EvaluationOrderingAnomaly
        );
        assert_eq!(
            temporal_one(DMonitorStateV1::empty(), 0).observation_freshness,
            CommunicationFreshness::MissingEvidence
        );
    }
    fn complete_sources(db: &Db) -> OperationsSources {
        db.sources()
            .with_project_graphs(BTreeMap::new())
            .with_deployment(DDeployStateV1::empty())
            .with_dmonitor(DMonitorStateV1::empty(), Default::default(), vec![])
            .unwrap()
    }
    #[test]
    fn critical_discovery_events_are_independent_of_recent_page() {
        let db = Db::new();
        for n in 0..5 {
            db.insert(event(
                &format!("e{n}"),
                "p",
                &format!("a{n}"),
                -100000,
                DatumEventSeverityV1::Info,
            ));
        }
        let q = OverviewQuery {
            limit: 2,
            recent_from_utc: text(0),
        };
        let v = complete_sources(&db).global_overview(at(1), &q).unwrap();
        assert!(available(v.recent_events).items.is_empty());
        assert_eq!(v.known_applications.page.items.len(), 2);
        assert!(v.known_applications.page.has_more);
        assert_eq!(
            v.known_applications.coverage.completeness,
            DiscoveryCompleteness::Complete
        );
        assert_eq!(v.known_projects.page.items, vec!["p"]);
        assert!(!v.known_projects.page.has_more);
    }
    #[test]
    fn critical_discovery_alerts_are_independent_of_attention_page() {
        let db = Db::new();
        for n in 0..5 {
            db.alert(event(
                &format!("e{n}"),
                "p",
                &format!("a{n}"),
                0,
                DatumEventSeverityV1::Warning,
            ));
        }
        let v = complete_sources(&db)
            .project_overview(
                "p",
                at(1),
                &OverviewQuery {
                    limit: 2,
                    recent_from_utc: text(0),
                },
            )
            .unwrap();
        assert!(v.known_applications.page.has_more);
        assert_eq!(
            v.known_applications.coverage.source_coverage["alerts"],
            DiscoverySourceCoverage::FullyScanned
        );
        assert!(v
            .known_applications
            .page
            .items
            .iter()
            .all(|a| a.provenance == vec!["alert", "event"]));
    }
    #[test]
    fn critical_discovery_unavailable_source_is_not_complete_zero() {
        let db = Db::new();
        let sources = complete_sources(&db).with_source_error(
            OperationsSource::Deployment,
            error(
                ReadErrorKind::SourceUnavailable,
                "ddeploy",
                "secret unavailable diagnostic",
            ),
        );
        let v = sources
            .global_overview(at(0), &OverviewQuery::at(at(0)))
            .unwrap();
        assert!(v.known_applications.page.items.is_empty());
        assert!(!v.known_applications.page.has_more);
        assert_eq!(
            v.known_applications.coverage.completeness,
            DiscoveryCompleteness::Partial
        );
        assert!(matches!(
            v.known_applications.coverage.source_coverage["ddeploy"],
            DiscoverySourceCoverage::Unavailable(_)
        ));
    }
    #[test]
    fn critical_discovery_corrupt_source_keeps_other_references() {
        let db = Db::new();
        db.insert(event("e", "p", "app", 0, DatumEventSeverityV1::Info));
        let mut evidence = state(0, 0, true);
        evidence.observation_index.clear();
        let v = complete_sources(&db)
            .with_dmonitor(evidence, Default::default(), expected())
            .unwrap()
            .global_overview(at(1), &OverviewQuery::at(at(1)))
            .unwrap();
        assert_eq!(v.known_applications.page.items[0].application_id, "app");
        assert_eq!(
            v.known_applications.coverage.completeness,
            DiscoveryCompleteness::Partial
        );
        assert!(matches!(
            v.known_applications.coverage.source_coverage["dmonitor"],
            DiscoverySourceCoverage::Corrupt(_)
        ));
    }
    #[test]
    fn critical_discovery_bounded_source_scan_marks_truncation() {
        let db = Db::new();
        for n in 0..4 {
            db.insert(event(
                &format!("e{n}"),
                "p",
                &format!("a{n}"),
                0,
                DatumEventSeverityV1::Info,
            ));
        }
        let c = db.repo().connection().unwrap();
        let (refs, _, coverage) = database_identities(&c, false, None, true, 2).unwrap();
        assert_eq!(refs.len(), 2);
        assert_eq!(coverage, DiscoverySourceCoverage::Truncated);
        let v = discovery(refs, 10, BTreeMap::from([("events".into(), coverage)]));
        assert!(!v.page.has_more);
        assert_eq!(v.coverage.completeness, DiscoveryCompleteness::Partial);
        let (_, projects, coverage) = database_identities(&c, false, None, false, 2).unwrap();
        assert_eq!(projects, vec!["p"]);
        assert_eq!(coverage, DiscoverySourceCoverage::FullyScanned);
    }
    #[test]
    fn critical_discovery_multiple_provenance_is_sorted_and_deduplicated() {
        let db = Db::new();
        db.alert(event("e", "p", "app", 0, DatumEventSeverityV1::Warning));
        let sources = complete_sources(&db)
            .with_deployment(deployment(true))
            .with_dmonitor(state(0, 0, true), Default::default(), expected())
            .unwrap();
        let v = sources
            .global_overview(at(1), &OverviewQuery::at(at(1)))
            .unwrap();
        assert_eq!(v.known_applications.page.items.len(), 1);
        assert_eq!(
            v.known_applications.page.items[0].provenance,
            vec![
                "accepted_dmap",
                "alert",
                "ddeploy_proposal",
                "dmonitor",
                "event"
            ]
        );
    }
    #[test]
    fn critical_discovery_production_cap_does_not_hide_independent_project() {
        let db = Db::new();
        for n in 0..=MAX_LIMIT {
            db.insert(event(
                &format!("e{n:04}"),
                if n == MAX_LIMIT { "q" } else { "p" },
                &format!("a{n:04}"),
                0,
                DatumEventSeverityV1::Info,
            ));
        }
        let q = OverviewQuery {
            limit: MAX_LIMIT,
            recent_from_utc: text(0),
        };
        let v = complete_sources(&db).global_overview(at(1), &q).unwrap();
        assert!(!v.known_applications.page.has_more);
        assert_eq!(
            v.known_applications.coverage.completeness,
            DiscoveryCompleteness::Partial
        );
        assert_eq!(
            v.known_applications.coverage.source_coverage["events"],
            DiscoverySourceCoverage::Truncated
        );
        assert_eq!(v.known_projects.page.items, vec!["p", "q"]);
        assert_eq!(
            v.known_projects.coverage.completeness,
            DiscoveryCompleteness::Complete
        );
    }
    #[test]
    fn critical_discovery_unconfigured_is_indeterminate_not_registry_zero() {
        let v = OperationsSources::default()
            .global_overview(at(0), &OverviewQuery::at(at(0)))
            .unwrap();
        assert_eq!(
            v.known_applications.coverage.completeness,
            DiscoveryCompleteness::Indeterminate
        );
        assert_eq!(
            v.known_projects.coverage.completeness,
            DiscoveryCompleteness::Indeterminate
        );
        assert!(matches!(
            v.canonical_application_count,
            SourceSection::NotConfigured
        ));
    }
    #[test]
    fn critical_timeline_recovery_is_reachable_beyond_first_page() {
        let (db, id) = recovered();
        let first = db.repo().alert_timeline_page(&id, None, 1, at(2)).unwrap();
        assert!(first.entries.has_more);
        let next = db
            .repo()
            .alert_timeline_page(&id, first.next_cursor.as_ref(), 1, at(2))
            .unwrap();
        assert_eq!(
            next.entries.items[0].relationships,
            vec![AlertTimelineRelationship::Recovery]
        );
        assert!(!next.entries.has_more);
        assert_eq!(next.generated_at_utc, text(2));
        assert!(db
            .repo()
            .alert_view(&id, at(2))
            .unwrap()
            .unwrap()
            .recovery_event
            .is_some());
    }
    #[test]
    fn critical_timeline_reopen_cycles_paginate_complete_union_once() {
        let (db, id) = recovered();
        // Lifecycle commands use their own real audit clock. Anchor this private
        // fixture's first resolution/checkpoint consistently at the fixed time,
        // so the next occurrence proves recurrence without wall-clock coupling.
        let c = Connection::open(&db.0).unwrap();
        c.execute(
            "UPDATE datum_alerts SET resolved_at_utc=? WHERE alert_id=?",
            params![text(1), id],
        )
        .unwrap();
        c.execute("UPDATE datum_alert_transitions SET transitioned_at_utc=? WHERE alert_id=? AND to_status='resolved'", params![text(1),id]).unwrap();
        drop(c);
        let pipeline = EventProducerPipeline::open(db.0.clone()).unwrap();
        monitoring(&pipeline, state(-301, -301, false), 2, "reopened");
        monitoring(&pipeline, state(2, 2, false), 3, "cleared-again");
        let mut cursor = None;
        let mut events = vec![];
        let mut recovery = 0;
        loop {
            let result = db
                .repo()
                .alert_timeline_page(&id, cursor.as_ref(), 1, at(4))
                .unwrap();
            for entry in result.entries.items {
                recovery += usize::from(
                    entry
                        .relationships
                        .contains(&AlertTimelineRelationship::Recovery),
                );
                events.push((
                    time_key(&entry.event.occurred_at_utc).unwrap(),
                    entry.event.event_id,
                ));
            }
            cursor = result.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(events.len(), 4);
        assert_eq!(recovery, 2);
        assert!(events.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(
            events
                .iter()
                .map(|(_, id)| id)
                .collect::<BTreeSet<_>>()
                .len(),
            4
        );
        assert_eq!(
            db.repo()
                .alert_view(&id, at(4))
                .unwrap()
                .unwrap()
                .condition_cleared_at_utc,
            Some(text(3))
        );
    }
    #[test]
    fn critical_timeline_relationship_union_deduplicates_equal_time_events() {
        let db = Db::new();
        let a = db.alert(event("z", "p", "app", 0, DatumEventSeverityV1::Warning));
        db.alert(event("a", "p", "app", 0, DatumEventSeverityV1::Warning));
        let first = db
            .repo()
            .alert_timeline_page(&a.alert_id, None, 1, at(1))
            .unwrap();
        let second = db
            .repo()
            .alert_timeline_page(&a.alert_id, first.next_cursor.as_ref(), 1, at(1))
            .unwrap();
        assert_eq!(first.entries.items[0].event.event_id, "a");
        assert_eq!(second.entries.items[0].event.event_id, "z");
        // Creation/association reference the same admitted event; it occurs once.
        assert_eq!(
            second.entries.items[0].relationships,
            vec![
                AlertTimelineRelationship::ConditionObservation,
                AlertTimelineRelationship::LifecycleReference
            ]
        );
        assert!(!second.entries.has_more);
    }
    #[test]
    fn critical_timeline_invalid_cursor_is_rejected() {
        let (db, id) = recovered();
        let cursor = AlertTimelineCursor {
            occurred_epoch_seconds: 0,
            occurred_subsec_nanos: 1_000_000_000,
            event_id: "x".into(),
        };
        assert_eq!(
            db.repo()
                .alert_timeline_page(&id, Some(&cursor), 1, at(0))
                .unwrap_err()
                .kind,
            ReadErrorKind::InvalidQuery
        );
    }
    fn metadata_event() -> super::super::datum_event::DatumEventV1 {
        let mut e = event("temporal", "p", "app", 0, DatumEventSeverityV1::Info);
        e.attributes = json!({"temporal_context":{"server_evaluated_at_utc":text(0),"evidence":[{
            "observation_id":"obs", "source_observed_at_utc":text(-3600),"server_received_at_utc":text(-5),
            "observed_receipt_delta_seconds":-3595,"source_age_seconds":3600,"receipt_age_seconds":5}]}});
        e
    }
    #[test]
    fn critical_metadata_valid_timestamps_and_signed_delta_are_typed() {
        let TemporalMetadata::Available(v) = temporal_metadata(&metadata_event()) else {
            panic!("valid metadata");
        };
        assert_eq!(v.evidence[0].observed_receipt_delta_seconds, -3595);
        assert_eq!(v.evidence[0].source_age_seconds, 3600);
        assert_eq!(v.server_evaluated_at_utc, text(0));
    }
    #[test]
    fn critical_metadata_absence_is_normal_and_does_not_invent_values() {
        assert_eq!(
            temporal_metadata(&event("e", "p", "app", 0, DatumEventSeverityV1::Info)),
            TemporalMetadata::Absent
        );
    }
    #[test]
    fn critical_metadata_invalid_shapes_preserve_canonical_event() {
        let db = Db::new();
        let mut e = metadata_event();
        e.attributes["temporal_context"] = json!({"server_evaluated_at_utc":12,"evidence":[]});
        assert!(e.validate().is_empty());
        db.insert(e);
        let v = db.repo().event_history(&Default::default()).unwrap();
        let TemporalMetadata::Invalid(issue) = &v.items[0].temporal_context else {
            panic!("invalid metadata expected");
        };
        assert_eq!(issue.code(), "invalid_temporal_metadata");
        assert_eq!(
            issue.scope().unwrap().application_id.as_deref(),
            Some("app")
        );
        assert_eq!(v.items[0].event_id, "temporal");
    }
    #[test]
    fn critical_metadata_delta_age_range_and_timestamp_checks_fail_explicitly() {
        let paths = [
            ("observed_receipt_delta_seconds", json!(3595)),
            ("source_age_seconds", json!(-1)),
            ("receipt_age_seconds", json!("5")),
            ("receipt_age_seconds", json!(u64::MAX)),
            ("server_received_at_utc", json!("2026-09-29T15:00:00+01:00")),
            ("source_observed_at_utc", Value::Null),
        ];
        for (key, value) in paths {
            let mut e = metadata_event();
            e.attributes["temporal_context"]["evidence"][0][key] = value;
            assert!(
                matches!(temporal_metadata(&e), TemporalMetadata::Invalid(_)),
                "{key}"
            );
        }
        let mut e = metadata_event();
        e.attributes["temporal_context"]["server_evaluated_at_utc"] = json!(text(1));
        assert!(matches!(
            temporal_metadata(&e),
            TemporalMetadata::Invalid(_)
        ));
    }
    #[test]
    fn critical_metadata_negative_ages_preserve_clock_anomalies() {
        let mut e = metadata_event();
        e.attributes["temporal_context"]["evidence"][0] = json!({
            "source_observed_at_utc":text(10),"server_received_at_utc":text(1),
            "observed_receipt_delta_seconds":9,"source_age_seconds":-10,"receipt_age_seconds":-1});
        let TemporalMetadata::Available(v) = temporal_metadata(&e) else {
            panic!("signed values valid");
        };
        assert_eq!(v.evidence[0].receipt_age_seconds, -1);
    }
    #[test]
    fn critical_errors_unavailable_database_does_not_serialize_path() {
        let secret =
            std::env::temp_dir().join(format!("secret-phase127-{}.db", uuid::Uuid::new_v4()));
        let e = OperationsRepository::new(secret.clone())
            .unwrap()
            .event_history(&Default::default())
            .unwrap_err();
        let serialized = serde_json::to_string(&e).unwrap();
        assert!(!serialized.contains("secret-phase127"));
        assert!(!serialized.contains("diagnostic"));
        assert_eq!(e.code(), "source_unavailable");
        assert!(e.safe_message().len() <= MAX_SAFE_MESSAGE_BYTES);
    }
    #[test]
    fn critical_errors_corrupt_snapshot_has_structured_safe_scope() {
        let mut evidence = state(0, 0, true);
        evidence.observation_index.clear();
        let sources = OperationsSources::default()
            .with_dmonitor(evidence, Default::default(), expected())
            .unwrap();
        let view = app(&sources);
        let SourceSection::Corrupt(issue) = view.temporal_quality else {
            panic!("corrupt evidence");
        };
        assert_eq!(
            issue.scope().unwrap(),
            &ReadIssueScope {
                project_id: Some("p".into()),
                application_id: Some("app".into()),
                subject: None,
            }
        );
        assert_eq!(issue.source(), "dmonitor");
        assert_eq!(issue.code(), "source_corrupt");
    }
    #[test]
    fn critical_errors_injected_secret_is_internal_only_and_utf8_bounded() {
        let detail = format!(
            "credential=SECRET; SQL SELECT; C:\\private\\database; {}",
            "界".repeat(2000)
        );
        let issue = OperationsReadError::source_failure(
            ReadErrorKind::SourceUnavailable,
            "untrusted source secret",
            detail.clone(),
        );
        assert_eq!(issue.internal_diagnostic(), detail);
        let json = serde_json::to_string(&issue).unwrap();
        for forbidden in ["SECRET", "SELECT", "private", "untrusted", "界"] {
            assert!(!json.contains(forbidden));
        }
        assert!(issue.safe_message().len() <= MAX_SAFE_MESSAGE_BYTES);
        assert_eq!(bounded_text("界界", 5), "界");
        let sources =
            OperationsSources::default().with_source_error(OperationsSource::DMonitor, issue);
        assert!(!serde_json::to_string(
            &sources
                .global_overview(at(0), &OverviewQuery::at(at(0)))
                .unwrap()
        )
        .unwrap()
        .contains("SECRET"));
    }
    #[test]
    fn critical_errors_scheduler_failure_retains_class_not_raw_details() {
        let mut s = scheduler();
        s.checks[0].last_evaluation_result = Some(MonitoringLastEvaluation::Failed {
            error: super::super::active_monitoring::MonitoringCheckError::EvaluationFailed(
                "secret SQL /private/path".into(),
            ),
        });
        let v = available(scheduler_view(&SourceSection::Available(s), at(0), 100));
        let json = serde_json::to_string(&v).unwrap();
        assert!(!json.contains("secret"));
        assert!(!json.contains("private"));
        assert!(json.contains("evaluation_failed"));
    }
}
