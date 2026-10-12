// Phase 136 -- Placement Workspace pure model: response validators and the
// deterministic current-vs-candidate diff. No DOM, no network, directly
// testable. Mirrors continuum-model.ts's validator style and
// dgraph-editor-model.ts's diff() style.
import type {
  CandidateResponse,
  ValidateResponse,
  ProposalResponse,
  ActiveResponse,
  ActiveDeploymentSummary,
  ContractRef,
  PlacementRef,
  PlacementDecisionRecord,
  PlacementProvenance,
  DDeployProposal,
  CanonicalDMap,
  AcceptResponse,
  DDeployAcceptance,
} from "../types/placement.ts";

function object(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
function text(value: unknown): value is string {
  return typeof value === "string";
}
function count(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}
function timestamp(value: unknown): boolean {
  return (
    typeof value === "string" &&
    /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d{1,9})?(?:Z|[+-]\d\d:\d\d)$/u.test(value) &&
    Number.isFinite(Date.parse(value))
  );
}
function digest(value: unknown): boolean {
  return typeof value === "string" && /^sha256:[a-f0-9]{64}$/u.test(value);
}

function contractRef(value: unknown): value is ContractRef {
  return object(value) && text(value.id) && count(value.revision) && digest(value.digest);
}
function placementRef(value: unknown): value is PlacementRef {
  return object(value) && text(value.service_id) && text(value.node_id);
}
function decisionRecord(value: unknown): value is PlacementDecisionRecord {
  return (
    object(value) &&
    text(value.service_id) &&
    text(value.node_id) &&
    text(value.required_dnode_abi) &&
    count(value.eligible_node_count) &&
    count(value.node_residual_cpu_cores_before) &&
    count(value.node_residual_memory_bytes_before) &&
    count(value.node_residual_cpu_cores_after) &&
    count(value.node_residual_memory_bytes_after)
  );
}
function provenance(value: unknown): value is PlacementProvenance {
  return (
    object(value) &&
    text(value.planner_version) &&
    text(value.policy_name) &&
    text(value.policy_version) &&
    text(value.dgraph_id) &&
    count(value.dgraph_revision) &&
    digest(value.dgraph_digest) &&
    text(value.dcontinuum_id) &&
    count(value.dcontinuum_revision) &&
    digest(value.dcontinuum_digest) &&
    Array.isArray(value.decisions) &&
    value.decisions.every(decisionRecord)
  );
}

export function validCandidateResponse(
  value: unknown,
  projectId: string,
  applicationId?: string,
): value is CandidateResponse {
  return (
    object(value) &&
    value.schema === "datum.placement-candidate/1" &&
    value.project_id === projectId &&
    text(value.application_id) &&
    (applicationId === undefined || value.application_id === applicationId) &&
    contractRef(value.dgraph_ref) &&
    contractRef(value.dcontinuum_ref) &&
    (value.expected_active_dmap_revision === null || count(value.expected_active_dmap_revision)) &&
    Array.isArray(value.placements) &&
    value.placements.every(placementRef) &&
    provenance(value.planner_provenance)
  );
}

function validationFinding(value: unknown): boolean {
  return object(value) && text(value.code) && text(value.message);
}

export function validValidateResponse(value: unknown): value is ValidateResponse {
  return (
    object(value) &&
    typeof value.valid === "boolean" &&
    Array.isArray(value.findings) &&
    value.findings.every(validationFinding) &&
    value.valid === (value.findings.length === 0) &&
    contractRef(value.dgraph_ref) &&
    contractRef(value.dcontinuum_ref) &&
    Array.isArray(value.candidate_placements) &&
    value.candidate_placements.every(placementRef) &&
    text(value.capacity_check_is_momentary) &&
    value.capacity_check_is_momentary.length > 0
  );
}

const PROPOSAL_SOURCES = ["operator", "deterministic", "ai"];
const PROPOSAL_STATES = ["pending", "accepted"];

function ddeployProposal(value: unknown, projectId: string, applicationId?: string): value is DDeployProposal {
  return (
    object(value) &&
    text(value.schema) &&
    text(value.proposal_id) &&
    value.project_id === projectId &&
    text(value.application_id) &&
    (applicationId === undefined || value.application_id === applicationId) &&
    PROPOSAL_SOURCES.includes(String(value.source)) &&
    timestamp(value.created_at_utc) &&
    (value.expected_active_dmap_revision === null || count(value.expected_active_dmap_revision)) &&
    contractRef(value.dgraph_ref) &&
    contractRef(value.dcontinuum_ref) &&
    Array.isArray(value.placements) &&
    value.placements.every(placementRef)
  );
}

