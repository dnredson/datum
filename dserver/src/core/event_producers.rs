//! Phase 126 adapters: record evidence/committed decisions, never decide them.
//! Scheduler startup remains explicit; no detached consumer or authority writes.
use super::{
    active_monitoring::{
        DMonitorFreshnessCheck, DMonitorStreamSelector, MonitoringCheckError, MonitoringCompletion,
        MonitoringCompletionResult, MonitoringEvidenceSnapshot, MonitoringFinding,
        MonitoringOutcome, MonitoringSchedulerStatus,
    },
    canonical_dmap::CanonicalContractReferenceV1,
    datum_alert::derive_correlation_key,
    datum_event::*,
    ddeploy::DDeployAcceptanceV1,
    ddeploy_v2::DDeployAcceptanceV2,
    dmonitor_freshness::{evaluate_freshness, DMonitorFreshnessPolicyV1, DMonitorFreshnessV1},
    jcs,
};
use crate::storage::{
    datum_alert_store::{self, AlertProcessingOutcome, DatumAlertStoreError},
    datum_event_store::{self, DatumEventAdmission, DatumEventStoreError},
};
use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{json, Value};
use std::{
    fmt,
    path::{Path, PathBuf},
};

pub const EVENT_ALERT_DB_PATH_ENV: &str = "DATUM_EVENT_ALERT_DB_PATH";
pub const DEFAULT_EVENT_ALERT_DB_PATH: &str = "datum-events.db";
pub const MAX_FINDINGS_PER_OCCURRENCE: usize = 64;
pub const MAX_EVIDENCE_REFS_PER_FINDING: usize = 16;

#[derive(Debug, Clone)]
pub enum ProducerError {
    BlockingTask(String),
    InvalidSource(String),
    Translation(String),
    PayloadTooLarge {
        bytes: usize,
        limit: usize,
    },
    InvalidCanonicalEvent(Vec<super::canonical_dgraph::CanonicalValidationFinding>),
    InvalidMonitoringResult(MonitoringCheckError),
    EventStore {
        event_id: String,
        source: DatumEventStoreError,
    },
    AlertProcessing {
        event_id: String,
        source: DatumAlertStoreError,
    },
    DatabasePath(String),
    DatabaseInitialization(DatumAlertStoreError),
}
impl fmt::Display for ProducerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ProducerError {}

