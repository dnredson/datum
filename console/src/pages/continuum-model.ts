import type {
  ContinuumView,
  ContinuumNode,
  NodeObservation,
  ObservationEvidence,
} from "../types/continuum.ts";

function object(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
function text(value: unknown): value is string {
  return typeof value === "string";
}
// Phase 140A finding: `generated_at_utc` (`core::continuum_view`) is
// server-generated via plain `Utc::now().to_rfc3339()`, which chrono
// renders with a `+00:00` offset suffix, never a literal `Z` -- see
// `dmonitor-convergence-model.ts`'s own identical fix/comment for the full
// explanation. Both valid RFC 3339 spellings of the same UTC instant are
// accepted.
function timestamp(value: unknown): boolean {
  return (
    typeof value === "string" &&
    /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d{1,9})?(?:Z|[+-]\d\d:\d\d)$/u.test(value) &&
    Number.isFinite(Date.parse(value))
  );
}
function count(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}
function integer(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value);
}
function finiteNonNegative(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

const FRESHNESS = ["fresh", "stale", "clock_skew"];
const HEALTH = ["unknown", "healthy", "degraded", "unhealthy"];
const CORRELATION = ["exact_match", "ref_missing", "mismatch", "not_applicable"];
const CONVERGENCE = ["converged", "diverged", "unknown"];


function runtimeUsage(value: unknown): boolean {
  return (
    object(value) &&
    finiteNonNegative(value.cpu_percent) &&
    count(value.memory_used_bytes) &&
    count(value.memory_limit_bytes) &&
    (value.memory_limit_bytes === 0 || Number(value.memory_used_bytes) <= Number(value.memory_limit_bytes)) &&
    count(value.pids)
  );
}

function observedMemory(value: unknown): boolean {
  return (
    object(value) &&
    count(value.total_bytes) &&
    count(value.used_bytes) &&
    count(value.available_bytes) &&
    Number(value.used_bytes) <= Number(value.total_bytes) &&
    Number(value.available_bytes) <= Number(value.total_bytes)
  );
}

function resourceObservation(value: unknown): boolean {
  if (!object(value)) return false;
  if (
    !text(value.resource_id) ||
    !timestamp(value.observed_at_utc) ||
    !timestamp(value.last_seen_utc) ||
    !integer(value.age_seconds) ||
    Number(value.age_seconds) < 0 ||
    !object(value.cpu) ||
    !count(value.cpu.logical_cores) ||
    Number(value.cpu.logical_cores) < 1 ||
    !observedMemory(value.memory) ||
    !observedMemory(value.swap)
  )
    return false;
  return (
    value.cpu.usage_percent === undefined ||
    (finiteNonNegative(value.cpu.usage_percent) && Number(value.cpu.usage_percent) <= 100)
  );
}

// Plain boolean (not a type-predicate) so it can be reused by both the
// admitted- and excluded-evidence shapes without TS narrowing `value` to
// the narrower `ObservationEvidence` type and then rejecting the extra
// `exclusion_reason` field the excluded variant also carries.
function evidenceFieldsValid(value: Record<string, unknown>): boolean {
  return (
    text(value.monitor_instance_id) &&
    (value.runtime_usage === undefined || runtimeUsage(value.runtime_usage)) &&
    FRESHNESS.includes(String(value.freshness)) &&
    HEALTH.includes(String(value.health)) &&
    CORRELATION.includes(String(value.authority_correlation)) &&
    timestamp(value.observed_at_utc) &&
    timestamp(value.received_at_utc) &&
    integer(value.age_seconds)
  );
}
function evidence(value: unknown): value is ObservationEvidence {
  return object(value) && evidenceFieldsValid(value);
}
function excludedEvidence(value: unknown): boolean {
  return (
    object(value) &&
    evidenceFieldsValid(value) &&
    (value.exclusion_reason === null || text(value.exclusion_reason))
  );
}
function observation(value: unknown): value is NodeObservation {
  return (
    object(value) &&
    text(value.project_id) &&
    text(value.application_id) &&
    text(value.service_id) &&
    CONVERGENCE.includes(String(value.placement_convergence)) &&
    Array.isArray(value.convergence_findings) &&
    value.convergence_findings.every(
      (f) => object(f) && text(f.code) && text(f.message),
    ) &&
    Array.isArray(value.admitted_evidence) &&
    value.admitted_evidence.every(evidence) &&
    typeof value.admitted_evidence_truncated === "boolean" &&
    Array.isArray(value.excluded_evidence) &&
    value.excluded_evidence.every(excludedEvidence) &&
    typeof value.excluded_evidence_truncated === "boolean"
  );
}
function placementRef(value: unknown): boolean {
  return (
    object(value) &&
    text(value.project_id) &&
    text(value.application_id) &&
    text(value.service_id)
  );
}
function node(value: unknown): value is ContinuumNode {
  return (
    object(value) &&
    text(value.node_id) &&
    object(value.structure) &&
    object(value.structure.platform) &&
    text(value.structure.platform.os) &&
    text(value.structure.platform.architecture) &&
    object(value.structure.capacity) &&
    count(value.structure.capacity.cpu_cores) &&
    count(value.structure.capacity.memory_bytes) &&
    Array.isArray(value.structure.dnode_abi_versions) &&
    value.structure.dnode_abi_versions.every(text) &&
    (value.resource_observation === undefined || resourceObservation(value.resource_observation)) &&
    Array.isArray(value.accepted_placements) &&
    value.accepted_placements.every(placementRef) &&
    typeof value.placements_truncated === "boolean" &&
    Array.isArray(value.observations) &&
    value.observations.every(observation) &&
    typeof value.observations_truncated === "boolean"
  );
}
function link(value: unknown): boolean {
  return (
    object(value) &&
    text(value.link_id) &&
    text(value.from_node_id) &&
    text(value.to_node_id)
  );
}

// `projectId` is undefined for the global route. When present, the
// response MUST carry a `project_overlay` for exactly that project --
// never a different one, never absent, and critically never fewer
// physical `nodes` than the global route would return for the same
// underlying registry state (we cannot prove that cross-request here, but
// we can and do prove the overlay's own project_id always matches what was
// requested, and that it is only an ADDITIVE overlay field, never a
// replacement for `nodes`).
export function validContinuumView(
  value: unknown,
  projectId?: string,
): value is ContinuumView {
  if (
    !object(value) ||
    value.schema !== "datum.continuum-view/1" ||
    !timestamp(value.generated_at_utc) ||
    !count(value.dnode_registry_revision) ||
    !Array.isArray(value.nodes) ||
    !value.nodes.every(node) ||
    !Array.isArray(value.links) ||
    !value.links.every(link) ||
    typeof value.truncated !== "boolean"
  )
    return false;
  if (projectId === undefined) return value.project_overlay === undefined;
  return (
    object(value.project_overlay) &&
    value.project_overlay.project_id === projectId &&
    Array.isArray(value.project_overlay.node_ids_in_use) &&
    value.project_overlay.node_ids_in_use.every(text)
  );
}
