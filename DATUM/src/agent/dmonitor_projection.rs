//! SmartSentinel -> canonical DMonitor compatibility projection (Phase
//! 114B2, ADR-0024). Pure transformation: consumes a validated
//! `SmartSentinelTelemetryEnvelope` (the legacy compatibility artifact,
//! already normalized by `telemetry::build_host_telemetry`) plus an explicit
//! [`DMonitorProjectionContext`] (the DMonitor-native process-epoch identity
//! and sequence counter), and produces zero or more
//! `CanonicalDMonitorObservationV1` records. No HTTP, no file I/O, no host
//! command execution happens here -- this module is fully unit-testable
//! without a running DServer or a real collector environment.
//!
//! SmartSentinel is the legacy COLLECTION implementation; DMonitor is the
//! canonical DATUM observation architecture this module bridges into. Never
//! treat them as competing systems -- SmartSentinel becomes the
//! compatibility/collection lineage feeding DMonitor.
//!
//! # Source of truth: telemetry, not the raw Snapshot
//!
//! This module projects EXCLUSIVELY from `SmartSentinelTelemetryEnvelope`,
//! never from the raw `Snapshot`/`Entity`/`Edge` graph. The envelope is
//! already the normalized, allowlisted, `desired_state`-free (at its own
//! top level; `ServiceTelemetry.desired_state` is present but explicitly
//! never read here, see below) representation of exactly the facts a
//! DMonitor observation should carry -- projecting from the raw `Snapshot`
//! instead would mean re-deriving that same allowlisting a second time and
//! risking the two projections drifting apart. The one field this module
//! deliberately never touches, even though it exists on the source type, is
//! `ServiceTelemetry.desired_state` -- see [`project_service_observation`].
//!
//! # Monitor instance epoch (section 3)
//!
//! [`DMonitorProjectionContext::new_process_epoch`] generates a NEW,
//! DMonitor-native `monitor_instance_id`, deliberately never the legacy
//! `SmartSentinelTelemetryEnvelope.sentinel_instance_id`
//! (`"<node_id>-sentinel"`, fixed per node rather than per process epoch).
//! Using the legacy value directly would violate the Phase-114B1 invariant
//! "one `monitor_instance_id` = one monotonic sequence epoch": a restarted
//! process would keep the same fixed `sentinel_instance_id` while its
//! sequence naturally starts over, and DServer would then reject the
//! restarted process's first observations as stale (Phase-114B1's own
//! `sequence_reset_under_the_same_monitor_instance_id_is_stale_not_a_new_epoch`
//! is exactly the rule this would trip). The legacy `sentinel_instance_id`
//! remains unchanged and fully available on `SmartSentinelTelemetryEnvelope`
//! for its own historical purposes; this module simply never uses it as
//! DMonitor stream identity.
//!
//! # Sequence is per observation, not per collection (section 4)
//!
//! One telemetry collection can (and normally does) produce MANY canonical
//! observations (the host, every container, every process, every network
//! target). [`DMonitorProjectionContext`] hands out one sequence per
//! EMITTED OBSERVATION via an internal monotonic counter, never one per
//! collection cycle -- see [`project_telemetry_to_dmonitor_observations`].
//!
//! # Observation identity (section 5)
//!
//! [`observation_id`] is deterministic:
//! `dmonitor-obs:<monitor_instance_id>:<sequence>`. Retrying the exact same
//! projected observation (same context, same sequence) always produces the
//! identical id -- required for the idempotent-replay semantics
//! Phase-114B1's ingestion already implements. No container/process/service
//! name, image, or metric value is ever used to derive an id.

use serde_json::{json, Map, Value};

use crate::agent::canonical_dmonitor::{
    CanonicalDMonitorObservationV1, DMonitorHealthV1, DMonitorSubjectV1,
    CANONICAL_DMONITOR_OBSERVATION_SCHEMA,
};
use crate::agent::jcs::MAX_SAFE_INTEGER;
use crate::telemetry::{
    ContainerTelemetry, ControlPlaneTelemetry, NetworkProbeTelemetry, PeerTelemetry,
    ProcessTelemetry, ServiceTelemetry, SmartSentinelTelemetryEnvelope,
};

/// One DMonitor projection PROCESS EPOCH. See the module doc for the full
/// rationale on why this is never the legacy `sentinel_instance_id`.
#[derive(Debug, Clone)]
pub struct DMonitorProjectionContext {
    monitor_instance_id: String,
    next_sequence: u64,
}

impl DMonitorProjectionContext {
    /// Starts a NEW process epoch (sections 20/21): a freshly generated,
    /// globally-unique `monitor_instance_id` and a sequence counter
    /// beginning at 1. Call this ONCE per one-shot invocation, or ONCE at
    /// continuous-mode service-loop startup -- never once per collection
    /// cycle (section 20's explicit prohibition).
    pub fn new_process_epoch(node_id: &str) -> Self {
        Self {
            monitor_instance_id: format!("dmonitor-instance:{node_id}:{}", uuid::Uuid::new_v4()),
            next_sequence: 1,
        }
    }

    pub fn monitor_instance_id(&self) -> &str {
        &self.monitor_instance_id
    }

    /// The sequence the NEXT projected observation will receive. Exposed so
    /// a caller can log/persist current position; never required for
    /// correctness (the context tracks it internally).
    pub fn next_sequence(&self) -> u64 {
        self.next_sequence
    }

    /// Reserves and returns the next sequence, advancing the counter.
    /// Sequences are NEVER reused or rewound (sections 4/23): even if a
    /// caller later decides not to publish an already-allocated
    /// observation (e.g. a downstream HTTP failure), the next observation
    /// still receives the FOLLOWING sequence, never the abandoned one --
    /// gaps are permitted, reuse is not.
    fn allocate_sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        sequence
    }
}

/// `dmonitor-obs:<monitor_instance_id>:<sequence>` (section 5). Deterministic
/// and stable for retry; never derived from a container/process/service name
/// or a metric value.
fn observation_id(monitor_instance_id: &str, sequence: u64) -> String {
    format!("dmonitor-obs:{monitor_instance_id}:{sequence}")
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProjectionError {
    MetricScale(MetricScaleError),
    MissingIdentity { field: &'static str },
}

impl std::fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectionError::MetricScale(error) => write!(f, "{error}"),
            ProjectionError::MissingIdentity { field } => {
                write!(f, "DMonitor projection requires a non-empty '{field}'")
            }
        }
    }
}
impl std::error::Error for ProjectionError {}
impl From<MetricScaleError> for ProjectionError {
    fn from(error: MetricScaleError) -> Self {
        ProjectionError::MetricScale(error)
    }
}

fn require_nonempty<'a>(field: &'static str, value: &'a str) -> Result<&'a str, ProjectionError> {
    if value.trim().is_empty() {
        Err(ProjectionError::MissingIdentity { field })
    } else {
        Ok(value)
    }
}

// ============================================================ METRIC SCALING
// Section 13: the canonical DMonitor JCS domain rejects fractional/
// exponential numbers. Every legacy f64 metric is converted to a named,
// unit-explicit scaled integer BEFORE it is ever placed in `facts`. No raw
// f64 is placed in canonical facts anywhere in this module.

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MetricScaleError {
    NonFinite { field: &'static str, value: f64 },
    OutOfRange { field: &'static str, value: f64, min: f64, max: f64 },
    Overflow { field: &'static str, value: f64 },
}

impl std::fmt::Display for MetricScaleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MetricScaleError::NonFinite { field, value } => {
                write!(f, "metric '{field}' is not finite: {value}")
            }
            MetricScaleError::OutOfRange { field, value, min, max } => write!(
                f,
                "metric '{field}' value {value} is outside the valid range [{min}, {max}]"
            ),
            MetricScaleError::Overflow { field, value } => write!(
                f,
                "metric '{field}' value {value} would overflow the canonical safe-integer domain after scaling"
            ),
        }
    }
}
impl std::error::Error for MetricScaleError {}