/// Dedicated durable file, NOT the optional legacy control-state fallback.
/// Resolve once and retain on the pipeline. Blank/memory/URI paths fail closed.
pub fn resolve_database_path(value: Option<&str>, base: &Path) -> Result<PathBuf, ProducerError> {
    let value = value.unwrap_or(DEFAULT_EVENT_ALERT_DB_PATH);
    if value.trim().is_empty()
        || value != value.trim()
        || value.to_ascii_lowercase().contains(":memory:")
        || value.to_ascii_lowercase().starts_with("file:")
    {
        return Err(ProducerError::DatabasePath(
            "requires a nonblank durable filesystem path".into(),
        ));
    }
    let path = PathBuf::from(value);
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes.len() == 2 || !matches!(bytes[2], b'/' | b'\\'))
    {
        return Err(ProducerError::DatabasePath(
            "drive-relative path is ambiguous".into(),
        ));
    }
    let path = if path.is_absolute() {
        path
    } else {
        base.join(path)
    };
    if !path.is_absolute() {
        return Err(ProducerError::DatabasePath(
            "resolved path must be absolute".into(),
        ));
    }
    Ok(path)
}
pub fn configured_database_path() -> Result<PathBuf, ProducerError> {
    let value = match std::env::var(EVENT_ALERT_DB_PATH_ENV) {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => return Err(ProducerError::DatabasePath(error.to_string())),
    };
    let base = std::env::current_dir().map_err(|e| ProducerError::DatabasePath(e.to_string()))?;
    resolve_database_path(value.as_deref(), &base)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertPolicy {
    HistoricalOnly,
    ProcessAsAlert,
}
/// Exact source/category/kind allowlist. Unknown classes stay historical only.
pub fn alert_policy(event: &DatumEventV1) -> AlertPolicy {
    if event.source == DatumEventSourceV1::ActiveMonitor
        && event.category == DatumEventCategoryV1::Observation
        && matches!(
            event.kind.as_str(),
            "observation_stale"
                | "observation_missing"
                | "observation_clock_skew"
                | "evaluation_before_receipt"
        )
    {
        AlertPolicy::ProcessAsAlert
    } else {
        AlertPolicy::HistoricalOnly
    }
}
#[derive(Debug)]
pub struct ProducedEvent {
    pub admission: DatumEventAdmission,
    pub alert: Option<AlertProcessingOutcome>,
    pub recovery: Option<datum_alert_store::ConditionRecoveryOutcome>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum CompletionDisposition {
    Evaluated,
    Satisfied,
    CheckError,
    Panicked,
    Cancelled,
}
#[derive(Debug)]
pub struct MonitoringProduction {
    pub disposition: CompletionDisposition,
    pub events: Vec<ProducedEvent>,
    pub ignored_finding_codes: Vec<String>,
    pub recovery_noops: Vec<(String, datum_alert_store::ConditionRecoveryOutcome)>,
}
/// Assign once when consuming a completion; retain this envelope for retries.
/// Separate executions get distinct IDs even if UTC repeats after correction.
pub struct MonitoringOccurrence {
    pub occurrence_id: String,
    pub completion: MonitoringCompletion,
}
impl MonitoringOccurrence {
    pub fn from_completion(completion: MonitoringCompletion) -> Self {
        Self {
            occurrence_id: uuid::Uuid::new_v4().to_string(),
            completion,
        }
    }
}

pub struct EventProducerPipeline {
    db_path: PathBuf,
}
impl EventProducerPipeline {
    pub fn open(path: PathBuf) -> Result<Self, ProducerError> {
        let base =
            std::env::current_dir().map_err(|e| ProducerError::DatabasePath(e.to_string()))?;
        let text = path
            .to_str()
            .ok_or_else(|| ProducerError::DatabasePath("non-UTF8 path".into()))?;
        let path = resolve_database_path(Some(text), &base)?;
        datum_alert_store::initialize_database(&path)
            .map_err(ProducerError::DatabaseInitialization)?;
        Ok(Self { db_path: path })
    }
    pub fn configured() -> Result<Self, ProducerError> {
        Self::open(process_runtime().db_path.clone()?)
    }
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
    /// Admission always precedes alerting; replays retry alert processing too.
    /// An alert failure never rolls back the already-admitted immutable event.
    pub fn record(&self, event: DatumEventV1) -> Result<ProducedEvent, ProducerError> {
        validate_event(&event)?;
        let admission =
            datum_event_store::admit_event(&self.db_path, &event).map_err(|source| {
                ProducerError::EventStore {
                    event_id: event.event_id.clone(),
                    source,
                }
            })?;
        let stored = match &admission {
            DatumEventAdmission::Inserted(s) | DatumEventAdmission::IdempotentReplay(s) => s,
        };
        let alert = if alert_policy(&stored.event) == AlertPolicy::ProcessAsAlert {
            Some(
                datum_alert_store::process_event(&self.db_path, stored).map_err(|source| {
                    ProducerError::AlertProcessing {
                        event_id: event.event_id.clone(),
                        source,
                    }
                })?,
            )
        } else {
            None
        };
        Ok(ProducedEvent {
            admission,
            alert,
            recovery: None,
        })
    }
    pub fn monitoring(
        &self,
        occurrence: &MonitoringOccurrence,
        snapshot: &MonitoringEvidenceSnapshot,
        binding: &DMonitorProducerBinding,
    ) -> Result<MonitoringProduction, ProducerError> {
        let (disposition, events, ignored_finding_codes) =
            translate_monitoring(occurrence, snapshot, binding)?;
        let clears = binding.clearable_conditions(occurrence, snapshot)?;
        // Validate/translate the entire batch before the first write. Stores
        // are individually durable, not one atomic batch; retry the envelope.
        let mut produced = Vec::new();
        let mut recovery_noops = Vec::new();
        let mut prepared_clears = Vec::new();
        // Only exact known conditions with an existing episode need recovery
        // history. A retained clear event is always reprocessed on replay.
        for condition in clears {
            let kind = condition.kind();
            let mut clear = binding.clearing_event(occurrence, snapshot, kind)?;
            let mut original = clear.clone();
            original.kind = kind.into();
            let key = derive_correlation_key(&original).map_err(ProducerError::Translation)?;
            let existing = datum_alert_store::list_alerts(
                &self.db_path,
                &datum_alert_store::AlertQuery {
                    correlation_key: Some(&key),
                    limit: 1,
                    ..Default::default()
                },
            )
            .map_err(|source| ProducerError::AlertProcessing {
                event_id: clear.event_id.clone(),
                source,
            })?;
            if existing.is_empty() {
                recovery_noops.push((
                    kind.into(),
                    datum_alert_store::ConditionRecoveryOutcome::NoMatchingAlert,
                ));
                continue;
            }
            use super::datum_alert::DatumAlertStatusV1;
            if matches!(
                existing[0].status,
                DatumAlertStatusV1::Resolved | DatumAlertStatusV1::Closed
            ) && datum_event_store::get_event(&self.db_path, &clear.event_id)
                .map_err(|source| ProducerError::EventStore {
                    event_id: clear.event_id.clone(),
                    source,
                })?
                .is_none()
            {
                let outcome = if existing[0].status == DatumAlertStatusV1::Closed {
                    datum_alert_store::ConditionRecoveryOutcome::AlreadyClosed(
                        existing[0].alert_id.clone(),
                    )
                } else {
                    datum_alert_store::ConditionRecoveryOutcome::AlreadyResolved(
                        existing[0].alert_id.clone(),
                    )
                };
                recovery_noops.push((kind.into(), outcome));
                continue;
            }
            clear.attributes["condition_correlation_key"] = json!(key);
            validate_event(&clear)?;
            prepared_clears.push((clear, kind));
        }
        for event in events {
            produced.push(self.record(event)?);
        }
        for (clear, kind) in prepared_clears {
            let mut recorded = self.record(clear)?; // cleared kinds are historical only
            let stored = match &recorded.admission {
                DatumEventAdmission::Inserted(s) | DatumEventAdmission::IdempotentReplay(s) => s,
            };
            recorded.recovery = Some(
                datum_alert_store::resolve_condition_from_event(&self.db_path, stored, kind)
                    .map_err(|source| ProducerError::AlertProcessing {
                        event_id: stored.event.event_id.clone(),
                        source,
                    })?,
            );
            produced.push(recorded);
        }
        Ok(MonitoringProduction {
            disposition,
            events: produced,
            ignored_finding_codes,
            recovery_noops,
        })
    }
}

/// Semantic provenance binding, not authentication or operational authority.
/// Only this configured DMonitor family/selector/policy can interpret its codes.
pub struct DMonitorProducerBinding {
    selector: DMonitorStreamSelector,
    policy: DMonitorFreshnessPolicyV1,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitoringCondition {
    Stale,
    Missing,
    ClockSkew,
    BeforeReceipt,
}
impl MonitoringCondition {
    pub fn kind(self) -> &'static str {
        match self {
            Self::Stale => "observation_stale",
            Self::Missing => "observation_missing",
            Self::ClockSkew => "observation_clock_skew",
            Self::BeforeReceipt => "evaluation_before_receipt",
        }
    }
}
impl DMonitorProducerBinding {
    pub fn new(
        selector: DMonitorStreamSelector,
        policy: DMonitorFreshnessPolicyV1,
    ) -> Result<Self, ProducerError> {
        DMonitorFreshnessCheck::new(
            std::sync::Arc::new(
                MonitoringEvidenceSnapshot::from_state(
                    super::dmonitor_state::DMonitorStateV1::empty(),
                )
                .map_err(ProducerError::InvalidMonitoringResult)?,
            ),
            selector.clone(),
            policy.clone(),
        )
        .map_err(ProducerError::InvalidMonitoringResult)?;
        Ok(Self { selector, policy })
    }
    fn validate(
        &self,
        o: &MonitoringOccurrence,
        snapshot: &MonitoringEvidenceSnapshot,
    ) -> Result<(), ProducerError> {
        let expected_id = self
            .selector
            .check_id()
            .map_err(|e| ProducerError::InvalidSource(e.to_string()))?;
        let MonitoringCompletionResult::Evaluation(e) = &o.completion.result else {
            return Ok(());
        };
        if e.check_id != expected_id {
            return Err(ProducerError::InvalidSource(
                "unbound DMonitor check identity".into(),
            ));
        }
        let check = DMonitorFreshnessCheck::new(
            std::sync::Arc::new(snapshot.clone()),
            self.selector.clone(),
            self.policy.clone(),
        )
        .map_err(ProducerError::InvalidMonitoringResult)?;
        let (at, _) = canonical_time(&e.evaluated_at_utc)?;
        let expected = check
            .evaluate_snapshot(snapshot, at)
            .map_err(ProducerError::InvalidMonitoringResult)?;
        if expected.outcome != e.outcome || expected.findings != e.findings {
            return Err(ProducerError::InvalidSource(
                "output is not this trusted DMonitor evaluation".into(),
            ));
        }
        Ok(())
    }
    fn clearable_conditions(
        &self,
        o: &MonitoringOccurrence,
        snapshot: &MonitoringEvidenceSnapshot,
    ) -> Result<Vec<MonitoringCondition>, ProducerError> {
        if !matches!(
            o.completion.result,
            MonitoringCompletionResult::Evaluation(_)
        ) {
            return Ok(vec![]);
        }
        self.validate(o, snapshot)?;
        let Some(record) = snapshot.stream_head(&self.selector) else {
            return Ok(vec![]);
        };
        let mut clears = vec![MonitoringCondition::Missing];
        let (evaluated, _) = canonical_time(&o.completion.evaluated_at_utc)?;
        let (received, _) = canonical_time(&record.received_at_utc)?;
        if evaluated < received {
            return Ok(clears);
        } // no age/skew inference from impossible ordering
        clears.push(MonitoringCondition::BeforeReceipt);
        let state = evaluate_freshness(&record, &self.policy, &o.completion.evaluated_at_utc)
            .map_err(ProducerError::InvalidSource)?;
        if state.freshness != DMonitorFreshnessV1::ClockSkew {
            clears.push(MonitoringCondition::ClockSkew);
        }
        if state.age_dimensions_satisfied {
            clears.push(MonitoringCondition::Stale);
        }
        Ok(clears)
    }
    fn clearing_event(
        &self,
        o: &MonitoringOccurrence,
        snapshot: &MonitoringEvidenceSnapshot,
        kind: &str,
    ) -> Result<DatumEventV1, ProducerError> {
        let record = snapshot.stream_head(&self.selector).ok_or_else(|| {
            ProducerError::InvalidSource("clear requires accepted stream evidence".into())
        })?;
        let mut event = envelope(
            self.selector.subject(),
            DatumEventSourceV1::ActiveMonitor,
            DatumEventCategoryV1::Observation,
            &format!("{kind}_cleared"),
            o.completion.evaluated_at_utc.clone(),
        );
        event.event_id = occurrence_id(json!([
            "monitoring_recovery/1",
            o.occurrence_id,
            o.completion.check_id,
            o.completion.evaluated_at_utc,
            kind,
            event.subject
        ]))?;
        event.evidence_refs = vec![DatumEventEvidenceRefV1::DMonitorObservation {
            observation_id: record.observation.observation_id,
        }];
        let (observed, observed_text) = canonical_time(&record.observation.observed_at_utc)?;
        let (received, received_text) = canonical_time(&record.received_at_utc)?;
        let (evaluated, evaluated_text) = canonical_time(&event.occurred_at_utc)?;
        event.attributes = json!({"producer": "dmonitor_recovery_v1", "condition_kind": kind,
            "check_id": o.completion.check_id, "evaluation_occurrence_id": o.occurrence_id,
            "temporal_context": {"server_evaluated_at_utc": evaluated_text, "evidence": [{
                "source_observed_at_utc": observed_text, "server_received_at_utc": received_text,
                "observed_receipt_delta_seconds": (observed-received).num_seconds(),
                "source_age_seconds": (evaluated-observed).num_seconds(), "receipt_age_seconds": (evaluated-received).num_seconds()}]}});
        validate_event(&event)?;
        Ok(event)
    }
}

fn canonical_time(value: &str) -> Result<(DateTime<Utc>, String), ProducerError> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|e| ProducerError::InvalidSource(e.to_string()))?
        .with_timezone(&Utc);
    Ok((parsed, parsed.to_rfc3339_opts(SecondsFormat::AutoSi, true)))
}
fn occurrence_id(value: Value) -> Result<String, ProducerError> {
    jcs::digest_sha256_hex(&value)
        .map(|s| format!("event:{s}"))
        .map_err(|e| ProducerError::Translation(format!("{e:?}")))
}
fn validate_event(event: &DatumEventV1) -> Result<(), ProducerError> {
    let bytes = serde_json::to_vec(&event.attributes)
        .map_err(|e| ProducerError::Translation(e.to_string()))?
        .len();
    if bytes > MAX_DATUM_EVENT_ATTRIBUTES_BYTES {
        return Err(ProducerError::PayloadTooLarge {
            bytes,
            limit: MAX_DATUM_EVENT_ATTRIBUTES_BYTES,
        });
    }
    let errors = event.validate();
    if !errors.is_empty() {
        return Err(ProducerError::InvalidCanonicalEvent(errors));
    }
    // Canonical integer/JSON/JCS rules remain authoritative too.
    recompute_datum_event_digest(event).map_err(ProducerError::Translation)?;
    Ok(())
}
fn envelope(
    subject: DatumEventSubjectV1,
    source: DatumEventSourceV1,
    category: DatumEventCategoryV1,
    kind: &str,
    occurred: String,
) -> DatumEventV1 {
    DatumEventV1 {
        schema: DATUM_EVENT_SCHEMA.into(),
        event_id: String::new(),
        project_id: subject.project_id().map(str::to_string),
        application_id: subject.application_id().map(str::to_string),
        source,
        category,
        kind: kind.into(),
        severity: DatumEventSeverityV1::Info,
        subject,
        occurred_at_utc: occurred,
        evidence_refs: vec![],
        attributes: json!({}),
        correlation_hint: None,
        confidence_permille: None,
        model_ref: None,
    }
}

