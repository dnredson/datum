# ADR-0024 — Canonical DMonitor observation model

- Status: Implemented for Phase 114A (canonical wire model, validation,
  digest, and unit tests), Phase 114B1 (DServer ingestion, durable
  persistence, sequence/idempotency ordering, and server-derived freshness
  evaluation), Phase 114B1.1 (a narrow correctness-closure audit fixing
  concrete gaps found in 114B1), Phase 114B2 (the production SmartSentinel
  -> canonical DMonitor compatibility projection and publication path), AND
  Phase 114C (server-derived desired-vs-observed placement convergence/drift
  interpretation -- see "Server ingestion (Phase 114B1)", "114B1.1
  correctness closure", "SmartSentinel compatibility bridge (Phase 114B2)",
  and "Derived convergence/drift interpretation (Phase 114C)" below).
  Readiness/`any_element` feed integration (114D) and live multinode
  validation (114E) are explicitly deferred to later phases and are not
  implemented by this ADR.
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

## Derived convergence/drift interpretation (Phase 114C)

`DMap`/`DForward`/etc. remain the ONE authority for what SHOULD exist.
`DMonitor` remains evidence of what WAS OBSERVED. Phase 114C ONLY compares
them, in a new pure module (`dserver::core::dmonitor_interpretation`) that
grants DMonitor no desired-state authority, produces no placement decision,
and performs no host/reconciliation action.

**Derived, never persisted.** Every result type
(`DMonitorApplicationConvergenceV1`, `DMonitorServicePlacementConvergenceV1`,
...) is a READ MODEL computed on demand from three explicit inputs: the
current active `PlacementAuthorityView` (via the ONE shared
`ddeploy::active_placement_authority` -- never `active_by_application`/
`phase113_v2.active` read independently), the current persisted
`DMonitorStateV1`, and an explicit `DMonitorFreshnessPolicyV1` + evaluation
time. Nothing is written back to `storage::control_state` or any other
persistence layer: desired authority can change, evidence ages, policy can
change, and evaluation time advances, so persisting a convergence boolean
would immediately create another staleness problem of its own.