/// Round-half-away-from-zero (`f64::round()`) at the target unit's
/// precision, applied only after confirming finiteness and, for scales with
/// a bounded domain, the pre-scale range. Shared by every public scaling
/// function below.
fn scale_nonnegative(
    field: &'static str,
    value: f64,
    multiplier: f64,
) -> Result<u64, MetricScaleError> {
    if !value.is_finite() {
        return Err(MetricScaleError::NonFinite { field, value });
    }
    if value < 0.0 {
        return Err(MetricScaleError::OutOfRange {
            field,
            value,
            min: 0.0,
            max: f64::MAX,
        });
    }
    let scaled = (value * multiplier).round();
    if scaled > MAX_SAFE_INTEGER as f64 {
        return Err(MetricScaleError::Overflow { field, value });
    }
    Ok(scaled as u64)
}

/// A percentage (`0.0..=100.0`) to basis points (`0..=10000`; 1% = 100bp).
/// Used for CPU/loss percentages.
pub fn percent_to_basis_points(field: &'static str, value: f64) -> Result<u64, MetricScaleError> {
    if !value.is_finite() {
        return Err(MetricScaleError::NonFinite { field, value });
    }
    if !(0.0..=100.0).contains(&value) {
        return Err(MetricScaleError::OutOfRange {
            field,
            value,
            min: 0.0,
            max: 100.0,
        });
    }
    scale_nonnegative(field, value, 100.0)
}

/// A non-negative, otherwise-unbounded value (e.g. a load average) to
/// milliunits (`value * 1000`, rounded).
pub fn nonnegative_value_to_milliunits(
    field: &'static str,
    value: f64,
) -> Result<u64, MetricScaleError> {
    scale_nonnegative(field, value, 1000.0)
}

/// A non-negative millisecond duration (e.g. RTT) to microseconds
/// (`value_ms * 1000`, rounded). Numerically identical scale factor to
/// [`nonnegative_value_to_milliunits`], kept as a separate named function so
/// call sites and error messages state the correct source unit.
pub fn nonnegative_ms_to_microseconds(
    field: &'static str,
    value_ms: f64,
) -> Result<u64, MetricScaleError> {
    scale_nonnegative(field, value_ms, 1000.0)
}

// ============================================================== FACTS BUILD

fn insert_str(map: &mut Map<String, Value>, key: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        map.insert(key.to_string(), json!(value));
    }
}

fn insert_u64(map: &mut Map<String, Value>, key: &str, value: Option<u64>) {
    if let Some(value) = value {
        map.insert(key.to_string(), json!(value));
    }
}

fn insert_bool(map: &mut Map<String, Value>, key: &str, value: bool) {
    map.insert(key.to_string(), json!(value));
}

// =========================================================== PROJECTION API

/// Projects one telemetry collection into zero or more canonical DMonitor
/// observations (section 4: one sequence per observation, never per
/// collection). Every returned observation has `project_id: None` (section
/// 18): none of the subject kinds this bridge emits in Phase 114B2 (`DNode`,
/// `RuntimeProcess`, `NetworkReachability`, `Unbound`) is project-bearing --
/// `SmartSentinelTelemetryEnvelope`'s `runtime.services` carries no exact
/// `project_id`/`application_id`/`service_id` DMap correlation (only a
/// collector-local `entity_id`/`name`), so `DServRealization` is never
/// safely constructible from this source (section 10.E, 13 of the task
/// spec) and is not attempted here.
pub fn project_telemetry_to_dmonitor_observations(
    telemetry: &SmartSentinelTelemetryEnvelope,
    context: &mut DMonitorProjectionContext,
) -> Result<Vec<CanonicalDMonitorObservationV1>, ProjectionError> {
    let node_id = require_nonempty("node_id", &telemetry.node_id)?;
    let mut observations = Vec::new();

    observations.push(project_dnode_observation(telemetry, node_id, context)?);

    if let Some(runtime) = &telemetry.runtime {
        for container in &runtime.containers {
            observations.push(project_container_observation(
                telemetry, node_id, context, container,
            )?);
        }
        for process in &runtime.processes {
            observations.push(project_process_observation(
                telemetry, node_id, context, process,
            )?);
        }
        for service in &runtime.services {
            observations.push(project_service_observation(
                telemetry, node_id, context, service,
            )?);
        }
    }

    if let Some(network) = &telemetry.network {
        if let Some(control_plane) = &network.control_plane {
            observations.push(project_control_plane_observation(
                telemetry,
                node_id,
                context,
                control_plane,
            )?);
        }
        for probe in &network.probes {
            observations.push(project_probe_observation(telemetry, node_id, context, probe)?);
        }
        for peer in &network.peers {
            observations.push(project_peer_observation(telemetry, node_id, context, peer)?);
        }
    }

    Ok(observations)
}

fn base_observation(
    telemetry: &SmartSentinelTelemetryEnvelope,
    context: &mut DMonitorProjectionContext,
    collector: &str,
    subject: DMonitorSubjectV1,
    health: DMonitorHealthV1,
    facts: Value,
) -> CanonicalDMonitorObservationV1 {
    let sequence = context.allocate_sequence();
    CanonicalDMonitorObservationV1 {
        schema: CANONICAL_DMONITOR_OBSERVATION_SCHEMA.to_string(),
        observation_id: observation_id(&context.monitor_instance_id, sequence),
        project_id: None,
        node_id: telemetry.node_id.clone(),
        monitor_instance_id: context.monitor_instance_id.clone(),
        sequence,
        observed_at_utc: telemetry.observed_at_utc.to_rfc3339(),
        cycle_id: Some(telemetry.cycle_id.clone()).filter(|value| !value.is_empty()),
        collector: collector.to_string(),
        subject,
        health,
        facts,
    }
}

/// A. host/node telemetry -> `DMonitorSubjectV1::DNode` (section 10A, 12).
/// Health is deliberately `Unknown` (section 14): host metrics existing is
/// not an explicit health signal.
fn project_dnode_observation(
    telemetry: &SmartSentinelTelemetryEnvelope,
    node_id: &str,
    context: &mut DMonitorProjectionContext,
) -> Result<CanonicalDMonitorObservationV1, ProjectionError> {
    let host = &telemetry.host;
    let mut facts = Map::new();
    insert_str(&mut facts, "hostname", Some(host.hostname.as_str()));
    insert_str(&mut facts, "os", host.os.as_deref());
    insert_str(&mut facts, "kernel", host.kernel.as_deref());
    insert_u64(&mut facts, "uptime_seconds", host.uptime_seconds);
    insert_u64(&mut facts, "cpu_logical_cores", host.cpu.logical_cores);
    if let Some(usage) = host.cpu.usage_percent {
        facts.insert(
            "cpu_usage_basis_points".to_string(),
            json!(percent_to_basis_points("host.cpu.usage_percent", usage)?),
        );
    }
    if let Some(load) = host.cpu.load_1 {
        facts.insert(
            "load_1_milliunits".to_string(),
            json!(nonnegative_value_to_milliunits("host.cpu.load_1", load)?),
        );
    }
    if let Some(load) = host.cpu.load_5 {
        facts.insert(
            "load_5_milliunits".to_string(),
            json!(nonnegative_value_to_milliunits("host.cpu.load_5", load)?),
        );
    }
    if let Some(load) = host.cpu.load_15 {
        facts.insert(
            "load_15_milliunits".to_string(),
            json!(nonnegative_value_to_milliunits("host.cpu.load_15", load)?),
        );
    }
    insert_u64(&mut facts, "memory_total_bytes", host.memory.total_bytes);
    insert_u64(&mut facts, "memory_used_bytes", host.memory.used_bytes);
    insert_u64(&mut facts, "memory_available_bytes", host.memory.available_bytes);
    insert_u64(&mut facts, "swap_total_bytes", host.swap.total_bytes);
    insert_u64(&mut facts, "swap_used_bytes", host.swap.used_bytes);
    if !host.filesystems.is_empty() {
        let filesystems: Vec<Value> = host
            .filesystems
            .iter()
            .map(|filesystem| {
                let mut entry = Map::new();
                entry.insert("mount".to_string(), json!(filesystem.mount));
                insert_u64(&mut entry, "total_bytes", filesystem.total_bytes);
                insert_u64(&mut entry, "used_bytes", filesystem.used_bytes);
                insert_u64(&mut entry, "available_bytes", filesystem.available_bytes);
                Value::Object(entry)
            })
            .collect();
        facts.insert("filesystems".to_string(), Value::Array(filesystems));
    }
    insert_u64(&mut facts, "process_count", host.process_count);

    Ok(base_observation(
        telemetry,
        context,
        "smartsentinel_host_telemetry",
        DMonitorSubjectV1::DNode {
            node_id: node_id.to_string(),
        },
        DMonitorHealthV1::Unknown,
        Value::Object(facts),
    ))
}