/// Built-in DMonitor finding allowlist; errors/panics are NOT fabricated findings.
pub fn translate_monitoring(
    occurrence: &MonitoringOccurrence,
    snapshot: &MonitoringEvidenceSnapshot,
    binding: &DMonitorProducerBinding,
) -> Result<(CompletionDisposition, Vec<DatumEventV1>, Vec<String>), ProducerError> {
    if occurrence.occurrence_id.trim().is_empty() {
        return Err(ProducerError::InvalidSource(
            "missing stable evaluation occurrence identity".into(),
        ));
    }
    let c = &occurrence.completion;
    let evaluation = match &c.result {
        MonitoringCompletionResult::Error(_) => {
            return Ok((CompletionDisposition::CheckError, vec![], vec![]))
        }
        MonitoringCompletionResult::Panicked => {
            return Ok((CompletionDisposition::Panicked, vec![], vec![]))
        }
        MonitoringCompletionResult::Cancelled => {
            return Ok((CompletionDisposition::Cancelled, vec![], vec![]))
        }
        MonitoringCompletionResult::Evaluation(e) => e,
    };
    evaluation
        .validate()
        .map_err(ProducerError::InvalidMonitoringResult)?;
    if c.check_id != evaluation.check_id || c.evaluated_at_utc != evaluation.evaluated_at_utc {
        return Err(ProducerError::InvalidSource(
            "completion/evaluation identity mismatch".into(),
        ));
    }
    if evaluation.findings.len() > MAX_FINDINGS_PER_OCCURRENCE {
        return Err(ProducerError::InvalidSource(
            "too many findings in one occurrence".into(),
        ));
    }
    binding.validate(occurrence, snapshot)?;
    if evaluation.outcome == MonitoringOutcome::Satisfied {
        return Ok((CompletionDisposition::Satisfied, vec![], vec![]));
    }
    let mut events = Vec::new();
    let mut ignored = Vec::new();
    let mut identities = std::collections::BTreeSet::new();
    for finding in &evaluation.findings {
        if !matches!(
            finding.finding_code.as_str(),
            "observation_stale"
                | "observation_missing"
                | "observation_clock_skew"
                | "evaluation_before_receipt"
        ) {
            ignored.push(finding.finding_code.clone());
            continue;
        }
        if finding.evidence_refs.len() > MAX_EVIDENCE_REFS_PER_FINDING {
            return Err(ProducerError::InvalidSource(
                "too many evidence references".into(),
            ));
        }
        let temporal = temporal_context(finding, snapshot, &evaluation.evaluated_at_utc)?;
        let mut event = envelope(
            finding.subject.clone(),
            DatumEventSourceV1::ActiveMonitor,
            DatumEventCategoryV1::Observation,
            &finding.finding_code,
            evaluation.evaluated_at_utc.clone(),
        );
        // Explicit initial translation policy: warnings for the selected
        // temporal classes, irrespective of a check's untrusted severity hint.
        event.severity = DatumEventSeverityV1::Warning;
        event.evidence_refs = finding.evidence_refs.clone();
        event.attributes = json!({"producer": "active_monitor_v1", "check_id": evaluation.check_id,
            "evaluation_occurrence_id": occurrence.occurrence_id, "severity_hint": finding.severity_hint,
            "finding_attributes": finding.attributes, "observed_value": finding.observed_value,
            "threshold": finding.threshold, "temporal_context": temporal});
        event.event_id = occurrence_id(json!([
            "monitoring/1",
            occurrence.occurrence_id,
            evaluation.check_id,
            evaluation.evaluated_at_utc,
            finding.finding_code,
            finding.subject
        ]))?;
        if !identities.insert(event.event_id.clone()) {
            return Err(ProducerError::InvalidSource(
                "duplicate finding identity in evaluation".into(),
            ));
        }
        validate_event(&event)?;
        events.push(event);
    }
    Ok((CompletionDisposition::Evaluated, events, ignored))
}

fn temporal_context(
    finding: &MonitoringFinding,
    snapshot: &MonitoringEvidenceSnapshot,
    evaluated: &str,
) -> Result<Value, ProducerError> {
    let (evaluated, evaluated_text) = canonical_time(evaluated)?;
    let mut records = vec![];
    for reference in &finding.evidence_refs {
        let DatumEventEvidenceRefV1::DMonitorObservation { observation_id } = reference else {
            return Err(ProducerError::InvalidSource(
                "DMonitor finding requires DMonitor evidence".into(),
            ));
        };
        let record = snapshot.observation(observation_id).ok_or_else(|| {
            ProducerError::InvalidSource(
                "referenced observation missing from validated snapshot".into(),
            )
        })?;
        let expected_subject = DatumEventSubjectV1::Other {
            project_id: Some(record.server_project_scope.clone()),
            entity_kind: "dmonitor_stream".into(),
            entity_id: super::dmonitor_state::stream_key(
                &record.server_project_scope,
                &record.observation.node_id,
                &record.observation.monitor_instance_id,
            ),
        };
        if finding.subject != expected_subject {
            return Err(ProducerError::InvalidSource(
                "finding/evidence stream scope mismatch".into(),
            ));
        }
        let (observed, observed_text) = canonical_time(&record.observation.observed_at_utc)?;
        let (received, received_text) = canonical_time(&record.received_at_utc)?;
        if let Some(value) = &finding.observed_value {
            for (key, expected) in [
                (
                    "observed_age_seconds",
                    json!((evaluated - observed).num_seconds()),
                ),
                (
                    "received_age_seconds",
                    json!((evaluated - received).num_seconds()),
                ),
                ("observed_at_utc", json!(record.observation.observed_at_utc)),
                ("received_at_utc", json!(record.received_at_utc)),
            ] {
                if value.get(key).is_some_and(|actual| *actual != expected) {
                    return Err(ProducerError::InvalidSource(
                        "finding/evidence temporal mismatch".into(),
                    ));
                }
            }
        }
        records.push(json!({"observation_id": observation_id,
            "source_observed_at_utc": observed_text, "server_received_at_utc": received_text,
            "observed_receipt_delta_seconds": (observed-received).num_seconds(),
            "source_age_seconds": (evaluated-observed).num_seconds(),
            "receipt_age_seconds": (evaluated-received).num_seconds()}));
    }
    if records.is_empty() && finding.finding_code != "observation_missing" {
        return Err(ProducerError::InvalidSource(
            "temporal finding lacks observation evidence".into(),
        ));
    }
    // Facts, not a new availability or NTP evaluator. The existing check's
    // classification and raw times are retained; no node_down mapping exists.
    Ok(
        json!({"server_evaluated_at_utc": evaluated_text, "evidence": records,
        "interpretation": "observation_time_quality_not_node_availability"}),
    )
}

