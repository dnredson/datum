// Phase 140A -- canonical historical read model types. Mirrors
// `dserver/src/core/dmonitor_history.rs`'s `DMonitorHistoryQueryResponseV1`/
// `HistoricalDMonitorObservationV1` and
// `dserver/src/api/dmonitor_history.rs`'s `GET
// /api/v1/dmonitor/history/:project_id` response field-for-field.
//
// This is a VIEW over already-persisted evidence, never a second
// monitoring model: it carries NO convergence/authority-correlation field
// at all (the backend never computes one for a historical row -- see that
// module's own doc comment on why), only the one authority-independent
// judgment it IS allowed to derive: `freshness`. Health/freshness enums are
// reused verbatim from `continuum.ts`, never redefined.
import type { DMonitorFreshness, DMonitorHealth } from "./continuum.ts";

export type { DMonitorFreshness, DMonitorHealth };

// Mirrors `DMonitorContractRefV1` (`dmap_ref` on several subject variants).
export interface DMonitorContractRef {
  id: string;
  revision: number;
  digest: string;
}
export interface DMonitorRuntimeBindingRef {
  binding_id: string;
  binding_revision: number;
  descriptor_digest: string;
}

// Full, closed-set mirror of `DMonitorSubjectV1` (8 variants, internally
// tagged on `kind`) -- field-for-field, never a permissive catch-all, so a
// backend response naming a KIND this type does not know about is
// correctly rejected by the validator rather than silently accepted as
// some fictional ninth shape.
export type DMonitorSubject =
  | { kind: "dnode"; node_id: string }
  | { kind: "runtime_process"; node_id: string; runtime_ref: string }
  | {
      kind: "dserv_realization";
      project_id: string;
      application_id: string;
      service_id: string;
      node_id: string;
      dmap_ref?: DMonitorContractRef;
    }
  | {
      kind: "dcode_execution";
      project_id: string;
      application_id: string;
      service_id: string;
      node_id: string;
      dcode_evidence_sha256?: string;
    }
  | {
      kind: "dcall_delivery";
      project_id: string;
      application_id: string;
      call_id: string;
      grant_id?: string;
      delivery_evidence_digest?: string;
    }
  | {
      kind: "diot_realization";
      project_id: string;
      application_id: string;
      diot_id: string;
      diot_placement_id: string;
      node_id: string;
      dmap_ref?: DMonitorContractRef;
      dforward_ref?: DMonitorContractRef;
      runtime_binding_ref?: DMonitorRuntimeBindingRef;
    }
  | { kind: "network_reachability"; node_id: string; target: string }
  | { kind: "unbound"; node_id: string; description?: string };

// One persisted observation, exposed honestly -- raw `subject`/`health`/
// both timestamps as actually recorded, plus the one derived,
// authority-independent `freshness` judgment. Never a `convergence` field.
export interface HistoricalDMonitorObservation {
  observation_id: string;
  node_id: string;
  monitor_instance_id: string;
  sequence: number;
  collector: string;
  subject: DMonitorSubject;
  health: DMonitorHealth;
  observed_at_utc: string;
  received_at_utc: string;
  freshness: DMonitorFreshness;
  facts: unknown;
}

export interface DMonitorHistoryFiltersEcho {
  application_id: string | null;
  node_id: string | null;
  service_id: string | null;
}

// `GET /api/v1/dmonitor/history/:project_id` response.
export interface DMonitorHistoryQueryResponse {
  schema: string;
  project_id: string;
  start_utc: string;
  end_utc: string;
  queried_at_utc: string;
  filters: DMonitorHistoryFiltersEcho;
  limit: number;
  count: number;
  truncated: boolean;
  observations: HistoricalDMonitorObservation[];
}

// Caller-side query filters -- the client never invents a 9th filter
// dimension (no PID/container-id/monitor_instance_id filtering, mirroring
// the backend's own deliberate identity-dimension restriction).
export interface DMonitorHistoryQuery {
  applicationId?: string;
  nodeId?: string;
  serviceId?: string;
  startUtc?: string;
  endUtc?: string;
  limit?: number;
}