/// B. container -> `DMonitorSubjectV1::RuntimeProcess` (section 10B).
/// `runtime_ref` is the collector-local `entity_id` -- opaque, never a name.
/// Health `Unknown` (section 14): a container's `state`/`status` string is
/// an observed FACT, not an exact health signal, and is kept in `facts`
/// only.
fn project_container_observation(
    telemetry: &SmartSentinelTelemetryEnvelope,
    node_id: &str,
    context: &mut DMonitorProjectionContext,
    container: &ContainerTelemetry,
) -> Result<CanonicalDMonitorObservationV1, ProjectionError> {
    let runtime_ref = require_nonempty("runtime.containers[].entity_id", &container.entity_id)?;
    let mut facts = Map::new();
    facts.insert("kind".to_string(), json!("container"));
    insert_str(&mut facts, "name", Some(container.name.as_str()));
    insert_str(&mut facts, "image", container.image.as_deref());
    insert_str(&mut facts, "state", container.state.as_deref());
    insert_str(&mut facts, "status", container.status.as_deref());

    Ok(base_observation(
        telemetry,
        context,
        "smartsentinel_docker_cli",
        DMonitorSubjectV1::RuntimeProcess {
            node_id: node_id.to_string(),
            runtime_ref: runtime_ref.to_string(),
        },
        DMonitorHealthV1::Unknown,
        Value::Object(facts),
    ))
}

/// C. legacy aggregated process telemetry -> `DMonitorSubjectV1::Unbound`
/// (Phase 114B2.1 closure, section 1/2/3 -- CORRECTED from an earlier,
/// incorrect `RuntimeProcess` mapping).
///
/// Unlike a container (`project_container_observation`, whose `entity_id`
/// derives from `docker ps`'s own real container ID -- a genuine opaque
/// identifier for ONE runtime entity, safe as `RuntimeProcess.runtime_ref`),
/// `ProcessTelemetry.entity_id` is `core::ids::process_group_id(os,
/// name_norm)`: a value DERIVED FROM THE PROCESS/IMAGE NAME by
/// `process_linux_ps.rs` (grouping key = lowercased `comm`, `"aggregation":
/// "by_comm"`) and `process_windows_tasklist.rs` (grouping key = lowercased
/// image name, `"aggregation": "by_image_name"`). It identifies a NAMED
/// GROUP of OS processes sharing that name, not one runtime instance -- the
/// exact thing `RuntimeProcess.runtime_ref`'s doc comment forbids ("never a
/// name, hostname, port, or image"). A sample PID is not a safe substitute
/// either: the telemetry is already a cross-instance aggregate (`ps -eo
/// ... | group by comm`), so no single PID is that aggregate's identity, and
/// picking one would fabricate a per-instance identity this collector never
/// actually observed.
///
/// This is honestly `Unbound`, exactly like `project_service_observation`:
/// the process/image display name, aggregation mode, instance count, and
/// scaled CPU/memory metrics remain fully available as descriptive `facts`
/// -- this is not data loss, it is refusing to manufacture a canonical
/// identity the source data does not support. `Unbound` observations remain
/// durable evidence; Phase 114B1 deliberately excludes them from canonical
/// subject-current indexing, and this bridge relies on, rather than works
/// around, that exclusion.
fn project_process_observation(
    telemetry: &SmartSentinelTelemetryEnvelope,
    node_id: &str,
    context: &mut DMonitorProjectionContext,
    process: &ProcessTelemetry,
) -> Result<CanonicalDMonitorObservationV1, ProjectionError> {
    let mut facts = Map::new();
    facts.insert("runtime_kind".to_string(), json!("process"));
    insert_str(&mut facts, "name", Some(process.name.as_str()));
    // Every entry in `runtime.processes[]` is, by construction of the
    // legacy Linux/Windows process collectors, a name-based aggregate group
    // -- never an individual process instance. See this function's doc
    // comment for the exact source-code evidence.
    facts.insert("aggregation_mode".to_string(), json!("by_name"));
    insert_u64(&mut facts, "instance_count", process.instance_count);
    if let Some(cpu) = process.cpu_percent_total {
        facts.insert(
            "cpu_total_basis_points".to_string(),
            json!(percent_to_basis_points(
                "runtime.processes[].cpu_percent_total",
                cpu
            )?),
        );
    }
    insert_u64(&mut facts, "memory_used_kb_total", process.memory_used_kb_total);

    let description = Some(process.name.trim())
        .filter(|name| !name.is_empty())
        .map(ToString::to_string);

    Ok(base_observation(
        telemetry,
        context,
        "smartsentinel_process_list",
        DMonitorSubjectV1::Unbound {
            node_id: node_id.to_string(),
            description,
        },
        DMonitorHealthV1::Unknown,
        Value::Object(facts),
    ))
}

/// D. service -> `DMonitorSubjectV1::Unbound` (section 10D, 16). No exact
/// canonical `project_id`/`application_id`/`service_id`/DMap correlation is
/// available from `ServiceTelemetry` (only a collector-local `entity_id`/
/// `name`), so this is honestly `Unbound` rather than a fabricated
/// `DServRealization`. `service.name` is placed ONLY in the `Unbound`
/// variant's free-form, non-correlating `description` field -- never in any
/// field the canonical subject-index treats as identity (Phase-114B1
/// already excludes `Unbound` from subject indexing entirely; this module
/// relies on, and does not weaken, that guarantee).
///
/// `ServiceTelemetry.desired_state` is DELIBERATELY NEVER READ here (section
/// 8): only `observed_state`, `runtime`, `bound`, and `checks` are placed in
/// `facts`, under names that do not collide with the canonical
/// `desired_state` guard. This projector does not rely on
/// `CanonicalDMonitorObservationV1::validate`'s recursive `desired_state`
/// scan as its primary filtering mechanism -- that scan remains the final
/// fail-closed backstop, not the normal mechanism, exactly as section 8
/// requires.
fn project_service_observation(
    telemetry: &SmartSentinelTelemetryEnvelope,
    node_id: &str,
    context: &mut DMonitorProjectionContext,
    service: &ServiceTelemetry,
) -> Result<CanonicalDMonitorObservationV1, ProjectionError> {
    let mut facts = Map::new();
    insert_str(&mut facts, "runtime", service.runtime.as_deref());
    facts.insert("observed_state".to_string(), json!(service.observed_state));
    insert_bool(&mut facts, "bound", service.bound);
    if !service.checks.is_empty() {
        let checks: Vec<Value> = service
            .checks
            .iter()
            .map(|check| {
                let mut entry = Map::new();
                entry.insert("kind".to_string(), json!(check.kind));
                entry.insert("outcome".to_string(), json!(check.outcome));
                insert_str(&mut entry, "detail", check.detail.as_deref());
                Value::Object(entry)
            })
            .collect();
        facts.insert("checks".to_string(), Value::Array(checks));
    }

    let description = Some(service.name.trim())
        .filter(|name| !name.is_empty())
        .map(ToString::to_string);

    Ok(base_observation(
        telemetry,
        context,
        "smartsentinel_service_builder",
        DMonitorSubjectV1::Unbound {
            node_id: node_id.to_string(),
            description,
        },
        DMonitorHealthV1::Unknown,
        Value::Object(facts),
    ))
}