pub fn acceptance_event_v1(a: &DDeployAcceptanceV1) -> Result<DatumEventV1, ProducerError> {
    if a.schema != super::ddeploy::DDEPLOY_ACCEPTANCE_SCHEMA {
        return Err(ProducerError::InvalidSource(
            "unsupported acceptance schema".into(),
        ));
    }
    acceptance_event(
        "ddeploy_acceptance/1",
        &a.acceptance_id,
        &a.proposal_id,
        &a.proposal_digest,
        &a.project_id,
        &a.application_id,
        &a.accepted_at_utc,
        &a.accepted_by,
        a.materialized_dmap_ref.clone(),
    )
}
pub fn acceptance_event_v2(a: &DDeployAcceptanceV2) -> Result<DatumEventV1, ProducerError> {
    if a.schema != super::ddeploy_v2::DDEPLOY_ACCEPTANCE_V2_SCHEMA {
        return Err(ProducerError::InvalidSource(
            "unsupported acceptance schema".into(),
        ));
    }
    acceptance_event(
        "ddeploy_acceptance/2",
        &a.acceptance_id,
        &a.proposal_id,
        &a.proposal_digest,
        &a.project_id,
        &a.application_id,
        &a.accepted_at_utc,
        &a.accepted_by,
        CanonicalContractReferenceV1 {
            id: a.materialized_dmap_ref.id.clone(),
            revision: a.materialized_dmap_ref.revision,
            digest: a.materialized_dmap_ref.digest.clone(),
        },
    )
}
fn acceptance_event(
    version: &str,
    acceptance: &str,
    proposal: &str,
    digest: &str,
    project: &str,
    application: &str,
    accepted_at: &str,
    accepted_by: &str,
    reference: CanonicalContractReferenceV1,
) -> Result<DatumEventV1, ProducerError> {
    if acceptance.trim().is_empty() || accepted_by.trim().is_empty() {
        return Err(ProducerError::InvalidSource(
            "invalid committed acceptance provenance".into(),
        ));
    }
    let (_, time) = canonical_time(accepted_at)?;
    let mut event = envelope(
        DatumEventSubjectV1::Application {
            project_id: project.into(),
            application_id: application.into(),
        },
        DatumEventSourceV1::DDeploy,
        DatumEventCategoryV1::ControlPlane,
        "dmap_accepted",
        time.clone(),
    );
    event.event_id = occurrence_id(json!([version, project, application, acceptance]))?;
    event.evidence_refs = vec![
        DatumEventEvidenceRefV1::DMap { reference },
        DatumEventEvidenceRefV1::Proposal {
            proposal_id: proposal.into(),
            proposal_digest: digest.into(),
        },
        DatumEventEvidenceRefV1::Other {
            evidence_kind: "ddeploy_acceptance".into(),
            reference_id: acceptance.into(),
        },
    ];
    event.attributes = json!({"producer": version, "acceptance_id": acceptance,
        "accepted_by": accepted_by, "temporal_context": {"server_authority_committed_at_utc": time}});
    validate_event(&event)?;
    Ok(event)
}

/// Called ONLY after the existing authority persistence succeeds (including
/// historical idempotent replay). Failure is diagnostic, never an API failure
/// that misrepresents the committed operation or a rollback instruction.
pub async fn report_after_authority_commit(
    context: PostAuthorityEventContext,
    event: Result<DatumEventV1, ProducerError>,
) {
    process_runtime().record_after_commit(context, event).await;
}

/// Captured at process initialization (or first access in an embedded caller).
/// Configuration errors are frozen too; disk initialization happens on blocking workers.
pub struct EventProducerRuntime {
    db_path: Result<PathBuf, ProducerError>,
}
static PROCESS_RUNTIME: std::sync::OnceLock<std::sync::Arc<EventProducerRuntime>> =
    std::sync::OnceLock::new();
