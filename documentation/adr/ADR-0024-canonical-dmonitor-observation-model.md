# ADR-0024 — Canonical DMonitor observation model

- Status: Implemented for Phase 114A (canonical wire model, validation,
  digest, and unit tests), Phase 114B1 (DServer ingestion, durable
  persistence, sequence/idempotency ordering, and server-derived freshness
  evaluation), Phase 114B1.1 (a narrow correctness-closure audit fixing
  concrete gaps found in 114B1), AND Phase 114B2 (the production
  SmartSentinel -> canonical DMonitor compatibility projection and
  publication path -- see "Server ingestion (Phase 114B1)", "114B1.1
  correctness closure", and "SmartSentinel compatibility bridge (Phase
  114B2)" below). Convergence and drift views (114C), readiness/
  `any_element` feed integration (114D), and live multinode validation
  (114E) are all explicitly deferred to later phases and are not implemented
  by this ADR.
- Scope: the canonical `datum.dmonitor-observation/1` wire contract that
  represents a single piece of DATUM-native, DATUM-owned observed-runtime
  evidence, and the authority boundary between it and every existing
  desired-state/placement contract.

## Context: a third kind of truth

DATUM already draws a hard line between canonical desired-state contracts
(`DGraph`, `DMap`, `DForward`, `DContinuum`, `ServiceArtifact`, `DCode`) and
operational registries used to realize them (DIoT runtime binding,
`DDeployStateV1`/v2 active-authority views). Both of those exist to answer
"what should be running, and what has been authorized to realize it."
Neither answers "what did an observer actually see happen." Historically
that third question was answered by SmartSentinel's legacy, pre-DATUM
telemetry model: an untyped `Snapshot`/`Entity`/`Edge` graph
(`DATUM/src/core/model.rs`) built by a 16-plugin discovery/health/recovery
pipeline (`DATUM/src/core/cycle.rs`), alongside a separately-typed
`SmartSentinelTelemetryEnvelope` (`DATUM/src/telemetry.rs`) used for the
compatibility-facing HTTP telemetry surface DServer already exposes
(`POST /api/v1/snapshots`, `POST /api/v1/sentinels/{node}/telemetry`).

Auditing that legacy model surfaced three structural properties that a
DATUM-native replacement must not inherit:

1. `Entity.attrs`/`Edge.attrs` are untyped `serde_json::Value` bags with no
   canonical identity contract at all -- two different collectors can
   describe "the same" runtime object in incompatible, unreconcilable ways.
2. `ServiceTelemetry` carries its own `desired_state: Option<String>` field.
   That field is a compatibility-surface artifact of the legacy model, not a
   canonical DATUM authority, and a DATUM-native observation type must not
   repeat the mistake of letting an observation carry, or even suggest, a
   competing placement decision.
3. Health, "is this still true now," structural validity, and "does this
   agree with the DMap," are all conflated into whatever ad hoc fields each
   legacy plugin happened to produce, rather than being four separable
   questions a downstream consumer can each answer independently.

## Decision

Introduce `datum.dmonitor-observation/1`
(`CanonicalDMonitorObservationV1`, `DATUM/src/agent/canonical_dmonitor.rs`)
as DATUM's canonical, DATUM-native representation of a single observed
fact about runtime reality. It is deliberately narrow:

- **One observation, not a batch or a snapshot.** Batching, ingestion, and
  persistence semantics are 114B's concern, not this wire contract's.
- **DMonitor is detective, never corrective, never a placement authority.**
  There is no field anywhere in this type, at any nesting level, that
  expresses or implies desired state. This is enforced two ways: no such
  field exists in the typed schema (`#[serde(deny_unknown_fields)]` makes
  injecting one a hard deserialization error), and the one intentionally
  open payload (`facts`, see below) is structurally scanned in `validate()`
  and rejected if it carries a `desired_state` key at any depth -- not only
  at the top level, so a collector cannot smuggle one in by nesting it
  inside a sub-object or array (e.g. a raw legacy `Entity.attrs` blob, which
  nests exactly this key under a `service` sub-object today). A DMonitor
  observation that disagrees with the active DMap is "observed divergence"
  for a future drift computation (114C) to derive -- it is never itself a
  competing authority, and nothing in this phase computes or stores that
  divergence.
- **Typed subject model, not a property bag.** `DMonitorSubjectV1` is a
  tagged enum with one variant per kind of thing DATUM can observe:
  `DNode`, `RuntimeProcess`, `DServRealization`, `DCodeExecution`,
  `DCallDelivery`, `DIoTRealization`, `NetworkReachability`, and `Unbound`.
  Each variant's required identity fields are exactly the fields needed to
  correlate that kind of subject with existing canonical authority --
  never a name, hostname, port, or image, which section 13 of the task
  specification identifies as unreliable, spoofable correlation keys.
- **Exact correlation or honest non-correlation, never a guess.** Where a
  subject variant can reference canonical authority (`DServRealization`'s
  `dmap_ref`, `DIoTRealization`'s `dforward_ref`), that reference is the
  same exact `{id, revision, digest}` shape already established by
  `ContractRefV2` in `canonical_dmap_v2.rs` (mirrored here as
  `DMonitorContractRefV1`, independent per ADR-0007, not reused
  cross-module) -- an approximate or partial match is not accepted; the
  reference is either exactly present or absent. The `Unbound` subject
  variant exists specifically so a collector that cannot establish exact
  correlation says so honestly instead of fabricating one.
- **Execution evidence is referenced, never duplicated.** `DCodeExecution`
  and `DCallDelivery` carry only optional digest/id strings
  (`dcode_evidence_sha256`, `grant_id`, `delivery_evidence_digest`) that
  point at an existing `DCodeInvocationEvidenceV1`
  (`dcode::evidence::dcode_evidence_sha256`) or `DCallDeliveryEvidenceV1`
  record. Neither evidence type is re-embedded or re-modeled here.
- **Health, freshness, syntactic validity, and convergence are four
  separate concepts, and this phase deliberately models only the first and
  the third.** `DMonitorHealthV1` (`Unknown`/`Healthy`/`Degraded`/
  `Unhealthy`) records only what the collector believed AT
  `observed_at_utc`; it carries no implicit expiry and this type has no
  "is this still fresh" field at all. Freshness is a property a consumer
  must compute from `observed_at_utc`/`sequence`/`monitor_instance_id`
  against its own policy -- deliberately not decided or hard-coded by this
  wire contract. Syntactic validity is what `validate()` establishes.
  Convergence (does this observation agree with active DMap/DForward
  authority) is explicitly out of scope for 114A and deferred to 114C; nothing
  in this ADR computes it.
- **`facts` stays an opaque, validated JSON payload, not a fully-typed union
  of every observable metric.** Typing every possible fact shape for every
  subject kind would inflate this phase's scope far past what a canonical
  identity contract requires; the identity/subject/provenance fields that
  section 12 of the task specification requires to be typed are typed. The
  one constraint `facts` inherits is structural: because the whole artifact
  is digested as a single canonical JCS document, every number inside
  `facts` falls under the same Stage 107A safe-integer-only numeric domain
  (`agent::jcs`) as the rest of DATUM's canonical model. A collector
  reporting a fractional metric (e.g. a CPU percentage) must scale it into
  an integer domain (e.g. per-mille) before placing it in `facts`; this is
  proven directly by a unit test that a fractional number in `facts` fails
  digest computation rather than being silently accepted or truncated.

Digest and validation follow the newer, stricter Phase-112+ canonical
convention rather than the older Stage-110-era one: `deny_unknown_fields`
on every struct, a fail-closed `validate()` returning every finding rather
than stopping at the first, and `recompute_dmonitor_observation_digest`
reusing `crate::agent::jcs::canonical_digest_sha256_hex` exactly as
`canonical_dgraph.rs`/`canonical_dmap_v2.rs`/`canonical_dforward.rs` already
do (ADR-0006). There are no unordered-set-valued fields on this type, so no
pre-sort step is needed before hashing.

## Server ingestion (Phase 114B1)

Phase 114B1 gives DServer the ability to receive and durably store canonical
DMonitor evidence WITHOUT changing the authority boundary Phase 114A
established: the canonical observation itself remains entirely node-authored
and is never mutated by the server. Three new modules implement this,
independently mirroring the agent-side wire contract per this project's
no-shared-crate discipline (ADR-0007), proven identical to the agent-side
type by a golden-vector test in both crates (`digest_matches_datum_golden_vector`
/ `digest_matches_dserver_golden_vector`), exactly as `dcall_mqtt_topic.rs`
already established for D-Call topic hashing:

- `dserver::core::canonical_dmonitor` -- the wire contract mirror and
  structural validator, field-for-field and finding-code-for-finding-code
  identical to `DATUM::agent::canonical_dmonitor`.
- `dserver::core::dmonitor_state` -- ingestion, stream/sequence/idempotency
  semantics, subject indexing, and persisted-state tamper validation.
- `dserver::core::dmonitor_freshness` -- the freshness policy/evaluator.

**Server-owned `received_at_utc`.** Every accepted or idempotently-replayed
observation is wrapped in a `DMonitorObservationRecordV1` carrying exactly
three server-owned facts alongside the untouched canonical observation:
`observation_digest` (server-recomputed, never trusted from a caller),
`server_project_scope`, and `received_at_utc`. `received_at_utc` is
generated by `api::dmonitor::post_dmonitor_observation` from `Utc::now()`
on every write; no request body field can supply or override it because
`CanonicalDMonitorObservationV1` (the request body's exact shape) has no such
field to populate in the first place.

**Project isolation.** The server-side project scope (the `:project_id` path
segment, matching the existing `/api/v1/diot/runtime-bindings/:project_id/...`
/ `/api/v1/dcall/calls/:project_id/...` path-parameter convention -- this
project has no header-based tenant mechanism) is authoritative for storage
and query isolation. If the canonical observation's own optional
`project_id` is `Some(...)`, it MUST exactly equal the request's project
scope or ingestion fails closed (`dmonitor_project_scope_mismatch`). If it is
`None` (a pure host/DNode-level observation, per ADR-0024's original
rationale for that field's optionality), the observation is stored under the
request's project scope WITHOUT mutating the canonical artifact to inject
one -- the stored `server_project_scope` and the canonical
`observation.project_id` are always allowed to differ in exactly this one
documented way, never silently reconciled.

**Stream identity and sequence/idempotency.** A stream is
`(server_project_scope, node_id, monitor_instance_id)`
(`dmonitor_state::stream_key`). Sequence numbers are compared only within one
exact stream; a new `monitor_instance_id` (e.g. after a monitor restart)
always starts an independent stream at whatever sequence it first reports,
even on the same node -- multiple simultaneous streams for the same node
are tolerated indefinitely, and nothing in this phase elects one
"authoritative" merely because it was received later (no leader election, no
lease, no takeover). Within one stream: a sequence greater than the current
head is accepted and becomes the new head; the same sequence with an
identical recomputed digest is an idempotent replay (the ORIGINAL
`received_at_utc` is preserved, the stream is not advanced); the same
sequence with a different digest is a `dmonitor_sequence_conflict`; a lower
sequence than the current head is a `dmonitor_sequence_stale` rejection, even
if that exact sequence was never seen before. `observation_id` carries its
own, separate collision rule checked first: the same `observation_id` with
an identical digest is a safe replay; with a different digest, it is a
`dmonitor_observation_id_conflict`, regardless of what the sequence rule
would otherwise decide.

**Subject indexing.** `dmonitor_state::subject_key` derives a deterministic
key from the typed subject's identity fields only -- never `facts`, a
hostname, a container/process name, or an image (ADR-0024's section-13
prohibition, preserved at the storage layer). `Unbound` observations key on
`node_id` alone and are never upgraded to a more specific subject by
inspecting the payload.

**Durable persistence and reload.** `DMonitorStateV1` is a new, separate
`DOMAIN_DMONITOR_STATE` control-state domain, persisted via the existing
`persist_domain_json`/`load_domain_json` one-atomic-blob-per-call convention.
v1 retains every accepted observation per stream indefinitely -- retention
and pruning are explicitly deferred, as is any historical/analytics query
API (only current stream heads and observation-ID lookup are exposed).
`validate_persisted_dmonitor_state_integrity`, invoked from `load_state`,
fails closed on: an unsupported state version, a malformed stream key, a
stream head pointing at a missing or inconsistent observation, a zero or
mismatched sequence, a node/monitor-instance/project-scope mismatch between
a stream and one of its own observations, a failed canonical
`validate()`, a tampered `observation_digest` (always recomputed, never
merely trusted), an invalid `received_at_utc`, or a dangling
`observation_index`/`subject_index` entry. Corrupt persisted state is never
silently repaired.

**Freshness is a separate, server-derived evaluation.** `DMonitorHealthV1`
is never rewritten and never consulted to decide freshness: an observation
can be `Healthy` and evaluate as `Stale`, or `Unhealthy` and evaluate as
`Fresh`, with no contradiction (`dmonitor_freshness`'s own test suite proves
both directions). "Current" (a stream's head, an ordering concept) is also
kept separate from "fresh" (a time-policy evaluation) -- the newest accepted
observation in a stream is not automatically fresh, proven by
`current_head_does_not_imply_fresh`. `DMonitorFreshnessPolicyV1` is an
explicit, caller-supplied typed policy (`max_observed_age_seconds`,
`max_received_age_seconds`, `max_future_clock_skew_seconds`); `evaluate_freshness`
is a pure function taking the evaluation time as an explicit parameter
(never reading the system clock itself), returning
`DMonitorFreshnessV1::{Fresh, Stale, ClockSkew}` plus the signed
`observed_age_seconds`/`received_age_seconds`. A node cannot manufacture
permanent freshness by claiming a future `observed_at_utc`: whenever
`observed_at_utc` exceeds `received_at_utc` by more than
`max_future_clock_skew_seconds`, the result is `ClockSkew`, which takes
precedence over the plain age comparisons rather than being folded into
`Fresh`. No production code path wires a default policy automatically; 114B1
provides the evaluator and leaves policy selection to whichever future
consumer (114D) needs it.

**Ingestion API.** `POST /api/v1/dmonitor/observations/:project_id` accepts
the request body AS the full canonical observation document (strict,
`deny_unknown_fields` at every level -- never a looser envelope), enforces
every rule above, and returns `{outcome: "accepted" | "idempotent_replay",
record}`. Minimal read surfaces: `GET .../streams` (every current stream
head for a project -- a caller filters by typed `subject` client-side; not a
general analytics/query API), `GET .../:node_id/:monitor_instance_id/head`
(one stream's current head), and `GET .../by-id/:observation_id`
(project-scoped; an observation accepted under a different project scope is
reported not-found, never leaked). The ingestion handler acquires only its
own new `DMONITOR_TRANSACTION` lock -- Phase 114B1 never reads DDeploy/DNode/
D-Code/D-Call/DIoT-runtime-binding state (there is no drift/convergence/
authority-currency check yet), so no cross-domain lock ordering applies yet.
A future phase that correlates a DMonitor observation against ACTIVE DMap/
DNode authority (114C) must place `DMONITOR_TRANSACTION` innermost, after
`DDEPLOY_TRANSACTION`/`DNODE_TRANSACTION`, consistent with the
`api::dcall`/`api::diot_runtime_binding` precedent -- not addressed here.

**Explicitly deferred beyond 114B1:** SmartSentinel-to-DMonitor projection/
compatibility conversion (114B2, after this server contract is proven
stable); drift/convergence comparison against active DMap/DGraph/DForward
authority (114C); readiness/`any_element` consumption of fresh DMonitor
evidence (114D); retention/pruning policy; and any general historical query
API.

## 114B1.1 correctness closure

A narrow follow-up audit of the 114B1 implementation found and fixed six
concrete correctness gaps (not a redesign of the persistence model) plus
strengthened test coverage in two more areas. Each is now load-bearing and
tested in both `DATUM::agent::canonical_dmonitor` and
`dserver::core::canonical_dmonitor`/`dmonitor_state`/`dmonitor_freshness`.

**1. `monitor_instance_id` is one monotonic sequence epoch.** The original
114A doc comment ("a monitor instance's own restarts are visible via this
staying constant while `sequence` resets or jumps") was ambiguous enough to
suggest the SAME `monitor_instance_id` could legitimately reset its
sequence after a restart. Corrected in both crates: a monitor process that
restarts while PRESERVING its own `monitor_instance_id` must also
preserve/continue its own sequence counter -- resetting to a lower sequence
under the SAME id is stale/out-of-order (already the correct runtime
behavior; only the documentation was wrong), never a fresh epoch. Only a
genuinely NEW `monitor_instance_id` starts one.

**2. Canonical project/subject-project consistency (real defect, fixed).**
`DServRealization`/`DCodeExecution`/`DCallDelivery`/`DIoTRealization` each
carry their own `project_id`, and nothing previously stopped
`CanonicalDMonitorObservationV1.project_id` from silently contradicting it
(`Some("project-a")` at the top level next to `subject.project_id ==
"project-b"`, or top-level `None` next to a project-bearing subject). Fixed
in the canonical artifact's OWN `validate()` in both crates (never left for
DServer's request-scoped project isolation to paper over): a project-bearing
subject's `project_id` MUST exactly equal a `Some(...)` top-level
`project_id`, or validation fails with `subject_project_id_inconsistent`.
Non-project-bearing subjects (`DNode`/`RuntimeProcess`/
`NetworkReachability`/`Unbound`) are unaffected and may still have
`project_id: None`.

**3. Cross-project `observation_id` replay does not leak.** Since
`server_project_scope` is server-assigned and is NOT part of the canonical
digest when `observation.project_id` is `None`, digest equality alone could
not have proven two ingestion requests were for the same project. Audited
and confirmed already fail-closed by construction: the idempotent-replay
check requires the FULL `stream_key` (which encodes
`server_project_scope`/`node_id`/`monitor_instance_id`) to match, not merely
the digest, so replaying the identical `observation_id`+digest under a
different project scope is rejected as `ObservationIdConflict`, never
returned as another project's persisted record. `observation_id` identity
is thus effectively global across server project scopes (not
project-namespaced) -- now explicit in `dmonitor_state`'s module doc and the
error variant (`ObservationIdConflict` now also carries
`existing_stream_key`/`existing_sequence`/`attempted_stream_key` so a
cross-project collision is diagnosable, not just rejected).

**4. Historical exact replay takes precedence over stale-sequence
rejection (deliberate policy, now explicit and tested).** An EXACT replay
(same `observation_id` + digest) of an already-persisted observation is
idempotent even after the stream head has since advanced past it, because
the `observation_id` identity check runs BEFORE the sequence rule. An
unseen, never-before-observed lower sequence remains correctly stale. This
gives retries deterministic behavior without permitting new history to be
inserted retroactively.

**5. Subject index tracks the latest observation PER SUBJECT PER STREAM,
not the stream's global head (real defect, fixed).** A single stream can
interleave several subjects across its sequence (seq 1 -> A, seq 2 -> B,
seq 3 -> A again); the stream's head is not necessarily the latest
observation for any one subject. `subject_index`'s value type changed from
`Vec<stream_key>` (mere stream membership) to `BTreeMap<stream_key,
sequence>` (the latest sequence for that subject WITHIN each stream), and a
new `DMonitorStateV1::subject_candidates` accessor resolves a subject to
its own latest record per stream, never the stream's unrelated head.

**6. `Unbound` observations are excluded from `subject_index` entirely
(real defect, fixed).** Indexing `Unbound` by `node_id` alone would falsely
imply every unrelated `Unbound` observation on the same node is "the same
subject" merely because they share a host -- exactly the correlation
`Unbound` exists to honestly refuse. `ingest_observation` now skips subject
indexing for `Unbound` entirely; such observations remain fully retrievable
via `by_observation_id`/`stream_head`, just never claimed as a correlatable
subject.

**Strengthened, not newly broken, areas:**

- **Bidirectional index integrity.** Reload validation now RECOMPUTES both
  `observation_index` and `subject_index` from `streams` from scratch
  (applying the exact same derivation `ingest_observation` does) and
  requires byte-for-byte equality with what was persisted, in addition to
  the existing itemized dangling-reference/digest-tamper checks. This
  catches classes the itemized checks alone would miss: a subject indexed
  under the wrong typed key, a subject index pointing at a stale (not
  latest) sequence, an `Unbound` observation indexed contrary to policy, or
  a persisted observation with no `observation_index` entry at all.
- **Stream head exactness.** Reload validation now also confirms
  `head_sequence` is EXACTLY the maximum key in `observations`, not merely
  an existing one (`{1,2,3}` with `head=2` is now correctly rejected, where
  it previously passed).
- **Persistence atomicity, confirmed by audit and by a new test
  (`failed_persist_leaves_no_durable_mutation`), not newly built.** DServer
  holds no globally-shared mutable DMonitor state: every request reloads
  fresh from disk, mutates only a local copy, and persists via one SQLite
  transaction before returning success. A persistence failure therefore
  already left no durable mutation and no route by which a later request
  could see a failed observation as accepted -- this was true of 114B1's
  architecture from the start; 114B1.1 only added the test and the module
  doc proving it explicitly.
- **Freshness clock-edge fail-closed rule.** `evaluate_freshness` now
  rejects (`Err`) an `evaluation_time` that precedes `received_at_utc` --
  evaluating "as of" a point before the server even received the record is
  an impossible ordering, not merely unusual, and silently returning a
  negative age classified as Fresh would have been misleading. Boundary
  cases (`observed_age == max_observed_age`, `future_skew ==
  max_future_clock_skew`, all-zero policy) are now explicitly tested as
  inclusive-at-the-boundary.
- **Cross-crate mirror coverage.** Extended from one golden digest vector to
  five, spanning all four `DMonitorHealthV1` values, an optional canonical
  ref present (`dmap_ref`, `dforward_ref`), optional evidence present
  (`dcode_evidence_sha256`), and both `project_id` states, plus a
  table-driven round-trip test (not a golden digest, per this closure's own
  "compact, not dozens of constants" guidance) covering the remaining
  subject variants (`RuntimeProcess`, `DCallDelivery` with evidence absent,
  `NetworkReachability`) in both crates.

## SmartSentinel compatibility bridge (Phase 114B2)

SmartSentinel is DATUM's legacy COLLECTION implementation; DMonitor is the
canonical DATUM observation architecture. They are never competing
monitoring systems -- Phase 114B2 makes SmartSentinel's existing, already-
working discovery/telemetry machinery the compatibility/collection lineage
that FEEDS canonical DMonitor, without deleting, renaming, or disabling any
existing SmartSentinel code path, output, or endpoint.

**Observation-only pipeline (section 9).** `core::cycle::run_observation_only_cycle_for_node_identity`
is a NEW, second cycle function, structurally distinct from the existing
`run_cycle_for_node_identity` (still used, unchanged, by
`discovery_collectors`, `operational_reconciliation`, and this same binary's
own legacy snapshot/telemetry outputs). Its plugin list
(`observation_only_plugins`) simply does not contain
`DesiredStatePlannerPlugin`, `ServiceTransitionsPlugin`,
`RecoveryPlannerPlugin`, or `ActionExecutorPlugin` -- there is no
configuration value that could cause any of them to run, unlike the
continuous service loop's own existing safety mechanism (mutating
`cfg.auto_recovery.enabled`/`cfg.desired_state.enabled` to `false` and
relying on each plugin to internally honor that flag before it plans or
executes anything). This IS a real, deliberate trade-off: `smartsentinel-observe`
now performs the discovery/telemetry collection TWICE per invocation/cycle
(once for the legacy outputs via the existing pipeline, once more via the
observation-only pipeline for DMonitor) rather than sharing one snapshot,
because the existing `run_cycle_for_node_identity` is shared with call sites
that genuinely need the full pipeline and must not be narrowed for all
callers. The doubled collection cost is accepted in exchange for a
structural (not config-dependent) read-only guarantee for the canonical
DMonitor source.

**Monitor instance epoch (section 3).** `agent::dmonitor_projection::DMonitorProjectionContext::new_process_epoch`
generates a NEW, DMonitor-native `monitor_instance_id`
(`dmonitor-instance:<node_id>:<uuid>`), deliberately never the legacy
`SmartSentinelTelemetryEnvelope.sentinel_instance_id`
(`"<node_id>-sentinel"`, fixed per node, not per process epoch). Using the
legacy value would violate Phase 114B1's "one `monitor_instance_id` = one
monotonic sequence epoch" invariant: a restarted process would keep the same
fixed id while its sequence naturally restarts, and DServer would correctly
reject the restarted process's first observations as stale. The legacy
`sentinel_instance_id` is untouched and remains fully available on
`SmartSentinelTelemetryEnvelope` for its own purposes.

**DMonitor sequence vs. legacy collection sequence (section 4).** One
telemetry collection produces MANY canonical observations (the host, every
container, every process, every service, every network target).
`DMonitorProjectionContext` hands out one sequence per EMITTED OBSERVATION
via an internal monotonic counter, never one per collection cycle -- entirely
independent of `SmartSentinelTelemetryEnvelope.sequence`, which remains a
legacy per-collection counter.

**Source of truth: telemetry, not the raw Snapshot (section 7).** The
projector (`agent::dmonitor_projection::project_telemetry_to_dmonitor_observations`)
consumes exclusively `SmartSentinelTelemetryEnvelope` -- the already-
normalized, allowlisted compatibility artifact -- never the raw
`Snapshot`/`Entity`/`Edge` graph. Re-deriving the same allowlisting a second
time from raw entities would risk the two projections silently diverging.

**Subject mappings** (corrected by the 114B2.1 closure below for
`runtime.processes[]`; read this paragraph together with that section for
the exact rationale). `host` -> `DMonitorSubjectV1::DNode`; each
`runtime.containers[]` entry -> `DMonitorSubjectV1::RuntimeProcess`
(`runtime_ref` = the collector-local, genuinely per-container `entity_id`,
opaque, never a name); each `runtime.processes[]` entry ->
`DMonitorSubjectV1::Unbound` (its `entity_id` is a NAME-derived aggregate
group identifier, not a per-instance runtime id, so it is never eligible for
`RuntimeProcess`); each `network.control_plane`/`network.probes[]`/
`network.peers[]` entry -> `DMonitorSubjectV1::NetworkReachability`; each
`runtime.services[]` entry -> `DMonitorSubjectV1::Unbound` (section 10D/16:
`ServiceTelemetry` carries only a collector-local `entity_id`/`name`, never
an exact canonical `project_id`/`application_id`/`service_id`, so
`DServRealization` is never safely constructible from this source and is
not attempted -- confirmed by `no_dserv_identity_is_ever_guessed_from_names`).
`DCodeExecution`, `DCallDelivery`, and `DIoTRealization` are not produced by
this bridge at all: SmartSentinel telemetry carries no exact corresponding
durable evidence/reference for any of them (section 11).

**Conservative health mapping (section 14; probe grading corrected by the
114B2.1 closure below).** `DNode` and container-`RuntimeProcess`
observations always use `DMonitorHealthV1::Unknown` -- metrics existing, or a
container's raw `state`/`status` string, is not an explicit health signal.
`NetworkReachability` observations from a source that exposes ONLY a binary
result (`ControlPlaneTelemetry`, `PeerTelemetry`) map the collector's exact
`reachable` boolean directly (`true` -> `Healthy`, `false` -> `Unhealthy`);
`NetworkReachability` observations from a source that also carries a packet-
loss degree (`NetworkProbeTelemetry`) use graded semantics instead (see
114B2.1 below) -- `reachable` alone is not sufficient there. `Unbound`
service AND process-group observations also use `Unknown`:
`ServiceCheckTelemetry.outcome` is free-form collector text with no fixed
enum, so it is preserved in `facts` only, never promoted to a health
verdict, and a process group's raw instance/CPU/memory metrics are, like a
container's `state`, not a health signal.

**Scaled integer units (section 13).** The canonical DMonitor JCS domain
rejects fractional numbers; every legacy `f64` metric is converted by
`agent::dmonitor_projection`'s scaling functions before being placed in
`facts`, with the unit named in the field itself: `percent_to_basis_points`
(`*_basis_points`, 1% = 100bp, domain `[0, 100]`), `nonnegative_value_to_milliunits`
(`*_milliunits`, unbounded non-negative values like load averages),
`nonnegative_ms_to_microseconds` (`*_microseconds`, RTTs). Every conversion
fails closed on non-finite input, an invalid pre-scale range, or a
post-scale value that would exceed the canonical safe-integer ceiling
(`agent::jcs::MAX_SAFE_INTEGER`) -- never a silent clamp or truncation.

**Exact-correlation requirement / `desired_state` exclusion (section 8).**
`ServiceTelemetry.desired_state` is read by NOTHING in
`agent::dmonitor_projection` -- only `observed_state`, `runtime`, `bound`,
and `checks` are placed in `facts`, under names chosen not to collide with
the canonical guard. The projector does not rely on
`CanonicalDMonitorObservationV1::validate`'s recursive `desired_state` scan
as its normal filtering mechanism; that scan (unchanged since 114A/114A.1)
remains the final fail-closed backstop, proven still load-bearing by
`no_projected_facts_contain_desired_state_at_any_depth`, which uses a
fixture whose `ServiceTelemetry.desired_state` IS genuinely populated.

**`Unbound` semantics preserved (section 16).** `ServiceTelemetry.name` is
placed ONLY in `Unbound`'s free-form, non-correlating `description` field
-- never in a field the canonical subject index treats as identity. Phase
114B1's exclusion of `Unbound` from subject indexing is relied upon, not
weakened, by this bridge.

**Dual publication during migration (section 19).** `smartsentinel-observe`
(both one-shot and, via `agent::service_loop`, continuous mode) now performs
DMonitor projection/publication as an ADDITIONAL output alongside the
existing snapshot JSON file, snapshot HTTP publication, SmartSentinel
telemetry HTTP publication, and telemetry file output -- all four legacy
outputs are unchanged. DMonitor publication failure is reported explicitly
(`dmonitor_accepted=`/`dmonitor_failed=`/`dmonitor_not_attempted=`/
`dmonitor_published=` in the one-shot status line; `dmonitor=ok(...)`/
`dmonitor=err(...)` per continuous-mode cycle) and never silently folded
into the legacy outputs' own `snapshot_published`/`telemetry_published`
flags -- a DMonitor failure does not cause the process to exit non-zero
(this is read-only evidence output, not a required side effect), but it is
never misreported as success.

**Publication failure semantics (section 23).** `outputs::dmonitor_http::DMonitorHttpOutput::publish_all`
publishes a cycle's observations in sequence order and stops at the FIRST
failure: already-accepted observations before that point stay accepted, the
failing one is reported failed, everything after it is reported
not-attempted. There is no retry loop. The next collection cycle allocates
entirely new sequences (Phase 114B1.1 already guarantees a stream's
sequence counter never rewinds or reuses an allocated number for a
different observation) rather than re-attempting what this cycle left
unpublished.

**Secret/redaction boundary (section 24).** The projector only ever reads
typed `SmartSentinelTelemetryEnvelope` fields (hostname, OS/kernel, CPU/
memory/swap/filesystem counters, container/process/service metadata,
network reachability) -- it never touches process environment variables,
container inspect payloads, or any credential/token field, because no such
field exists anywhere on `SmartSentinelTelemetryEnvelope` for it to read.

### 114B2.1 semantic-correctness closure

An independent review of the 114B2 bridge found and fixed two concrete
semantic gaps (not a redesign) plus re-verified the safe-integer domain
enforcement already in place.

**1. Legacy aggregated process telemetry no longer masquerades as
`RuntimeProcess` identity.** `ProcessTelemetry.entity_id` is
`core::ids::process_group_id(os, name_norm)` -- a value DERIVED FROM THE
PROCESS/IMAGE NAME by both collectors (`process_linux_ps.rs`: grouping key =
lowercased `comm`, `"aggregation": "by_comm"`; `process_windows_tasklist.rs`:
grouping key = lowercased image name, `"aggregation": "by_image_name"`). It
identifies a NAMED GROUP of OS processes sharing that name, never one
runtime instance, which is exactly what `RuntimeProcess.runtime_ref`'s own
doc comment forbids ("never a name, hostname, port, or image"). Contrast
with a container: `docker_linux_cli.rs`'s `entity_id` derives from
`docker ps`'s own real per-container ID (`ids::container_id("docker",
&row.id)`) -- a genuine opaque identifier for ONE runtime entity, safe as
`RuntimeProcess.runtime_ref` exactly as originally documented. Fixed:
`project_process_observation` now emits `DMonitorSubjectV1::Unbound`
(`description` = the process/image display name, never canonical
identity), keeping `runtime_kind`, `name`, `aggregation_mode: "by_name"`,
`instance_count`, `cpu_total_basis_points`, and `memory_used_kb_total` as
descriptive `facts`. `project_container_observation` (containers) is
unchanged and remains `RuntimeProcess`. This is not data loss: `Unbound`
observations remain durable evidence; Phase 114B1 already excludes them
from canonical subject-current indexing, and this bridge relies on that
exclusion rather than working around it.

**2. Network probe health no longer treats partial packet loss as
`Healthy`.** The Linux ping collector (`connection_ping_linux.rs`) defines
its own `available` boolean as `received > 0 && loss_pct < 100`, so
`reachable == true` does NOT imply `loss == 0`. `NetworkProbeTelemetry`
(which carries an explicit `loss_percent`) previously collapsed straight to
`reachable ? Healthy : Unhealthy`, silently reporting e.g. 50% packet loss
as fully healthy. Fixed with `probe_health_from_loss`: `unreachable OR
loss == 100%` -> `Unhealthy`; `reachable AND loss == 0%` -> `Healthy`;
`reachable AND 0% < loss < 100%` -> `Degraded`; anything else (no loss data,
or an out-of-range value `SmartSentinelTelemetryEnvelope::validate` would
already have rejected upstream) -> `Unknown`, never a guess. No RTT
threshold was introduced -- RTT remains an observed metric only.
`ControlPlaneTelemetry` and `PeerTelemetry` are UNCHANGED and correctly kept
their binary `reachable -> Healthy`/`Unhealthy` mapping: neither carries a
packet-loss degree at all (`ControlPlaneTelemetry` is a plain TCP-connect
probe; `PeerTelemetry` has no loss field), so a plain binary mapping is the
exact, non-guessing interpretation section 7 permits for a source shaped
that way -- this is a reachability-health interpretation only, never
application health or DServ convergence.

**Safe-integer domain: re-verified, not defective.** All three scaling
functions (`percent_to_basis_points`, `nonnegative_value_to_milliunits`,
`nonnegative_ms_to_microseconds`) already route their overflow check through
one shared private helper (`scale_nonnegative`) that compares the SCALED
result against `agent::jcs::MAX_SAFE_INTEGER` (not merely `u64::MAX`) before
casting -- a value that fits `u64` but exceeds the canonical safe-integer
ceiling was already rejected at the helper itself, never merely surfacing
later at digest time. Strengthened with explicit end-to-end proof
(`overflowing_metric_fails_the_whole_projection_not_only_a_later_digest_call`)
that a single overflowing metric anywhere in a collection fails the WHOLE
`project_telemetry_to_dmonitor_observations` call, and a boundary test
(`scaled_value_exceeding_the_canonical_safe_integer_ceiling_fails_at_the_helper_not_only_at_digest_time`)
proving the check is against the SCALED value, not the raw input.

**Remaining legacy compatibility surfaces (unaffected).** `Snapshot`/
`Entity`/`Edge`, `SmartSentinelTelemetryEnvelope`, the 16-plugin
`core::cycle::run_cycle`/`run_cycle_for_node_identity` pipeline, and the
existing DServer `/api/v1/snapshots`/`/api/v1/sentinels/*` routes remain
exactly as they were before this phase -- SmartSentinel has not been
deleted, renamed, or replaced.

## Migration posture for the legacy SmartSentinel model

This ADR does not migrate or remove any existing SmartSentinel code path.
`Snapshot`/`Entity`/`Edge`, `SmartSentinelTelemetryEnvelope`, the 16-plugin
`core/cycle.rs` pipeline, `smartsentinel-observe`, and the existing DServer
`/api/v1/snapshots` and `/api/v1/sentinels/*` routes remain exactly as they
are and continue to be the live compatibility surface. `DMonitor` is
introduced alongside them as the new DATUM-native target, not a
replacement made in this phase. Which specific legacy responsibilities are
later re-expressed as `DMonitor` observations, which remain
compatibility-only, and which are retired outright is the subject of the
migration responsibility matrix produced during this phase's audit and is
intentionally not pre-empted by this ADR -- each SmartSentinel
responsibility converts on its own merits, not by blanket rule.

The empty, seven-months-stale top-level `smartsentinel/` Python directory
(`agent.py`, `collectors/`, `config/`, `core/`, `outputs/`, all zero bytes,
single historical commit "First deploy. Windows working") was inspected and
confirmed to be a placeholder with zero live content. It carries no
authority over this decision and is not a migration source.

## Consequences

- A DATUM-native collector can now emit a canonical, independently
  verifiable, digestible observation without inventing its own ad hoc JSON
  shape, and without any risk of that observation being mistaken for
  desired-state authority by a future consumer -- the schema makes that
  category error structurally impossible, not merely discouraged by
  convention.
- DServer can now durably accept, order, and idempotently replay canonical
  DMonitor evidence, and evaluate its freshness against an explicit policy,
  without any change to who owns desired-state authority -- see "Server
  ingestion (Phase 114B1)" above.
- `smartsentinel-observe` (one-shot and continuous) now projects real
  SmartSentinel telemetry into canonical DMonitor observations and publishes
  them through the Phase-114B1 production ingestion route, proven end to end
  against a real DServer process (`b1_b2_integration_proof`) -- see
  "SmartSentinel compatibility bridge (Phase 114B2)" above. No other
  SmartSentinel binary/output was touched.
- No drift/convergence computation exists yet. An observation that
  contradicts active DMap/DForward authority is representable (it is just
  an observation), but nothing yet derives or flags that contradiction --
  114C's job.
- No readiness/`any_element` feed integration exists yet -- 114D's job.
- No live multinode validation of this model exists yet -- 114E's job, and
  per this project's established evidentiary discipline (see the Phase 113B
  live-multinode closure precedent), any future claim of live multinode
  DMonitor behavior must be hedged exactly as rigorously: proven claims
  stated as proven, unproven claims stated as unproven, causal attribution
  offered as the best-supported explanation rather than mathematical proof.

## Alternatives considered

- **Extend `SmartSentinelTelemetryEnvelope` in place** instead of
  introducing a new canonical type. Rejected: that type is a compatibility
  surface with its own `desired_state` field and untyped nested telemetry
  bags; extending it would either inherit those problems or require
  breaking its existing wire contract, which the existing
  `/api/v1/sentinels/*` consumers depend on today.
- **Model `facts` as a fully-typed enum per subject kind** instead of an
  opaque validated JSON value. Rejected for 114A as premature: the set of
  observable facts per subject kind is not yet settled by any consumer, and
  over-specifying it now risks a schema that has to be broken again once
  114B's real collectors and 114D's real consumers exist. The canonical
  identity guarantees this phase is scoped to deliver do not require it.
- **Let `DMonitorContractRefV1` reuse `canonical_dmap_v2::ContractRefV2`
  directly** instead of an independent mirror. Rejected to preserve this
  project's established independent-mirror discipline between distinct
  canonical modules, matching how DATUM and DServer already independently
  mirror the same logical shapes rather than sharing a crate.