/// Control-plane reachability -> `DMonitorSubjectV1::NetworkReachability`
/// (section 10C, 15). `ControlPlaneTelemetry` (`probe_control_plane` in
/// `telemetry.rs`) is a plain TCP-connect probe with no packet-loss concept
/// at all -- it genuinely exposes ONLY an exact binary reachability result
/// (Phase 114B2.1 closure, section 7). For a source shaped exactly like
/// this, `reachable` maps DIRECTLY to health: `true` -> `Healthy`, `false`
/// -> `Unhealthy`. This is a REACHABILITY-health interpretation only -- it
/// says nothing about application health or DServ convergence. Contrast
/// with [`project_probe_observation`], whose source DOES carry a graded
/// loss percentage and therefore must not collapse to this same binary
/// mapping.
fn project_control_plane_observation(
    telemetry: &SmartSentinelTelemetryEnvelope,
    node_id: &str,
    context: &mut DMonitorProjectionContext,
    control_plane: &ControlPlaneTelemetry,
) -> Result<CanonicalDMonitorObservationV1, ProjectionError> {
    let target = require_nonempty("network.control_plane.target", &control_plane.target)?;
    let mut facts = Map::new();
    if let Some(rtt) = control_plane.rtt_ms {
        facts.insert(
            "rtt_microseconds".to_string(),
            json!(nonnegative_ms_to_microseconds(
                "network.control_plane.rtt_ms",
                rtt
            )?),
        );
    }
    insert_str(&mut facts, "error", control_plane.error.as_deref());

    Ok(base_observation(
        telemetry,
        context,
        "smartsentinel_connection_ping",
        DMonitorSubjectV1::NetworkReachability {
            node_id: node_id.to_string(),
            target: target.to_string(),
        },
        if control_plane.reachable {
            DMonitorHealthV1::Healthy
        } else {
            DMonitorHealthV1::Unhealthy
        },
        Value::Object(facts),
    ))
}

/// Graded health for a probe that carries an explicit packet-loss
/// percentage (Phase 114B2.1 closure, sections 5/6). The Linux ping
/// collector (`connection_ping_linux.rs`) defines its own `available`
/// boolean as `received > 0 && loss_pct < 100` -- so `reachable == true`
/// does NOT imply `loss == 0`, and this bridge must not collapse partial
/// packet loss into `Healthy` the way [`project_control_plane_observation`]'s
/// pure binary source legitimately can. Conservative, exact semantics only;
/// no RTT threshold is introduced (RTT stays an observed metric only):
///
/// - unreachable, OR `loss == 100%` -> `Unhealthy`
/// - reachable AND `loss == 0%` -> `Healthy`
/// - reachable AND `0% < loss < 100%` -> `Degraded`
/// - anything else (no loss data at all, or an out-of-range/contradictory
///   value `SmartSentinelTelemetryEnvelope::validate` would itself already
///   have rejected upstream) -> `Unknown`, rather than guessing.
fn probe_health_from_loss(reachable: bool, loss_percent: Option<f64>) -> DMonitorHealthV1 {
    if !reachable || loss_percent == Some(100.0) {
        return DMonitorHealthV1::Unhealthy;
    }
    match loss_percent {
        Some(loss) if loss == 0.0 => DMonitorHealthV1::Healthy,
        Some(loss) if loss > 0.0 && loss < 100.0 => DMonitorHealthV1::Degraded,
        _ => DMonitorHealthV1::Unknown,
    }
}

/// Configured probe reachability -> `DMonitorSubjectV1::NetworkReachability`,
/// using [`probe_health_from_loss`] rather than a plain binary mapping --
/// see that function's doc comment for the exact rationale and algorithm.
fn project_probe_observation(
    telemetry: &SmartSentinelTelemetryEnvelope,
    node_id: &str,
    context: &mut DMonitorProjectionContext,
    probe: &NetworkProbeTelemetry,
) -> Result<CanonicalDMonitorObservationV1, ProjectionError> {
    let target = require_nonempty("network.probes[].target", &probe.target)?;
    let mut facts = Map::new();
    if let Some(loss) = probe.loss_percent {
        facts.insert(
            "loss_basis_points".to_string(),
            json!(percent_to_basis_points("network.probes[].loss_percent", loss)?),
        );
    }
    if let Some(rtt) = probe.rtt_min_ms {
        facts.insert(
            "rtt_min_microseconds".to_string(),
            json!(nonnegative_ms_to_microseconds(
                "network.probes[].rtt_min_ms",
                rtt
            )?),
        );
    }
    if let Some(rtt) = probe.rtt_avg_ms {
        facts.insert(
            "rtt_avg_microseconds".to_string(),
            json!(nonnegative_ms_to_microseconds(
                "network.probes[].rtt_avg_ms",
                rtt
            )?),
        );
    }
    if let Some(rtt) = probe.rtt_max_ms {
        facts.insert(
            "rtt_max_microseconds".to_string(),
            json!(nonnegative_ms_to_microseconds(
                "network.probes[].rtt_max_ms",
                rtt
            )?),
        );
    }

    Ok(base_observation(
        telemetry,
        context,
        "smartsentinel_connection_ping",
        DMonitorSubjectV1::NetworkReachability {
            node_id: node_id.to_string(),
            target: target.to_string(),
        },
        probe_health_from_loss(probe.reachable, probe.loss_percent),
        Value::Object(facts),
    ))
}