pub fn process_runtime() -> std::sync::Arc<EventProducerRuntime> {
    PROCESS_RUNTIME
        .get_or_init(|| {
            std::sync::Arc::new(EventProducerRuntime {
                db_path: configured_database_path(),
            })
        })
        .clone()
}
impl EventProducerRuntime {
    pub fn from_path(path: &str, base: &Path) -> Result<Self, ProducerError> {
        Ok(Self {
            db_path: Ok(resolve_database_path(Some(path), base)?),
        })
    }
    pub async fn record_after_commit(
        &self,
        context: PostAuthorityEventContext,
        event: Result<DatumEventV1, ProducerError>,
    ) {
        let path = self.db_path.clone();
        // Normal requests own and await this handle. If the request is cancelled,
        // started blocking work cannot be cancelled; the worker still logs errors.
        let _ = owned_post_commit_work(context, move || {
            event.and_then(|event| EventProducerPipeline::open(path?)?.record(event))
        })
        .await;
    }
}
async fn owned_post_commit_work(
    context: PostAuthorityEventContext,
    work: impl FnOnce() -> Result<ProducedEvent, ProducerError> + Send + 'static,
) -> Result<ProducedEvent, ProducerError> {
    let worker_context = context.clone();
    match tokio::task::spawn_blocking(move || {
        let result = work();
        if let Err(error) = &result {
            emit_post_commit_diagnostic(&worker_context, error);
        }
        result
    })
    .await
    {
        Ok(result) => result,
        Err(error) => {
            let error = ProducerError::BlockingTask(error.to_string());
            emit_post_commit_diagnostic(&context, &error);
            Err(error)
        }
    }
}
#[derive(Debug, Clone)]
pub struct PostAuthorityEventContext {
    project_id: String,
    application_id: String,
    acceptance_id: String,
    producer_kind: String,
}
fn diagnostic_bound(value: &str, max: usize) -> String {
    let mut end = value.len().min(max);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].into()
}
impl PostAuthorityEventContext {
    pub fn new(project: &str, application: &str, acceptance: &str, kind: &str) -> Self {
        Self {
            project_id: diagnostic_bound(project, 256),
            application_id: diagnostic_bound(application, 256),
            acceptance_id: diagnostic_bound(acceptance, 256),
            producer_kind: diagnostic_bound(kind, 64),
        }
    }
    pub fn diagnostic(&self, error: &ProducerError) -> Value {
        let class = match error {
            ProducerError::InvalidSource(_) => "invalid_source",
            ProducerError::Translation(_) => "translation",
            ProducerError::PayloadTooLarge { .. } => "payload_too_large",
            ProducerError::InvalidCanonicalEvent(_) => "invalid_canonical_event",
            ProducerError::InvalidMonitoringResult(_) => "invalid_monitoring_result",
            ProducerError::EventStore { .. } => "event_store",
            ProducerError::AlertProcessing { .. } => "alert_processing",
            ProducerError::DatabasePath(_) => "database_path",
            ProducerError::DatabaseInitialization(_) => "database_initialization",
            ProducerError::BlockingTask(_) => "blocking_task",
        };
        // Fixed message avoids exposing arbitrary evidence/labels/credentials from
        // underlying errors. The structured class identifies the failing seam.
        json!({"message": "authority operation succeeded; canonical event recording failed",
            "project_id": self.project_id, "application_id": self.application_id,
            "acceptance_id": self.acceptance_id, "producer_kind": self.producer_kind,
            "error_class": class, "error_message": format!("{class} recording failure; authority remains committed")})
    }
}
fn emit_post_commit_diagnostic(context: &PostAuthorityEventContext, error: &ProducerError) {
    use std::io::Write;
    // Logging failure must not turn an already-committed authority into a panic.
    let _ = writeln!(
        std::io::stderr().lock(),
        "phase126: {}",
        context.diagnostic(error)
    );
}
/// Explicit operational diagnostics for callers consuming scheduler output.
/// No claim that all evaluations were recorded; inspect/log these counters.
pub fn report_scheduler_output_loss(status: &MonitoringSchedulerStatus) {
    if status.dropped_output_full != 0 || status.dropped_output_closed != 0 {
        eprintln!(
            "phase126: scheduler output loss: full={}, closed={}",
            status.dropped_output_full, status.dropped_output_closed
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        active_monitoring::{
            DMonitorFreshnessCheck, DMonitorStreamSelector, MonitoringCheck, MonitoringEvaluation,
            MonitoringScheduler, MonitoringSchedulerConfig,
        },
        canonical_dmonitor::*,
        dmonitor_freshness::DMonitorFreshnessPolicyV1,
        dmonitor_state::{ingest_observation, DMonitorStateV1},
    };
    use super::*;
    use crate::storage::datum_alert_store::AlertQuery;
    use std::sync::Arc;
    struct TestDb(PathBuf);
    impl TestDb {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("phase126-{}.db", uuid::Uuid::new_v4())))
        }
        fn pipeline(&self) -> EventProducerPipeline {
            EventProducerPipeline::open(self.0.clone()).unwrap()
        }
    }
    impl Drop for TestDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    fn time(seconds: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-29T15:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
            + chrono::Duration::seconds(seconds)
    }
    fn text(seconds: i64) -> String {
        time(seconds).to_rfc3339_opts(SecondsFormat::AutoSi, true)
    }
    fn snapshot(observed: i64, received: i64) -> MonitoringEvidenceSnapshot {
        let mut state = DMonitorStateV1::empty();
        ingest_observation(
            &mut state,
            "project-1",
            CanonicalDMonitorObservationV1 {
                schema: CANONICAL_DMONITOR_OBSERVATION_SCHEMA.into(),
                observation_id: "dmonitor-obs:phase126".into(),
                project_id: None,
                node_id: "node-1".into(),
                monitor_instance_id: "monitor-1".into(),
                sequence: 1,
                observed_at_utc: text(observed),
                cycle_id: None,
                collector: "test_collector".into(),
                subject: DMonitorSubjectV1::DNode {
                    node_id: "node-1".into(),
                },
                health: DMonitorHealthV1::Healthy,
                facts: json!({}),
            },
            &text(received),
        )
        .unwrap();
        MonitoringEvidenceSnapshot::from_state(state).unwrap()
    }
    fn check(snapshot: MonitoringEvidenceSnapshot) -> Arc<DMonitorFreshnessCheck> {
        Arc::new(
            DMonitorFreshnessCheck::new(
                Arc::new(snapshot),
                DMonitorStreamSelector::new("project-1", "node-1", "monitor-1").unwrap(),
                DMonitorFreshnessPolicyV1::default(),
            )
            .unwrap(),
        )
    }
    fn binding() -> DMonitorProducerBinding {
        DMonitorProducerBinding::new(
            DMonitorStreamSelector::new("project-1", "node-1", "monitor-1").unwrap(),
            DMonitorFreshnessPolicyV1::default(),
        )
        .unwrap()
    }
    async fn occurrence(
        snapshot: &MonitoringEvidenceSnapshot,
        wall: i64,
        id: &str,
    ) -> MonitoringOccurrence {
        let result = check(snapshot.clone()).evaluate(time(wall)).await.unwrap();
        let check_id = DMonitorStreamSelector::new("project-1", "node-1", "monitor-1")
            .unwrap()
            .check_id()
            .unwrap();
        MonitoringOccurrence {
            occurrence_id: id.into(),
            completion: MonitoringCompletion {
                check_id: check_id.clone(),
                evaluated_at_utc: text(wall),
                completed_at_utc: text(wall),
                result: MonitoringCompletionResult::Evaluation(MonitoringEvaluation {
                    check_id,
                    evaluated_at_utc: text(wall),
                    outcome: result.outcome,
                    findings: result.findings,
                }),
            },
        }
    }
    fn event_of(produced: &ProducedEvent) -> &datum_event_store::StoredDatumEvent {
        match &produced.admission {
            DatumEventAdmission::Inserted(s) | DatumEventAdmission::IdempotentReplay(s) => s,
        }
    }
    #[tokio::test]
    async fn temporal_matrix_preserves_all_clock_domains_and_never_claims_node_down() {
        for (observed, received, wall, expected) in [
            (0, 0, 0, None), // A: source/receipt/evaluation equal, satisfied.
            (-3600, 0, 0, Some("observation_stale")), // B: recent communication, old source.
            (-3600, -3600, 0, Some("observation_stale")), // C: old receipt too.
            (3600, 0, 0, Some("observation_clock_skew")), // D: source ahead.
            (-600, 0, 1, Some("observation_stale")), // E: newly received older fact.
        ] {
            let s = snapshot(observed, received);
            let o = occurrence(&s, wall, "fixed-retry-id").await;
            let (_, events, _) = translate_monitoring(&o, &s, &binding()).unwrap();
            if let Some(kind) = expected {
                let e = &events[0];
                assert_eq!(e.kind, kind);
                assert_ne!(e.kind, "node_down");
                assert!(e.validate().is_empty());
                assert_eq!(e.occurred_at_utc, text(wall));
                let temporal = &e.attributes["temporal_context"];
                assert_eq!(temporal["server_evaluated_at_utc"], text(wall));
                let evidence = &temporal["evidence"][0];
                assert_eq!(evidence["source_observed_at_utc"], text(observed));
                assert_eq!(evidence["server_received_at_utc"], text(received));
                assert_eq!(
                    evidence["observed_receipt_delta_seconds"],
                    observed - received
                );
                assert_eq!(evidence["receipt_age_seconds"], wall - received);
                assert_eq!(
                    s.observation("dmonitor-obs:phase126")
                        .unwrap()
                        .received_at_utc,
                    text(received)
                );
            } else {
                assert!(events.is_empty());
            }
        }
    }
    #[tokio::test]
    async fn stale_clock_skew_missing_before_receipt_have_explicit_policy() {
        for (s, wall, kind) in [
            (snapshot(-301, -301), 0, "observation_stale"),
            (snapshot(31, 0), 0, "observation_clock_skew"),
            (
                MonitoringEvidenceSnapshot::from_state(DMonitorStateV1::empty()).unwrap(),
                0,
                "observation_missing",
            ),
            (snapshot(1, 1), 0, "evaluation_before_receipt"),
        ] {
            let db = TestDb::new();
            let pipeline = db.pipeline();
            let result = pipeline
                .monitoring(&occurrence(&s, wall, kind).await, &s, &binding())
                .unwrap();
            let event = &event_of(&result.events[0]).event;
            assert_eq!(event.kind, kind);
            assert_eq!(alert_policy(event), AlertPolicy::ProcessAsAlert);
            assert!(matches!(
                result.events[0].alert,
                Some(AlertProcessingOutcome::Created(_))
            ));
        }
    }
    #[tokio::test]
    async fn satisfied_errors_panic_cancelled_unknown_findings_do_not_make_fake_events() {
        let s = snapshot(0, 0);
        let db = TestDb::new();
        let pipeline = db.pipeline();
        let mut o = occurrence(&s, 0, "no-output").await;
        assert_eq!(
            pipeline.monitoring(&o, &s, &binding()).unwrap().disposition,
            CompletionDisposition::Satisfied
        );
        for (result, disposition) in [
            (
                MonitoringCompletionResult::Error(MonitoringCheckError::EvaluationFailed(
                    "failure".into(),
                )),
                CompletionDisposition::CheckError,
            ),
            (
                MonitoringCompletionResult::Panicked,
                CompletionDisposition::Panicked,
            ),
            (
                MonitoringCompletionResult::Cancelled,
                CompletionDisposition::Cancelled,
            ),
        ] {
            o.completion.result = result;
            let out = pipeline.monitoring(&o, &s, &binding()).unwrap();
            assert_eq!(out.disposition, disposition);
            assert!(out.events.is_empty());
        }
        let mut o = occurrence(&snapshot(-301, -301), 0, "unknown").await;
        if let MonitoringCompletionResult::Evaluation(e) = &mut o.completion.result {
            e.findings[0].finding_code = "future_check".into();
        }
        assert!(matches!(
            pipeline.monitoring(&o, &s, &binding()),
            Err(ProducerError::InvalidSource(_))
        ));
        assert!(datum_event_store::list_events(&db.0, &Default::default())
            .unwrap()
            .is_empty());
    }
    #[tokio::test]
    async fn retry_repeated_occurrences_and_clock_condition_have_separate_identity_and_correlation()
    {
        let db = TestDb::new();
        let p = db.pipeline();
        let s = snapshot(-301, -301);
        let o = occurrence(&s, 0, "one-execution").await;
        let first = p.monitoring(&o, &s, &binding()).unwrap();
        let retry = p.monitoring(&o, &s, &binding()).unwrap();
        assert!(matches!(
            retry.events[0].admission,
            DatumEventAdmission::IdempotentReplay(_)
        ));
        assert_eq!(event_of(&first.events[0]), event_of(&retry.events[0]));
        assert!(matches!(
            retry.events[0].alert,
            Some(AlertProcessingOutcome::AlreadyProcessed(_))
        ));
        let next = p
            .monitoring(
                &occurrence(&s, 0, "another-execution-same-UTC").await,
                &s,
                &binding(),
            )
            .unwrap();
        assert_ne!(
            event_of(&first.events[0]).event.event_id,
            event_of(&next.events[0]).event.event_id
        );
        assert_eq!(
            next.events[0]
                .alert
                .as_ref()
                .unwrap()
                .alert()
                .occurrence_count,
            2
        );
        let skew = snapshot(31, 0);
        p.monitoring(
            &occurrence(&skew, 0, "skew-execution").await,
            &skew,
            &binding(),
        )
        .unwrap();
        assert_eq!(
            datum_event_store::list_events(&db.0, &Default::default())
                .unwrap()
                .len(),
            4
        );
        assert_eq!(
            datum_alert_store::list_alerts(&db.0, &AlertQuery::default())
                .unwrap()
                .len(),
            2
        );
        drop(p);
        let restarted = db.pipeline();
        assert_eq!(
            datum_event_store::list_events(restarted.db_path(), &Default::default())
                .unwrap()
                .len(),
            4
        );
        assert_eq!(
            datum_alert_store::list_alerts(restarted.db_path(), &Default::default())
                .unwrap()
                .len(),
            2
        );
    }
    #[tokio::test]
    async fn payload_and_source_validation_fail_before_any_writes() {
        let db = TestDb::new();
        let p = db.pipeline();
        let s = snapshot(-301, -301);
        let mut o = occurrence(&s, 0, "bounded").await;
        if let MonitoringCompletionResult::Evaluation(e) = &mut o.completion.result {
            e.findings[0].attributes = json!({"too_large": "x".repeat(16384)});
        }
        assert!(matches!(
            p.monitoring(&o, &s, &binding()),
            Err(ProducerError::InvalidSource(_))
        ));
        let mut o = occurrence(&s, 0, "invalid").await;
        if let MonitoringCompletionResult::Evaluation(e) = &mut o.completion.result {
            e.outcome = MonitoringOutcome::Satisfied;
        }
        assert!(matches!(
            p.monitoring(&o, &s, &binding()),
            Err(ProducerError::InvalidMonitoringResult(_))
        ));
        let o = occurrence(&s, 0, "missing-reference").await;
        assert!(matches!(
            p.monitoring(&o, &snapshot(0, 0), &binding()),
            Err(ProducerError::InvalidSource(_))
        ));
        let mut o = occurrence(&s, 0, "invalid-canonical").await;
        if let MonitoringCompletionResult::Evaluation(e) = &mut o.completion.result {
            e.findings[0].evidence_refs = vec![DatumEventEvidenceRefV1::DMonitorObservation {
                observation_id: "missing".into(),
            }];
        }
        assert!(matches!(
            p.monitoring(&o, &s, &binding()),
            Err(ProducerError::InvalidSource(_))
        ));
        let (_, events, _) =
            translate_monitoring(&occurrence(&s, 0, "canonical-check").await, &s, &binding())
                .unwrap();
        let mut invalid = events[0].clone();
        invalid.attributes = json!({"too_large": "x".repeat(16384)});
        assert!(matches!(
            p.record(invalid),
            Err(ProducerError::PayloadTooLarge { .. })
        ));
        let mut invalid = events[0].clone();
        invalid.kind = "Bad token".into();
        assert!(matches!(
            p.record(invalid),
            Err(ProducerError::InvalidCanonicalEvent(_))
        ));
        assert!(datum_event_store::list_events(&db.0, &Default::default())
            .unwrap()
            .is_empty());
    }
    #[tokio::test]
    async fn stored_before_alerts_and_alert_failure_is_retryable_without_event_loss() {
        let db = TestDb::new();
        let p = db.pipeline();
        let s = snapshot(-301, -301);
        let (_, events, _) =
            translate_monitoring(&occurrence(&s, 0, "ordered").await, &s, &binding()).unwrap();
        let connection = rusqlite::Connection::open(&db.0).unwrap();
        connection.execute_batch("CREATE TRIGGER fail_alert BEFORE INSERT ON datum_alerts BEGIN SELECT RAISE(ABORT,'test alert failure'); END;").unwrap();
        assert!(matches!(
            p.record(events[0].clone()),
            Err(ProducerError::AlertProcessing { .. })
        ));
        assert!(datum_event_store::get_event(&db.0, &events[0].event_id)
            .unwrap()
            .is_some());
        connection.execute_batch("DROP TRIGGER fail_alert").unwrap();
        let retry = p.record(events[0].clone()).unwrap();
        assert!(matches!(
            retry.admission,
            DatumEventAdmission::IdempotentReplay(_)
        ));
        assert!(matches!(
            retry.alert,
            Some(AlertProcessingOutcome::Created(_))
        ));
        connection.execute_batch("CREATE TRIGGER fail_event BEFORE INSERT ON datum_events BEGIN SELECT RAISE(ABORT,'test event failure'); END;").unwrap();
        let (_, events, _) =
            translate_monitoring(&occurrence(&s, 1, "not-admitted").await, &s, &binding()).unwrap();
        assert!(matches!(
            p.record(events[0].clone()),
            Err(ProducerError::EventStore { .. })
        ));
        assert!(datum_event_store::get_event(&db.0, &events[0].event_id)
            .unwrap()
            .is_none());
        assert_eq!(
            datum_alert_store::list_alerts(&db.0, &Default::default()).unwrap()[0].occurrence_count,
            1
        );
    }
    #[test]
    fn durable_path_has_no_memory_fallback_and_invalid_database_fails_closed() {
        let base = std::env::temp_dir();
        assert_eq!(
            resolve_database_path(None, &base).unwrap(),
            base.join(DEFAULT_EVENT_ALERT_DB_PATH)
        );
        for path in ["", " ", ":memory:", "file:any?mode=memory", " padded "] {
            assert!(matches!(
                resolve_database_path(Some(path), &base),
                Err(ProducerError::DatabasePath(_))
            ));
        }
        assert!(EventProducerPipeline::open(base).is_err());
    }
    #[test]
    fn live_acceptance_call_sites_follow_persist_and_do_not_turn_recording_errors_into_api_failure()
    {
        let src = include_str!("../api/ddeploy.rs");
        for (start, end) in [
            (
                "pub async fn post_accept_proposal(",
                "fn accept_error_to_api(",
            ),
            (
                "pub async fn post_accept_proposal_v2(",
                "fn accept_v2_error_to_api(",
            ),
        ] {
            let body = src.split(start).nth(1).unwrap().split(end).next().unwrap();
            let persist = body.find("persist_state(&db_path").unwrap();
            let emit = body.find("report_after_authority_commit(").unwrap();
            let response = body.find("Ok(JsonResponse(Accept").unwrap();
            assert!(persist < emit && emit < response);
            assert!(!body[emit..response].contains("map_err"));
        }
    }
    #[tokio::test]
    async fn duplicate_findings_count_bounds_and_scope_contradictions_are_rejected() {
        let s = snapshot(-301, -301);
        let mut o = occurrence(&s, 0, "duplicates").await;
        if let MonitoringCompletionResult::Evaluation(e) = &mut o.completion.result {
            e.findings.push(e.findings[0].clone());
        }
        assert!(matches!(
            translate_monitoring(&o, &s, &binding()),
            Err(ProducerError::InvalidSource(_))
        ));
        if let MonitoringCompletionResult::Evaluation(e) = &mut o.completion.result {
            e.findings = vec![e.findings[0].clone(); MAX_FINDINGS_PER_OCCURRENCE + 1];
        }
        assert!(matches!(
            translate_monitoring(&o, &s, &binding()),
            Err(ProducerError::InvalidSource(_))
        ));
        let mut o = occurrence(&s, 0, "scope").await;
        if let MonitoringCompletionResult::Evaluation(e) = &mut o.completion.result {
            e.findings[0].project_id = Some("wrong-project".into());
        }
        assert!(matches!(
            translate_monitoring(&o, &s, &binding()),
            Err(ProducerError::InvalidMonitoringResult(_))
        ));
        let mut o = occurrence(&s, 0, "hint").await;
        if let MonitoringCompletionResult::Evaluation(e) = &mut o.completion.result {
            e.findings[0].severity_hint =
                super::super::active_monitoring::MonitoringSeverityHint::Critical;
        }
        assert!(matches!(
            translate_monitoring(&o, &s, &binding()),
            Err(ProducerError::InvalidSource(_))
        ));
        let o = occurrence(&s, 0, "trusted-hint").await;
        let (_, events, _) = translate_monitoring(&o, &s, &binding()).unwrap();
        assert_eq!(events[0].severity, DatumEventSeverityV1::Warning);
        assert!(
            serde_json::to_vec(&events[0].attributes).unwrap().len()
                <= MAX_DATUM_EVENT_ATTRIBUTES_BYTES
        );
    }

    #[test]
    fn v2_acceptance_uses_immutable_reference_and_timestamp_and_is_historical_only() {
        let mut a = DDeployAcceptanceV2 {
            schema: super::super::ddeploy_v2::DDEPLOY_ACCEPTANCE_V2_SCHEMA.into(),
            acceptance_id: "accepted-v2".into(),
            proposal_id: "proposal-v2".into(),
            proposal_digest: format!("sha256:{}", "a".repeat(64)),
            project_id: "p".into(),
            application_id: "app".into(),
            accepted_at_utc: "2026-09-29T12:00:00-03:00".into(),
            accepted_by: "operator".into(),
            materialized_dmap_ref: super::super::canonical_dmap_v2::ContractRefV2 {
                id: "dmap-v2".into(),
                revision: 7,
                digest: format!("sha256:{}", "b".repeat(64)),
            },
            projection_ref: None,
        };
        let db = TestDb::new();
        let p = db.pipeline();
        let event = acceptance_event_v2(&a).unwrap();
        assert_eq!(event.occurred_at_utc, text(0));
        assert!(
            matches!(&event.evidence_refs[0],DatumEventEvidenceRefV1::DMap{reference} if reference.revision == 7)
        );
        assert_eq!(alert_policy(&event), AlertPolicy::HistoricalOnly);
        assert!(p.record(event.clone()).unwrap().alert.is_none());
        assert!(matches!(
            p.record(event).unwrap().admission,
            DatumEventAdmission::IdempotentReplay(_)
        ));
        a.schema = "unknown".into();
        assert!(matches!(
            acceptance_event_v2(&a),
            Err(ProducerError::InvalidSource(_))
        ));
    }

    #[tokio::test]
    async fn critical_four_recoveries_are_scoped_historical_idempotent_and_never_close() {
        use super::super::datum_alert::{DatumAlertActorTypeV1, DatumAlertStatusV1};
        for (bad, kind) in [
            (snapshot(-301, -301), "observation_stale"),
            (
                MonitoringEvidenceSnapshot::from_state(DMonitorStateV1::empty()).unwrap(),
                "observation_missing",
            ),
            (snapshot(31, 0), "observation_clock_skew"),
            (snapshot(1, 1), "evaluation_before_receipt"),
        ] {
            let db = TestDb::new();
            let p = db.pipeline();
            let opened = p
                .monitoring(&occurrence(&bad, 0, "active").await, &bad, &binding())
                .unwrap();
            let id = opened.events[0]
                .alert
                .as_ref()
                .unwrap()
                .alert()
                .alert_id
                .clone();
            assert_eq!(
                opened.events[0].alert.as_ref().unwrap().alert().status,
                DatumAlertStatusV1::Open
            );
            if kind == "observation_missing" || kind == "observation_clock_skew" {
                datum_alert_store::acknowledge_alert(
                    &db.0,
                    &id,
                    super::super::datum_alert::DatumAlertActorV1::system(),
                    None,
                )
                .unwrap();
            }
            if kind == "observation_clock_skew" {
                datum_alert_store::start_alert_work(
                    &db.0,
                    &id,
                    super::super::datum_alert::DatumAlertActorV1::system(),
                    None,
                )
                .unwrap();
            }
            let good = snapshot(0, 0);
            let o = occurrence(&good, 1, "clear").await;
            let cleared = p.monitoring(&o, &good, &binding()).unwrap();
            let clear = cleared
                .events
                .iter()
                .find(|e| event_of(e).event.kind == format!("{kind}_cleared"))
                .unwrap();
            assert!(matches!(
                clear.recovery,
                Some(datum_alert_store::ConditionRecoveryOutcome::Resolved(_))
            ));
            let clear_id = event_of(clear).event.event_id.clone();
            assert!(clear.alert.is_none());
            let alert = datum_alert_store::get_alert(&db.0, &id).unwrap().unwrap();
            assert_eq!(alert.status, DatumAlertStatusV1::Resolved);
            assert_eq!(alert.occurrence_count, 1); // clear is not a confirmation
            assert_eq!(
                alert.resolved_by.as_ref().unwrap().actor_type,
                DatumAlertActorTypeV1::System
            );
            let history = datum_alert_store::list_alert_transitions(&db.0, &id, 100).unwrap();
            assert_eq!(
                history.last().unwrap().related_event_id,
                Some(clear_id.clone())
            );
            assert_eq!(
                history.last().unwrap().reason.as_deref(),
                Some("monitoring condition cleared")
            );
            let retry = p.monitoring(&o, &good, &binding()).unwrap();
            assert!(retry.events.iter().any(|e| matches!(
                e.recovery,
                Some(datum_alert_store::ConditionRecoveryOutcome::AlreadyApplied)
            )));
            assert_eq!(
                datum_alert_store::list_alert_transitions(&db.0, &id, 100)
                    .unwrap()
                    .len(),
                history.len()
            );
            // Another normal evaluation is not another recovery storm.
            assert!(p
                .monitoring(
                    &occurrence(&good, 2, "later-normal").await,
                    &good,
                    &binding()
                )
                .unwrap()
                .events
                .is_empty());
            drop(p);
            let restarted = db.pipeline();
            assert!(datum_event_store::get_event(restarted.db_path(), &clear_id)
                .unwrap()
                .is_some());
            assert_eq!(
                datum_alert_store::get_alert(restarted.db_path(), &id)
                    .unwrap()
                    .unwrap()
                    .status,
                DatumAlertStatusV1::Resolved
            );
        }
    }

    #[tokio::test]
    async fn critical_cross_condition_recovery_does_not_assert_global_health() {
        use super::super::datum_alert::DatumAlertStatusV1;
        let db = TestDb::new();
        let p = db.pipeline();
        let bad = snapshot(-301, -301);
        let opened = p
            .monitoring(&occurrence(&bad, 0, "stale").await, &bad, &binding())
            .unwrap();
        let stale_id = opened.events[0]
            .alert
            .as_ref()
            .unwrap()
            .alert()
            .alert_id
            .clone();
        let skew = snapshot(31, 0);
        p.monitoring(&occurrence(&skew, 1, "now-skew").await, &skew, &binding())
            .unwrap();
        let alerts = datum_alert_store::list_alerts(&db.0, &Default::default()).unwrap();
        assert_eq!(
            alerts
                .iter()
                .find(|a| a.kind == "observation_stale")
                .unwrap()
                .status,
            DatumAlertStatusV1::Resolved
        );
        assert_eq!(
            alerts
                .iter()
                .find(|a| a.kind == "observation_clock_skew")
                .unwrap()
                .status,
            DatumAlertStatusV1::Open
        );
        // Sane skew but stale ages clears skew only, independently reopening/confirming stale if applicable.
        let stale = snapshot(-301, -301);
        p.monitoring(
            &occurrence(&stale, 2, "still-stale").await,
            &stale,
            &binding(),
        )
        .unwrap();
        assert_eq!(
            datum_alert_store::list_alerts(&db.0, &Default::default())
                .unwrap()
                .iter()
                .find(|a| a.kind == "observation_clock_skew")
                .unwrap()
                .status,
            DatumAlertStatusV1::Resolved
        );
        assert_eq!(
            datum_alert_store::get_alert(&db.0, &stale_id)
                .unwrap()
                .unwrap()
                .occurrence_count,
            2
        );
    }

    #[tokio::test]
    async fn critical_clear_cannot_modify_closed_or_newer_recurrence_and_requires_admission() {
        use super::super::datum_alert::{DatumAlertActorV1, DatumAlertStatusV1};
        let db = TestDb::new();
        let p = db.pipeline();
        let s = snapshot(-301, -301);
        let opened = p
            .monitoring(&occurrence(&s, 0, "active").await, &s, &binding())
            .unwrap();
        let id = opened.events[0]
            .alert
            .as_ref()
            .unwrap()
            .alert()
            .alert_id
            .clone();
        let good = snapshot(0, 0);
        let o = occurrence(&good, 1, "clear").await;
        let output = p.monitoring(&o, &good, &binding()).unwrap();
        let stored = event_of(&output.events[0]).clone();
        let mut unpersisted = stored.clone();
        unpersisted.event.event_id = "never-admitted".into();
        assert!(datum_alert_store::resolve_condition_from_event(
            &db.0,
            &unpersisted,
            "observation_stale"
        )
        .is_err());
        let later = (Utc::now() - time(0)).num_seconds() + 3600;
        let again = snapshot(later - 301, later - 301);
        p.monitoring(
            &occurrence(&again, later, "recurrence").await,
            &again,
            &binding(),
        )
        .unwrap();
        let before = datum_alert_store::get_alert(&db.0, &id).unwrap().unwrap();
        assert_eq!(before.status, DatumAlertStatusV1::Open);
        assert_eq!(
            datum_alert_store::resolve_condition_from_event(&db.0, &stored, "observation_stale")
                .unwrap(),
            datum_alert_store::ConditionRecoveryOutcome::AlreadyApplied
        );
        assert_eq!(
            datum_alert_store::get_alert(&db.0, &id).unwrap().unwrap(),
            before
        );
        datum_alert_store::resolve_alert(&db.0, &id, DatumAlertActorV1::system(), None).unwrap();
        datum_alert_store::close_alert(&db.0, &id, DatumAlertActorV1::system(), None).unwrap();
        let before = datum_alert_store::get_alert(&db.0, &id).unwrap().unwrap();
        let output = p
            .monitoring(
                &occurrence(&good, 2, "closed-clear").await,
                &good,
                &binding(),
            )
            .unwrap();
        assert!(output.events.is_empty());
        assert!(output.recovery_noops.iter().any(|(_, o)| matches!(
            o,
            datum_alert_store::ConditionRecoveryOutcome::AlreadyClosed(_)
        )));
        assert_eq!(
            datum_alert_store::get_alert(&db.0, &id).unwrap().unwrap(),
            before
        );
    }

    #[tokio::test]
    async fn critical_recovery_event_failure_and_transition_failure_are_safe_and_retryable() {
        let db = TestDb::new();
        let p = db.pipeline();
        let s = snapshot(-301, -301);
        let opened = p
            .monitoring(&occurrence(&s, 0, "active").await, &s, &binding())
            .unwrap();
        let id = opened.events[0]
            .alert
            .as_ref()
            .unwrap()
            .alert()
            .alert_id
            .clone();
        let good = snapshot(0, 0);
        let o = occurrence(&good, 1, "clear").await;
        let connection = rusqlite::Connection::open(&db.0).unwrap();
        connection.execute_batch("CREATE TRIGGER fail_clear_event BEFORE INSERT ON datum_events WHEN NEW.kind LIKE '%_cleared' BEGIN SELECT RAISE(ABORT,'test clear failure'); END;").unwrap();
        assert!(matches!(
            p.monitoring(&o, &good, &binding()),
            Err(ProducerError::EventStore { .. })
        ));
        assert_eq!(
            datum_alert_store::get_alert(&db.0, &id)
                .unwrap()
                .unwrap()
                .status,
            super::super::datum_alert::DatumAlertStatusV1::Open
        );
        connection.execute_batch("DROP TRIGGER fail_clear_event; CREATE TRIGGER fail_clear_transition BEFORE INSERT ON datum_alert_transitions WHEN NEW.to_status='resolved' BEGIN SELECT RAISE(ABORT,'test transition failure'); END;").unwrap();
        assert!(matches!(
            p.monitoring(&o, &good, &binding()),
            Err(ProducerError::AlertProcessing { .. })
        ));
        assert_eq!(
            datum_alert_store::get_alert(&db.0, &id)
                .unwrap()
                .unwrap()
                .status,
            super::super::datum_alert::DatumAlertStatusV1::Open
        );
        connection
            .execute_batch("DROP TRIGGER fail_clear_transition;")
            .unwrap();
        let output = p.monitoring(&o, &good, &binding()).unwrap();
        assert!(matches!(
            output.events[0].admission,
            DatumEventAdmission::IdempotentReplay(_)
        ));
        assert!(matches!(
            output.events[0].recovery,
            Some(datum_alert_store::ConditionRecoveryOutcome::Resolved(_))
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn critical_sqlite_busy_blocking_work_does_not_pin_async_worker() {
        let db = TestDb::new();
        let p = db.pipeline();
        let s = snapshot(-301, -301);
        let (_, events, _) =
            translate_monitoring(&occurrence(&s, 0, "busy").await, &s, &binding()).unwrap();
        let connection = rusqlite::Connection::open(&db.0).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE;").unwrap();
        let (entered, ready) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(owned_post_commit_work(
            PostAuthorityEventContext::new("p", "app", "accept", "test"),
            move || {
                entered.send(()).unwrap();
                p.record(events[0].clone())
            },
        ));
        ready.await.unwrap();
        // Same one-thread async runtime still schedules this sibling while SQLite cannot write.
        let (progress, progressed) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            progress.send(()).unwrap();
        });
        progressed.await.unwrap();
        assert!(!handle.is_finished());
        connection.execute_batch("ROLLBACK;").unwrap();
        assert!(handle.await.unwrap().is_ok());
    }

    #[test]
    fn windows_paths_and_contextual_diagnostics_are_bounded_and_explicit() {
        let db = TestDb::new();
        let base = db.0.parent().unwrap();
        for bad in [
            "",
            " padded ",
            ":memory:",
            "file:",
            "file::memory:?cache=shared",
            "C:events.db",
            "c:",
        ] {
            assert!(resolve_database_path(Some(bad), base).is_err());
        }
        assert!(resolve_database_path(Some("normal.db"), base)
            .unwrap()
            .is_absolute());
        #[cfg(windows)]
        assert!(resolve_database_path(Some("C:\\data\\events.db"), base)
            .unwrap()
            .is_absolute());
        let nested = base
            .join(format!("phase126-dir-{}", uuid::Uuid::new_v4()))
            .join("child")
            .join("events.db");
        let p = EventProducerPipeline::open(nested.clone()).unwrap();
        drop(p);
        std::fs::remove_file(&nested).unwrap();
        std::fs::remove_dir(nested.parent().unwrap()).unwrap();
        std::fs::remove_dir(nested.parent().unwrap().parent().unwrap()).unwrap();
        let context = PostAuthorityEventContext::new(&"p".repeat(1000), "app", "accept", "v1");
        let output = context.diagnostic(&ProducerError::Translation("secret credentials".into()));
        assert_eq!(output["project_id"].as_str().unwrap().len(), 256);
        assert_eq!(output["application_id"], "app");
        assert_eq!(output["acceptance_id"], "accept");
        assert_eq!(output["producer_kind"], "v1");
        assert_eq!(output["error_class"], "translation");
        assert!(!output.to_string().contains("secret"));
    }

    #[test]
    fn process_configuration_is_stable_after_environment_mutation_in_isolated_process() {
        let a = TestDb::new();
        let b = TestDb::new();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "core::event_producers::tests::runtime_environment_child",
                "--ignored",
                "--nocapture",
            ])
            .env(EVENT_ALERT_DB_PATH_ENV, &a.0)
            .env("PHASE126_CHILD_DB_B", &b.0)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(a.0.exists());
        assert!(!b.0.exists());
    }
    #[tokio::test]
    #[ignore = "executed by isolated process_configuration test"]
    async fn runtime_environment_child() {
        let a = configured_database_path().unwrap();
        let runtime = process_runtime();
        std::env::set_var(
            EVENT_ALERT_DB_PATH_ENV,
            std::env::var("PHASE126_CHILD_DB_B").unwrap(),
        );
        let s = snapshot(-301, -301);
        let (_, events, _) =
            translate_monitoring(&occurrence(&s, 0, "environment").await, &s, &binding()).unwrap();
        runtime
            .record_after_commit(
                PostAuthorityEventContext::new("p", "app", "a", "test"),
                Ok(events[0].clone()),
            )
            .await;
        assert!(datum_event_store::get_event(&a, &events[0].event_id)
            .unwrap()
            .is_some());
        assert_eq!(EventProducerPipeline::configured().unwrap().db_path(), a);
    }

    #[tokio::test]
    async fn arbitrary_custom_check_cannot_fabricate_missing_or_stale_evidence() {
        let fresh = snapshot(0, 0);
        let stale = snapshot(-301, -301);
        let forged = occurrence(&stale, 0, "forged-stale").await;
        assert!(translate_monitoring(&forged, &fresh, &binding()).is_err());
        let missing = MonitoringEvidenceSnapshot::from_state(DMonitorStateV1::empty()).unwrap();
        let forged = occurrence(&missing, 0, "forged-missing").await;
        assert!(translate_monitoring(&forged, &fresh, &binding()).is_err());
        let mut forged = occurrence(&stale, 0, "wrong-check").await;
        if let MonitoringCompletionResult::Evaluation(e) = &mut forged.completion.result {
            e.check_id =
                super::super::active_monitoring::MonitoringCheckId::new("custom_check").unwrap();
            forged.completion.check_id = e.check_id.clone();
        }
        assert!(translate_monitoring(&forged, &stale, &binding()).is_err());
        assert!(
            translate_monitoring(&occurrence(&stale, 0, "trusted").await, &stale, &binding())
                .is_ok()
        );
        let db = TestDb::new();
        let p = db.pipeline();
        let no_prior = p
            .monitoring(&occurrence(&fresh, 0, "no-prior").await, &fresh, &binding())
            .unwrap();
        assert!(no_prior.events.is_empty());
        assert!(no_prior
            .recovery_noops
            .iter()
            .all(|(_, o)| *o == datum_alert_store::ConditionRecoveryOutcome::NoMatchingAlert));
    }

    #[tokio::test]
    async fn real_scheduler_output_is_consumable_without_detached_worker_and_shutdown_is_clean() {
        let s = snapshot(-301, -301);
        let db = TestDb::new();
        let p = db.pipeline();
        struct FixedClock;
        impl super::super::active_monitoring::MonitoringClock for FixedClock {
            fn now(&self) -> super::super::active_monitoring::MonitoringTime {
                super::super::active_monitoring::MonitoringTime {
                    schedule_time: std::time::Duration::ZERO,
                    evaluated_at_utc: time(0),
                }
            }
        }
        let scheduler = MonitoringScheduler::new_with_clock(
            MonitoringSchedulerConfig::default(),
            vec![check(s.clone()).registration(true).unwrap()],
            Arc::new(FixedClock),
        )
        .unwrap();
        let mut handle = scheduler.start().unwrap();
        let completion = handle.recv().await.unwrap();
        let o = MonitoringOccurrence::from_completion(completion);
        let out = p.monitoring(&o, &s, &binding()).unwrap();
        assert_eq!(out.events.len(), 1);
        report_scheduler_output_loss(&scheduler.status().unwrap());
        scheduler.shutdown().await;
        assert!(handle.recv().await.is_none());
    }
}
