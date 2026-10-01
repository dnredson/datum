# Phase 122 — Canonical `DatumEvent` Model

**Status:** implemented, unit-tested, not yet committed. Hardened per an
independent adversarial freeze review. Not wired into any producer.
Additive only — no existing behavior outside this module was changed.

**Scope:** `dserver/src/core/datum_event.rs` (new module), registered in
`dserver/src/core/mod.rs`. Nothing else changed.

## Why this phase exists

The Phase 121 audit found that DServer has no canonical, typed, versioned
notion of "an occurrence happened." What looks the most event-like today,
`build_global_events` (`core/events.rs`), is architecturally the opposite of
an event log: it recomputes a full snapshot on every request, overwrites the
prior result in Firestore with no history, hardcodes severity to `"high"`/
`"medium"` and status to permanently `"open"`, and mints a **new,
non-deterministic id** for its `impact_summary` entry on every single call.
Calling the same endpoint twice with nothing changed produces two different
"events" describing the same fact. Phase 122 exists so that Phase 126's
producers have something better to emit into than that pattern.

## Freeze review and hardening pass

Before this contract could be considered frozen, an independent adversarial
review (documented findings F1–F14) inspected the initial implementation
and found several gaps between the stated intent and what was actually
enforced. All freeze-blocking and high-severity findings were resolved in
this hardening pass; this document reflects the *final*, hardened contract.
The sections below describe the current, post-review behavior throughout —
prior drafts of this document (and the original, now-superseded validation
rules) are not reproduced here.

## Event != Alert != Incident

- **`DatumEvent`** (this phase) is a normalized record of **something that
  happened** — an occurrence, immutable by contract once admitted.
- **`DatumAlert`** (Phase 124, not yet implemented) is a **persistent
  condition/lifecycle derived from one or more `DatumEvent`s** — it owns
  `open`/`acknowledged`/`resolved`/`suppressed`, `acknowledged_by`,
  `acknowledged_at`, `resolved_at`, `first_seen`/`last_seen` as
  condition-bookkeeping, and `occurrence_count` as dedup state.
- An **incident** (informal, not yet a typed concept anywhere in this
  codebase) would sit a level above `DatumAlert` again — correlating
  multiple alerts into one human-facing story.

`DatumEventV1` carries **none** of the alert-lifecycle vocabulary above.
Enforced three ways: `#[serde(deny_unknown_fields)]` on the struct;
a static contract-assertion test (`no_alert_lifecycle_field_exists_on_the_contract`)
whitelisting the exact top-level key set; and a deserialization-rejection
test (`alert_lifecycle_fields_are_rejected_as_unknown_by_deserialization`)
proving each forbidden name is actually rejected, not merely absent from
the struct definition.

## Immutability — contract semantics, not Rust-level enforcement

`DatumEvent` represents an immutable occurrence **by contract**. Phase 122
exposes no event-lifecycle mutation method — there is no `pub fn` anywhere
in this module that takes `&mut DatumEventV1`; every operation (`validate`,
`recompute_datum_event_digest`, serialization) is read-only over an
existing value. Phase 123 will enforce append-only persistence/admission
semantics on top of that.

This is **not** a Rust-level structural guarantee, and this document does
not claim it is: every field on `DatumEventV1` remains an ordinary `pub`
value, and can technically be reassigned by any caller holding a `&mut`
reference, before admission (e.g. `event.severity = Critical;` compiles
fine). The absence of a mutator API rules out a *convenience* path to
mutation, not a *possible* one. Genuine immutability is Phase 123's job
(a write-once, append-only store), not this type's.

## Event identity vs. alert/correlation identity

`event_id` identifies exactly **one admitted occurrence**. It is stable
across a retried submission of that same occurrence (idempotency), but a
producer **must** mint a new `event_id` for a genuinely new, independent
occurrence, even one that shares its `kind`/`subject`/`severity` with a
prior event.