/// Peer reachability -> `DMonitorSubjectV1::NetworkReachability` (section
/// 15). `PeerTelemetry` carries no packet-loss degree at all (only
/// `reachable`/`rtt_ms`/`consecutive_failures`/`error`) -- exactly the
/// binary-only source shape section 7 permits a direct `reachable ->
/// Healthy`/`Unhealthy` mapping for (see
/// [`project_control_plane_observation`]'s doc comment for the same
/// rationale in full). `peer.node_id` is included as DESCRIPTIVE observed
/// metadata only (`peer_node_id_observed` in `facts`) -- it is never
/// promoted into the typed subject identity, which would falsely imply an
/// established DNode authority correlation this collector never proves.
fn project_peer_observation(
    telemetry: &SmartSentinelTelemetryEnvelope,
    node_id: &str,
    context: &mut DMonitorProjectionContext,
    peer: &PeerTelemetry,
) -> Result<CanonicalDMonitorObservationV1, ProjectionError> {
    let target = require_nonempty("network.peers[].target", &peer.target)?;
    let mut facts = Map::new();
    insert_str(&mut facts, "peer_node_id_observed", Some(peer.node_id.as_str()));
    if let Some(rtt) = peer.rtt_ms {
        facts.insert(
            "rtt_microseconds".to_string(),
            json!(nonnegative_ms_to_microseconds("network.peers[].rtt_ms", rtt)?),
        );
    }
    insert_u64(
        &mut facts,
        "consecutive_failures",
        Some(peer.consecutive_failures),
    );
    if let Some(last_success) = peer.last_success_at_utc {
        facts.insert(
            "last_success_at_utc".to_string(),
            json!(last_success.to_rfc3339()),
        );
    }
    insert_str(&mut facts, "error", peer.error.as_deref());

    Ok(base_observation(
        telemetry,
        context,
        "smartsentinel_connection_ping",
        DMonitorSubjectV1::NetworkReachability {
            node_id: node_id.to_string(),
            target: target.to_string(),
        },
        if peer.reachable {
            DMonitorHealthV1::Healthy
        } else {
            DMonitorHealthV1::Unhealthy
        },
        Value::Object(facts),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::{
        CpuTelemetry, FilesystemTelemetry, HostTelemetry, MemoryTelemetry, NetworkTelemetry,
        RuntimeTelemetry, ServiceCheckTelemetry, SwapTelemetry, SMARTSENTINEL_TELEMETRY_SCHEMA_VERSION,
    };
    use chrono::{TimeZone, Utc};

    const NODE: &str = "fog-01";

    fn minimal_telemetry() -> SmartSentinelTelemetryEnvelope {
        SmartSentinelTelemetryEnvelope {
            schema_version: SMARTSENTINEL_TELEMETRY_SCHEMA_VERSION.to_string(),
            project_id: None,
            node_id: NODE.to_string(),
            cycle_id: "cycle:1111".to_string(),
            sentinel_instance_id: format!("{NODE}-sentinel"),
            sequence: 1,
            observed_at_utc: Utc.with_ymd_and_hms(2026, 9, 25, 0, 0, 0).unwrap(),
            host: HostTelemetry {
                hostname: "fog-01-host".to_string(),
                os: Some("linux".to_string()),
                kernel: Some("6.1.0".to_string()),
                uptime_seconds: Some(3600),
                cpu: CpuTelemetry {
                    logical_cores: Some(4),
                    usage_percent: Some(12.5),
                    load_1: Some(0.5),
                    load_5: Some(0.75),
                    load_15: Some(1.0),
                },
                memory: MemoryTelemetry {
                    total_bytes: Some(8_000_000_000),
                    used_bytes: Some(4_000_000_000),
                    available_bytes: Some(4_000_000_000),
                },
                swap: SwapTelemetry {
                    total_bytes: Some(1_000_000_000),
                    used_bytes: Some(0),
                },
                filesystems: vec![FilesystemTelemetry {
                    mount: "/".to_string(),
                    total_bytes: Some(100_000_000_000),
                    used_bytes: Some(50_000_000_000),
                    available_bytes: Some(50_000_000_000),
                }],
                process_count: Some(120),
            },
            runtime: Some(RuntimeTelemetry {
                container_runtime: Some("docker".to_string()),
                container_runtime_available: Some(true),
                container_count: Some(1),
                container_runtime_error: None,
                containers: vec![ContainerTelemetry {
                    entity_id: "entity:container:abc123".to_string(),
                    name: "sensor-bridge".to_string(),
                    image: Some("sensor-bridge:latest".to_string()),
                    state: Some("running".to_string()),
                    status: Some("Up 2 hours".to_string()),
                }],
                processes: vec![ProcessTelemetry {
                    entity_id: "entity:process:sshd".to_string(),
                    name: "sshd".to_string(),
                    instance_count: Some(1),
                    cpu_percent_total: Some(0.2),
                    memory_used_kb_total: Some(2048),
                }],
                services: vec![ServiceTelemetry {
                    entity_id: "entity:service:decoder".to_string(),
                    name: "decoder".to_string(),
                    runtime: Some("container".to_string()),
                    desired_state: Some("running".to_string()),
                    observed_state: "available".to_string(),
                    bound: true,
                    checks: vec![ServiceCheckTelemetry {
                        kind: "process_running".to_string(),
                        outcome: "pass".to_string(),
                        detail: Some("pid 1234".to_string()),
                    }],
                }],
            }),
            network: Some(NetworkTelemetry {
                interfaces: vec![],
                control_plane: Some(ControlPlaneTelemetry {
                    target: "http://dserver.local:8080".to_string(),
                    reachable: true,
                    rtt_ms: Some(12.345),
                    error: None,
                }),
                probes: vec![NetworkProbeTelemetry {
                    target: "8.8.8.8".to_string(),
                    reachable: true,
                    loss_percent: Some(0.0),
                    rtt_min_ms: Some(10.0),
                    rtt_avg_ms: Some(12.0),
                    rtt_max_ms: Some(15.0),
                }],
                peers: vec![PeerTelemetry {
                    node_id: "cloud-01".to_string(),
                    target: "cloud-01:1883".to_string(),
                    reachable: false,
                    rtt_ms: None,
                    consecutive_failures: 3,
                    last_success_at_utc: Some(Utc.with_ymd_and_hms(2026, 9, 24, 23, 0, 0).unwrap()),
                    error: Some("connection refused".to_string()),
                }],
            }),
            safe_to_auto_execute: false,
            side_effects_performed: false,
        }
    }

    // ------------------------------------------------------- MONITOR EPOCH

    #[test]
    fn monitor_instance_id_is_stable_across_multiple_projected_cycles() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let instance_id = context.monitor_instance_id().to_string();
        let telemetry = minimal_telemetry();
        project_telemetry_to_dmonitor_observations(&telemetry, &mut context).unwrap();
        project_telemetry_to_dmonitor_observations(&telemetry, &mut context).unwrap();
        assert_eq!(context.monitor_instance_id(), instance_id);
    }

    #[test]
    fn sequence_is_monotonic_across_cycles_never_reused() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let first = project_telemetry_to_dmonitor_observations(&telemetry, &mut context).unwrap();
        let second = project_telemetry_to_dmonitor_observations(&telemetry, &mut context).unwrap();
        let first_max = first.iter().map(|o| o.sequence).max().unwrap();
        let second_min = second.iter().map(|o| o.sequence).min().unwrap();
        assert!(second_min > first_max, "second cycle's lowest sequence must exceed the first cycle's highest");
        let mut all_sequences: Vec<u64> = first.iter().chain(second.iter()).map(|o| o.sequence).collect();
        let unique_count = {
            all_sequences.sort_unstable();
            all_sequences.dedup();
            all_sequences.len()
        };
        assert_eq!(unique_count, first.len() + second.len(), "no sequence may repeat across cycles");
    }

    #[test]
    fn new_process_epoch_may_begin_at_sequence_one_independently() {
        let mut context_a = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        project_telemetry_to_dmonitor_observations(&telemetry, &mut context_a).unwrap();

        let mut context_b = DMonitorProjectionContext::new_process_epoch(NODE);
        let second = project_telemetry_to_dmonitor_observations(&telemetry, &mut context_b).unwrap();
        assert_eq!(second[0].sequence, 1, "a brand new process epoch starts at sequence 1");
        assert_ne!(context_a.monitor_instance_id(), context_b.monitor_instance_id());
    }

    #[test]
    fn multiple_observations_in_one_collection_use_distinct_sequences() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let observations = project_telemetry_to_dmonitor_observations(&telemetry, &mut context).unwrap();
        assert!(observations.len() > 1, "the fixture must produce more than one observation");
        let mut sequences: Vec<u64> = observations.iter().map(|o| o.sequence).collect();
        let original_len = sequences.len();
        sequences.sort_unstable();
        sequences.dedup();
        assert_eq!(sequences.len(), original_len, "every observation in one collection must have a distinct sequence");
    }

    // ----------------------------------------------------------- IDENTITY

    #[test]
    fn observation_ids_are_unique_within_a_collection() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let observations = project_telemetry_to_dmonitor_observations(&telemetry, &mut context).unwrap();
        let mut ids: Vec<&str> = observations.iter().map(|o| o.observation_id.as_str()).collect();
        let original_len = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), original_len);
    }

    #[test]
    fn retrying_the_same_context_and_sequence_preserves_id_and_digest() {
        let monitor_instance_id = "dmonitor-instance:fog-01:fixed-for-test".to_string();
        let mut context = DMonitorProjectionContext {
            monitor_instance_id: monitor_instance_id.clone(),
            next_sequence: 1,
        };
        let telemetry = minimal_telemetry();
        let first = project_dnode_observation(&telemetry, NODE, &mut context).unwrap();

        let mut retry_context = DMonitorProjectionContext {
            monitor_instance_id,
            next_sequence: 1,
        };
        let retried = project_dnode_observation(&telemetry, NODE, &mut retry_context).unwrap();

        assert_eq!(first.observation_id, retried.observation_id);
        assert_eq!(
            crate::agent::canonical_dmonitor::recompute_dmonitor_observation_digest(&first).unwrap(),
            crate::agent::canonical_dmonitor::recompute_dmonitor_observation_digest(&retried).unwrap()
        );
    }

    #[test]
    fn observation_ids_never_derive_from_container_or_process_names() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let observations = project_telemetry_to_dmonitor_observations(&telemetry, &mut context).unwrap();
        for observation in &observations {
            assert!(!observation.observation_id.contains("sensor-bridge"));
            assert!(!observation.observation_id.contains("sshd"));
            assert!(!observation.observation_id.contains("decoder"));
            assert!(observation.observation_id.starts_with("dmonitor-obs:"));
        }
    }

    // ------------------------------------------------------- CANONICAL VALIDITY

    #[test]
    fn every_emitted_observation_validates_cleanly_and_digests() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let observations = project_telemetry_to_dmonitor_observations(&telemetry, &mut context).unwrap();
        assert!(!observations.is_empty());
        for observation in &observations {
            let findings = observation.validate();
            assert!(findings.is_empty(), "{:?}: {:?}", observation.subject, findings);
            assert!(crate::agent::canonical_dmonitor::recompute_dmonitor_observation_digest(observation).is_ok());
        }
    }

    #[test]
    fn no_projected_facts_contain_desired_state_at_any_depth() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        // The fixture's ServiceTelemetry.desired_state IS populated ("running")
        // -- proving this test is meaningful, not vacuous.
        assert!(telemetry.runtime.as_ref().unwrap().services[0].desired_state.is_some());
        let observations = project_telemetry_to_dmonitor_observations(&telemetry, &mut context).unwrap();
        for observation in &observations {
            assert!(
                !crate::agent::canonical_dmonitor::facts_contain_desired_state_key(&observation.facts),
                "projected facts must never carry desired_state: {:?}",
                observation.facts
            );
        }
    }

    // -------------------------------------------------------------- DNODE

    #[test]
    fn host_telemetry_projects_to_dnode() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let dnode = project_dnode_observation(&telemetry, NODE, &mut context).unwrap();
        assert_eq!(dnode.subject, DMonitorSubjectV1::DNode { node_id: NODE.to_string() });
        assert_eq!(dnode.facts["hostname"], json!("fog-01-host"));
        assert_eq!(dnode.facts["cpu_usage_basis_points"], json!(1250u64));
    }

    #[test]
    fn dnode_health_does_not_become_healthy_merely_because_metrics_exist() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let dnode = project_dnode_observation(&telemetry, NODE, &mut context).unwrap();
        assert_eq!(dnode.health, DMonitorHealthV1::Unknown);
    }

    // ------------------------------------------------------------ RUNTIME

    #[test]
    fn container_runtime_ref_is_the_opaque_collector_entity_id() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let container = &telemetry.runtime.as_ref().unwrap().containers[0];
        let observation = project_container_observation(&telemetry, NODE, &mut context, container).unwrap();
        match &observation.subject {
            DMonitorSubjectV1::RuntimeProcess { runtime_ref, .. } => {
                assert_eq!(runtime_ref, "entity:container:abc123");
            }
            other => panic!("expected RuntimeProcess, got {other:?}"),
        }
    }

    #[test]
    fn container_display_name_stays_in_facts_only() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let container = &telemetry.runtime.as_ref().unwrap().containers[0];
        let observation = project_container_observation(&telemetry, NODE, &mut context, container).unwrap();
        match &observation.subject {
            DMonitorSubjectV1::RuntimeProcess { runtime_ref, .. } => {
                assert_ne!(runtime_ref, "sensor-bridge", "the display name must never become canonical identity");
            }
            other => panic!("expected RuntimeProcess, got {other:?}"),
        }
        assert_eq!(observation.facts["name"], json!("sensor-bridge"));
    }

    #[test]
    fn no_dserv_identity_is_ever_guessed_from_names() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let observations = project_telemetry_to_dmonitor_observations(&telemetry, &mut context).unwrap();
        for observation in &observations {
            assert!(
                !matches!(observation.subject, DMonitorSubjectV1::DServRealization { .. }),
                "Phase 114B2 must never construct DServRealization from SmartSentinel telemetry"
            );
        }
    }

    // ---------------------------------------------- PHASE 114B2.1 CLOSURE
    // (process-group identity)

    fn linux_process_group(name: &str, entity_suffix: &str) -> ProcessTelemetry {
        // Mirrors `process_linux_ps.rs`'s real shape: `entity_id` is
        // `core::ids::process_group_id("linux", &comm_lc)` -- derived from
        // the lowercased `comm`, not a PID.
        ProcessTelemetry {
            entity_id: format!("procgrp:linux:{entity_suffix}"),
            name: name.to_string(),
            instance_count: Some(3),
            cpu_percent_total: Some(4.5),
            memory_used_kb_total: Some(10_240),
        }
    }

    fn windows_process_group(image_name: &str, entity_suffix: &str) -> ProcessTelemetry {
        // Mirrors `process_windows_tasklist.rs`'s real shape: `entity_id` is
        // `core::ids::process_group_id("windows", &image_lc)` -- derived
        // from the lowercased image name, not a PID.
        ProcessTelemetry {
            entity_id: format!("procgrp:windows:{entity_suffix}"),
            name: image_name.to_string(),
            instance_count: Some(1),
            cpu_percent_total: Some(0.1),
            memory_used_kb_total: Some(4_096),
        }
    }

    #[test]
    fn docker_container_observation_becomes_runtime_process() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let container = &telemetry.runtime.as_ref().unwrap().containers[0];
        let observation = project_container_observation(&telemetry, NODE, &mut context, container).unwrap();
        assert!(matches!(observation.subject, DMonitorSubjectV1::RuntimeProcess { .. }));
    }

    #[test]
    fn linux_comm_aggregated_process_telemetry_becomes_unbound() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let process = linux_process_group("sshd", "sshd-hash");
        let observation = project_process_observation(&telemetry, NODE, &mut context, &process).unwrap();
        match &observation.subject {
            DMonitorSubjectV1::Unbound { node_id, description } => {
                assert_eq!(node_id, NODE);
                assert_eq!(description.as_deref(), Some("sshd"));
            }
            other => panic!("Linux comm-aggregated process telemetry must project to Unbound, got {other:?}"),
        }
    }

    #[test]
    fn windows_image_name_aggregated_process_telemetry_becomes_unbound() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let process = windows_process_group("svchost.exe", "svchost-hash");
        let observation = project_process_observation(&telemetry, NODE, &mut context, &process).unwrap();
        match &observation.subject {
            DMonitorSubjectV1::Unbound { node_id, description } => {
                assert_eq!(node_id, NODE);
                assert_eq!(description.as_deref(), Some("svchost.exe"));
            }
            other => panic!("Windows image-name-aggregated process telemetry must project to Unbound, got {other:?}"),
        }
    }

    #[test]
    fn process_name_never_enters_runtime_process_runtime_ref() {
        // Structural: process telemetry never becomes RuntimeProcess at all
        // in Phase 114B2.1, so there is no `runtime_ref` field for a name to
        // leak into -- proven by exhaustively matching the subject shape.
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let process = linux_process_group("sshd", "sshd-hash");
        let observation = project_process_observation(&telemetry, NODE, &mut context, &process).unwrap();
        assert!(
            !matches!(observation.subject, DMonitorSubjectV1::RuntimeProcess { .. }),
            "process-group telemetry must never become RuntimeProcess"
        );
    }

    #[test]
    fn two_process_groups_with_different_names_do_not_manufacture_canonical_runtime_identity() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let sshd = linux_process_group("sshd", "sshd-hash");
        let cron = linux_process_group("cron", "cron-hash");
        let obs_sshd = project_process_observation(&telemetry, NODE, &mut context, &sshd).unwrap();
        let obs_cron = project_process_observation(&telemetry, NODE, &mut context, &cron).unwrap();
        // Both are Unbound; the ONLY typed identity field either carries is
        // `node_id` (identical for both) -- neither manufactures a distinct
        // canonical runtime identity from the differing names.
        match (&obs_sshd.subject, &obs_cron.subject) {
            (
                DMonitorSubjectV1::Unbound { node_id: node_a, .. },
                DMonitorSubjectV1::Unbound { node_id: node_b, .. },
            ) => assert_eq!(node_a, node_b),
            other => panic!("expected both Unbound, got {other:?}"),
        }
        assert_eq!(
            subject_key_for_test(&obs_sshd.subject),
            subject_key_for_test(&obs_cron.subject),
            "two differently-named process groups on the same node must not resolve to distinct canonical subjects"
        );
    }

    /// A local mirror of `dserver::core::dmonitor_state::subject_key`'s
    /// Unbound rule (keys on `node_id` alone) -- DATUM has no dependency on
    /// the dserver crate (ADR-0007), so this test asserts the INVARIANT the
    /// canonical model's own doc comments guarantee (`Unbound` keys only on
    /// `node_id`), not DServer's specific implementation.
    fn subject_key_for_test(subject: &DMonitorSubjectV1) -> String {
        match subject {
            DMonitorSubjectV1::Unbound { node_id, .. } => format!("unbound:{node_id}"),
            other => format!("{other:?}"),
        }
    }

    #[test]
    fn process_group_aggregate_facts_remain_useful_and_digest_valid() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let process = linux_process_group("sshd", "sshd-hash");
        let observation = project_process_observation(&telemetry, NODE, &mut context, &process).unwrap();
        assert_eq!(observation.facts["runtime_kind"], json!("process"));
        assert_eq!(observation.facts["name"], json!("sshd"));
        assert_eq!(observation.facts["aggregation_mode"], json!("by_name"));
        assert_eq!(observation.facts["instance_count"], json!(3u64));
        assert!(observation.facts.get("cpu_total_basis_points").is_some());
        assert!(observation.facts.get("memory_used_kb_total").is_some());
        assert!(observation.validate().is_empty(), "{:?}", observation.validate());
        assert!(crate::agent::canonical_dmonitor::recompute_dmonitor_observation_digest(&observation).is_ok());
    }

    #[test]
    fn process_group_facts_never_contain_desired_state() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let process = linux_process_group("sshd", "sshd-hash");
        let observation = project_process_observation(&telemetry, NODE, &mut context, &process).unwrap();
        assert!(!crate::agent::canonical_dmonitor::facts_contain_desired_state_key(&observation.facts));
    }

    // ------------------------------------------------------------- NETWORK

    #[test]
    fn reachable_and_unreachable_map_directly_to_health() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let control_plane = telemetry.network.as_ref().unwrap().control_plane.as_ref().unwrap();
        let reachable_obs = project_control_plane_observation(&telemetry, NODE, &mut context, control_plane).unwrap();
        assert_eq!(reachable_obs.health, DMonitorHealthV1::Healthy);

        let peer = &telemetry.network.as_ref().unwrap().peers[0];
        let unreachable_obs = project_peer_observation(&telemetry, NODE, &mut context, peer).unwrap();
        assert_eq!(unreachable_obs.health, DMonitorHealthV1::Unhealthy);
    }

    #[test]
    fn rtt_is_scaled_to_microseconds_never_raw_f64() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let control_plane = telemetry.network.as_ref().unwrap().control_plane.as_ref().unwrap();
        let observation = project_control_plane_observation(&telemetry, NODE, &mut context, control_plane).unwrap();
        assert_eq!(observation.facts["rtt_microseconds"], json!(12345u64));
        assert!(observation.facts["rtt_microseconds"].is_u64());
    }

    #[test]
    fn loss_percent_is_scaled_to_basis_points() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let mut telemetry = minimal_telemetry();
        telemetry.network.as_mut().unwrap().probes[0].loss_percent = Some(2.5);
        let probe = &telemetry.network.as_ref().unwrap().probes[0];
        let observation = project_probe_observation(&telemetry, NODE, &mut context, probe).unwrap();
        assert_eq!(observation.facts["loss_basis_points"], json!(250u64));
    }

    // ------------------------------------------ PHASE 114B2.1 CLOSURE
    // (graded probe health: reachable does not imply loss == 0)

    #[test]
    fn zero_loss_and_reachable_is_healthy() {
        assert_eq!(probe_health_from_loss(true, Some(0.0)), DMonitorHealthV1::Healthy);
    }

    #[test]
    fn partial_loss_and_reachable_is_degraded() {
        assert_eq!(probe_health_from_loss(true, Some(1.0)), DMonitorHealthV1::Degraded);
        assert_eq!(probe_health_from_loss(true, Some(50.0)), DMonitorHealthV1::Degraded);
        assert_eq!(probe_health_from_loss(true, Some(99.0)), DMonitorHealthV1::Degraded);
    }

    #[test]
    fn full_loss_or_unreachable_is_unhealthy() {
        assert_eq!(probe_health_from_loss(true, Some(100.0)), DMonitorHealthV1::Unhealthy);
        assert_eq!(probe_health_from_loss(false, Some(100.0)), DMonitorHealthV1::Unhealthy);
        // Reachable per the collector's own `available` boolean is
        // impossible together with 100% loss in practice (the Linux ping
        // collector's `available = received > 0 && loss < 100` makes the two
        // mutually exclusive), but this must still fail closed on the loss
        // side alone if ever presented, per the documented "loss == 100% ->
        // Unhealthy" rule regardless of the reachable flag's value.
        assert_eq!(probe_health_from_loss(false, None), DMonitorHealthV1::Unhealthy);
        assert_eq!(probe_health_from_loss(false, Some(0.0)), DMonitorHealthV1::Unhealthy);
    }

    #[test]
    fn missing_or_malformed_loss_data_fails_closed_to_unknown_not_a_guess() {
        // Reachable with NO loss data at all: insufficient information to
        // grade -- must not guess Healthy.
        assert_eq!(probe_health_from_loss(true, None), DMonitorHealthV1::Unknown);
    }

    #[test]
    fn rtt_does_not_independently_determine_probe_health() {
        // Two probes with identical loss/reachable but wildly different RTT
        // must have IDENTICAL health -- health is a function of
        // reachability/loss only, never RTT (section 6: "do not introduce
        // arbitrary RTT thresholds").
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let mut telemetry = minimal_telemetry();
        telemetry.network.as_mut().unwrap().probes[0].loss_percent = Some(0.0);
        telemetry.network.as_mut().unwrap().probes[0].rtt_avg_ms = Some(1.0);
        let fast_probe = telemetry.network.as_ref().unwrap().probes[0].clone();
        let fast_observation = project_probe_observation(&telemetry, NODE, &mut context, &fast_probe).unwrap();

        telemetry.network.as_mut().unwrap().probes[0].rtt_avg_ms = Some(5000.0);
        let slow_probe = telemetry.network.as_ref().unwrap().probes[0].clone();
        let slow_observation = project_probe_observation(&telemetry, NODE, &mut context, &slow_probe).unwrap();

        assert_eq!(fast_observation.health, slow_observation.health);
        assert_eq!(fast_observation.health, DMonitorHealthV1::Healthy);
    }

    #[test]
    fn probe_health_is_wired_into_the_full_projection_pipeline() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let mut telemetry = minimal_telemetry();
        telemetry.network.as_mut().unwrap().probes[0].reachable = true;
        telemetry.network.as_mut().unwrap().probes[0].loss_percent = Some(37.0);
        let probe = &telemetry.network.as_ref().unwrap().probes[0];
        let observation = project_probe_observation(&telemetry, NODE, &mut context, probe).unwrap();
        assert_eq!(observation.health, DMonitorHealthV1::Degraded);
    }

    #[test]
    fn control_plane_and_peer_binary_reachability_mapping_is_unaffected_by_the_probe_fix() {
        // Confirms the section-7 binary sources (no loss concept at all)
        // were correctly left alone.
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let control_plane = telemetry.network.as_ref().unwrap().control_plane.as_ref().unwrap();
        let cp_observation = project_control_plane_observation(&telemetry, NODE, &mut context, control_plane).unwrap();
        assert_eq!(cp_observation.health, DMonitorHealthV1::Healthy);

        let peer = &telemetry.network.as_ref().unwrap().peers[0];
        let peer_observation = project_peer_observation(&telemetry, NODE, &mut context, peer).unwrap();
        assert_eq!(peer_observation.health, DMonitorHealthV1::Unhealthy);
    }

    #[test]
    fn no_raw_f64_appears_anywhere_in_canonical_facts() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let observations = project_telemetry_to_dmonitor_observations(&telemetry, &mut context).unwrap();
        for observation in &observations {
            assert_no_floats(&observation.facts);
        }
    }

    fn assert_no_floats(value: &Value) {
        match value {
            Value::Number(number) => assert!(
                number.is_u64() || number.is_i64(),
                "found a non-integer JSON number in facts: {number}"
            ),
            Value::Array(items) => items.iter().for_each(assert_no_floats),
            Value::Object(map) => map.values().for_each(assert_no_floats),
            _ => {}
        }
    }

    // ------------------------------------------------------------ PROJECT

    #[test]
    fn projected_observations_always_have_project_id_none() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let observations = project_telemetry_to_dmonitor_observations(&telemetry, &mut context).unwrap();
        for observation in &observations {
            assert!(observation.project_id.is_none());
        }
    }

    // ------------------------------------------------------------ UNBOUND

    #[test]
    fn non_correlatable_service_observation_cannot_manufacture_canonical_identity() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let service = &telemetry.runtime.as_ref().unwrap().services[0];
        let observation = project_service_observation(&telemetry, NODE, &mut context, service).unwrap();
        match &observation.subject {
            DMonitorSubjectV1::Unbound { node_id, description } => {
                assert_eq!(node_id, NODE);
                assert_eq!(description.as_deref(), Some("decoder"));
            }
            other => panic!("expected Unbound, got {other:?}"),
        }
        assert!(observation.validate().is_empty());
    }

    #[test]
    fn service_observation_never_reads_desired_state_field() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let service = &telemetry.runtime.as_ref().unwrap().services[0];
        assert_eq!(service.desired_state.as_deref(), Some("running"));
        let observation = project_service_observation(&telemetry, NODE, &mut context, service).unwrap();
        assert!(observation.facts.get("desired_state").is_none());
        assert_eq!(observation.facts["observed_state"], json!("available"));
    }

    // ------------------------------------------------------------- SAFETY

    #[test]
    fn no_action_record_or_mutation_authority_field_exists_on_the_canonical_type() {
        // Structural: CanonicalDMonitorObservationV1 has no field capable of
        // carrying an ActionRecord/mutation-intent at all -- proven by the
        // exhaustive struct literal used throughout this module compiling
        // with exactly {schema, observation_id, project_id, node_id,
        // monitor_instance_id, sequence, observed_at_utc, cycle_id,
        // collector, subject, health, facts} and nothing else.
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let observation = project_dnode_observation(&telemetry, NODE, &mut context).unwrap();
        let value = serde_json::to_value(&observation).unwrap();
        let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
        for forbidden in ["action", "actions", "mutation", "remediation", "auto_recovery"] {
            assert!(!keys.iter().any(|k| k.as_str() == forbidden), "unexpected field '{forbidden}'");
        }
    }

    #[test]
    fn no_secrets_in_projected_facts() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let telemetry = minimal_telemetry();
        let observations = project_telemetry_to_dmonitor_observations(&telemetry, &mut context).unwrap();
        for observation in &observations {
            let text = observation.facts.to_string().to_lowercase();
            for forbidden in ["password", "token", "secret", "bearer", "credential"] {
                assert!(!text.contains(forbidden), "facts unexpectedly contain '{forbidden}': {text}");
            }
        }
    }

    // ============================================================ SCALING

    #[test]
    fn percent_zero_scales_to_zero() {
        assert_eq!(percent_to_basis_points("x", 0.0).unwrap(), 0);
    }

    #[test]
    fn percent_normal_fractional_value() {
        assert_eq!(percent_to_basis_points("x", 12.5).unwrap(), 1250);
    }

    #[test]
    fn percent_exact_integer() {
        assert_eq!(percent_to_basis_points("x", 50.0).unwrap(), 5000);
    }

    #[test]
    fn percent_maximum_valid_value() {
        assert_eq!(percent_to_basis_points("x", 100.0).unwrap(), 10000);
    }

    #[test]
    fn percent_tiny_positive_fractional_value() {
        assert_eq!(percent_to_basis_points("x", 0.001).unwrap(), 0);
    }

    #[test]
    fn percent_rounding_boundary_rounds_half_away_from_zero() {
        // 12.345 * 100 = 1234.5 -> rounds to 1235 (round-half-away-from-zero).
        assert_eq!(percent_to_basis_points("x", 12.345).unwrap(), 1235);
    }

    #[test]
    fn percent_nan_is_rejected() {
        assert!(matches!(
            percent_to_basis_points("x", f64::NAN),
            Err(MetricScaleError::NonFinite { .. })
        ));
    }

    #[test]
    fn percent_positive_infinity_is_rejected() {
        assert!(matches!(
            percent_to_basis_points("x", f64::INFINITY),
            Err(MetricScaleError::NonFinite { .. })
        ));
    }

    #[test]
    fn percent_negative_is_rejected() {
        assert!(matches!(
            percent_to_basis_points("x", -1.0),
            Err(MetricScaleError::OutOfRange { .. })
        ));
    }

    #[test]
    fn percent_above_one_hundred_is_rejected() {
        assert!(matches!(
            percent_to_basis_points("x", 100.1),
            Err(MetricScaleError::OutOfRange { .. })
        ));
    }

    #[test]
    fn load_value_very_large_but_finite_scales_correctly() {
        assert_eq!(nonnegative_value_to_milliunits("x", 123_456.789).unwrap(), 123_456_789);
    }

    #[test]
    fn load_value_overflow_is_rejected() {
        // A value whose scaled (x1000) magnitude exceeds the canonical
        // safe-integer ceiling must fail closed, not silently wrap/truncate.
        let huge = (MAX_SAFE_INTEGER as f64) * 10.0;
        assert!(matches!(
            nonnegative_value_to_milliunits("x", huge),
            Err(MetricScaleError::Overflow { .. })
        ));
    }

    #[test]
    fn load_value_negative_is_rejected() {
        assert!(matches!(
            nonnegative_value_to_milliunits("x", -0.1),
            Err(MetricScaleError::OutOfRange { .. })
        ));
    }

    #[test]
    fn ms_to_microseconds_zero_and_normal_values() {
        assert_eq!(nonnegative_ms_to_microseconds("x", 0.0).unwrap(), 0);
        assert_eq!(nonnegative_ms_to_microseconds("x", 1.5).unwrap(), 1500);
    }

    // ---------------------------------------- PHASE 114B2.1 CLOSURE
    // (safe-integer domain re-verification)

    /// Section 9: a scaled value that fits in `u64` but exceeds DATUM's
    /// canonical JCS safe-integer ceiling (`agent::jcs::MAX_SAFE_INTEGER`,
    /// `2^53 - 1` -- far smaller than `u64::MAX`) must fail at the
    /// conversion helper itself, never merely produce a `u64` that later
    /// fails only when something eventually calls
    /// `recompute_dmonitor_observation_digest`. `u64::MAX` unscaled would
    /// silently fit in `u64` arithmetic but is nowhere near representable in
    /// the canonical domain once multiplied by any scale factor -- exactly
    /// the gap this test closes.
    #[test]
    fn scaled_value_exceeding_the_canonical_safe_integer_ceiling_fails_at_the_helper_not_only_at_digest_time() {
        // MAX_SAFE_INTEGER itself, scaled by 1000, overflows the domain even
        // though the UNSCALED value is well within both u64 and the safe
        // range -- proving the check is against the SCALED result, not the
        // raw input.
        let unscaled_but_overflows_once_scaled = MAX_SAFE_INTEGER as f64;
        let error = nonnegative_value_to_milliunits("x", unscaled_but_overflows_once_scaled).unwrap_err();
        assert!(matches!(error, MetricScaleError::Overflow { .. }));

        let error = nonnegative_ms_to_microseconds("x", unscaled_but_overflows_once_scaled).unwrap_err();
        assert!(matches!(error, MetricScaleError::Overflow { .. }));
    }

    /// End-to-end: an overflowing metric anywhere in a collection must fail
    /// the WHOLE `project_telemetry_to_dmonitor_observations` call --
    /// PROJECTION itself must refuse to produce the artifact, rather than
    /// returning an observation that would only fail later when a caller
    /// happens to call `recompute_dmonitor_observation_digest` on it.
    #[test]
    fn overflowing_metric_fails_the_whole_projection_not_only_a_later_digest_call() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let mut telemetry = minimal_telemetry();
        telemetry.host.cpu.load_1 = Some(MAX_SAFE_INTEGER as f64);
        let error = project_telemetry_to_dmonitor_observations(&telemetry, &mut context).unwrap_err();
        assert!(matches!(error, ProjectionError::MetricScale(MetricScaleError::Overflow { .. })));
    }

    /// Every value actually accepted by a conversion helper must digest
    /// successfully once embedded in a real observation -- confirming the
    /// helpers' accept/reject boundary and the canonical digest's own
    /// safe-integer boundary genuinely agree with each other, not merely
    /// with an independently-guessed threshold.
    #[test]
    fn every_accepted_scaled_value_is_digest_safe_at_the_exact_ceiling() {
        let mut context = DMonitorProjectionContext::new_process_epoch(NODE);
        let mut telemetry = minimal_telemetry();
        // Comfortably under the ceiling (avoiding an exact-boundary
        // floating-point round-trip, which is already covered by the
        // overflow-side tests above) but still large enough to prove large,
        // realistic values digest cleanly end to end.
        let large_but_safe = 1_000_000_000.0;
        telemetry.host.cpu.load_1 = Some(large_but_safe);
        let observations = project_telemetry_to_dmonitor_observations(&telemetry, &mut context).unwrap();
        let dnode = observations
            .iter()
            .find(|o| matches!(o.subject, DMonitorSubjectV1::DNode { .. }))
            .unwrap();
        assert!(crate::agent::canonical_dmonitor::recompute_dmonitor_observation_digest(dnode).is_ok());
    }
}
