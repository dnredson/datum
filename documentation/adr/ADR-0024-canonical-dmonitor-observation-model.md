# ADR-0024 — Canonical DMonitor observation model

- Status: Implemented for Phase 114A (canonical wire model, validation,
  digest, and unit tests only). Collection/persistence (114B), convergence
  and drift views (114C), readiness/`any_element` feed integration (114D),
  and live multinode validation (114E) are all explicitly deferred to later
  phases and are not implemented by this ADR.
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
- No DServer-side ingestion, storage, or query surface exists yet. Nothing
  currently consumes `datum.dmonitor-observation/1`. Building that consumer
  (independently mirrored per ADR-0007, exactly as `DGraph`/`DMap`/
  `DForward` already are) is 114B's job.
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