export function validProposalResponse(
  value: unknown,
  projectId: string,
  applicationId?: string,
): value is ProposalResponse {
  return (
    object(value) &&
    ddeployProposal(value.proposal, projectId, applicationId) &&
    digest(value.proposal_digest) &&
    PROPOSAL_STATES.includes(String(value.state))
  );
}

function canonicalDMap(value: unknown, projectId: string): value is CanonicalDMap {
  return (
    object(value) &&
    text(value.schema) &&
    value.project_id === projectId &&
    text(value.application_id) &&
    text(value.dmap_id) &&
    count(value.revision) &&
    contractRef(value.dgraph_ref) &&
    contractRef(value.dcontinuum_ref) &&
    Array.isArray(value.placements) &&
    value.placements.every(placementRef)
  );
}

function activeDeploymentSummary(value: unknown, projectId: string): value is ActiveDeploymentSummary {
  return (
    object(value) &&
    value.project_id === projectId &&
    text(value.application_id) &&
    canonicalDMap(value.active_dmap, projectId) &&
    digest(value.dmap_digest) &&
    text(value.proposal_id) &&
    text(value.acceptance_id) &&
    timestamp(value.accepted_at_utc) &&
    text(value.operational_dgraph_id) &&
    count(value.operational_revision)
  );
}

export function validActiveResponse(value: unknown, projectId: string): value is ActiveResponse {
  return (
    object(value) &&
    value.project_id === projectId &&
    Array.isArray(value.deployments) &&
    value.deployments.every((d) => activeDeploymentSummary(d, projectId))
  );
}

// ------------------------------------------------------------- Phase 137

function operationalProjectionRef(value: unknown): boolean {
  return object(value) && text(value.operational_dgraph_id) && count(value.revision);
}

// `accepted_by` is validated only as a non-empty string -- it is a
// caller-supplied AUDIT LABEL, never authenticated identity (ADR-0013 §6).
function ddeployAcceptance(value: unknown, projectId: string, applicationId?: string): value is DDeployAcceptance {
  return (
    object(value) &&
    text(value.schema) &&
    text(value.acceptance_id) &&
    text(value.proposal_id) &&
    digest(value.proposal_digest) &&
    value.project_id === projectId &&
    text(value.application_id) &&
    (applicationId === undefined || value.application_id === applicationId) &&
    timestamp(value.accepted_at_utc) &&
    text(value.accepted_by) &&
    value.accepted_by.length > 0 &&
    contractRef(value.materialized_dmap_ref) &&
    operationalProjectionRef(value.operational_projection_ref)
  );
}

// `POST /api/v1/ddeploy/proposals/:id/accept` response validator. Never
// treats `materialized_dmap`/`currently_active` as a lasting snapshot --
// callers must still re-fetch `GET /active` for the live authoritative
// state (see that field's own type-level doc comment).
export function validAcceptResponse(
  value: unknown,
  projectId: string,
  applicationId?: string,
): value is AcceptResponse {
  return (
    object(value) &&
    ddeployAcceptance(value.acceptance, projectId, applicationId) &&
    canonicalDMap(value.materialized_dmap, projectId) &&
    text(value.operational_dgraph_id) &&
    count(value.operational_revision) &&
    typeof value.idempotent_replay === "boolean" &&
    typeof value.currently_active === "boolean"
  );
}

// ----------------------------------------------------------------- Diff

export type PlacementChange = "unchanged" | "moved" | "added" | "removed";

export interface PlacementDiffEntry {
  service_id: string;
  from_node_id: string | null;
  to_node_id: string | null;
  change: PlacementChange;
}

// Deterministic, keyed by graph-local service_id ONLY -- never by
// ServiceDefinition name (placement never references one). `current` is
// the active accepted placement (empty if none); `candidate` is the local
// draft being edited. Pure, sorted by service_id for stable rendering.
export function placementDiff(current: PlacementRef[], candidate: PlacementRef[]): PlacementDiffEntry[] {
  const currentByService = new Map(current.map((p) => [p.service_id, p.node_id]));
  const candidateByService = new Map(candidate.map((p) => [p.service_id, p.node_id]));
  const serviceIds = new Set([...currentByService.keys(), ...candidateByService.keys()]);
  const entries: PlacementDiffEntry[] = [];
  for (const serviceId of serviceIds) {
    const from = currentByService.get(serviceId) ?? null;
    const to = candidateByService.get(serviceId) ?? null;
    const change: PlacementChange =
      from === null && to !== null
        ? "added"
        : from !== null && to === null
          ? "removed"
          : from === to
            ? "unchanged"
            : "moved";
    entries.push({ service_id: serviceId, from_node_id: from, to_node_id: to, change });
  }
  entries.sort((a, b) => a.service_id.localeCompare(b.service_id));
  return entries;
}