**Four separate dimensions, never collapsed.** Freshness (is the evidence
recent enough to trust right now), authority correlation (does the
evidence's own `dmap_ref` exactly match CURRENT authority), placement
convergence (does admitted evidence prove the desired placement is what is
actually running), and observed health (what the collector believed about
the subject's own health) are kept structurally independent. `Fresh !=
Healthy`, `Healthy != Converged`, `Converged != Ready`, and `Diverged` does
not necessarily imply `Unhealthy` -- proven directly by
`exact_placement_with_unhealthy_health_still_converges`/
`exact_placement_with_degraded_health_still_converges`: a DMap may correctly
place a service on a node while that service happens to be currently
unhealthy.

**Reason codes.** The main convergence state is a small closed
`DMonitorConvergenceV1::{Converged, Diverged, Unknown}`; every "why" is a
stable string finding code kept separate from that state:
`exact_match`, `no_evidence`, `stale_evidence`, `clock_skew`,
`authority_ref_missing`, `authority_mismatch`, `wrong_node`,
`unexpected_realization`, `conflicting_fresh_evidence`,
`no_active_placement_authority`, `no_desired_placements`,
`insufficient_canonical_correlation`.

**Active authority source.** Every D-Serv comparison reads ONLY
`PlacementAuthorityView`'s shared fields (`dmap_id`/`revision`/`digest`/
`placements`), never anything `v2`-specific -- `datum.dmap/1` and
`datum.dmap/2` active authority are compared identically, with no
DMap1/DMap2 fork in this module at all (proven by
`dmap1_exact_ref_correct_node_fresh_is_converged`/
`dmap2_exact_ref_correct_node_fresh_is_converged` sharing the exact same
assertions against the same evaluator).

**Freshness precedence.** Freshness qualification happens BEFORE
authority-correlation evidence can prove CURRENT convergence. Stale or
clock-skew-invalid evidence remains historically valid (never deleted,
never rejected from persistence) but cannot prove `Converged` -- if the
only relevant evidence is stale/clock-skew-invalid, the result is `Unknown`
(with `stale_evidence`/`clock_skew` findings), NEVER `Diverged`: absence of
current proof is not proof of drift.

**Exact D-Map correlation, never "helpful" matching.** A `DServRealization`
observation with no `dmap_ref` cannot prove convergence
(`authority_ref_missing`). A fresh observation whose `dmap_ref` does not
EXACTLY match current `{id, revision, digest}` -- wrong revision, or the
same id/revision with a different digest -- is `authority_mismatch`, never
"helpfully" treated as current-authority evidence merely because
service_id/node_id agree.

**Wrong-node / unexpected-realization / conflicting-evidence semantics.**
The evaluator scans ALL relevant `DServRealization` evidence for
`(project_id, application_id, service_id)` across every observed node, not
merely a lookup keyed by the desired node (`latest_dserv_realizations_per_stream`,
which replicates `dmonitor_state`'s own "latest per exact subject per
stream" rule directly against the typed subject enum, since
`subject_key`/`subject_candidates` require an exact node up front and
therefore cannot discover realizations of the same service on a DIFFERENT
node). Given the admitted (fresh + exact-authority-match) evidence set:
evidence only on the desired node -> `Converged`
(`exact_match`); evidence only on ONE other node -> `Diverged`
(`wrong_node`); evidence on the desired node AND an extra other node ->
`Diverged` (`unexpected_realization`); evidence on TWO OR MORE different
non-desired nodes with none on the desired node -> `Diverged`
(`conflicting_fresh_evidence` -- this project's precise, documented
definition of "materially conflicting" for 114C). No admitted evidence at
all -> `Unknown`, with every distinct exclusion reason actually observed
among the excluded evidence (in priority order:
`authority_ref_missing`/`authority_mismatch`/`clock_skew`/`stale_evidence`),
or `no_evidence` if nothing relevant exists at all.

**No implicit monitor leader.** Multiple monitor streams reporting on the
same subject are each evaluated independently -- "latest received wins" is
never used as an implicit leader election. A stale stream never vetoes a
fresh stream's proof (`fresh_good_evidence_is_not_vetoed_by_stale_conflicting_evidence`);
two agreeing fresh streams both support the same result
(`two_fresh_agreeing_streams_are_converged`); health disagreement between
monitor instances is preserved in `observed_health_claims` but never alters
`convergence` (`health_disagreement_does_not_alter_placement_convergence`,
and no health-majority vote is ever computed).

**Application-level aggregation.** `Diverged` beats `Unknown` beats
`Converged`: if any desired service is positively proven divergent, the
application is `Diverged`; else if any required service cannot be proven
(missing/stale/insufficient evidence), `Unknown`; only if every desired
service is `Converged` is the application `Converged`. An application with
ZERO desired D-Serv placements is explicitly `Unknown`
(`no_desired_placements`) -- `all([]) == true` is deliberately NOT treated
as operational convergence.

**No active authority / historical evidence.** A project/application with
no active accepted placement authority at all yields `Unknown`
(`no_active_placement_authority`), never `Diverged` -- this is absence of
desired deployment authority, not runtime drift. Historical DMonitor
evidence correlated to an old D-Map revision remains fully persisted and
inspectable, but fresh evidence correlated to an old revision still cannot
prove CURRENT convergence -- exposed as `authority_mismatch`, never treated
as corrupted evidence and never deleted.

**Current SmartSentinel D-Serv-evidence limitation (documented honestly).**
Phase 114B2 deliberately does not manufacture `DServRealization` from
container/process/image names or ports (there is no exact
`project_id`/`application_id`/`service_id` correlation available from that
source). A normal SmartSentinel-only deployment therefore currently has
ZERO canonical `DServRealization` evidence, and the correct 114C result is
`Unknown`/`no_evidence` -- this module never falls back to `RuntimeProcess`
or `Unbound` observations to infer D-Serv convergence. Producing real
`DServRealization` evidence is a future collector's job, not 114C's.

**DIoT correlation capability/limitation (audit finding).**
`DMonitorSubjectV1::DIoTRealization` (`{project_id, application_id,
dforward_ref: Option<ContractRef>, diot_placement_id: Option<String>}`) was
audited against what exact current DMap/2 realization placement proof would
require, and found INSUFFICIENT on three independent counts: (1) no observed
node field at all (unlike `DServRealization`, which carries its own
`node_id` -- the observation's own top-level `node_id`, the OBSERVER's
identity, is not a safe substitute, since nothing guarantees the observing
collector and the DIoT realization it reports on run on the same node); (2)
no D-Map/2 reference at all (only `dforward_ref` -- DForward-authority
correlation is not DMap/2 revision/digest correlation); (3) no reference to
Phase 113B1's existing `DIoTRuntimeBindingV1` runtime-binding authority
(`binding_id`/`binding_revision`/`descriptor_digest` -- a separate,
already-existing operational authority for "which broker does this
placement currently resolve to" that `DIoTRealization` does not reference at
all). `evaluate_diot_placement_convergence` therefore ALWAYS returns
`Unknown`/`insufficient_canonical_correlation`, proven even for a
hypothetically ideal `dforward_ref` match
(`diot_convergence_never_returns_converged_even_with_a_hypothetically_perfect_dforward_match`).
This is a documented, tested, deliberate Phase 114C outcome -- the canonical
`datum.dmonitor-observation/1` schema was NOT widened in this pass to
improvise a fix; if DIoT placement convergence is wanted, it requires a
reviewed schema-enrichment decision (adding an observed node field, a
`dmap_ref`, and/or a runtime-binding reference to `DIoTRealization`) that
this ADR explicitly defers rather than pre-empts.

**D-Code/D-Call kept out of scope.** `DCodeExecution` and `DCallDelivery`
evidence is valuable execution/delivery evidence with distinct semantics
from D-Serv placement: a successful D-Call does not prove every desired
service is converged, and a D-Code invocation does not replace D-Map
placement authority. Neither is consumed by
`evaluate_application_dserv_convergence` in 114C.

**API.** `POST /api/v1/dmonitor/convergence/:project_id/:application_id/evaluate`
(`dserver::api::dmonitor_convergence`), matching this project's established
`:project_id`-in-path convention. `POST` despite being read-only because the
request body carries an explicit, per-request DIAGNOSTIC freshness policy
(`max_observed_age_seconds`/`max_received_age_seconds`/
`max_future_clock_skew_seconds`, strict `deny_unknown_fields`, no
`evaluated_at_utc` accepted -- the server generates it from `Utc::now()`).
The policy is validated (`validate_freshness_policy`, rejecting any bound
exceeding `i64::MAX` -- a `u64` value beyond that silently two's-complement-
wraps to negative when cast to `i64` inside `evaluate_freshness`, which
would otherwise make `max_future_clock_skew_seconds: u64::MAX` spuriously
classify nearly every real evaluation as `ClockSkew`) BEFORE any lock is
acquired. Lock order: `DDEPLOY_TRANSACTION -> DMONITOR_TRANSACTION`,
extending the established chain; no D-Node/D-Call/D-IoT-runtime-binding lock
is acquired (neither domain's state is read). This is 114C's diagnostic
policy only -- 114D will define the server-owned GOVERNANCE consumption
policy for readiness, which is not the same thing and is not implemented
here.

**Deferred to 114D/114E:** readiness decisions, `any_element` selection,
placement eligibility changes, mutation authorization, reconciliation,
restart/redeploy action, automatic correction, retention/pruning, AI/GNN
policy, live multinode validation, health-majority election, monitor leader
election.

**DIoT correlation gap: standing blocker for 114D.** The DIoT-placement
correlation gap documented above (no observed node field, no D-Map/2
reference, no runtime-binding-authority reference on
`DMonitorSubjectV1::DIoTRealization`) is NOT a blocker for closing D-Serv
convergence in 114C -- D-Serv placement convergence is fully derivable and
implemented independently of it. It IS, however, a standing blocker that
**must be resolved before Phase 114D can claim any DMonitor-backed
`any_element` readiness signal for a DIoT-realized dependency**: 114D cannot
honestly report "this DIoT placement is ready" from evidence that cannot
prove which node it runs on or which D-Map/2 revision it belongs to. This
gap remains open and tracked here, not silently carried forward.

### Phase 114C.1 closure audit

An independent review of the 114C implementation found and fixed one real
architectural gap, plus resolved an inventory-count discrepancy.

**1. Finding-code inventory: 13, not 12 (report-only mismatch, not a code
defect).** The Phase 114C final report's prose summary said "12 stable
string codes" while actually listing (and the implementation actually
declaring) 13: the summary undercounted its own list by one. All 13 were
already consistent across the implementation, this ADR, and the test suite;
`dmonitor_interpretation::ALL_FINDING_CODES` now exists as the single
authoritative inventory, with a test
(`finding_code_inventory_has_exactly_thirteen_stable_codes`) asserting its
exact size and uniqueness so this can never again silently drift between
code and prose. `FINDING_UNSUPPORTED_SUBJECT_EVIDENCE` was kept (never
removed merely to make the count round): it is declared but not currently
emitted by any code path (both `evaluate_service_placement` and the
unexpected-realization scan only ever receive records already filtered to
`DServRealization`), reserved for a future generalization of this module
across multiple subject kinds.

**2. Unexpected `service_id` detection (real defect, fixed).** The original
114C `evaluate_application_dserv_convergence` only ever asked, per DESIRED
service, "does evidence exist for this exact service?" -- it had no way to
discover a fresh, exact-authority-matching `DServRealization` for a
`service_id` that has NO desired placement in the active D-Map at all
(`latest_dserv_realizations_per_stream` was parameterized by a single,
already-known `service_id`). A deployment with a correctly-placed desired
service AND a fresh, exact-authority-matching realization of a completely
different, undesired service would have been reported `Converged` --
"every desired service has one correct observation" is not the same claim
as "nothing unexpected is running." Fixed: the single-service enumeration
was replaced by `latest_dserv_realizations_for_application` (scans the
WHOLE project/application, grouped by `service_id` afterward), and a new
`unexpected_realizations: Vec<DMonitorUnexpectedServiceEvidenceV1>` field on
`DMonitorApplicationConvergenceV1` carries evidence for every `service_id`
absent from the desired set. A PROVEN (admitted, i.e. fresh +
exact-authority-matching) unexpected realization escalates the application
to `Diverged` with `unexpected_realization`, regardless of how the desired
services fared; a stale or old-D-Map-correlated unexpected observation is
preserved as excluded evidence but never escalates convergence (mirroring
the exact same freshness-precedence rule already established for desired
services). `wrong_node` (a DESIRED service observed on the wrong node) and
`unexpected_realization` (a service with no desired placement at all) are
kept structurally distinct -- proven directly by
`wrong_node_desired_service_is_distinct_from_unexpected_service_id`.

**3. Zero-desired-placements edge case, corrected.** Section 18's original
`no_desired_placements`/`Unknown` result for an application with zero
desired D-Serv placements is still correct when there is also no
current/fresh evidence at all. But a zero-desired-placement application
with a FRESH, exact-authority-matching `DServRealization` for some service
is now correctly `Diverged`/`unexpected_realization` rather than
`Unknown`/`no_desired_placements` -- every currently-running realization is,
by definition, unexpected when nothing at all is desired. Zero desired
placements plus only stale/historical evidence remains `Unknown` (never
`Diverged`), consistent with the freshness-precedence rule.

**4. A second, related latent defect discovered and fixed while
implementing the above.** The original per-service evidence scan
(`latest_dserv_realizations_per_stream`) tracked only ONE "latest for this
stream" value per `service_id`, overwriting it on every match regardless of
the observed `node_id`. Since a `DServRealization` subject's canonical
identity includes `node_id` (two different nodes realizing the "same"
service are two DIFFERENT subjects, not one subject whose value changed),
a single stream reporting the same service on two different nodes,
interleaved, would have silently discarded the earlier node's observation
the moment the later node's observation was seen -- even though Phase
114B1.1 already established that a stream's "latest per subject" tracking
must be per EXACT subject, not per some coarser grouping. Fixed as part of
the `latest_dserv_realizations_for_application` rewrite, which now keys its
per-stream "latest" map by the full `(service_id, node_id)` pair -- proven
directly by
`same_service_two_different_nodes_within_one_stream_are_both_tracked`. This
was a real gap in the original implementation, not a hypothetical one: it
would have caused a genuine wrong-node/unexpected-realization false
negative for any stream (a central control-plane collector, for instance)
that ever reported on the same service across more than one node.

## Canonical DIoT correlation closure (Phase 114D0A)

### The gap this pass closes

Phase 114C's audit (see "114C.1 closure audit" above) proved that the
ORIGINAL `DMonitorSubjectV1::DIoTRealization` shape --
`{project_id, application_id, dforward_ref: Option<ContractRef>,
diot_placement_id: Option<String>}` -- could never support exact CURRENT
DIoT placement convergence, for three independent reasons: it carried no
observed `node_id` at all, no `dmap_ref` at all (only `dforward_ref`), and
no reference to Phase 113B1's `DIoTRuntimeBindingV1` operational authority.
`evaluate_diot_placement_convergence` therefore always failed closed to
`Unknown`/`insufficient_canonical_correlation`, by design, regardless of
input. Phase 114D0A closes this gap at the model and interpretation layer:
canonical DIoT observation correlation, DATUM/DServer mirror parity, exact
correlation against DMap/2 + DForward + DIoT runtime-binding authority, and
a real derived DIoT convergence engine. It deliberately does NOT implement
the long-running runtime producer/heartbeat, readiness, `any_element`, or
any reconciliation/mutation -- those remain out of scope (114D0B and later).

### Enriched `DIoTRealization` shape

`DIoTRealization` gained three REQUIRED identity fields and three OPTIONAL
authority-correlation fields, following exactly the pattern already
established for `DServRealization` (required identity, optional
correlation refs):

```rust
DIoTRealization {
    project_id: String,
    application_id: String,
    diot_id: String,
    diot_placement_id: String,
    node_id: String,
    dmap_ref: Option<DMonitorContractRefV1>,
    dforward_ref: Option<DMonitorContractRefV1>,
    runtime_binding_ref: Option<DMonitorRuntimeBindingRefV1>,
}
```

`node_id`, `diot_id`, and `diot_placement_id` moved from absent/optional to
REQUIRED non-empty: a `DIoTRealization` observation that cannot honestly
name which node it was observed running on, or which governed DIoT/
placement it claims to realize, is not admissible evidence at all, never a
degraded-but-valid one. A new top-level consistency check
(`subject_node_id_inconsistent`) requires `observation.node_id ==
subject.node_id` for this variant specifically -- a governed local-runtime
DIoT observation must be of the OBSERVING node's own realization, mirroring
the same "no helpful inference across possibly-different nodes" discipline
Phase 114C already applied to D-Serv comparison logic.

### A new type, not a reused one: `DMonitorRuntimeBindingRefV1`

```rust
pub struct DMonitorRuntimeBindingRefV1 {
    pub binding_id: String,
    pub binding_revision: u64,
    pub descriptor_digest: String,
}
```

This is deliberately a SEPARATE type from `DMonitorContractRefV1`, even
though both are structurally `{id-like, revision, digest}` triples.
`DMonitorContractRefV1` references a CANONICAL CONTRACT (a DMap/DForward
revision) -- desired-state authority. A `DIoTRuntimeBindingV1` is
OPERATIONAL authority: a server-derived, currently-resolved answer to
"which broker does this DIoT placement resolve to right now," itself
re-derived from and validated against canonical contract authority (Phase
113B1), but not a contract itself. Collapsing the two into one type would
blur that distinction at exactly the place it matters most: an observation
correlating against operational binding authority is making a different,
narrower claim than one correlating against a DMap/DForward contract
reference, and the type system now makes that difference visible in the
wire schema itself.

### Exact-correlation algorithm (`evaluate_application_diot_convergence`)

`dserver::core::dmonitor_interpretation::evaluate_diot_placement_convergence`
(the Phase 114C stub that always returned `Unknown`) is replaced by
`evaluate_application_diot_convergence`, a whole-application evaluator
mirroring `evaluate_application_dserv_convergence`'s own structure (one
evaluation per desired placement plus a whole-application
unexpected-realization scan, both built from a single
`latest_diot_realizations_for_application` enumeration keyed by the full
`(diot_id, diot_placement_id, node_id)` triple -- the DIoT analogue of
114C.1's `(service_id, node_id)` fix). For each admitted piece of evidence
to prove CURRENT convergence, ALL of the following must hold:

1. **Freshness** -- `dmonitor_freshness::evaluate_freshness` accepts it (the
   same freshness-precedence rule as D-Serv: stale/clock-skewed evidence can
   prove `Unknown`, never `Diverged` nor `Converged`).
2. **DMap correlation** -- the observation's `dmap_ref` exactly matches the
   CURRENT active authority's `{dmap_id, revision, digest}`.
3. **DForward correlation** -- the observation's `dforward_ref` exactly
   matches the current authority's `{dforward_id, dforward_revision,
   dforward_digest}` (from `PlacementAuthorityV2Extras`).
4. **Runtime-binding correlation** -- the observation's `runtime_binding_ref`
   exactly matches the CURRENTLY REGISTERED `DIoTRuntimeBindingV1` for this
   exact `(project_id, application_id, diot_placement_id)`, looked up from
   `DIoTRuntimeBindingRegistryStateV1::get` -- never merely "some binding
   once existed."
5. **Runtime-binding currentness** -- that currently-registered binding
   itself passes `verify_runtime_binding_still_current` (Phase 113B1's ONE
   production fail-closed check, reused verbatim, never reimplemented)
   against the SAME `DDeployStateV1`/`DNodeRegistryStateV1` this evaluation
   was given.
6. **Node match** -- the subject's own `node_id` equals the desired
   placement's `node_id` (wrong-node / conflicting-multi-node-evidence
   combinatorics are identical to D-Serv's `evaluate_service_placement`).

Any single failure excludes the evidence with the single most specific
reason, in priority order (a stale/never-registered/mismatched binding is
reported before a generic authority mismatch, which is reported before a
generic freshness failure) -- six new finding codes were added to
`ALL_FINDING_CODES` for this (`runtime_binding_not_registered`,
`runtime_binding_ref_missing`, `runtime_binding_mismatch`,
`runtime_binding_stale`, `dmap_v1_no_diot_support`,
`no_desired_diot_placements`), extending the Phase 114C.1 inventory from 13
to 19 entries -- none of the original 13 were redefined or renamed.
`AuthorityCorrelationV1` gained a `NotApplicable` variant reserved for a
correlation dimension that does not apply in context; it is not currently
constructed because `datum.dmap/1` (which has no DIoT placement concept at
all) is handled once at the whole-application level, before any
per-placement DForward comparison is reached, rather than per-dimension.

### Transitive artifact authority (why the runtime-binding hop matters)

An exact `runtime_binding_ref` match proves something a raw `dmap_ref`/
`dforward_ref` match alone cannot: `DIoTRuntimeBindingV1` (Phase 113B1)
already carries `artifact_id`/`execution_generation`/
`execution_projection_digest`, derived from and continuously re-validated
against the CURRENT `diot_artifact_authority` the moment
`verify_runtime_binding_still_current` runs. So correlating against the
CURRENT registered binding transitively proves correlation against current
DMap/DForward/artifact-execution authority in one step:
`DMonitor runtime_binding_ref -> exact DIoTRuntimeBindingV1 descriptor ->
current DMap/DForward/artifact authority`. The descriptor digest is an
integrity/correlation mechanism inside this governed state model, not a
cryptographic authentication mechanism, and `broker_uri` deliberately never
participates in this identity chain -- a broker URI is a network coordinate
a binding currently resolves to, never a component of WHICH placement is
being observed.

### No server-synthesized observation

The mere existence of a registered `DIoTRuntimeBindingV1` never, by itself,
proves a placement is alive: `evaluate_application_diot_convergence` never
synthesizes admitted evidence from registry state alone. A currently
registered-and-current binding with zero fresh, exactly-correlating
`DIoTRealization` observations remains `Unknown`/`no_evidence` -- exactly as
a desired D-Serv placement with an active DMap entry but no observations is
`Unknown`, never `Converged`, under the pre-existing D-Serv rule. This is
also why a "proven unexpected DIoT realization" (the DIoT analogue of Phase
114C.1's D-Serv `unexpected_realizations` escalation) is structurally
unreachable, not merely untested: `verify_runtime_binding_still_current`
itself fails the instant a binding's `placement_id` is absent from the
current active DMap/2's `diot_placements`, so a binding can never be both
"currently registered and current" and "for a placement outside the desired
set" at the same time. `unexpected_realizations` is still tracked (for
transparency of excluded/historical evidence, matching D-Serv's Case C
precedent) but can never escalate DIoT convergence to `Diverged`.

### DServ and DIoT convergence remain separate results

`post_evaluate_dmonitor_convergence`'s response now carries `dserv`
(`DMonitorApplicationConvergenceV1`, unchanged) and `diot`
(`DMonitorApplicationDiotConvergenceV1`, new) as two independent top-level
fields. DIoT evidence being missing or `Unknown` never changes an
already-computed D-Serv `Converged`/`Diverged` result, and vice versa --
the same independence rule Phase 114C already applied between freshness,
authority correlation, placement convergence, and observed health.

### Lock order extended

DIoT convergence additionally reads `DNodeRegistryStateV1` and
`DIoTRuntimeBindingRegistryStateV1` (via `verify_runtime_binding_still_current`),
so `post_evaluate_dmonitor_convergence` now acquires all four transaction
locks, always in this order, for the whole handler (even the D-Serv
portion): `DDEPLOY_TRANSACTION -> DNODE_TRANSACTION ->
DIOT_RUNTIME_BINDING_TRANSACTION -> DMONITOR_TRANSACTION`. This is the SAME
order `diot_runtime_binding.rs`'s own registration/lookup handlers already
use and the same order Phase 114B1's own module documentation reserved in
advance for exactly this future combination -- audited, not invented, for
this pass.

### Pre-integration schema-version decision

`datum.dmonitor-observation/1` is evolved IN PLACE rather than versioned to
`/2`. This is safe because `agent/dmonitor-v1` (the branch carrying the
entire Phase 114 arc) has never merged to `main`: `origin/main` has no
consumer of the old `DIoTRealization` shape to break. Once this work
integrates, any FUTURE incompatible change to a canonical DMonitor subject
shape must use the normal `/2` versioning discipline this project applies
everywhere else.

### Phase 114D0B producer seam: audited, not implemented

Per this pass's explicit scope, the long-running DIoT runtime producer was
audited but NOT built. The exact seam is
`DATUM/src/dcall/delivery.rs::run_mqtt_dcall_listener` (`dserver_url,
dnode_root, project_id, application_id, diot_placement_id, stop`): it loads
local D-Node identity, calls `client::fetch_current_runtime_binding`
(`GET .../current`), validates `transport == "mqtt"`, connects via
`RumqttcPubSub::connect`, and calls `MqttPubSub::subscribe` -- the exact
point after which the listener can HONESTLY claim "this realization is
alive." It then enters a `while !stop.load(...)` 500ms poll loop, which is
where a periodic DMonitor heartbeat should originate in 114D0B. No
reconnect/disconnect logic exists today (a `poll` error propagates via `?`
and exits the function), which 114D0B will need to address alongside adding
the heartbeat itself. Separately, `deliver_via_diot_pubsub` (the source
side of a DCall grant) already independently re-verifies the current
binding matches a grant's bound authority
(`binding_id`/`binding_revision`/`descriptor_digest`/`transport`/
`project_id`/`application_id`/`placement_id`) before opening any
connection -- confirming this exact-match comparison pattern already has
production precedent elsewhere in the codebase, independent of this pass's
new DMonitor-side comparison.

### Deferred to later phases

Readiness, `any_element`, and reconciliation/mutation semantics remain
entirely out of scope for 114D0A (and were not touched). Live multinode
validation of the DIoT correlation algorithm itself remains 114E's job, per
the same evidentiary-discipline carryover already stated for D-Serv
convergence above.

## Live governed DIoT runtime evidence producer (Phase 114D0B)

Phase 114D0A closed the canonical/interpretation-layer gap; this phase
makes a REAL running governed DIoT/MQTT realization emit that evidence.
DServer-side code is completely untouched -- DServer remains observer only
(ingestion/persistence/freshness/interpretation), never a producer.

### Why `subscribe()` returning `Ok` was never sufficient

Independent review of `rumqttc` 0.24's synchronous `Client`/`Connection` API
confirmed the audit's premise exactly: `Client::subscribe`/`try_subscribe`
only enqueue a Subscribe request onto an internal channel and return
immediately -- no network I/O happens at all until the FIRST call to
`Connection::recv`/`recv_timeout`, and the OLD `RumqttcPubSub::poll`
discarded every non-`Publish` incoming packet (`Ok(Ok(_)) => continue`),
so `ConnAck`/`SubAck` were never even observed anywhere in this codebase.
A caller that treated `subscribe()` returning `Ok(())` as "the destination
is listening" was therefore trusting a value that says nothing about
whether the broker ever accepted the connection or the subscription at all.

### Protocol-ready condition implemented

A new, ADDITIVE trait, `dcall::mqtt::MqttListenerSession::establish_and_confirm`,
drains the connection's own event stream and classifies exactly:

- the first `Packet::ConnAck` must carry `ConnectReturnCode::Success`;
- the FIRST `Packet::SubAck` observed after that must carry an
  all-`Success` (never `Failure`) `return_codes` list for this listener's
  one subscription;
- any `Packet::Publish` observed in between is buffered (never dropped) so
  `poll` still delivers it once steady-state polling begins;
- anything else (a connection error, a SubAck before any ConnAck, or the
  bounded confirmation window elapsing with neither observed) is
  `MqttSessionReadiness::ConnectionFailed`/`SubscribeRejected`, never
  `Ready`.

`MqttPubSub` itself (the trait `deliver_via_diot_pubsub`, the SOURCE side of
D-Call delivery, depends on) is completely unchanged -- `MqttListenerSession`
extends it additively, so source-side delivery is untouched by this work
(confirmed by the full pre-existing 38-test `dcall::delivery` suite passing
unchanged, plus 1 new coexistence test).

### Actual rumqttc reconnect semantics found (section 23's audit)

`rumqttc::Connection::recv`/`iter` are documented, in the crate's own source,
as: *"Continuing to poll will reconnect to the broker if there is a
disconnection."* The underlying library primitive is genuinely capable of
transparent reconnection on continued polling. However, DATUM's OWN
`RumqttcPubSub::poll` wrapper converts any non-timeout `ConnectionError`
into a hard `anyhow::Error`, and the existing (pre-114D0B)
`run_mqtt_dcall_listener` propagates that error via `?`, terminating the
function immediately. At the DATUM APPLICATION level, therefore, reconnect
was NOT part of existing production behavior (category A: terminates on
connection loss) -- only theoretically available in the underlying
dependency, never exercised. Phase 114D0B preserves this exact fail-closed
termination behavior unchanged and does NOT invent a reconnect
architecture: a definite transport failure still ends the listener; the
only addition is a best-effort `Unhealthy` observation emitted immediately
before that same termination.

### Monitor epoch, sequence, and observation identity

The Phase 114B2 process-epoch/sequence machinery
(`DMonitorProjectionContext`) was extracted, unchanged in behavior, into a
new neutral module `agent::dmonitor_emission` (renamed
`DMonitorEmissionContext` there; `dmonitor_projection` keeps a `pub use ...
as DMonitorProjectionContext` alias so every existing external caller and
test compiles unchanged). Both `agent::dmonitor_projection` (SmartSentinel)
and the new `agent::diot_runtime_dmonitor` (DIoT runtime) now share the
IDENTICAL "one process epoch = one `monitor_instance_id` = one monotonic
sequence counter, never reused or rewound" rule from this ONE module,
rather than risking a second, subtly different reimplementation. One
`DMonitorEmissionContext::new_process_epoch` is created once per listener
process; a restart creates a fresh epoch starting at sequence 1.

### The DIoT runtime observation builder

`agent::diot_runtime_dmonitor::build_diot_realization_observation` (wrapped
by `DiotRuntimeMonitorEmitter::emit`, which also allocates the sequence) is
a pure, fully unit-tested function producing one `CanonicalDMonitorObservationV1`
per call. Every subject/reference field
(`project_id`/`application_id`/`diot_id`/`diot_placement_id`/`node_id`/
`dmap_ref`/`dforward_ref`/`runtime_binding_ref`) comes from an explicit
`DiotRuntimeBindingIdentity`, itself built ONCE from the startup CURRENT
runtime-binding response (`dcall::client::CurrentRuntimeBindingResponse`,
the exact same production fetch `run_mqtt_dcall_listener` already made
pre-114D0B) and never re-derived mid-session. `facts` is a small fixed
allowlist -- `runtime_state` (`"subscribed"`/`"transport_failure"`, a
closed category set, never raw error text) and `heartbeat` (bool) -- and
never carries `broker_uri`, credentials, raw topic/filter strings, tokens,
grants, or secret configuration; a dedicated test
(`no_broker_uri_or_secret_field_appears_anywhere_in_the_observation`)
scans the fully serialized observation for exactly these forbidden
substrings.

### Session-binding immutability (section 8)

`DiotRuntimeMonitorEmitter` carries its bound `DiotRuntimeBindingIdentity`
in a private field with no setter -- structurally, nothing in this
listener's lifetime can make it start referencing a DIFFERENT binding
revision after construction. If server authority advances to binding N+1
while this MQTT session remains open under N, every observation this
session emits keeps honestly referencing N; Phase 114D0A's
`verify_runtime_binding_still_current` check is exactly what then
classifies N as no longer current on the server side. This session never
"refreshes" its own authority reference in place.

### Initial and periodic Healthy evidence

Immediately after `establish_and_confirm` returns `Ready`, the listener
emits ONE `Healthy` observation (`heartbeat: false`). While the session
remains established, the steady-state loop emits a fresh `Healthy`
observation (`heartbeat: true`, new sequence, new `observation_id`, new
`observed_at_utc`, SAME `monitor_instance_id` and binding reference)
whenever `--dmonitor-heartbeat-interval-seconds` (default 30, validated
`> 0`, wired through `smartsentinel-dcall-mqtt-listen`) has elapsed. This
cadence is purely OPERATIONAL emission configuration -- it is NOT Phase
114D1's freshness/readiness policy, which DServer will evaluate this
evidence against independently.

### Transport-failure and graceful-shutdown semantics

A definite transport failure (a previously-`Ready` session's `poll`
returning `Err`, or `establish_and_confirm` itself failing) stops Healthy
heartbeats and emits exactly ONE best-effort `Unhealthy` observation
(`runtime_state: "transport_failure"`) before the listener's existing
fail-closed termination is preserved unchanged. Graceful shutdown
(`stop` set, e.g. by Ctrl-C) is explicitly NOT a transport fault: no
`Unhealthy` observation is emitted merely because the operator
intentionally stopped the listener -- the last `Healthy` evidence is left
to age to Stale under Phase 114C's existing freshness rule, which is
already a safe, honest terminal state requiring no special-casing here.

### Non-blocking, bounded DMonitor publication (section 18/19)

`dcall::dmonitor_publisher::DMonitorPublisherHandle` spawns exactly ONE
background worker thread per listener process epoch, draining a bounded
`std::sync::mpsc::sync_channel` (capacity 8) and calling the EXISTING
production `DMonitorHttpOutput::publish` synchronously -- no second HTTP
protocol is introduced (reusing the exact Phase 114B2 output, per section
30). The listener's own thread only ever calls `try_publish`
(`SyncSender::try_send` underneath), which NEVER blocks: a full queue or a
disconnected worker silently DROPS the job (logged diagnostically) rather
than waiting. Because every sequence number is allocated (via
`DiotRuntimeMonitorEmitter::emit`, itself calling
`DMonitorEmissionContext::allocate_sequence`) strictly BEFORE a job is ever
offered to the publisher, a dropped job produces an ordinary, already-
permitted sequence gap (Phase 114B1's own established rule) -- never a
reused sequence number. Proven directly by
`dcall::dmonitor_publisher::tests::try_publish_never_blocks_even_when_the_sink_never_replies`
and `queue_saturation_drops_observations_without_blocking_and_without_reusing_ids`
(a deliberately unresponsive HTTP sink, `try_publish` still returning in
well under 500ms).

### DMonitor publication failure is never MQTT runtime failure (section 20)

`emit_and_publish`'s only interaction with the publisher is `try_publish`,
whose return value is not even inspected by the listener loop -- a publish
failure (non-2xx, transport error, timeout) is isolated entirely inside the
worker thread (logged, then the worker moves on to the next queued job) and
can never make the calling MQTT session itself `Unhealthy`, retry in a
loop, or trigger remediation/host mutation. MQTT runtime health and
DMonitor evidence-delivery health remain two independent systems, exactly
as Phase 114B2's own established rule requires.

### No server-synthesized liveness (section 36)

No DServer code was touched in this phase. DServer never runs a timer or
job that creates DMonitor observations, and never synthesizes evidence from
`DIoTRuntimeBindingRegistryStateV1` -- the registration alone still proves
nothing about runtime liveness (Phase 114D0A's own rule, now actually
exercised end to end): DServer remains exactly ingestion, persistence,
freshness, and interpretation; the MQTT listener is the ONLY producer.

### D0A + D0B end-to-end evidence chain (section 31)

`DATUM/tests/dmonitor_d0a_d0b_integration.rs` (`#[ignore]`d, mirroring the
established `dmonitor_b2_integration.rs`/`dcall_mqtt_governed_e2e.rs`
real-DServer-subprocess convention) builds a REAL DMap/2 + DForward + DIoT
placement + runtime binding through the production HTTP proposal/
acceptance/registration flow, fetches the CURRENT binding through the exact
same `dcall::client::fetch_current_runtime_binding` seam
`run_mqtt_dcall_listener` itself calls, builds one `Healthy` observation
through the PRODUCTION Phase 114D0B builder, publishes it through the
PRODUCTION `DMonitorHttpOutput`, and proves the REAL Phase 114D0A
convergence endpoint returns `diot.convergence == "converged"` with exact
`dmap_correlation`/`dforward_correlation`/`runtime_binding_correlation` all
`"exact_match"`, `runtime_binding_current: true`, and `freshness:
"fresh"`. It deliberately does not require a real MQTT broker or the
listener subprocess -- MQTT session-readiness classification is proven
independently at the unit level (`dcall::mqtt`'s deterministic
`InMemoryBroker`-backed state-machine tests), since a real broker is an
external dependency this proof does not need in order to exercise the part
that genuinely spans both phases and crates: authority registration ->
production observation construction -> production HTTP publication -> real
server-side exact correlation.

### Readiness still deferred

Phase 114D0B implements ONLY evidence production. No readiness policy, no
`any_element`, no placement selection, no reconciliation, no automatic
corrective action, no monitor leader election/health voting, and no
retention/pruning were implemented or touched. An exact-current DIoT
realization may legitimately be `convergence = Converged` with
`health = Unhealthy` simultaneously (Phase 114C/114D0A's health/convergence
independence, unchanged) -- Phase 114D1 is what will decide whether such a
state counts as "ready."

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
- D-Serv placement drift/convergence is now derivable, read-only, from
  `dserver::core::dmonitor_interpretation::evaluate_application_dserv_convergence`
  -- see "Derived convergence/drift interpretation (Phase 114C)" above. DIoT
  placement convergence is now ALSO derivable, read-only, from
  `evaluate_application_diot_convergence`, exactly correlating against
  DMap/2 + DForward + DIoT runtime-binding authority -- see "Canonical DIoT
  correlation closure (Phase 114D0A)" above. D-Code/D-Call evidence remains
  deliberately out of scope for placement convergence.
- A REAL governed MQTT DIoT realization now produces canonical `Healthy`/
  `Unhealthy` DMonitor evidence -- protocol-confirmed session readiness
  (ConnAck+SubAck, never a bare `subscribe()` return), a periodic heartbeat
  referencing an immutable session-binding authority, and non-blocking
  bounded publication that can never stall MQTT delivery -- see "Live
  governed DIoT runtime evidence producer (Phase 114D0B)" above, proven end
  to end against a real DServer process
  (`dmonitor_d0a_d0b_integration.rs::d0a_d0b_end_to_end_diot_evidence_converges`).
  DServer itself was not modified: it remains observer only.
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