This is deliberately **not** a condition-level identity. Two independent
observations of "node X is unhealthy" five minutes apart are two different
`DatumEventV1`s with two different `event_id`s. Recognizing "these are the
same ongoing problem" is Phase 124's job — `DatumAlert` correlation, built
from whatever key the Alert Manager derives, independently of `event_id`.
Phase 122 implements **no condition-level deduplication**; proven directly
by `two_independent_occurrences_of_the_same_condition_are_both_valid_with_distinct_ids`.

`correlation_hint` is a separate, optional field: producer-supplied
metadata only, **not** the canonical alert correlation key. Phase 124's
Alert Manager may accept it, reject it, normalize it, or derive its own key
entirely independently.

`recompute_datum_event_digest` computes a JCS+SHA-256 digest (ADR-0006)
over the **full admitted record**, `event_id` AND `occurred_at_utc`
included — proven by dedicated tests
(`digest_changes_when_event_id_changes`,
`digest_changes_when_occurred_at_utc_changes`,
`same_canonical_event_retried_produces_the_same_digest`). This is a
content fingerprint for integrity/replay verification, explicitly **not**
a condition-correlation key: three distinct concepts — replay-content-
identity (the digest), occurrence-identity (`event_id`), and
condition-identity (Phase 124's future correlation key) — stay three
distinct mechanisms.

## Category vs. kind

`category: DatumEventCategoryV1` is a small, closed, typed enum (seven
variants — `Observation`, `Runtime`, `ControlPlane`, `Health`, `Analysis`,
`Operator`, `Model`) carried forward from the Phase 121 audit's own
event-classification section.

`kind: String` is deliberately **not** a closed enum, to avoid forcing a
schema-breaking change to `DatumEventV1` every time Phase 126 wires one
more producer. `category` gives a stable, typed, closed dimension to
query/filter on; `kind` stays open within it, validated as non-empty
`lower_snake_case`.

**`category` and `kind` are validated fully independently, deliberately.**
`DatumEventV1::validate` guarantees `kind`'s own *syntactic* validity —
never a semantic category/kind pairing. A `Model`-category event with a
`ControlPlane`-shaped `kind` (e.g. `kind = "proposal_accepted"`) currently
validates cleanly, and this is a tested, intentional invariant
(`category_and_kind_are_validated_independently`), not an oversight: a
hardcoded `category → allowed kind` table would itself be exactly the
premature global taxonomy this phase is avoiding. That boundary belongs to
Phase 126, once real producers exist to inform it — as a lightweight
per-producer convention or test, not a table baked into this foundational
contract.

## Severity

Four levels: `Info`, `Warning`, `High`, `Critical` — deliberately not
eight-to-ten. Explicitly not the same axis as health state, readiness,
convergence, a DMonitor/DForward finding code, or subject runtime state —
see the enum's own doc comment in the source file for the full comparison.

## Source — provenance only, extensible without a schema bump

`source: DatumEventSourceV1` covers 15 known internal subsystems as
strongly-typed variants (`DMonitor`, `SentinelTelemetry`, `ActiveMonitor`,
`DDeploy`, `Reconciliation`, `DForward`, `DIoT`, `DCode`, `DEngine`,
`Runtime`, `ServiceArtifact`, `RuleEngine`, `Operator`, `Gnn`, `Llm`), plus
**one** additional variant: `Other(String)`.

The freeze review found the original fully-closed enum would hard-fail
deserialization for any old reader receiving an event from a source it
didn't recognize — a real forward-compatibility gap given the roadmap's own
mention of future external adapters/plugins. The fix is a **hand-written**
`Serialize`/`Deserialize` implementation (not `#[derive]`), chosen
specifically so the wire format stays exactly a bare JSON string in every
case:

```json
"source": "dmonitor"
"source": "gnn"
"source": "future_subsystem"
```

There is no nested wrapper distinguishing known from unknown values. A
recognized token (e.g. `"dmonitor"`) deserializes into its typed variant; any
other non-empty `lower_snake_case` string deserializes into
`Other(String)`, preserving the exact unrecognized value rather than
collapsing it into a generic literal or failing outright — proven by
`unknown_source_round_trips_and_preserves_identity`. `all_known_sources_serialize_to_the_original_lowercase_tokens`
proves every one of the 15 known tokens is byte-identical to before this
change.

**Backward/forward compatibility semantics:**
- A brand-new producer subsystem can start emitting `"source":
  "future_subsystem"` **today**, against the current `datum.event/1`
  schema, with zero code change to this enum. An old reader that doesn't
  recognize it deserializes into `Other("future_subsystem")` instead of
  failing the whole record — graceful degradation, not a hard break.
- Giving that subsystem a dedicated typed variant later is a purely
  additive Rust/code change: previously-`Other`-wrapped events start
  deserializing into the new typed variant instead, with **no wire-format
  change at all**. **No `datum.event/2` schema bump is required merely to
  introduce a new producer.**
- `Other`'s string is validated as non-empty `lower_snake_case`
  (`source_other_invalid_format`) and rejected if it collides with an
  already-known token (`source_other_collides_with_known_value` — construct
  the typed variant instead).

`source = Gnn`/`Llm`/`Other(...)` identifies provenance only — nothing
reads `source` to decide whether to trust or act on an event differently.

## Subject — one typed scope per event, project available for partitioning

Unchanged by this hardening pass except for one direct-validation fix (see
below). `subject: DatumEventSubjectV1` is a tagged enum with one variant
per operational entity kind an event may concern: `DServerGlobal` (no
scope at all), `Project`, `Application`, `DNode`, `DServ`, `DCall`, `DIoT`,
`ServiceArtifact`, `RuntimeInstance`, `DependencyEdge`,
`DeploymentProposal`, `DMap`, and an `Other` escape hatch.

`project_id`/`application_id` are also carried at the top level of
`DatumEventV1` itself, cross-validated against `subject.project_id()`/
`subject.application_id()` for consistency — proven independently for both
axes (`project_id_inconsistency_with_subject_is_rejected` and
`application_id_inconsistency_with_subject_is_rejected`).

**Fix (F9):** `DatumEventSubjectV1::Other`'s own validator now directly
rejects an empty-but-present `project_id` (`Other { project_id: Some(""),
... }`) with a clean `missing_field` finding, rather than relying
incidentally on the top-level cross-check (which previously produced a
confusing `"must be Some(\"\")"` message only when the top-level field
happened to be set too) — proven by
`other_subject_empty_but_present_project_id_is_rejected_directly`.

## Evidence references — pointers, never copies

`evidence_refs: Vec<DatumEventEvidenceRefV1>` is a tagged enum: pointers
into existing authoritative/observational records, never a copy of their
content. `DatumEventV1` never carries a copy of a
`CanonicalDMonitorObservationV1`, an `OperationalEvidenceEvent`, a
`CanonicalDMapV1`, or a `ServiceArtifact` — only a stable pointer.

**DMap evidence-reference reuse (F3):** the `DMap` variant now reuses
`canonical_dmap::CanonicalContractReferenceV1` directly
(`#[serde(flatten)] reference: CanonicalContractReferenceV1`) instead of
hand-duplicating its `{id, revision, digest}` shape, closing a same-crate
duplication the freeze review found had no cross-crate (ADR-0007-style)
justification, unlike `DMonitorContractRefV1`'s genuine cross-crate mirror.

This reuse was accepted **only after proving wire compatibility, not
assuming it** — and the proof surfaced a real discovery along the way:
`DatumEventEvidenceRefV1`'s tag values were **never** given an explicit
`#[serde(rename = ...)]` in the original implementation (unlike
`DatumEventSubjectV1`'s variants, which are all explicit lowercase
`snake_case`). Every evidence-ref tag — before and after this patch — is
therefore the literal default Rust variant name: `"DMap"` (PascalCase), not
`"dmap"`. The reuse change itself does not alter this in any way — it was
`"DMap"` before, and remains `"DMap"` after — proven by
`dmap_evidence_ref_wire_representation_is_unchanged_after_reuse`, whose
first draft actually asserted the *wrong* (lowercase) expectation and
failed, catching this discovery empirically rather than by assumption.
A second test, `dmap_evidence_ref_still_rejects_unknown_fields_after_reuse`,
independently proves the `#[serde(flatten)]`-based reuse did **not**
silently weaken `deny_unknown_fields` for this variant (a real risk with
`flatten`, checked empirically rather than assumed away).

**Follow-up recommendation, explicitly out of scope for this patch:** the
PascalCase-vs-snake_case tag inconsistency across `DatumEventEvidenceRefV1`
(contrasted with `DatumEventSubjectV1`'s consistent snake_case) is a
pre-existing characteristic, not something this DRY-only change should
have bundled a wire-format rename into. A future small, dedicated pass
could add explicit lowercase renames to all seven evidence-ref tags for
consistency — but that is a genuine wire-format change requiring its own
justification and is deliberately not done here.

## Attributes — diagnostic metadata, structurally bounded, no key-name ban

`attributes: Value` remains the one concession to producer-specific
extensibility, and the envelope itself stays otherwise strongly typed.

**Removed (F1): the `desired_state`-key ban.** The original implementation
recursively rejected any `attributes` key literally spelled `desired_state`,
intending to enforce the "does not mutate desired state" authority rule.
The freeze review found this both **ineffective** (trivially bypassed by
renaming the key — `target_state`, `desired_state_v2`, anything else — so
it protected nothing a careless or adversarial producer couldn't route
around) and **actively harmful** (it rejected legitimate *descriptive* use
of that exact name — e.g. a `desired_state_mismatch` diagnostic event
wanting to show the desired value side-by-side with what was observed,
under the most natural possible key name).

`DatumEvent` may **describe** desired state. It must never **be** or
**mutate** desired state. That distinction is now enforced
architecturally, not lexically — see "Authority boundary" below. A key
literally named `desired_state` anywhere in `attributes` is now permitted,
proven by `descriptive_desired_state_key_in_attributes_is_now_permitted`
and `nested_diagnostic_attributes_object_is_valid`.

**Added (F6): a 16 KiB structural size bound.**
`MAX_DATUM_EVENT_ATTRIBUTES_BYTES = 16 * 1024`. `attributes`' architectural
purpose is small, producer-specific diagnostic metadata — never an
embedded authoritative document (a full DMap, a full DMonitor observation,
or any alternate state store). Nothing else in this contract enforced that
principle; the size bound is what makes it structurally real. Validated
against the compact `serde_json::to_vec` byte length. Proven exactly at
the boundary (`attributes_at_exactly_the_size_limit_are_valid`, one byte
over rejected via `attributes_one_byte_over_the_size_limit_are_rejected`)
and with a clearly-over-limit case
(`attributes_well_above_the_size_limit_are_rejected`), all using
programmatically-generated padding (`"x".repeat(n)`), not giant literals.

**Retained unchanged:** `attributes` must be a JSON object (not an
array/scalar at the top level); every number in it must be finite
(consistent with this contract's JCS digest support). **No depth limit was
added** — see "Stack-safety investigation" below for why.

**Stack-safety investigation (required by the hardening brief, reported
here in full):** `serde_json`'s `Value` deserialization has no built-in
recursion-depth limit — unlike deserializing into a fixed Rust struct
(whose recursion depth is bounded by the type's own shape), building a
`Value` tree from arbitrary JSON has no such guarantee, so a pathological
input (e.g. thousands of nested empty arrays, which are extremely
byte-efficient — a handful of bytes per nesting level) could in principle
exhaust the stack. This exposure exists at the **JSON-parsing boundary** —
wherever raw bytes are deserialized into a `DatumEventV1` (a future
ingestion/API layer that does not exist in this phase) — not in this
module's own recursive validators specifically: by the time `attributes`
reaches `validate()`, the `Value` tree has already been built, so a depth
counter added inside `json_numbers_are_finite` would not close the actual
risk window — the overflow, if it happens, already happened during
deserialization, upstream of anywhere this module's code runs. Adding a
depth limit here would add complexity without providing the safety it
appears to promise. **Conclusion: no depth limit added in this phase**,
consistent with "do not add a depth limit unless implementation safety
requires it" — the real mitigation (e.g. a depth-counting deserializer)
belongs at the future ingestion boundary (a Phase 123+ concern), not in
this type's validator. The 16 KiB size bound does substantially reduce
practical exposure (very little pathological nesting fits in 16 KiB) without
being a formal guarantee.

## GNN/LLM model provenance (F2)

**New rule:** if `source` is `Gnn` or `Llm`, `model_ref` **must** be
present and non-empty — **independent of whether `confidence_permille` is
present**. The freeze review found the original implementation only tied
`model_ref` to *confidence presence*, never to *source*, meaning a
`Gnn`-sourced event with no confidence and no model reference validated
cleanly — directly contradicting the intended rule that any GNN/LLM-sourced
event must identify the model that produced it.

Truth table (all four combinations tested, `Gnn` and `Llm` both covered):

| confidence_permille | model_ref | Result |
|---|---|---|
| absent | absent | **INVALID** (`model_ref_required_for_model_source`) |
| present | absent | **INVALID** (`model_ref_required_for_model_source`, and also `confidence_requires_model_ref`) |
| absent | present | **VALID** |
| present | present | **VALID** |

The pre-existing rule — `confidence_permille` present requires a non-empty
`model_ref` — is **kept unchanged** and applies to *any* source, not only
`Gnn`/`Llm`, since a future non-Gnn/Llm "model-like" source could plausibly
want to report a confidence too. The two rules are independent and both
enforced; `gnn_with_confidence_but_no_model_ref_is_rejected` proves the
source-based rule fires regardless of the confidence-based one.

`confidence_permille` remains fully optional for `Gnn`/`Llm` — a
deterministic model may report provenance with no confidence score at all
(`gnn_with_model_ref_and_no_confidence_is_valid`). Confidence is expressed
as parts-per-thousand (`0..=1000`), not a bare `f64`, so the type stays
within the JCS safe-integer contract used by every other canonical DATUM
digest — mirroring the repository's own `_permille` convention (e.g.
DMonitor's `cpu_usage_permille`).

## Canonical UTC timestamp (F5)

`occurred_at_utc` must now be the **one canonical textual RFC 3339
representation** for its instant, not merely *a* valid RFC 3339 string.
The freeze review found the original rule only checked parseability,
meaning `"2026-09-29T12:00:00-03:00"` and `"2026-09-29T15:00:00Z"` — the
identical instant — produced *different* digests, and the field's own
`_utc`-implying name was never actually enforced.

**Canonicalization rule** (`canonicalize_rfc3339_utc`): parse as RFC 3339,
convert to UTC, re-render with `chrono`'s `SecondsFormat::AutoSi` and a
literal `Z` suffix (never `+00:00`). `AutoSi` snaps the sub-second part to
the **smallest of four discrete tiers** that exactly represents the parsed
nanosecond value: no fractional part, milliseconds (`.SSS`), microseconds
(`.SSSSSS`), or nanoseconds (`.SSSSSSSSS`) — verified empirically, not
assumed: `500_000_000` ns canonicalizes to `.500Z` (the millisecond tier),
not `.5Z` (an initial draft of this rule's documentation and tests assumed
the latter and was corrected once the actual test run proved otherwise).
This tiered approach satisfies "do not unintentionally collapse meaningful
precision" — a value with genuine microsecond/nanosecond significance
keeps that tier; it is never truncated below the precision it actually
carries.

`DatumEventV1::validate` **rejects** any `occurred_at_utc` that does not
already equal the canonical form byte-for-byte
(`timestamp_not_canonical`) — it never silently rewrites the submitted
value. Examples:

| Submitted | Result |
|---|---|
| `2026-09-29T00:00:00Z` | valid (already canonical) |
| `2026-09-29T00:00:00.500Z` | valid (canonical millisecond tier) |
| `2026-09-28T21:00:00-03:00` (same instant as `2026-09-29T00:00:00Z`) | **rejected**, non-UTC offset |
| `2026-09-29T00:00:00.500000000Z` (same instant as `.500Z`) | **rejected**, non-canonical fractional tier |
| `not-a-timestamp` | **rejected**, unparseable (`invalid_timestamp`) |

Digest sensitivity to this field is proven directly by
`digest_changes_when_occurred_at_utc_changes`.

## Relationship to existing contracts

Unchanged from the original Phase 122 scope: `build_global_events`,
`OperationalEvidenceEvent`, DMonitor contracts, sentinel telemetry, and the
existing dashboard APIs are all **entirely unchanged** by this phase —
still additive only. Migrating/wiring these legacy artifacts onto
`DatumEvent` remains explicit future work (Phase 126).

## Authority boundary — what actually prevents this from becoming an authority

The freeze review's most important finding: a field-level naming
restriction (the removed `desired_state`-key ban) does **not**, by itself,
prevent `DatumEvent` from becoming a de facto authority — it is trivially
bypassed by renaming a key. What actually protects the boundary is
architectural, and is stated here explicitly rather than left implicit:

- **`DatumEvent` has no control-plane mutation path.** This module imports
  nothing from, and writes nothing to, `core::ddeploy`,
  `core::operational_dgraph`, or any other mutating subsystem. No function
  here takes a `&mut` reference to any other subsystem's state — proven
  structurally (nothing to mutate) and by
  `contract_exposes_no_mutation_or_authorization_method` as a documented
  checkpoint.
- **Future producers (Phase 126) emit events only AFTER observing or
  performing their canonical operation.** An event is a record of what
  already happened via the existing authoritative path (e.g.
  `ddeploy::accept_proposal`'s own commit), never a request for something
  to happen.
- **Future consumers in the Event/Alert/Operations path must not gate or
  mutate DDeploy, DMap, or reconciliation.** Phase 124's Alert Manager,
  Phase 125's scheduler, and Phase 127's read models must all remain
  strictly read-only/observational with respect to those subsystems — no
  write-back path from a Console, through the event/alert layer, into
  placement or reconciliation.

Canonical truths remain exactly where the Phase 121 audit found them:

| Concept | Canonical source |
|---|---|
| Placement | Accepted DMap (`core::ddeploy`) |
| Desired runtime | deployment/reconciliation desired state |
| Observed runtime | DMonitor / sentinel telemetry evidence |
| Health | existing health/readiness/convergence evaluators |
| Freshness | existing timestamp + policy evaluators |
| Convergence | existing convergence evaluator |
| AI/GNN conclusion | non-authoritative inference only |

A `DatumEventV1` may *refer to* these sources via `evidence_refs`. It must
never embed, replace, or re-derive them.

## What this phase deliberately does not do

Per its own scope boundary: no event persistence, no Event Store, no
`DatumAlert`, no deduplication, no correlation engine, no scheduler, no
wired producers, no modified DDeploy/DMonitor/reconciliation behavior, no
new API endpoints, no Console functionality. `build_global_events`,
`OperationalEvidenceEvent`, DMonitor contracts, sentinel telemetry, and the
existing dashboard APIs are all unchanged.
