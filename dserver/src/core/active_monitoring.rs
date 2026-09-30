//! Phase 125's internal, time-driven active monitoring scheduler.
//!
//! This module evaluates observations. It does not own, repair, or mutate the
//! operational state those observations describe. A finding is not a
//! DatumEvent, Alert, or infrastructure-health verdict.

use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, RwLock, TryLockError},
    time::Duration,
};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use tokio::{task::JoinHandle, time::MissedTickBehavior};

use crate::core::{
    datum_event::{DatumEventEvidenceRefV1, DatumEventSeverityV1, DatumEventSubjectV1},
    dmonitor_freshness::{
        evaluate_freshness, validate_freshness_policy, DMonitorFreshnessPolicyV1,
        DMonitorFreshnessV1,
    },
    dmonitor_state::{
        stream_key, validate_persisted_dmonitor_state_integrity, DMonitorObservationRecordV1,
        DMonitorStateV1,
    },
};

pub const DEFAULT_CHECK_CADENCE: Duration = Duration::from_secs(30);
pub const DEFAULT_SCHEDULER_POLL_INTERVAL: Duration = Duration::from_secs(1);
pub const DEFAULT_MAX_CONCURRENT_CHECKS: usize = 4;
pub const DEFAULT_OUTPUT_CAPACITY: usize = 64;
const MONITORING_SUBJECT_KIND: &str = "dmonitor_stream";
const MAX_MONITORING_STATUS_MESSAGE_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct MonitoringCheckId(String);

impl MonitoringCheckId {
    pub fn new(value: impl Into<String>) -> Result<Self, MonitoringConfigError> {
        let value = value.into();
        if value.trim().is_empty() || value.trim() != value {
            return Err(MonitoringConfigError::InvalidCheckId(
                "check id must be non-empty and trimmed".into(),
            ));
        }
        Ok(Self(value))
    }

    pub fn validate(&self) -> Result<(), MonitoringConfigError> {
        Self::new(self.0.clone()).map(|_| ())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for MonitoringCheckId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitoringSchedulerConfig {
    pub poll_interval: Duration,
    pub max_concurrent_checks: usize,
    pub output_capacity: usize,
}

impl Default for MonitoringSchedulerConfig {
    fn default() -> Self {
        Self {
            poll_interval: DEFAULT_SCHEDULER_POLL_INTERVAL,
            max_concurrent_checks: DEFAULT_MAX_CONCURRENT_CHECKS,
            output_capacity: DEFAULT_OUTPUT_CAPACITY,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonitoringConfigError {
    ZeroPollInterval,
    ZeroConcurrency,
    ZeroOutputCapacity,
    ZeroCheckCadence(MonitoringCheckId),
    InvalidCheckId(String),
    DuplicateCheckId(MonitoringCheckId),
    TimeOverflow(MonitoringCheckId),
}

impl std::fmt::Display for MonitoringConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroPollInterval => write!(f, "scheduler poll interval must be non-zero"),
            Self::ZeroConcurrency => write!(f, "scheduler concurrency must be non-zero"),
            Self::ZeroOutputCapacity => write!(f, "output capacity must be non-zero"),
            Self::ZeroCheckCadence(id) => {
                write!(f, "check '{}' cadence must be non-zero", id.as_str())
            }
            Self::InvalidCheckId(message) => write!(f, "{message}"),
            Self::DuplicateCheckId(id) => {
                write!(f, "duplicate monitoring check id '{}'", id.as_str())
            }
            Self::TimeOverflow(id) => write!(f, "cadence time overflow for '{}'", id.as_str()),
        }
    }
}
impl std::error::Error for MonitoringConfigError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitoringOutcome {
    Satisfied,
    Finding,
    Indeterminate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitoringSeverityHint {
    Info,
    Warning,
    High,
    Critical,
}

impl From<DatumEventSeverityV1> for MonitoringSeverityHint {
    fn from(value: DatumEventSeverityV1) -> Self {
        match value {
            DatumEventSeverityV1::Info => Self::Info,
            DatumEventSeverityV1::Warning => Self::Warning,
            DatumEventSeverityV1::High => Self::High,
            DatumEventSeverityV1::Critical => Self::Critical,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitoringFinding {
    pub finding_code: String,
    /// A Phase 126 translation hint only; never canonical alert authority.
    pub severity_hint: MonitoringSeverityHint,
    /// Reuses the DatumEvent subject contract rather than inventing identity.
    pub subject: DatumEventSubjectV1,
    pub project_id: Option<String>,
    pub application_id: Option<String>,
    pub observed_value: Option<Value>,
    pub threshold: Option<Value>,
    pub evidence_refs: Vec<DatumEventEvidenceRefV1>,
    pub attributes: Value,
}

impl MonitoringFinding {
    pub fn new(
        finding_code: impl Into<String>,
        severity_hint: MonitoringSeverityHint,
        subject: DatumEventSubjectV1,
        observed_value: Option<Value>,
        threshold: Option<Value>,
        evidence_refs: Vec<DatumEventEvidenceRefV1>,
        attributes: Value,
    ) -> Result<Self, MonitoringCheckError> {
        let finding_code = finding_code.into();
        if finding_code.trim().is_empty() || !attributes.is_object() {
            return Err(MonitoringCheckError::InvalidResult(
                "finding code must be non-empty and attributes must be an object".into(),
            ));
        }
        Ok(Self {
            finding_code,
            severity_hint,
            project_id: subject.project_id().map(ToOwned::to_owned),
            application_id: subject.application_id().map(ToOwned::to_owned),
            subject,
            observed_value,
            threshold,
            evidence_refs,
            attributes,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitoringEvaluation {
    pub check_id: MonitoringCheckId,
    pub evaluated_at_utc: String,
    pub outcome: MonitoringOutcome,
    pub findings: Vec<MonitoringFinding>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MonitoringCheckResult {
    pub outcome: MonitoringOutcome,
    pub findings: Vec<MonitoringFinding>,
}

impl MonitoringCheckResult {
    pub fn new(
        outcome: MonitoringOutcome,
        findings: Vec<MonitoringFinding>,
    ) -> Result<Self, MonitoringCheckError> {
        if (matches!(outcome, MonitoringOutcome::Satisfied) && !findings.is_empty())
            || (matches!(outcome, MonitoringOutcome::Finding) && findings.is_empty())
        {
            return Err(MonitoringCheckError::InvalidResult(
                "Satisfied requires no findings; Finding requires at least one".into(),
            ));
        }
        Ok(Self { outcome, findings })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "message")]
pub enum MonitoringCheckError {
    EvidenceUnavailable(String),
    InvalidEvidence(String),
    EvaluationFailed(String),
    InvalidResult(String),
}

impl std::fmt::Display for MonitoringCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EvidenceUnavailable(s) => write!(f, "evidence unavailable: {s}"),
            Self::InvalidEvidence(s) => write!(f, "invalid evidence: {s}"),
            Self::EvaluationFailed(s) => write!(f, "evaluation failed: {s}"),
            Self::InvalidResult(s) => write!(f, "invalid check result: {s}"),
        }
    }
}
impl std::error::Error for MonitoringCheckError {}

impl MonitoringFinding {
    pub fn validate(&self) -> Result<(), MonitoringCheckError> {
        let invalid = |s: &str| MonitoringCheckError::InvalidResult(s.into());
        if self.finding_code.trim().is_empty() || self.finding_code.trim() != self.finding_code {
            return Err(invalid("finding code must be nonblank and trimmed"));
        }
        if !self.attributes.is_object() {
            return Err(invalid("attributes must be a JSON object"));
        }
        if self.project_id.as_deref() != self.subject.project_id()
            || self.application_id.as_deref() != self.subject.application_id()
        {
            return Err(invalid("finding scope contradicts canonical subject scope"));
        }
        // Canonical subject variants carry identity strings; reject blank
        // identities without interpreting them as health/authorization.
        let subject = serde_json::to_value(&self.subject)
            .map_err(|e| MonitoringCheckError::InvalidResult(e.to_string()))?;
        if subject.as_object().is_none_or(|fields| {
            fields
                .values()
                .any(|v| v.as_str().is_some_and(|id| id.trim().is_empty()))
        }) {
            return Err(invalid("canonical subject identity must be nonblank"));
        }
        Ok(())
    }
}
impl MonitoringCheckResult {
    pub fn validate(&self) -> Result<(), MonitoringCheckError> {
        Self::validate_parts(&self.outcome, &self.findings)
    }

    fn validate_parts(
        outcome: &MonitoringOutcome,
        findings: &[MonitoringFinding],
    ) -> Result<(), MonitoringCheckError> {
        if (*outcome == MonitoringOutcome::Satisfied && !findings.is_empty())
            || (*outcome == MonitoringOutcome::Finding && findings.is_empty())
        {
            return Err(MonitoringCheckError::InvalidResult(
                "outcome/findings contradiction".into(),
            ));
        }
        for finding in findings {
            finding.validate()?;
        }
        Ok(())
    }
}
impl MonitoringEvaluation {
    pub fn validate(&self) -> Result<(), MonitoringCheckError> {
        self.check_id
            .validate()
            .map_err(|e| MonitoringCheckError::InvalidResult(e.to_string()))?;
        let parsed = DateTime::parse_from_rfc3339(&self.evaluated_at_utc)
            .map_err(|e| MonitoringCheckError::InvalidResult(e.to_string()))?;
        if format_utc(parsed.with_timezone(&Utc)) != self.evaluated_at_utc {
            return Err(MonitoringCheckError::InvalidResult(
                "evaluation timestamp must be canonical UTC".into(),
            ));
        }
        MonitoringCheckResult::validate_parts(&self.outcome, &self.findings)
    }
}
impl MonitoringCheckError {
    fn bounded(self) -> Self {
        match self {
            Self::EvidenceUnavailable(s) => Self::EvidenceUnavailable(bounded_message(s)),
            Self::InvalidEvidence(s) => Self::InvalidEvidence(bounded_message(s)),
            Self::EvaluationFailed(s) => Self::EvaluationFailed(bounded_message(s)),
            Self::InvalidResult(s) => Self::InvalidResult(bounded_message(s)),
        }
    }
}

pub type MonitoringCheckFuture<'a> =
    Pin<Box<dyn Future<Output = Result<MonitoringCheckResult, MonitoringCheckError>> + Send + 'a>>;

/// Checks must be deterministic for the same evidence and explicit time,
/// must not mutate operational state, and must not block a Tokio worker.
/// Checks must cooperate with cancellation. Any child work must be owned and
/// cancelled/joined by the check; detached or uncancellable blocking work is
/// outside the scheduler's shutdown guarantee. Panics are isolated.
pub trait MonitoringCheck: Send + Sync + 'static {
    fn evaluate(&self, evaluated_at_utc: DateTime<Utc>) -> MonitoringCheckFuture<'_>;
}

pub struct MonitoringCheckRegistration {
    pub check_id: MonitoringCheckId,
    pub cadence: Duration,
    pub enabled: bool,
    pub check: Arc<dyn MonitoringCheck>,
}

impl MonitoringCheckRegistration {
    pub fn new(
        check_id: MonitoringCheckId,
        cadence: Duration,
        enabled: bool,
        check: Arc<dyn MonitoringCheck>,
    ) -> Self {
        Self {
            check_id,
            cadence,
            enabled,
            check,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitoringTime {
    pub schedule_time: Duration,
    pub evaluated_at_utc: DateTime<Utc>,
}
/// Scheduler clock boundary. Evaluation checks never read this clock.
pub trait MonitoringClock: Send + Sync + 'static {
    fn now(&self) -> MonitoringTime;
}
pub struct SystemMonitoringClock {
    origin: tokio::time::Instant,
}
impl Default for SystemMonitoringClock {
    fn default() -> Self {
        Self {
            origin: tokio::time::Instant::now(),
        }
    }
}
impl MonitoringClock for SystemMonitoringClock {
    fn now(&self) -> MonitoringTime {
        MonitoringTime {
            schedule_time: self.origin.elapsed(),
            evaluated_at_utc: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum MonitoringCompletionResult {
    Evaluation(MonitoringEvaluation),
    Error(MonitoringCheckError),
    Panicked,
    Cancelled,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitoringCompletion {
    pub check_id: MonitoringCheckId,
    pub evaluated_at_utc: String,
    pub completed_at_utc: String,
    pub result: MonitoringCompletionResult,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum MonitoringLastEvaluation {
    Completed {
        outcome: MonitoringOutcome,
        finding_count: usize,
    },
    Failed {
        error: MonitoringCheckError,
    },
    Panicked,
    Cancelled,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitoringSchedulerAction {
    Started,
    SkippedOverlap,
    DeferredAtCapacity,
    OutputDroppedFull,
    OutputReceiverClosed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitoringLifecycle {
    Running,
    Stopping,
    Stopped,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitoringCheckStatus {
    pub check_id: MonitoringCheckId,
    pub enabled: bool,
    pub cadence: Duration,
    pub running: bool,
    pub last_started_at_utc: Option<String>,
    pub last_completed_at_utc: Option<String>,
    pub last_evaluation_result: Option<MonitoringLastEvaluation>,
    pub last_scheduler_action: Option<MonitoringSchedulerAction>,
    /// Monotonic deadline, not an estimate of future UTC.
    pub next_due_schedule_time: Option<Duration>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitoringSchedulerStatus {
    pub lifecycle: MonitoringLifecycle,
    pub checks: Vec<MonitoringCheckStatus>,
    pub completed_count: u64,
    pub dropped_output_full: u64,
    pub dropped_output_closed: u64,
    pub driver_polls: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonitoringSchedulerError {
    InvalidConfiguration(MonitoringConfigError),
    ScheduleMovedBackwards,
    Shutdown,
    AlreadyStarted,
    OutputReceiverTaken,
    NoRuntime,
    Poisoned,
}
impl std::fmt::Display for MonitoringSchedulerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfiguration(e) => write!(f, "{e}"),
            Self::ScheduleMovedBackwards => write!(f, "monotonic time moved backwards"),
            Self::Shutdown => write!(f, "scheduler stopping or stopped"),
            Self::AlreadyStarted => write!(f, "driver already started"),
            Self::OutputReceiverTaken => write!(f, "output receiver already taken"),
            Self::NoRuntime => write!(f, "execution requires a live Tokio runtime"),
            Self::Poisoned => write!(f, "scheduler lock poisoned"),
        }
    }
}
impl std::error::Error for MonitoringSchedulerError {}

struct CheckRuntime {
    registration: MonitoringCheckRegistration,
    next_due: Option<Duration>,
    running: bool,
    last_started: Option<DateTime<Utc>>,
    last_completed: Option<DateTime<Utc>>,
    last_result: Option<MonitoringLastEvaluation>,
    last_action: Option<MonitoringSchedulerAction>,
}
struct OwnedExecution {
    abort: tokio::task::AbortHandle,
    supervisor: JoinHandle<()>,
}
struct SchedulerState {
    checks: Vec<CheckRuntime>,
    cursor: usize,
    last_schedule: Option<Duration>,
    lifecycle: MonitoringLifecycle,
    executions: Vec<OwnedExecution>,
    driver: Option<JoinHandle<()>>,
    background_started: bool,
    runtime: Option<tokio::runtime::Handle>,
    output: Option<tokio::sync::mpsc::Sender<MonitoringCompletion>>,
    completed_count: u64,
    dropped_full: u64,
    dropped_closed: u64,
    driver_polls: u64,
}
struct SchedulerInner {
    config: MonitoringSchedulerConfig,
    clock: Arc<dyn MonitoringClock>,
    state: Mutex<SchedulerState>,
    lifecycle: tokio::sync::watch::Sender<MonitoringLifecycle>,
    completed: tokio::sync::watch::Sender<u64>,
    polled: tokio::sync::watch::Sender<u64>,
}
/// Tasks reference inner state, never this owner. Last-owner Drop initiates
/// abort/cleanup even in manual mode.
pub struct MonitoringScheduler {
    inner: Arc<SchedulerInner>,
    receiver: Mutex<Option<tokio::sync::mpsc::Receiver<MonitoringCompletion>>>,
}
impl MonitoringScheduler {
    pub fn new(
        config: MonitoringSchedulerConfig,
        checks: Vec<MonitoringCheckRegistration>,
    ) -> Result<Arc<Self>, MonitoringSchedulerError> {
        Self::new_with_clock(config, checks, Arc::new(SystemMonitoringClock::default()))
    }
    pub fn new_with_clock(
        config: MonitoringSchedulerConfig,
        registrations: Vec<MonitoringCheckRegistration>,
        clock: Arc<dyn MonitoringClock>,
    ) -> Result<Arc<Self>, MonitoringSchedulerError> {
        let invalid = MonitoringSchedulerError::InvalidConfiguration;
        if config.poll_interval.is_zero() {
            return Err(invalid(MonitoringConfigError::ZeroPollInterval));
        }
        if config.max_concurrent_checks == 0 {
            return Err(invalid(MonitoringConfigError::ZeroConcurrency));
        }
        if config.output_capacity == 0 {
            return Err(invalid(MonitoringConfigError::ZeroOutputCapacity));
        }
        let initial = clock.now().schedule_time;
        let mut ids = std::collections::BTreeSet::new();
        let mut checks = Vec::new();
        for registration in registrations {
            registration.check_id.validate().map_err(invalid)?;
            if registration.cadence.is_zero() {
                return Err(invalid(MonitoringConfigError::ZeroCheckCadence(
                    registration.check_id,
                )));
            }
            if !ids.insert(registration.check_id.clone()) {
                return Err(invalid(MonitoringConfigError::DuplicateCheckId(
                    registration.check_id,
                )));
            }
            checked_next_due(initial, registration.cadence, &registration.check_id)?;
            checks.push(CheckRuntime {
                next_due: registration.enabled.then_some(initial),
                registration,
                running: false,
                last_started: None,
                last_completed: None,
                last_result: None,
                last_action: None,
            });
        }
        checks.sort_by(|a, b| a.registration.check_id.cmp(&b.registration.check_id));
        let (output, receiver) = tokio::sync::mpsc::channel(config.output_capacity);
        let (lifecycle, _) = tokio::sync::watch::channel(MonitoringLifecycle::Running);
        let (completed, _) = tokio::sync::watch::channel(0);
        let (polled, _) = tokio::sync::watch::channel(0);
        Ok(Arc::new(Self {
            inner: Arc::new(SchedulerInner {
                config,
                clock,
                lifecycle,
                completed,
                polled,
                state: Mutex::new(SchedulerState {
                    checks,
                    cursor: 0,
                    last_schedule: None,
                    lifecycle: MonitoringLifecycle::Running,
                    executions: Vec::new(),
                    driver: None,
                    background_started: false,
                    runtime: None,
                    output: Some(output),
                    completed_count: 0,
                    dropped_full: 0,
                    dropped_closed: 0,
                    driver_polls: 0,
                }),
            }),
            receiver: Mutex::new(Some(receiver)),
        }))
    }
    pub fn status(&self) -> Result<MonitoringSchedulerStatus, MonitoringSchedulerError> {
        let s = self
            .inner
            .state
            .lock()
            .map_err(|_| MonitoringSchedulerError::Poisoned)?;
        Ok(MonitoringSchedulerStatus {
            lifecycle: s.lifecycle,
            checks: s
                .checks
                .iter()
                .map(|r| MonitoringCheckStatus {
                    check_id: r.registration.check_id.clone(),
                    enabled: r.registration.enabled,
                    cadence: r.registration.cadence,
                    running: r.running,
                    last_started_at_utc: r.last_started.map(format_utc),
                    last_completed_at_utc: r.last_completed.map(format_utc),
                    last_evaluation_result: r.last_result.clone(),
                    last_scheduler_action: r.last_action,
                    next_due_schedule_time: r.next_due,
                })
                .collect(),
            completed_count: s.completed_count,
            dropped_output_full: s.dropped_full,
            dropped_output_closed: s.dropped_closed,
            driver_polls: s.driver_polls,
        })
    }
    /// Manual collection uses the same bounded channel as background mode.
    pub fn take_output_receiver(
        &self,
    ) -> Result<tokio::sync::mpsc::Receiver<MonitoringCompletion>, MonitoringSchedulerError> {
        self.receiver
            .lock()
            .map_err(|_| MonitoringSchedulerError::Poisoned)?
            .take()
            .ok_or(MonitoringSchedulerError::OutputReceiverTaken)
    }
    /// Dispatch only. Owned supervisors join and collect completions independently
    /// of future ticks, including while UTC moves backward.
    pub fn run_due_checks_at(
        &self,
        schedule_time: Duration,
        evaluated_at_utc: DateTime<Utc>,
    ) -> Result<usize, MonitoringSchedulerError> {
        dispatch_due(&self.inner, schedule_time, evaluated_at_utc)
    }
    pub fn start(self: &Arc<Self>) -> Result<MonitoringSchedulerTask, MonitoringSchedulerError> {
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| MonitoringSchedulerError::NoRuntime)?;
        let mut receiver = self
            .receiver
            .lock()
            .map_err(|_| MonitoringSchedulerError::Poisoned)?;
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| MonitoringSchedulerError::Poisoned)?;
        if state.lifecycle != MonitoringLifecycle::Running {
            return Err(MonitoringSchedulerError::Shutdown);
        }
        if state.background_started {
            return Err(MonitoringSchedulerError::AlreadyStarted);
        }
        let receiver = receiver
            .take()
            .ok_or(MonitoringSchedulerError::OutputReceiverTaken)?;
        state.background_started = true;
        state.runtime = Some(runtime.clone());
        let inner = self.inner.clone();
        let mut lifecycle = inner.lifecycle.subscribe();
        let mut interval = tokio::time::interval(inner.config.poll_interval);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        state.driver = Some(runtime.spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    changed = lifecycle.changed() => {
                        if changed.is_err() || *lifecycle.borrow() != MonitoringLifecycle::Running { break; }
                    }
                    _ = interval.tick() => {
                        let time = inner.clock.now();
                        if dispatch_due(&inner, time.schedule_time, time.evaluated_at_utc).is_err() {
                            initiate_shutdown(&inner);
                            break;
                        }
                        let mut s = inner.state.lock().unwrap_or_else(|p| p.into_inner());
                        s.driver_polls = s.driver_polls.saturating_add(1);
                        inner.polled.send_replace(s.driver_polls);
                    }
                }
            }
        }));
        Ok(MonitoringSchedulerTask {
            scheduler: self.clone(),
            receiver,
        })
    }
    pub fn request_shutdown(&self) {
        initiate_shutdown(&self.inner);
    }
    /// All callers await the shared Stopped barrier, independent of cancellation
    /// of any one caller. Runtime must stay alive; custom checks must cooperate
    /// with task cancellation and own/cancel any child work.
    pub async fn shutdown(&self) {
        let mut lifecycle = self.inner.lifecycle.subscribe();
        self.request_shutdown();
        loop {
            if *lifecycle.borrow_and_update() == MonitoringLifecycle::Stopped {
                return;
            }
            if lifecycle.changed().await.is_err() {
                return;
            }
        }
    }
}
impl Drop for MonitoringScheduler {
    fn drop(&mut self) {
        initiate_shutdown(&self.inner);
    }
}
pub struct MonitoringSchedulerTask {
    scheduler: Arc<MonitoringScheduler>,
    pub receiver: tokio::sync::mpsc::Receiver<MonitoringCompletion>,
}
impl MonitoringSchedulerTask {
    pub async fn recv(&mut self) -> Option<MonitoringCompletion> {
        self.receiver.recv().await
    }
    pub async fn shutdown(self) {
        self.scheduler.shutdown().await;
    }
}
impl Drop for MonitoringSchedulerTask {
    fn drop(&mut self) {
        self.scheduler.request_shutdown();
    }
}

fn dispatch_due(
    inner: &Arc<SchedulerInner>,
    now: Duration,
    evaluated_at: DateTime<Utc>,
) -> Result<usize, MonitoringSchedulerError> {
    let mut state = inner
        .state
        .lock()
        .map_err(|_| MonitoringSchedulerError::Poisoned)?;
    if state.lifecycle != MonitoringLifecycle::Running {
        return Err(MonitoringSchedulerError::Shutdown);
    }
    if state.last_schedule.is_some_and(|previous| now < previous) {
        return Err(MonitoringSchedulerError::ScheduleMovedBackwards);
    }
    // Preflight before mutating/spawning any execution.
    for r in &state.checks {
        if r.registration.enabled && r.next_due.is_some_and(|due| now >= due) {
            checked_next_due(now, r.registration.cadence, &r.registration.check_id)?;
        }
    }
    state.last_schedule = Some(now);
    state.executions.retain(|e| !e.supervisor.is_finished());
    let mut available = inner
        .config
        .max_concurrent_checks
        .saturating_sub(state.checks.iter().filter(|r| r.running).count());
    let len = state.checks.len();
    let start = state.cursor;
    let mut started = 0;
    for offset in 0..len {
        let index = (start + offset) % len;
        let r = &mut state.checks[index];
        if !r.registration.enabled || r.next_due.is_none_or(|due| now < due) {
            continue;
        }
        if r.running {
            r.next_due = Some(checked_next_due(
                now,
                r.registration.cadence,
                &r.registration.check_id,
            )?);
            r.last_action = Some(MonitoringSchedulerAction::SkippedOverlap);
            continue;
        }
        if available == 0 {
            r.last_action = Some(MonitoringSchedulerAction::DeferredAtCapacity);
            continue; // Original deadline retained, remains eligible.
        }
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| MonitoringSchedulerError::NoRuntime)?;
        let check = r.registration.check.clone();
        let id = r.registration.check_id.clone();
        r.next_due = Some(checked_next_due(now, r.registration.cadence, &id)?);
        r.running = true;
        r.last_started = Some(evaluated_at);
        r.last_action = Some(MonitoringSchedulerAction::Started);
        let child = runtime.spawn(async move { check.evaluate(evaluated_at).await });
        let abort = child.abort_handle();
        let supervisor_inner = inner.clone();
        let supervisor = runtime.spawn(async move {
            let joined = child.await;
            let completed_at = supervisor_inner.clock.now().evaluated_at_utc;
            let result = match joined {
                Ok(Ok(result)) => {
                    let evaluation = MonitoringEvaluation {
                        check_id: id.clone(),
                        evaluated_at_utc: format_utc(evaluated_at),
                        outcome: result.outcome,
                        findings: result.findings,
                    };
                    match evaluation.validate() {
                        Ok(()) => MonitoringCompletionResult::Evaluation(evaluation),
                        Err(error) => MonitoringCompletionResult::Error(error.bounded()),
                    }
                }
                Ok(Err(error)) => MonitoringCompletionResult::Error(error.bounded()),
                Err(error) if error.is_panic() => MonitoringCompletionResult::Panicked,
                Err(_) => MonitoringCompletionResult::Cancelled,
            };
            let summary = match &result {
                MonitoringCompletionResult::Evaluation(e) => MonitoringLastEvaluation::Completed {
                    outcome: e.outcome.clone(),
                    finding_count: e.findings.len(),
                },
                MonitoringCompletionResult::Error(error) => MonitoringLastEvaluation::Failed {
                    error: error.clone(),
                },
                MonitoringCompletionResult::Panicked => MonitoringLastEvaluation::Panicked,
                MonitoringCompletionResult::Cancelled => MonitoringLastEvaluation::Cancelled,
            };
            let completion = MonitoringCompletion {
                check_id: id,
                evaluated_at_utc: format_utc(evaluated_at),
                completed_at_utc: format_utc(completed_at),
                result,
            };
            let mut state = supervisor_inner
                .state
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            state.checks[index].running = false;
            state.checks[index].last_completed = Some(completed_at);
            state.checks[index].last_result = Some(summary);
            match state.output.as_ref().map(|tx| tx.try_send(completion)) {
                Some(Err(tokio::sync::mpsc::error::TrySendError::Full(_))) => {
                    state.dropped_full = state.dropped_full.saturating_add(1);
                    state.checks[index].last_action =
                        Some(MonitoringSchedulerAction::OutputDroppedFull);
                }
                Some(Err(tokio::sync::mpsc::error::TrySendError::Closed(_))) => {
                    state.dropped_closed = state.dropped_closed.saturating_add(1);
                    state.checks[index].last_action =
                        Some(MonitoringSchedulerAction::OutputReceiverClosed);
                }
                _ => {}
            }
            state.completed_count = state.completed_count.saturating_add(1);
            supervisor_inner
                .completed
                .send_replace(state.completed_count);
        });
        state.executions.push(OwnedExecution { abort, supervisor });
        state.runtime = Some(runtime);
        state.cursor = (index + 1) % len;
        available -= 1;
        started += 1;
    }
    Ok(started)
}

fn initiate_shutdown(inner: &Arc<SchedulerInner>) {
    let mut state = inner.state.lock().unwrap_or_else(|p| p.into_inner());
    if state.lifecycle != MonitoringLifecycle::Running {
        return;
    }
    state.lifecycle = MonitoringLifecycle::Stopping;
    inner.lifecycle.send_replace(MonitoringLifecycle::Stopping);
    for r in &mut state.checks {
        r.next_due = None;
    }
    let driver = state.driver.take();
    if let Some(driver) = &driver {
        driver.abort();
    }
    let executions = std::mem::take(&mut state.executions);
    for e in &executions {
        e.abort.abort();
    }
    if driver.is_none() && executions.is_empty() {
        state.output.take();
        state.lifecycle = MonitoringLifecycle::Stopped;
        inner.lifecycle.send_replace(MonitoringLifecycle::Stopped);
        return;
    }
    let cleanup_inner = inner.clone();
    // Every owned task records its originating runtime, including when Drop
    // is called from another thread. Worker owns all joins independently.
    let runtime = state
        .runtime
        .as_ref()
        .expect("owned tasks have a runtime")
        .clone();
    runtime.spawn(async move {
        if let Some(driver) = driver {
            let _ = driver.await;
        }
        for execution in executions {
            let _ = execution.supervisor.await;
        }
        let mut state = cleanup_inner
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        for r in &mut state.checks {
            r.running = false;
        }
        state.output.take();
        state.lifecycle = MonitoringLifecycle::Stopped;
        cleanup_inner
            .lifecycle
            .send_replace(MonitoringLifecycle::Stopped);
    });
}
fn checked_next_due(
    now: Duration,
    cadence: Duration,
    id: &MonitoringCheckId,
) -> Result<Duration, MonitoringSchedulerError> {
    now.checked_add(cadence).ok_or_else(|| {
        MonitoringSchedulerError::InvalidConfiguration(MonitoringConfigError::TimeOverflow(
            id.clone(),
        ))
    })
}
fn format_utc(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}
fn bounded_message(value: String) -> String {
    if value.len() <= MAX_MONITORING_STATUS_MESSAGE_BYTES {
        return value;
    }
    let mut boundary = MAX_MONITORING_STATUS_MESSAGE_BYTES;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value[..boundary].to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DMonitorStreamSelector {
    pub project_scope: String,
    pub node_id: String,
    pub monitor_instance_id: String,
}

impl DMonitorStreamSelector {
    pub fn new(
        project_scope: impl Into<String>,
        node_id: impl Into<String>,
        monitor_instance_id: impl Into<String>,
    ) -> Result<Self, MonitoringCheckError> {
        let selector = Self {
            project_scope: project_scope.into(),
            node_id: node_id.into(),
            monitor_instance_id: monitor_instance_id.into(),
        };
        if selector.project_scope.trim().is_empty()
            || selector.node_id.trim().is_empty()
            || selector.monitor_instance_id.trim().is_empty()
        {
            return Err(MonitoringCheckError::InvalidEvidence(
                "DMonitor selector fields must be non-empty".into(),
            ));
        }
        Ok(selector)
    }

    pub fn check_id(&self) -> Result<MonitoringCheckId, MonitoringConfigError> {
        MonitoringCheckId::new(format!(
            "dmonitor_freshness:{}",
            stream_key(
                &self.project_scope,
                &self.node_id,
                &self.monitor_instance_id
            )
        ))
    }

    fn subject(&self) -> DatumEventSubjectV1 {
        DatumEventSubjectV1::Other {
            project_id: Some(self.project_scope.clone()),
            entity_kind: MONITORING_SUBJECT_KIND.into(),
            entity_id: stream_key(
                &self.project_scope,
                &self.node_id,
                &self.monitor_instance_id,
            ),
        }
    }
}

/// Immutable publication of an existing accepted DMonitor state. Construction
/// reuses the canonical relational/digest validator once per publication.
/// It stores the original state behind Arc; no second state/history is built.
#[derive(Clone)]
pub struct MonitoringEvidenceSnapshot {
    state: Arc<DMonitorStateV1>,
}
impl MonitoringEvidenceSnapshot {
    pub fn from_state(state: DMonitorStateV1) -> Result<Self, MonitoringCheckError> {
        validate_persisted_dmonitor_state_integrity(&state)
            .map_err(MonitoringCheckError::InvalidEvidence)?;
        Ok(Self {
            state: Arc::new(state),
        })
    }
    fn stream_head(
        &self,
        selector: &DMonitorStreamSelector,
    ) -> Option<DMonitorObservationRecordV1> {
        self.state
            .stream_head(
                &selector.project_scope,
                &selector.node_id,
                &selector.monitor_instance_id,
            )
            .cloned()
    }
}
/// Publication is validated before it becomes visible. A busy snapshot lock
/// is an availability error, distinct from an absent accepted stream.
pub trait DMonitorEvidenceSource: Send + Sync + 'static {
    fn snapshot(&self) -> Result<MonitoringEvidenceSnapshot, MonitoringCheckError>;
}
impl DMonitorEvidenceSource for MonitoringEvidenceSnapshot {
    fn snapshot(&self) -> Result<MonitoringEvidenceSnapshot, MonitoringCheckError> {
        Ok(self.clone())
    }
}
impl DMonitorEvidenceSource for RwLock<MonitoringEvidenceSnapshot> {
    fn snapshot(&self) -> Result<MonitoringEvidenceSnapshot, MonitoringCheckError> {
        match self.try_read() {
            Ok(snapshot) => Ok(snapshot.clone()),
            Err(TryLockError::WouldBlock) => Err(MonitoringCheckError::EvidenceUnavailable(
                "snapshot publication busy".into(),
            )),
            Err(TryLockError::Poisoned(_)) => Err(MonitoringCheckError::EvidenceUnavailable(
                "snapshot lock poisoned".into(),
            )),
        }
    }
}

pub struct DMonitorFreshnessCheck {
    source: Arc<dyn DMonitorEvidenceSource>,
    selector: DMonitorStreamSelector,
    policy: DMonitorFreshnessPolicyV1,
}

impl DMonitorFreshnessCheck {
    pub fn new(
        source: Arc<dyn DMonitorEvidenceSource>,
        selector: DMonitorStreamSelector,
        policy: DMonitorFreshnessPolicyV1,
    ) -> Result<Self, MonitoringCheckError> {
        DMonitorStreamSelector::new(
            selector.project_scope.clone(),
            selector.node_id.clone(),
            selector.monitor_instance_id.clone(),
        )?;
        validate_freshness_policy(&policy).map_err(MonitoringCheckError::InvalidEvidence)?;
        Ok(Self {
            source,
            selector,
            policy,
        })
    }

    pub fn registration(
        self: Arc<Self>,
        enabled: bool,
    ) -> Result<MonitoringCheckRegistration, MonitoringConfigError> {
        Ok(MonitoringCheckRegistration::new(
            self.selector.check_id()?,
            DEFAULT_CHECK_CADENCE,
            enabled,
            self,
        ))
    }

    fn finding(
        &self,
        outcome: MonitoringOutcome,
        code: &str,
        severity: MonitoringSeverityHint,
        observed: Option<Value>,
        threshold: Option<Value>,
        evidence_refs: Vec<DatumEventEvidenceRefV1>,
        attributes: Value,
    ) -> Result<MonitoringCheckResult, MonitoringCheckError> {
        let finding = MonitoringFinding::new(
            code,
            severity,
            self.selector.subject(),
            observed,
            threshold,
            evidence_refs,
            attributes,
        )?;
        MonitoringCheckResult::new(outcome, vec![finding])
    }
}

impl MonitoringCheck for DMonitorFreshnessCheck {
    fn evaluate(&self, evaluated_at_utc: DateTime<Utc>) -> MonitoringCheckFuture<'_> {
        Box::pin(async move {
            let snapshot = self.source.snapshot()?;
            let Some(record) = snapshot.stream_head(&self.selector) else {
                return self.finding(
                    MonitoringOutcome::Indeterminate, "observation_missing", MonitoringSeverityHint::Warning, None,
                    Some(json!({
                        "max_observed_age_seconds": self.policy.max_observed_age_seconds,
                        "max_received_age_seconds": self.policy.max_received_age_seconds,
                    })),
                    Vec::new(),
                    json!({"node_id": self.selector.node_id, "monitor_instance_id": self.selector.monitor_instance_id}),
                );
            };
            let evaluation_time = format_utc(evaluated_at_utc);
            let freshness = match evaluate_freshness(&record, &self.policy, &evaluation_time) {
                Ok(result) => result,
                Err(error) => {
                    let before_receipt =
                        chrono::DateTime::parse_from_rfc3339(&record.received_at_utc)
                            .map(|value| evaluated_at_utc < value.with_timezone(&Utc))
                            .unwrap_or(false);
                    if !before_receipt {
                        return Err(MonitoringCheckError::InvalidEvidence(error));
                    }
                    return self.finding(
                        MonitoringOutcome::Indeterminate, "evaluation_before_receipt", MonitoringSeverityHint::Warning,
                        Some(json!({"evaluated_at_utc": evaluation_time, "received_at_utc": record.received_at_utc})),
                        None,
                        vec![DatumEventEvidenceRefV1::DMonitorObservation {
                            observation_id: record.observation.observation_id,
                        }],
                        json!({"node_id": self.selector.node_id, "monitor_instance_id": self.selector.monitor_instance_id}),
                    );
                }
            };
            let evidence_ref = DatumEventEvidenceRefV1::DMonitorObservation {
                observation_id: record.observation.observation_id,
            };
            match freshness.freshness {
                DMonitorFreshnessV1::Fresh =>
                    MonitoringCheckResult::new(MonitoringOutcome::Satisfied, Vec::new()),
                DMonitorFreshnessV1::Stale => self.finding(
                    MonitoringOutcome::Finding, "observation_stale", MonitoringSeverityHint::Warning,
                    Some(json!({
                        "observed_age_seconds": freshness.observed_age_seconds,
                        "received_age_seconds": freshness.received_age_seconds,
                    })),
                    Some(json!({
                        "max_observed_age_seconds": self.policy.max_observed_age_seconds,
                        "max_received_age_seconds": self.policy.max_received_age_seconds,
                    })),
                    vec![evidence_ref],
                    json!({"node_id": self.selector.node_id, "monitor_instance_id": self.selector.monitor_instance_id}),
                ),
                DMonitorFreshnessV1::ClockSkew => self.finding(
                    MonitoringOutcome::Indeterminate, "observation_clock_skew", MonitoringSeverityHint::Warning,
                    Some(json!({
                        "observed_at_utc": record.observation.observed_at_utc,
                        "received_at_utc": record.received_at_utc,
                        "observed_age_seconds": freshness.observed_age_seconds,
                        "received_age_seconds": freshness.received_age_seconds,
                    })),
                    Some(json!({"max_future_clock_skew_seconds": self.policy.max_future_clock_skew_seconds})),
                    vec![evidence_ref],
                    json!({"node_id": self.selector.node_id, "monitor_instance_id": self.selector.monitor_instance_id}),
                ),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::canonical_dmonitor::{
        CanonicalDMonitorObservationV1, DMonitorHealthV1, DMonitorSubjectV1,
        CANONICAL_DMONITOR_OBSERVATION_SCHEMA,
    };
    use crate::core::dmonitor_state::ingest_observation;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::sync::oneshot;

    fn utc(seconds: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-29T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
            + chrono::Duration::seconds(seconds)
    }
    struct FixedClock(Mutex<MonitoringTime>);
    impl FixedClock {
        fn new() -> Arc<Self> {
            Arc::new(Self(Mutex::new(MonitoringTime {
                schedule_time: Duration::ZERO,
                evaluated_at_utc: utc(0),
            })))
        }
        fn set(&self, schedule: u64, wall: i64) {
            *self.0.lock().unwrap() = MonitoringTime {
                schedule_time: Duration::from_secs(schedule),
                evaluated_at_utc: utc(wall),
            };
        }
    }
    impl MonitoringClock for FixedClock {
        fn now(&self) -> MonitoringTime {
            *self.0.lock().unwrap()
        }
    }
    struct CountCheck(Arc<AtomicUsize>);
    impl MonitoringCheck for CountCheck {
        fn evaluate(&self, _: DateTime<Utc>) -> MonitoringCheckFuture<'_> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                Ok(MonitoringCheckResult::new(MonitoringOutcome::Satisfied, vec![]).unwrap())
            })
        }
    }
    fn counter() -> (Arc<dyn MonitoringCheck>, Arc<AtomicUsize>) {
        let count = Arc::new(AtomicUsize::new(0));
        (Arc::new(CountCheck(count.clone())), count)
    }
    fn registration(
        id: &str,
        cadence: u64,
        check: Arc<dyn MonitoringCheck>,
    ) -> MonitoringCheckRegistration {
        MonitoringCheckRegistration::new(
            MonitoringCheckId::new(id).unwrap(),
            Duration::from_secs(cadence),
            true,
            check,
        )
    }
    fn scheduler(
        config: MonitoringSchedulerConfig,
        registrations: Vec<MonitoringCheckRegistration>,
    ) -> (Arc<MonitoringScheduler>, Arc<FixedClock>) {
        let clock = FixedClock::new();
        (
            MonitoringScheduler::new_with_clock(config, registrations, clock.clone()).unwrap(),
            clock,
        )
    }
    fn drive(s: &MonitoringScheduler, clock: &FixedClock, tick: u64, wall: i64) -> usize {
        clock.set(tick, wall);
        s.run_due_checks_at(Duration::from_secs(tick), utc(wall))
            .unwrap()
    }
    async fn completed(s: &MonitoringScheduler, count: u64) {
        let mut rx = s.inner.completed.subscribe();
        loop {
            if *rx.borrow_and_update() >= count {
                return;
            }
            rx.changed().await.unwrap();
        }
    }
    async fn polled(s: &MonitoringScheduler) {
        let mut rx = s.inner.polled.subscribe();
        loop {
            if *rx.borrow_and_update() > 0 {
                return;
            }
            rx.changed().await.unwrap();
        }
    }
    fn finding() -> MonitoringFinding {
        MonitoringFinding::new(
            "observation_stale",
            MonitoringSeverityHint::Warning,
            DatumEventSubjectV1::Application {
                project_id: "p".into(),
                application_id: "a".into(),
            },
            Some(json!({"age": 301})),
            Some(json!({"limit": 300})),
            vec![],
            json!({"diagnostic": "preserved"}),
        )
        .unwrap()
    }
    struct ResultCheck(MonitoringCheckResult);
    impl MonitoringCheck for ResultCheck {
        fn evaluate(&self, _: DateTime<Utc>) -> MonitoringCheckFuture<'_> {
            let result = self.0.clone();
            Box::pin(async move { Ok(result) })
        }
    }
    struct FailingCheck;
    impl MonitoringCheck for FailingCheck {
        fn evaluate(&self, _: DateTime<Utc>) -> MonitoringCheckFuture<'_> {
            Box::pin(async { Err(MonitoringCheckError::EvidenceUnavailable("busy".into())) })
        }
    }
    struct PanicOnce(AtomicUsize);
    impl MonitoringCheck for PanicOnce {
        fn evaluate(&self, _: DateTime<Utc>) -> MonitoringCheckFuture<'_> {
            Box::pin(async move {
                if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                    panic!("isolated check panic");
                }
                Ok(MonitoringCheckResult::new(MonitoringOutcome::Satisfied, vec![]).unwrap())
            })
        }
    }

    #[tokio::test]
    async fn due_cadence_status_and_skipped_backlog() {
        let (check, count) = counter();
        let (s, clock) = scheduler(
            MonitoringSchedulerConfig::default(),
            vec![registration("one", 30, check)],
        );
        let mut output = s.take_output_receiver().unwrap();
        assert_eq!(drive(&s, &clock, 0, 0), 1);
        let first = output.recv().await.unwrap();
        assert_eq!(first.evaluated_at_utc, format_utc(utc(0)));
        assert_eq!(drive(&s, &clock, 29, 29), 0);
        assert_eq!(drive(&s, &clock, 30, 30), 1);
        output.recv().await.unwrap();
        assert_eq!(drive(&s, &clock, 330, 330), 1);
        output.recv().await.unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 3);
        let r = &s.status().unwrap().checks[0];
        assert_eq!(r.last_started_at_utc, Some(format_utc(utc(330))));
        assert_eq!(r.last_completed_at_utc, Some(format_utc(utc(330))));
        assert_eq!(r.next_due_schedule_time, Some(Duration::from_secs(360)));
        assert!(matches!(
            r.last_evaluation_result,
            Some(MonitoringLastEvaluation::Completed { .. })
        ));
        s.shutdown().await;
    }

    #[tokio::test]
    async fn multiple_cadences_and_status_order_are_stable() {
        let (a, ac) = counter();
        let (b, bc) = counter();
        let (s, clock) = scheduler(
            MonitoringSchedulerConfig::default(),
            vec![registration("b", 5, b), registration("a", 2, a)],
        );
        drive(&s, &clock, 0, 0);
        completed(&s, 2).await;
        drive(&s, &clock, 2, 2);
        completed(&s, 3).await;
        assert_eq!(
            (ac.load(Ordering::SeqCst), bc.load(Ordering::SeqCst)),
            (2, 1)
        );
        drive(&s, &clock, 5, 5);
        completed(&s, 5).await;
        assert_eq!(bc.load(Ordering::SeqCst), 2);
        assert_eq!(s.status().unwrap().checks[0].check_id.as_str(), "a");
        s.shutdown().await;
    }

    #[tokio::test]
    async fn critical_independent_clocks_backward_forward_and_correction() {
        let (check, count) = counter();
        let (s, clock) = scheduler(
            MonitoringSchedulerConfig::default(),
            vec![registration("clock", 10, check)],
        );
        let mut output = s.take_output_receiver().unwrap();
        for (tick, wall) in [(0, 0), (10, -100), (20, 3600), (30, 1)] {
            assert_eq!(drive(&s, &clock, tick, wall), 1);
            assert_eq!(
                output.recv().await.unwrap().evaluated_at_utc,
                format_utc(utc(wall))
            );
            assert_eq!(drive(&s, &clock, tick, wall + 100000), 0); // UTC jump alone is not due.
        }
        assert_eq!(count.load(Ordering::SeqCst), 4);
        let before = s.status().unwrap();
        assert!(matches!(
            s.run_due_checks_at(Duration::from_secs(29), utc(99999)),
            Err(MonitoringSchedulerError::ScheduleMovedBackwards)
        ));
        assert_eq!(s.status().unwrap(), before);
        s.shutdown().await;
    }

    #[tokio::test]
    async fn critical_fairness_ten_checks_capacity_four() {
        let mut counts = Vec::new();
        let mut regs = Vec::new();
        for i in 0..10 {
            let (check, count) = counter();
            counts.push(count);
            regs.push(registration(&format!("check_{i:02}"), 1, check));
        }
        let (s, clock) = scheduler(
            MonitoringSchedulerConfig {
                max_concurrent_checks: 4,
                ..Default::default()
            },
            regs,
        );
        for cycle in 0..3 {
            assert_eq!(drive(&s, &clock, cycle, cycle as i64), 4);
            completed(&s, (cycle + 1) * 4).await;
            assert!(s.status().unwrap().checks.iter().all(|r| !r.running));
        }
        assert!(counts.iter().all(|n| n.load(Ordering::SeqCst) > 0));
        assert_eq!(
            counts
                .iter()
                .map(|n| n.load(Ordering::SeqCst))
                .collect::<Vec<_>>(),
            vec![2, 2, 1, 1, 1, 1, 1, 1, 1, 1]
        );
        s.shutdown().await;
    }

    struct DropNotice(Option<oneshot::Sender<()>>);
    impl Drop for DropNotice {
        fn drop(&mut self) {
            if let Some(tx) = self.0.take() {
                let _ = tx.send(());
            }
        }
    }
    struct GatedCheck {
        entered: Mutex<Option<oneshot::Sender<()>>>,
        release: Mutex<Option<oneshot::Receiver<()>>>,
        terminated: Mutex<Option<oneshot::Sender<()>>>,
        count: Arc<AtomicUsize>,
    }
    impl MonitoringCheck for GatedCheck {
        fn evaluate(&self, _: DateTime<Utc>) -> MonitoringCheckFuture<'_> {
            self.count.fetch_add(1, Ordering::SeqCst);
            let entered = self.entered.lock().unwrap().take();
            let release = self.release.lock().unwrap().take();
            let notice = DropNotice(self.terminated.lock().unwrap().take());
            Box::pin(async move {
                let _notice = notice;
                if let Some(tx) = entered {
                    let _ = tx.send(());
                }
                if let Some(rx) = release {
                    let _ = rx.await;
                }
                Ok(MonitoringCheckResult::new(MonitoringOutcome::Satisfied, vec![]).unwrap())
            })
        }
    }
    type Gate = (
        Arc<GatedCheck>,
        oneshot::Receiver<()>,
        oneshot::Sender<()>,
        oneshot::Receiver<()>,
    );
    fn gated() -> Gate {
        let (entered, entered_rx) = oneshot::channel();
        let (release_tx, release) = oneshot::channel();
        let (terminated, terminated_rx) = oneshot::channel();
        (
            Arc::new(GatedCheck {
                entered: Mutex::new(Some(entered)),
                release: Mutex::new(Some(release)),
                terminated: Mutex::new(Some(terminated)),
                count: Arc::new(AtomicUsize::new(0)),
            }),
            entered_rx,
            release_tx,
            terminated_rx,
        )
    }

    #[tokio::test]
    async fn critical_no_overlap_sibling_progress_and_completion_without_tick() {
        let (slow, entered, release, terminated) = gated();
        let (fast, count) = counter();
        let (s, clock) = scheduler(
            MonitoringSchedulerConfig {
                max_concurrent_checks: 2,
                ..Default::default()
            },
            vec![
                registration("slow", 1, slow.clone()),
                registration("fast", 1, fast),
            ],
        );
        let mut output = s.take_output_receiver().unwrap();
        drive(&s, &clock, 0, 0);
        entered.await.unwrap();
        assert_eq!(output.recv().await.unwrap().check_id.as_str(), "fast");
        assert_eq!(drive(&s, &clock, 1, -20), 1); // sibling due, slow overlaps
        output.recv().await.unwrap();
        assert_eq!(slow.count.load(Ordering::SeqCst), 1);
        assert_eq!(count.load(Ordering::SeqCst), 2);
        let status = s.status().unwrap();
        let slow_status = status
            .checks
            .iter()
            .find(|r| r.check_id.as_str() == "slow")
            .unwrap();
        assert!(slow_status.running);
        assert_eq!(
            slow_status.last_scheduler_action,
            Some(MonitoringSchedulerAction::SkippedOverlap)
        );
        clock.set(1, -100);
        release.send(()).unwrap();
        terminated.await.unwrap();
        let done = output.recv().await.unwrap(); // No further tick, backward UTC.
        assert_eq!(done.check_id.as_str(), "slow");
        assert_eq!(done.completed_at_utc, format_utc(utc(-100)));
        assert!(s.status().unwrap().checks.iter().all(|r| !r.running));
        s.shutdown().await;
    }

    #[tokio::test]
    async fn critical_active_shutdown_aborts_joins_and_closes_output() {
        let (check, entered, _release, terminated) = gated();
        let (s, clock) = scheduler(
            MonitoringSchedulerConfig::default(),
            vec![registration("gated", 1, check)],
        );
        let mut output = s.take_output_receiver().unwrap();
        drive(&s, &clock, 0, 0);
        entered.await.unwrap();
        s.shutdown().await;
        terminated.await.unwrap();
        assert_eq!(s.status().unwrap().lifecycle, MonitoringLifecycle::Stopped);
        assert!(matches!(
            output.recv().await.unwrap().result,
            MonitoringCompletionResult::Cancelled
        ));
        assert!(output.recv().await.is_none());
        assert!(matches!(
            s.run_due_checks_at(Duration::from_secs(1), utc(1)),
            Err(MonitoringSchedulerError::Shutdown)
        ));
    }

    #[tokio::test]
    async fn critical_manual_owner_drop_aborts_and_cleanup_closes_channel() {
        let (check, entered, _release, terminated) = gated();
        let (s, clock) = scheduler(
            MonitoringSchedulerConfig::default(),
            vec![registration("drop", 1, check)],
        );
        let mut output = s.take_output_receiver().unwrap();
        drive(&s, &clock, 0, 0);
        entered.await.unwrap();
        drop(s);
        terminated.await.unwrap();
        assert!(matches!(
            output.recv().await.unwrap().result,
            MonitoringCompletionResult::Cancelled
        ));
        assert!(output.recv().await.is_none()); // Shared worker joined supervisor.
    }

    // Deliberately gate a destructor on another runtime worker so tests can
    // prove the shutdown barrier waits for actual task termination.
    struct TeardownGate {
        entered: Option<oneshot::Sender<()>>,
        release: Arc<std::sync::Barrier>,
        terminated: Arc<AtomicBool>,
    }
    impl Drop for TeardownGate {
        fn drop(&mut self) {
            if let Some(tx) = self.entered.take() {
                let _ = tx.send(());
            }
            self.release.wait();
            self.terminated.store(true, Ordering::SeqCst);
        }
    }
    struct TeardownCheck {
        started: Mutex<Option<oneshot::Sender<()>>>,
        guard: Mutex<Option<TeardownGate>>,
    }
    impl MonitoringCheck for TeardownCheck {
        fn evaluate(&self, _: DateTime<Utc>) -> MonitoringCheckFuture<'_> {
            let guard = self.guard.lock().unwrap().take().unwrap();
            let started = self.started.lock().unwrap().take().unwrap();
            Box::pin(async move {
                let _guard = guard;
                let _ = started.send(());
                std::future::pending::<Result<MonitoringCheckResult, MonitoringCheckError>>().await
            })
        }
    }
    fn teardown_check() -> (
        Arc<TeardownCheck>,
        oneshot::Receiver<()>,
        oneshot::Receiver<()>,
        Arc<std::sync::Barrier>,
        Arc<AtomicBool>,
    ) {
        let (started, started_rx) = oneshot::channel();
        let (entered, entered_rx) = oneshot::channel();
        let release = Arc::new(std::sync::Barrier::new(2));
        let terminated = Arc::new(AtomicBool::new(false));
        (
            Arc::new(TeardownCheck {
                started: Mutex::new(Some(started)),
                guard: Mutex::new(Some(TeardownGate {
                    entered: Some(entered),
                    release: release.clone(),
                    terminated: terminated.clone(),
                })),
            }),
            started_rx,
            entered_rx,
            release,
            terminated,
        )
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn critical_concurrent_shutdown_callers_share_termination_barrier() {
        let (check, started, teardown, release, terminated) = teardown_check();
        let (s, clock) = scheduler(
            MonitoringSchedulerConfig::default(),
            vec![registration("barrier", 1, check)],
        );
        drive(&s, &clock, 0, 0);
        started.await.unwrap();
        s.request_shutdown();
        teardown.await.unwrap();
        assert_eq!(s.status().unwrap().lifecycle, MonitoringLifecycle::Stopping);
        let (entered_a, mut entered_a_rx) = oneshot::channel();
        let (entered_b, mut entered_b_rx) = oneshot::channel();
        let sa = s.clone();
        let ta = terminated.clone();
        let a = tokio::spawn(async move {
            let _ = entered_a.send(());
            sa.shutdown().await;
            assert!(ta.load(Ordering::SeqCst));
        });
        let sb = s.clone();
        let tb = terminated.clone();
        let b = tokio::spawn(async move {
            let _ = entered_b.send(());
            sb.shutdown().await;
            assert!(tb.load(Ordering::SeqCst));
        });
        (&mut entered_a_rx).await.unwrap();
        (&mut entered_b_rx).await.unwrap();
        assert!(!a.is_finished() && !b.is_finished());
        release.wait();
        a.await.unwrap();
        b.await.unwrap();
        assert_eq!(s.status().unwrap().lifecycle, MonitoringLifecycle::Stopped);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn critical_cancelled_shutdown_wait_does_not_cancel_cleanup() {
        let (check, started, teardown, release, terminated) = teardown_check();
        let (s, clock) = scheduler(
            MonitoringSchedulerConfig::default(),
            vec![registration("cancel", 1, check)],
        );
        drive(&s, &clock, 0, 0);
        started.await.unwrap();
        let waiter = s.clone();
        let wait = tokio::spawn(async move {
            waiter.shutdown().await;
        });
        teardown.await.unwrap(); // Worker initiated and cancellation in progress.
        wait.abort();
        assert!(wait.await.unwrap_err().is_cancelled());
        assert_eq!(s.status().unwrap().lifecycle, MonitoringLifecycle::Stopping);
        release.wait();
        s.shutdown().await;
        assert!(terminated.load(Ordering::SeqCst));
        assert_eq!(s.status().unwrap().lifecycle, MonitoringLifecycle::Stopped);
    }

    #[tokio::test]
    async fn critical_background_output_preserves_finding_and_handle_drop_cleans_up() {
        let expected = finding();
        let check = Arc::new(ResultCheck(
            MonitoringCheckResult::new(MonitoringOutcome::Finding, vec![expected.clone()]).unwrap(),
        ));
        let (s, _) = scheduler(
            MonitoringSchedulerConfig {
                poll_interval: Duration::from_secs(3600),
                ..Default::default()
            },
            vec![registration("background", 30, check)],
        );
        let mut handle = s.start().unwrap();
        let completion = handle.recv().await.unwrap();
        match completion.result {
            MonitoringCompletionResult::Evaluation(e) => assert_eq!(e.findings, vec![expected]),
            other => panic!("unexpected output {other:?}"),
        }
        assert_eq!(s.status().unwrap().completed_count, 1);
        drop(handle);
        s.shutdown().await;
        assert_eq!(s.status().unwrap().lifecycle, MonitoringLifecycle::Stopped);
    }

    #[tokio::test]
    async fn critical_output_full_and_closed_are_nonblocking_and_recorded() {
        let (check, count) = counter();
        let (s, clock) = scheduler(
            MonitoringSchedulerConfig {
                output_capacity: 1,
                ..Default::default()
            },
            vec![registration("full", 1, check)],
        );
        let mut output = s.take_output_receiver().unwrap();
        drive(&s, &clock, 0, 0);
        completed(&s, 1).await; // Do not drain.
        drive(&s, &clock, 1, 1);
        completed(&s, 2).await;
        let status = s.status().unwrap();
        assert_eq!(status.dropped_output_full, 1);
        assert!(!status.checks[0].running);
        assert!(matches!(
            status.checks[0].last_evaluation_result,
            Some(MonitoringLastEvaluation::Completed { .. })
        ));
        assert_eq!(
            status.checks[0].last_scheduler_action,
            Some(MonitoringSchedulerAction::OutputDroppedFull)
        );
        output.recv().await.unwrap();
        drop(output);
        drive(&s, &clock, 2, 2);
        completed(&s, 3).await;
        assert_eq!(s.status().unwrap().dropped_output_closed, 1);
        assert_eq!(count.load(Ordering::SeqCst), 3);
        s.shutdown().await;
    }

    #[tokio::test]
    async fn critical_panic_error_capacity_restoration_and_later_cycle() {
        let (ok, count) = counter();
        let (s, clock) = scheduler(
            MonitoringSchedulerConfig {
                max_concurrent_checks: 2,
                ..Default::default()
            },
            vec![
                registration("a_panic", 1, Arc::new(PanicOnce(AtomicUsize::new(0)))),
                registration("b_error", 1, Arc::new(FailingCheck)),
                registration("c_ok", 1, ok),
            ],
        );
        let mut output = s.take_output_receiver().unwrap();
        assert_eq!(drive(&s, &clock, 0, 0), 2);
        completed(&s, 2).await;
        let first = output.recv().await.unwrap();
        let second = output.recv().await.unwrap();
        assert!([&first.result, &second.result]
            .iter()
            .any(|r| matches!(r, MonitoringCompletionResult::Panicked)));
        assert!([&first.result, &second.result].iter().any(|r| matches!(
            r,
            MonitoringCompletionResult::Error(MonitoringCheckError::EvidenceUnavailable(_))
        )));
        let status = s.status().unwrap();
        assert!(status.checks.iter().all(|r| !r.running));
        assert_eq!(
            status.checks[0].last_evaluation_result,
            Some(MonitoringLastEvaluation::Panicked)
        );
        assert_eq!(drive(&s, &clock, 1, 1), 2);
        completed(&s, 4).await;
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(matches!(
            s.status().unwrap().checks[0].last_evaluation_result,
            Some(MonitoringLastEvaluation::Completed { .. })
        ));
        s.shutdown().await;
    }

    #[tokio::test]
    async fn disabled_dispatch_and_empty_background_registry() {
        let (check, count) = counter();
        let mut reg = registration("disabled", 1, check);
        reg.enabled = false;
        let (s, clock) = scheduler(MonitoringSchedulerConfig::default(), vec![reg]);
        assert_eq!(drive(&s, &clock, 100, 100), 0);
        assert_eq!(count.load(Ordering::SeqCst), 0);
        assert!(s.status().unwrap().checks[0]
            .next_due_schedule_time
            .is_none());
        s.shutdown().await;
        let (empty, _) = scheduler(
            MonitoringSchedulerConfig {
                poll_interval: Duration::from_secs(3600),
                ..Default::default()
            },
            vec![],
        );
        let handle = empty.start().unwrap();
        polled(&empty).await;
        assert!(empty.status().unwrap().checks.is_empty());
        handle.shutdown().await;
        assert_eq!(empty.status().unwrap().driver_polls, 1); // Interval sleeps between polls.
        assert_eq!(
            empty.status().unwrap().lifecycle,
            MonitoringLifecycle::Stopped
        );
    }

    #[test]
    fn configuration_and_identity_validation_defend_both_entry_paths() {
        for text in [r#""""#, r#""   ""#, r#"" padded ""#] {
            assert!(serde_json::from_str::<MonitoringCheckId>(text).is_err());
        }
        assert_eq!(
            serde_json::from_str::<MonitoringCheckId>(r#""valid""#)
                .unwrap()
                .as_str(),
            "valid"
        );
        let (check, _) = counter();
        for reg in [
            MonitoringCheckRegistration::new(
                MonitoringCheckId(" ".into()),
                Duration::from_secs(1),
                true,
                check.clone(),
            ),
            registration("zero", 0, check.clone()),
        ] {
            assert!(
                MonitoringScheduler::new(MonitoringSchedulerConfig::default(), vec![reg]).is_err()
            );
        }
        assert!(MonitoringScheduler::new(
            MonitoringSchedulerConfig::default(),
            vec![
                registration("same", 1, check.clone()),
                registration("same", 1, check)
            ]
        )
        .is_err());
        for config in [
            MonitoringSchedulerConfig {
                poll_interval: Duration::ZERO,
                ..Default::default()
            },
            MonitoringSchedulerConfig {
                max_concurrent_checks: 0,
                ..Default::default()
            },
            MonitoringSchedulerConfig {
                output_capacity: 0,
                ..Default::default()
            },
        ] {
            assert!(MonitoringScheduler::new(config, vec![]).is_err());
        }
        let a = DMonitorStreamSelector::new("p", "n", "m").unwrap();
        assert_eq!(a.check_id().unwrap(), a.check_id().unwrap());
    }

    #[tokio::test]
    async fn invalid_public_results_are_typed_failures() {
        let mut bad_scope = finding();
        bad_scope.application_id = Some("contradiction".into());
        let mut bad_attrs = finding();
        bad_attrs.attributes = json!([]);
        let mut blank = finding();
        blank.finding_code = " ".into();
        let results = [
            MonitoringCheckResult {
                outcome: MonitoringOutcome::Finding,
                findings: vec![],
            },
            MonitoringCheckResult {
                outcome: MonitoringOutcome::Satisfied,
                findings: vec![finding()],
            },
            MonitoringCheckResult {
                outcome: MonitoringOutcome::Finding,
                findings: vec![bad_scope],
            },
            MonitoringCheckResult {
                outcome: MonitoringOutcome::Finding,
                findings: vec![bad_attrs],
            },
            MonitoringCheckResult {
                outcome: MonitoringOutcome::Finding,
                findings: vec![blank],
            },
        ];
        for result in results {
            let (s, clock) = scheduler(
                MonitoringSchedulerConfig::default(),
                vec![registration("invalid", 1, Arc::new(ResultCheck(result)))],
            );
            let mut output = s.take_output_receiver().unwrap();
            drive(&s, &clock, 0, 0);
            assert!(matches!(
                output.recv().await.unwrap().result,
                MonitoringCompletionResult::Error(MonitoringCheckError::InvalidResult(_))
            ));
            assert!(matches!(
                s.status().unwrap().checks[0].last_evaluation_result,
                Some(MonitoringLastEvaluation::Failed {
                    error: MonitoringCheckError::InvalidResult(_)
                })
            ));
            s.shutdown().await;
        }
        assert!(
            MonitoringCheckResult::new(MonitoringOutcome::Indeterminate, vec![])
                .unwrap()
                .validate()
                .is_ok()
        );
        assert!(
            MonitoringCheckResult::new(MonitoringOutcome::Indeterminate, vec![finding()])
                .unwrap()
                .validate()
                .is_ok()
        );
    }

    fn accepted_state(observed: i64, received: i64) -> DMonitorStateV1 {
        let mut state = DMonitorStateV1::empty();
        let observation = CanonicalDMonitorObservationV1 {
            schema: CANONICAL_DMONITOR_OBSERVATION_SCHEMA.into(),
            observation_id: "dmonitor-obs:test-1".into(),
            project_id: None,
            node_id: "node-1".into(),
            monitor_instance_id: "monitor-1".into(),
            sequence: 1,
            observed_at_utc: format_utc(utc(observed)),
            cycle_id: None,
            collector: "test_collector".into(),
            subject: DMonitorSubjectV1::DNode {
                node_id: "node-1".into(),
            },
            health: DMonitorHealthV1::Healthy,
            facts: json!({"cpu_usage_permille":100}),
        };
        ingest_observation(
            &mut state,
            "project-1",
            observation,
            &format_utc(utc(received)),
        )
        .unwrap();
        state
    }
    fn freshness(
        state: DMonitorStateV1,
        policy: DMonitorFreshnessPolicyV1,
    ) -> DMonitorFreshnessCheck {
        DMonitorFreshnessCheck::new(
            Arc::new(MonitoringEvidenceSnapshot::from_state(state).unwrap()),
            DMonitorStreamSelector::new("project-1", "node-1", "monitor-1").unwrap(),
            policy,
        )
        .unwrap()
    }
    fn policy() -> DMonitorFreshnessPolicyV1 {
        DMonitorFreshnessPolicyV1 {
            max_observed_age_seconds: 300,
            max_received_age_seconds: 300,
            max_future_clock_skew_seconds: 30,
        }
    }

    #[tokio::test]
    async fn freshness_determinism_exact_whole_second_boundary_and_evidence_immutability() {
        let state = accepted_state(-300, -300);
        let before = state.clone();
        let snapshot = MonitoringEvidenceSnapshot::from_state(state).unwrap();
        let source = Arc::new(RwLock::new(snapshot));
        let check = DMonitorFreshnessCheck::new(
            source.clone(),
            DMonitorStreamSelector::new("project-1", "node-1", "monitor-1").unwrap(),
            policy(),
        )
        .unwrap();
        let equal = check.evaluate(utc(0)).await.unwrap();
        assert_eq!(equal.outcome, MonitoringOutcome::Satisfied);
        assert!(equal.findings.is_empty());
        assert_eq!(
            check
                .evaluate(utc(0) + chrono::Duration::milliseconds(999))
                .await
                .unwrap()
                .outcome,
            MonitoringOutcome::Satisfied
        );
        let stale = check.evaluate(utc(1)).await.unwrap();
        assert_eq!(stale.outcome, MonitoringOutcome::Finding);
        assert_eq!(stale, check.evaluate(utc(1)).await.unwrap());
        assert_eq!(
            serde_json::to_vec(&stale.findings).unwrap(),
            serde_json::to_vec(&check.evaluate(utc(1)).await.unwrap().findings).unwrap()
        );
        assert_eq!(stale.findings[0].finding_code, "observation_stale");
        assert_eq!(
            stale.findings[0].observed_value.as_ref().unwrap()["observed_age_seconds"],
            301
        );
        assert_eq!(
            stale.findings[0].evidence_refs,
            vec![DatumEventEvidenceRefV1::DMonitorObservation {
                observation_id: "dmonitor-obs:test-1".into()
            }]
        );
        assert_eq!(*source.read().unwrap().state, before);
    }

    #[tokio::test]
    async fn freshness_observed_and_received_dimensions_are_independent() {
        let observed_fresh_received_stale = freshness(accepted_state(-280, -301), policy())
            .evaluate(utc(0))
            .await
            .unwrap();
        assert_eq!(
            observed_fresh_received_stale.outcome,
            MonitoringOutcome::Finding
        );
        let ages = observed_fresh_received_stale.findings[0]
            .observed_value
            .as_ref()
            .unwrap();
        assert_eq!(ages["observed_age_seconds"], 280);
        assert_eq!(ages["received_age_seconds"], 301);
        let observed_stale_received_fresh = freshness(accepted_state(-301, -1), policy())
            .evaluate(utc(0))
            .await
            .unwrap();
        assert_eq!(
            observed_stale_received_fresh.outcome,
            MonitoringOutcome::Finding
        );
        assert_eq!(
            observed_stale_received_fresh.findings[0]
                .observed_value
                .as_ref()
                .unwrap()["received_age_seconds"],
            1
        );
        assert_eq!(
            observed_stale_received_fresh.findings[0]
                .observed_value
                .as_ref()
                .unwrap()["observed_age_seconds"],
            301
        );
        assert_eq!(
            freshness(accepted_state(-300, -300), policy())
                .evaluate(utc(0))
                .await
                .unwrap()
                .outcome,
            MonitoringOutcome::Satisfied
        );
    }

    #[tokio::test]
    async fn missing_skew_tolerance_and_evaluation_before_receipt() {
        let missing = freshness(DMonitorStateV1::empty(), policy())
            .evaluate(utc(0))
            .await
            .unwrap();
        assert_eq!(missing.outcome, MonitoringOutcome::Indeterminate);
        assert_eq!(missing.findings[0].finding_code, "observation_missing");
        assert_eq!(
            freshness(accepted_state(30, 0), policy())
                .evaluate(utc(0))
                .await
                .unwrap()
                .outcome,
            MonitoringOutcome::Satisfied
        );
        let skew = freshness(accepted_state(31, 0), policy())
            .evaluate(utc(0))
            .await
            .unwrap();
        assert_eq!(skew.outcome, MonitoringOutcome::Indeterminate);
        assert_eq!(skew.findings[0].finding_code, "observation_clock_skew");
        let before = freshness(accepted_state(0, 1), policy())
            .evaluate(utc(0))
            .await
            .unwrap();
        assert_eq!(before.outcome, MonitoringOutcome::Indeterminate);
        assert_eq!(before.findings[0].finding_code, "evaluation_before_receipt");
    }

    #[test]
    fn corrupt_snapshot_is_never_published_as_missing_evidence() {
        let good = accepted_state(0, 0);
        let key = stream_key("project-1", "node-1", "monitor-1");
        for mutate in [0, 1, 2, 3, 4, 5, 6] {
            let mut state = good.clone();
            let stream = state.streams.get_mut(&key).unwrap();
            match mutate {
                0 => stream.head_sequence = 99,
                1 => stream.head_observation_id = "wrong".into(),
                2 => stream.observations.get_mut(&1).unwrap().observation_digest = "wrong".into(),
                3 => {
                    stream
                        .observations
                        .get_mut(&1)
                        .unwrap()
                        .observation
                        .project_id = Some("other".into())
                }
                4 => {
                    stream
                        .observations
                        .get_mut(&1)
                        .unwrap()
                        .server_project_scope = "other".into()
                }
                5 => stream.observations.get_mut(&1).unwrap().observation.node_id = "wrong".into(),
                _ => {
                    stream
                        .observations
                        .get_mut(&1)
                        .unwrap()
                        .observation
                        .observed_at_utc = "malformed".into()
                }
            }
            assert!(matches!(
                MonitoringEvidenceSnapshot::from_state(state),
                Err(MonitoringCheckError::InvalidEvidence(_))
            ));
        }
        assert!(MonitoringEvidenceSnapshot::from_state(DMonitorStateV1::empty()).is_ok());
    }

    struct CorruptSource(DMonitorStateV1);
    impl DMonitorEvidenceSource for CorruptSource {
        fn snapshot(&self) -> Result<MonitoringEvidenceSnapshot, MonitoringCheckError> {
            MonitoringEvidenceSnapshot::from_state(self.0.clone())
        }
    }
    #[tokio::test]
    async fn corrupt_source_is_error_and_busy_publication_is_availability_error() {
        let mut corrupt = accepted_state(0, 0);
        corrupt.streams.values_mut().next().unwrap().head_sequence = 99;
        let check = DMonitorFreshnessCheck::new(
            Arc::new(CorruptSource(corrupt)),
            DMonitorStreamSelector::new("project-1", "node-1", "monitor-1").unwrap(),
            policy(),
        )
        .unwrap();
        assert!(matches!(
            check.evaluate(utc(0)).await,
            Err(MonitoringCheckError::InvalidEvidence(_))
        ));
        let source = Arc::new(RwLock::new(
            MonitoringEvidenceSnapshot::from_state(accepted_state(0, 0)).unwrap(),
        ));
        let check = DMonitorFreshnessCheck::new(
            source.clone(),
            DMonitorStreamSelector::new("project-1", "node-1", "monitor-1").unwrap(),
            policy(),
        )
        .unwrap();
        let _writer = source.write().unwrap();
        assert!(matches!(
            check.evaluate(utc(0)).await,
            Err(MonitoringCheckError::EvidenceUnavailable(_))
        ));
    }
}
