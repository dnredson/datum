# ADR-0017 — Canonical DATUM contracts v1: `datum.dgraph/1`, `datum.dcontinuum/1`, `datum.dmap/1`

- Status: Accepted and implemented (Stage 110).
- Decision refs: ADR-0001 (canonical vocabulary), ADR-0002 (D-Map single
  placement carrier), ADR-0003 (`datum-dnode/0` D-Node ABI), ADR-0004
  (D-Serv execution contract host invariance), ADR-0006 (canonical digest,
  JCS+SHA-256), ADR-0007 (no new shared model crate), ADR-0009
  (`application_id` distinct from `project_id`).
  **Not** ADR-0015 (Stage 108 `RuntimeKind` vocabulary) -- independent
  review v4 corrected an earlier version of this ADR that incorrectly
  reused `RuntimeKind` as D-Serv execution semantics; see "D-Graph:
  application-plane semantics" below for the full correction.
- Scope: types, validators and canonical digests only. No placement
  algorithm, no D-Deploy proposal/acceptance workflow, no operational
  projection activation, no D-Monitor. See "Non-goals" below.

## Why three separate canonical contracts

The DATUM reference model separates three concerns that this repository had,
until now, never given a typed, validated, compiled representation:

- **`datum.dgraph/1`** answers "what does the application look like" --
  D-Serv vertices and D-Call edges, placement-free.
- **`datum.dcontinuum/1`** answers "what infrastructure does DATUM know
  about" -- D-Node identity, platform, and declared execution capabilities,
  structural/declared, never live telemetry.
- **`datum.dmap/1`** answers "where does each D-Serv run" -- the single
  authoritative binding of the first two onto each other.

Keeping them as three separate types (rather than one combined "deployment"
object) is the same discipline ADR-0001/ADR-0002 already established in
prose; Stage 110 is where that discipline first becomes enforced Rust code.
Each answers a different question, changes for a different reason, and is
owned by a different part of the eventual D-Deploy lifecycle (D-Compile
produces a D-Graph; an operator/inventory process produces a D-Continuum;
D-Deploy acceptance produces a D-Map) -- collapsing them would make it
impossible to, for example, reuse one D-Graph across two installations
(ADR-0009) or compare two D-Maps of the same application against a stable
D-Continuum.

## Relationship to the Stage 107 hand-authored VS1 fixtures

**This is the one place this ADR must be explicit, because it is a real,
visible naming situation, not a hidden one.** `scripts/fixtures/datum-contracts/vs1/dgraph.*.json`,
`dcontinuum.*.json`, `dmap.*.json`, and `dforward.*.json` already exist,
already declare `"schema": "datum.dgraph/1"` / `"datum.dcontinuum/1"` /
`"datum.dmap/1"`, and are already validated by
`scripts/check_datum_contract_fixtures.py`. They are **not** wire-compatible
with the types this ADR introduces.

This is not a contradiction to paper over; it is exactly what those
fixtures' own documentation already says they are:
`documentation/reference/vs1-contract-fixtures.md` states plainly that they
are "hand-authored and structurally consistent; they are not produced by a
compiler," that "there is no `datum-model` crate yet," and that real
JCS+SHA-256 digests "arrive with the `datum-model` crate in Stage 108."
`documentation/reference/datum-canonical-model.md` likewise says "the typed
model, its validators and JSON Schemas generated from those types arrive
with the `datum-model` crate in **Stage 108**." None of that happened:
Stage 108 became the runtime-kind dispatch seam (ADR-0015), not a
`datum-model` crate, and ADR-0007 explicitly deferred any such crate
indefinitely ("Do NOT create a new shared crate," reaffirmed for this
stage). The VS1 fixtures are a hand-authored **specification sketch** for a
considerably larger future vision -- D-Compile source lineage
(`source.dscript_id`, `mapping_style`), typed ports with payload schemas and
QoS, D-Forward/D-IoT placement, D-Node ABI/host-kind negotiation, and an
explicit D-Deploy proposal-acceptance decision record embedded in the D-Map
itself (`decision.kind`, `proposal_ref`, `accepted_by`) -- none of which any
Rust code has ever read, written, or validated.

Stage 110's explicit brief is a **deliberately smaller v1**: the mandatory
mapping surface is exactly D-Serv -> D-Node (matching this repository's
actual, currently-implemented v0.1 execution model -- a single
`target_node` per service, nothing richer), no D-IoT/D-Forward machinery, no
ABI/host-kind NEGOTIATION (choosing among several possible ABIs or hosts),
and no D-Deploy proposal/acceptance workflow. (Independent review v4 added
a single, static ABI COMPATIBILITY check -- does a D-Serv's one required
ABI match a D-Node's declared capability set -- which is a rejection rule
over a caller-supplied mapping, never a negotiation or selection process;
see "D-Graph: application-plane semantics" below.)
Building the VS1 shape now would mean implementing exactly the Stage 111
D-Deploy v0 lifecycle and the D-Forward contract this ADR is explicitly
scoped not to touch.

**Resolution (revised, independent review v2 / P0 fix): directory placement
alone does not resolve a wire-schema-ID collision.** A serialized JSON
document carries its own `"schema"` field; it does not carry the directory
it was read from. The v1 draft of this ADR originally resolved this section
by directory separation alone (new fixtures under a new `canonical-v1/`
directory, VS1 fixtures left exactly as they were) -- an independent review
correctly identified that as insufficient: `dgraph.sensor-pipeline.json`
still self-declared `"schema": "datum.dgraph/1"`, identical to the string a
real `CanonicalDGraphV1` instance declares, for a structurally incompatible
shape. **The actual resolution implemented:**

- The Stage 107 VS1 sketch fixtures' `schema` field was migrated, file by
  file, to explicit, non-colliding identifiers:
  `datum.x-fixture.vs1-dgraph/1`, `datum.x-fixture.vs1-dcontinuum/1`,
  `datum.x-fixture.vs1-dmap/1`. This is a **label change only** -- every
  field, validator, and finding code in `scripts/check_datum_contract_fixtures.py`'s
  `v_dgraph`/`v_dcontinuum`/`v_dmap` is otherwise byte-identical; only the
  literal string each function compares `schema` against changed, in the
  same commit as the fixtures. `datum.dforward/1` was deliberately NOT
  touched -- it never collided with anything Stage 110 introduces.
- The new, real, compiled v1 types and their fixtures live under a
  **new, sibling directory**, `scripts/fixtures/datum-contracts/canonical-v1/`
  -- directory separation is now a CONVENIENCE (keeping the two families'
  files apart on disk), not the load-bearing part of the fix.
- A **new, separate** checker script,
  `scripts/check_canonical_datum_contracts_v1.py`, validates only
  `canonical-v1/` and additionally runs a `check_schema_id_uniqueness`
  pass over **all DATUM contract fixtures**: it walks
  `scripts/fixtures/datum-contracts/**/*.json` -- the entire fixture tree,
  whatever directories exist under it, not two hardcoded names -- and
  asserts by actual `schema` FIELD VALUE that a production ID (`datum.dgraph/1`,
  `datum.dcontinuum/1`, `datum.dmap/1`) is claimed ONLY by a file whose path
  is under `canonical-v1/`. (Independent review v2's first version of this
  check enumerated exactly two named directories, `canonical-v1/` and
  `vs1/`; independent review v3 found that a third fixture family added
  later under a different sibling directory would have gone completely
  unscanned, and widened the walk to the whole tree -- see PASS O below.)
  This is the regression guard: if a future edit reintroduces
  `"datum.dgraph/1"` into any non-canonical-v1 fixture (e.g. by copy-paste
  from an old version, in `vs1/` or any other family), this check fails
  immediately, regardless of which directory the offending file lives in.
  Proven to actually detect the regression, not merely currently pass by
  coincidence, by
  `SchemaIdUniqueness.test_a_third_fixture_family_reclaiming_a_production_schema_id_is_caught`
  in the checker's own selftest.
- The VS1 fixtures are **not deleted or reinterpreted** -- only their
  `schema` label moved. They remain exactly what their own documentation
  already said they were: a specification sketch for a future, richer
  D-Deploy v0 vision. Whether and how that vision is eventually reconciled
  with the Stage 110 v1 types (extending v1 in place, or introducing a
  `datum.dmap/2` for the richer shape) is an explicit **open question for
  Stage 111 and later**, not something this ADR resolves.
- `documentation/reference/datum-canonical-model.md`,
  `documentation/reference/vs1-contract-fixtures.md`, and
  `scripts/fixtures/datum-contracts/vs1/README.md` are updated to record
  the migration and stop claiming Stage 108 delivers the typed model -- see
  "Documentation updates" below.

**Audit confirming no remaining collision (all DATUM contract fixtures):**
`check_schema_id_uniqueness` (run as part of
`python scripts/check_canonical_datum_contracts_v1.py`) reports, against
the real repository state: the three production IDs found in exactly 23
fixtures, all under `canonical-v1/` (3 positive + 20 of the 23 negative
fixtures whose deliberate violation is not the schema field itself; the
other 3 negative fixtures deliberately declare an invalid schema and are
excluded from this count by design), and **zero** occurrences anywhere else
in the fixture tree (`vs1/` included, plus any other family that might be
added later).

## D-Graph: application-plane semantics, cycles allowed

**Independent review v4 correction: D-Serv execution semantics are
host-independent, never Stage 108's `RuntimeKind`.** An earlier version of
this ADR (and the Stage 110 v1/v2/v3 code it described) reused
`execution_requirement.runtime_kind` (Stage 108/ADR-0015 vocabulary:
`container`/`native_process`) as the canonical D-Serv execution
requirement. That was wrong: ADR-0004 already states, and always stated,
"the D-Serv execution contract is host-independent" and lists
`datum.dgraph/1` explicitly among the contracts a host-kind field must
never appear in; ADR-0015 already states, and always stated, that its
`RuntimeKind` vocabulary is scoped to "the existing operational
service-artifact reconciliation/dispatch path" and that "D-Map, D-Graph
typed contracts... are untouched" by its introduction. The Stage 110 v1
implementation violated both of those already-Accepted ADRs by importing
`crate::core::runtime_kind::RuntimeKind` into the new canonical D-Graph/
D-Continuum/D-Map modules. Independent review v4 removed that import
entirely and replaced the field with the host-independent D-Node ABI
(ADR-0003) instead -- see the ABI paragraph below. This correction does
not modify ADR-0004 or ADR-0015; it brings this ADR's Stage 110 code back
into compliance with what those two ADRs already required.

`CanonicalDGraphV1` (`dserver::core::canonical_dgraph`,
`DATUM::agent::canonical_dgraph`, independently mirrored per ADR-0007) is
`G = (V, E)`: `V` = `CanonicalDServV1` (`service_id` +
`execution_requirement.dnode_abi`), `E` = `CanonicalDCallV1`
(`call_id`, `source_service_id`, `target_service_id`). Deliberately minimal:
no D-Compile source lineage, no typed ports/payload schemas, no QoS, no
`project_id` (ADR-0009: "the D-Graph itself never references a project"),
no artifact/content-hash reference (that belongs to a future D-Code/artifact
contract or the operational layer, never to the application-plane graph).

**D-Call cycles are explicitly accepted (D-4, ADR-0001).** `validate()`
never runs a cycle-detection algorithm at all -- there is no code path that
could reject `A -> B -> A`, and a degenerate self-call `A -> A` is equally
legal. Both are covered by regression tests
(`legal_two_node_cycle_is_accepted`, `self_call_is_accepted`, mirrored in
both crates and independently in the Python checker's
`DGraphCycleAcceptance` test class). The only thing ever rejected about a
D-Call is a **dangling** endpoint -- `source_service_id`/`target_service_id`
naming no declared `service_id`. This is a structurally different check
from cycle detection and is implemented as one: `known_services.contains(...)`,
never a graph traversal. The deployment-dependency graph derived for
installation ordering (a completely different structure, "Wave" in ADR-0001
terms) is out of Stage 110's scope entirely; nothing in this file computes
it.

`execution_requirement.dnode_abi` is validated by direct comparison against
`DNODE_ABI_V0` ("datum-dnode/0", ADR-0003) -- the ONLY accepted D-Node ABI
for v0.1, host-independent by construction: no field named `runtime_kind`,
`host_kind`, `"container"`, or `"native_process"` appears anywhere in this
module's D-Serv execution semantics, and no import of
`crate::core::runtime_kind` exists in this file at all (confirmed by a
repository-wide regression grep, see "Regression audit" below).

## D-Continuum: structural inventory, not telemetry

`CanonicalDContinuumV1` carries `project_id` (ADR-0001: "an implementation
scoping choice, not part of the canonical definition" -- but REQUIRED at
this v1 wire-contract level, unlike the Stage 107 fixtures' optional
treatment; Stage 110's identity list makes it mandatory), `dcontinuum_id`,
`revision`, a typed `CanonicalDNodeV1` list, and an optional static
`CanonicalDLinkV1` topology list.

Each node declares `platform` (`os`, `architecture`) and
`capabilities.dnode_abi_versions` -- the SET of host-independent D-Node ABI
versions (ADR-0003) that node supports. This is capability-driven by
design: Stage 112 placement can determine whether a node can host a given
D-Serv's D-Code purely by checking set membership on a typed `Vec<String>`
validated against `DNODE_ABI_V0`, never by inspecting an arbitrary
node-class string, a bespoke taxonomy enum, or (independent review v4's
correction) Stage 108's `RuntimeKind`. The three real fixture nodes
(`cloud-01`, `fog-01`: both support `datum-dnode/0`;
`mist-arm-gateway-01`: an empty capability set, modelling a restricted/
lightweight element that cannot host any D-Code under the current
single-ABI v0.1 model) demonstrate v0.1 heterogeneity via ABI capability
AND static resource capacity (see below) -- never via a host-executor-kind
partition, which is exactly the leak independent review v4 removed.

**Why an empty capability set, not a second ABI value, models
"restricted":** v0.1 defines exactly one accepted D-Node ABI
(`datum-dnode/0`). A node either supports it (can host any D-Code) or does
not (`dnode_abi_versions: []`); there is no intermediate "restricted ABI"
concept to invent, and none was invented. This is the direct replacement
for the old, incorrect model where "restricted" meant "native_process-only,
not container-capable" -- a HOST-EXECUTOR distinction that never belonged
in this application-plane capability set to begin with.

**Platform readiness (audited, independent review v3): `platform.os`/
`platform.architecture` are declared but deliberately NOT yet consumed by
D-Map validation.** The existing Stage 107B-A OCI image pinning path
(`dserver::core::service_artifact_registry`/`oci_registry_resolver`) already
resolves a D-Serv's container artifact to a MULTI-platform manifest set
(`ResolvedImageIdentityV1.platforms`), with the platform-specific manifest
selected at binding/execution time against the actual host -- never a
single static platform requirement declared at the D-Graph/D-Map layer.
Stage 109's native-process path is symmetric: `platform_supported()` is
checked at execution-attempt time on the agent
(`DATUM::agent::native_process_runtime`), not as a structural declaration
this stage's types would validate. Neither existing v0.1 path commits to a
stable, single per-D-Serv platform requirement that a Stage 110 contract
would need to represent and Stage 112 would need to consume; that
compatibility resolution is Stage 111 artifact-binding territory, using
information (the resolved multi-platform manifest set, the actual node
platform at bind time) this stage's frozen contracts do not carry and are
not required to. No platform-compatibility field or check was added to
`CanonicalDMapV1`; no code or documentation claims `platform.os`/
`platform.architecture` currently affects `CanonicalDMapV1` validation --
`platform` remains part of a D-Node's declared identity, unused by D-Map's
validation rules in this v0.1 slice. If a future stage commits container
and native-process artifacts to a single fixed target platform ahead of
binding, this is the explicit, evidence-recorded point where that decision
would be added -- not something this ADR resolves now.

**No live telemetry field exists on this type, structurally, not just by
convention.** There is no CPU/RSS/swap/RTT/packet-loss/temperature/
restart-count/health field anywhere in `CanonicalDContinuumV1` or any type
it contains -- confirmed by a dedicated regression test in both crates
(`no_telemetry_field_exists_on_the_type`) that serializes a valid instance
and asserts none of those key names appear anywhere in the resulting JSON.
A `CanonicalDContinuumV1` revision changes only when the DECLARED inventory
itself changes (a node added/removed, a capability declaration changed).
Stage 113 D-Monitor is expected to attach OBSERVATIONS elsewhere (its own
schema, keyed by `node_id`, never mutating this structural identity model);
nothing in this stage builds that attachment mechanism, a collection loop,
or a history store -- there is exactly one place telemetry could leak into
this type (a field on `CanonicalDNodeV1` or `CanonicalDContinuumV1` itself),
and the regression test above proves that place is empty.

Static topology only: `CanonicalDLinkV1` is `link_id` + `from_node_id` +
`to_node_id`, nothing else -- no live RTT/loss/bandwidth sample, ever.

## D-Serv/D-Node static resource contract (Stage 110 P0 fix, independent review v3)

Independent review v2 concluded (`stage112-readiness-audit.txt`, v2) that
Stage 112 v0.1 was capability-only scope and added no resource fields --
independent review v3 rejected that conclusion: the "do not implement
placement" boundary was a STAGE 110 scoping instruction (do not build
Stage 112's algorithm during Stage 110), not a redefinition of what Stage
112's algorithm is allowed to consume once it exists. The accepted v0.1
architecture target is capability/RESOURCE placement, and resource
FEASIBILITY (does a candidate mapping fit, structurally) is not resource
OPTIMIZATION (which node is best) -- Stage 110 can and must supply the
former's static inputs without building the latter.

Both `CanonicalDServV1` and `CanonicalDNodeV1` therefore now carry the
smallest possible static resource surface:

- `CanonicalDServV1.resource_requirement: CanonicalDServResourceRequirementV1
  { cpu_cores: u32, memory_bytes: u64 }` -- the DESIRED minimum this D-Serv
  needs, declared application/deployment semantics exactly like
  `execution_requirement`, required (not optional, not defaulted).
- `CanonicalDNodeV1.capacity: CanonicalNodeCapacityV1 { cpu_cores: u32,
  memory_bytes: u64 }` -- the DECLARED/STRUCTURAL static capacity of that
  node, required, exactly mirroring `resource_requirement`'s units so a
  feasibility comparison never needs unit conversion.

Both are exact integer quantities only. Neither type, nor any type either
one contains, has or will ever have a field for CPU utilization, RSS, used/
available/current-free memory, load average, swap usage, temperature, disk
usage, RTT, or packet loss -- those remain Stage 113 D-Monitor observations,
attached elsewhere, never mixed into this structural/declared model.
`validate()` rejects `cpu_cores < 1` or `memory_bytes < 1` on either type
(`invalid_cpu_requirement`/`invalid_memory_requirement` for a D-Serv,
`invalid_node_cpu_capacity`/`invalid_node_memory_capacity` for a D-Node) --
a zero-valued field is treated as malformed input, never silently accepted
as "no requirement"/"unlimited capacity." A `CanonicalDContinuumV1`
revision changes when a node's declared capacity changes, exactly like it
does for a capability-set change; a `CanonicalDGraphV1` revision changes
when a service's resource requirement changes, exactly like it does for a
runtime-kind change -- both digest functions already hash the whole struct,
so no digest-function code changed, only the struct shape did.

## D-Map: placement CONTRACT, not a placement algorithm

`CanonicalDMapV1` binds `project_id`, `application_id`, `dmap_id`,
`revision`, an exact `dgraph_ref`/`dcontinuum_ref` (see "Reference binding"
below), and a `placements: Vec<CanonicalDServPlacementV1>` list (`service_id`
+ `node_id`).

**Cardinality (Stage 110/v0.1): exactly one placement per D-Serv.** This
mirrors the actual, currently-implemented execution model exactly --
`OperationalArtifactAssignment.target_node` (`DATUM/src/agent/operational_reconciliation.rs`)
is a single `String`, never a set, and Stage 105-109's real reconciliation
code has never had a multi-target or replica concept. `validate()` rejects
a `service_id` appearing more than once (`duplicate_placement`);
`validate_against()` rejects any D-Graph service that has NO placement at
all (`missing_placement`) -- both directions of the single-cardinality
invariant are enforced, never merely assumed. No replication, quorum, or
load-balancing semantics were invented; a future stage that needs a
multi-target model gets a new, explicit decision, not a silent
reinterpretation of this one.

**Two-tier validation, matching the brief's explicit split between
self-contained and cross-contract checks:**

- `validate(&self)`: schema, identity, revision, and placement cardinality
  -- never touches another contract.
- `validate_against(&self, dgraph, dcontinuum)`: reference identity/digest
  matching, `application_id`/`project_id` cross-checks, unknown
  service/node, and runtime-capability compatibility. Requires the caller to
  have already resolved the referenced `CanonicalDGraphV1`/
  `CanonicalDContinuumV1` objects (by `(id, revision)` natural key, exactly
  like the Stage 107 checker's own `idx` lookup pattern) -- `validate_against`
  itself never fetches, guesses, or falls back to "the only D-Graph I could
  find."

**This is CONTRACT validation, never a placement algorithm.** Every check
in both functions is a REJECTION rule over an already-fully-formed
`CanonicalDMapV1` the caller supplies. There is no scoring function, no
"pick the best node," no fallback-if-unplaced logic, no loop that tries
candidate nodes, and no mutation of `self` or any argument anywhere in
either function -- both take `&self` (and `&CanonicalDGraphV1`/
`&CanonicalDContinuumV1`) and return only a `Vec<CanonicalValidationFinding>`.
Stage 112 (placement) and Stage 111 (the D-Deploy proposal/acceptance
workflow that would call `validate_against` on a candidate before accepting
it) are the stages that ever construct a `CanonicalDMapV1` from scratch;
Stage 110 supplies only the type and the rejection rules.

**Resource feasibility (Stage 110 P0 fix, independent review v3):**
`validate_cross_references` (and therefore the complete `validate_against`)
additionally rejects a mapping that is STATICALLY impossible given the
declared resource contract above -- still a rejection rule, never a
scheduler:

- Individual fit: a placed D-Serv's `resource_requirement` must not exceed
  its target D-Node's `capacity`, checked independently for CPU
  (`insufficient_node_cpu`) and memory (`insufficient_node_memory`).
- Aggregate fit: when multiple D-Servs are placed on the SAME node, their
  COMBINED requirement must not exceed that node's capacity, even when each
  individually fits (`aggregate_node_cpu_exceeded`/
  `aggregate_node_memory_exceeded`). The aggregation uses checked integer
  arithmetic; an overflow is treated as exceeding capacity (an overflowed
  sum can never represent a satisfiable real request) rather than silently
  saturating to a misleadingly small total or panicking.

Both checks only ever ADD a rejection finding; neither reassigns a
placement, tries an alternate node, or mutates `self`/any argument -- proven
by `resource_feasibility_rejection_never_mutates_the_dmap` in both crates.

## Reference binding: `id` + `revision` + `digest`

`CanonicalContractReferenceV1 { id, revision, digest }` is the shape the
brief suggested and this ADR adopts verbatim. `validate_against` checks
`id`/`revision` for an exact natural-key match FIRST; only when that passes
does it recompute the referenced contract's own canonical digest and
compare it to the recorded `digest` field. This two-step order matters: a
stale-revision case (`dgraph_reference_mismatch`) and a
correct-revision-but-tampered-content case (`dgraph_digest_mismatch`) are
DISTINCT, separately tested finding codes, never collapsed into one
generic "reference invalid" -- a caller (or, later, an authorization layer)
needs to be able to tell "this D-Map is stale" apart from "this D-Map's
D-Graph reference claims a revision that no longer hashes to what it did."
Regression tests for both (`stale_revision_with_correct_id_rejected`,
`correct_revision_with_wrong_digest_rejected`) exist in both crates.

No self-reference: `CanonicalDMapV1` has no digest-of-itself field, and
`recompute_dmap_digest` never reads `dgraph_ref.digest`/`dcontinuum_ref.digest`
as anything other than plain string fields it hashes exactly once, the same
as any other field -- there is no digest chain, no recursive hashing, no
field that depends on its own hash.

**Reference digest SHAPE is structurally validated (Stage 110 P1 fix,
independent review v3).** Independent review v2's `validate()` checked
`dgraph_ref.id`/`revision` and `dcontinuum_ref.id`/`revision` for presence,
but never the SHAPE of `dgraph_ref.digest`/`dcontinuum_ref.digest`
themselves -- an empty string, a bare 64-hex digest missing the `sha256:`
prefix, uppercase hex, or a wrong-length digest all passed `validate()`
cleanly (the mismatch, if any, would only ever surface later, from
`validate_cross_references`, and only if the malformed value happened not
to equal the recomputed digest by sheer coincidence). `validate()` now
additionally requires both digest fields to satisfy
`jcs::is_canonical_digest_format` -- the exact ADR-0006 `sha256:<64
lowercase hex>` form, the same helper `dserver::core::jcs` already exposed
and already used elsewhere in this codebase -- emitting `invalid_digest_format`
on failure. This is a STRUCTURAL check on `CanonicalDMapV1` alone,
independent of whether the digest happens to match anything; content
equality remains `validate_cross_references`'s job and is unweakened by
this fix (`well_formed_but_wrong_digest_still_triggers_mismatch_not_only_format_error`
proves a well-formed-but-wrong digest still produces `dgraph_digest_mismatch`,
never merely `invalid_digest_format`).

## Canonicalization and digest semantics

Digests reuse the existing, already-conformance-tested JCS+SHA-256 module in
each crate (`dserver::core::jcs`, `DATUM::agent::jcs` -- ADR-0006) via the
established `canonical_digest_sha256_hex(&serde_json::to_value(...))`
pattern already used by `execution_projection::recompute_native_digest`. No
new canonicalization implementation was written.

**The array-ordering decision, stated explicitly (the brief required this
not be left ambiguous):** `services`, `calls`, `nodes`, `links`, and
`placements` are canonically **unordered sets**, each keyed by its natural
id field (`service_id`, `call_id`, `node_id`, `link_id`, `service_id`
respectively); a node's `capabilities.dnode_abi_versions` is likewise an
unordered set of declared capabilities. Before computing a digest, each
`recompute_*_digest` function clones the input and sorts every such
collection by its key, THEN serializes and hashes. The wire JSON itself is
**never** reordered by these functions -- serialization/round-tripping
elsewhere preserves whatever order a producer supplied; only the bytes fed
into the digest are normalized. Consequence: two semantically identical
contracts that merely list their elements in a different order hash
identically (`digest_is_order_independent_for_*` tests, both crates, plus
the Python checker's `digest_of_*` functions performing the identical
sort-then-hash and independently agreeing with both Rust sides on the real
fixtures). Duplicate-ID inputs are still caught by `validate()`/`validate_against()`
independent of digest computation -- order-independence for hashing and
duplicate rejection for validation are deliberately separate concerns, never
conflated.

## Cross-crate proof

Unlike the Stage 107A execution-projection mapper fixtures (which need
`dserver_X`/`agent_X` dual sections because the two crates' SOURCE types
genuinely differ in shape), the canonical-v1 wire shape is IDENTICAL by
construction on both sides -- there is no pre-existing divergent
representation to reconcile. This makes the cross-crate proof simpler: one
shared JSON fixture per schema
(`scripts/fixtures/datum-contracts/canonical-v1/{dgraph,dcontinuum,dmap}.smart-sentinel-lab.json`),
each carrying a real, JCS+SHA-256-computed `expected_digest_sha256` (never
the Stage 107 placeholder digest -- this is a REAL cross-crate mapper
fixture in the same sense as the Stage 107A/108 mapper fixtures already
are). Both crates independently deserialize the SAME file into their own
`CanonicalDGraphV1`/`CanonicalDContinuumV1`/`CanonicalDMapV1`, assert
`validate()`/`validate_against()` return no findings, recompute the digest,
and assert it equals the recorded value --
`dserver_cross_crate_fixture_produces_the_expected_digest[_and_validates]`
in `DATUM`'s test modules, `agent_cross_crate_fixture_produces_the_expected_digest[_and_validates]`
in `dserver`'s. A THIRD, fully independent recomputation (Python
stdlib `hashlib`/`json.dumps(sort_keys=True)`, sharing no code with either
Rust JCS module) is performed by `scripts/check_canonical_datum_contracts_v1.py`
against the same three fixtures and agrees with both. All three
implementations converging on the same 64 fixed hex digits is the actual
cross-crate (and cross-language) proof, not a compile-time type-compatibility
claim.

The fixture content itself is realistic, not `foo`/`bar`: `chirpstack-gateway-bridge`,
`smart-adapter`, and `atom-controller` (naming the Magistrala Atom
component) are real service names from this project's actual experimental
environment (see CLAUDE.md's "Experimental environment" section). Each
D-Serv's canonical `execution_requirement.dnode_abi` is `datum-dnode/0` --
the same host-independent ABI for all three, per the D-Serv host-invariance
correction above; their real, DIFFERENT host implementations (one runs as
a container, one as a native process, today) are operational-plane facts
recorded elsewhere (`ServiceArtifact.runtime.kind`, ADR-0015), never in
this canonical application-plane contract. The restricted
`mist-arm-gateway-01` node (empty `dnode_abi_versions`) is deliberately
left UNUSED by the positive D-Map fixture's placements, and a dedicated
negative fixture (`dmap.incompatible-dnode-abi.json`) places a D-Serv onto
it specifically to prove ABI-capability-driven placement validation is
exercised, not merely declared.

## Identity: `project_id` vs `application_id` vs every `*_id`

Per ADR-0009, preserved exactly:

- `application_id`: application-plane identity, carried by `CanonicalDGraphV1`
  and `CanonicalDMapV1` (which must match its referenced D-Graph's
  `application_id` -- `application_mismatch` finding). NEVER carried by
  `CanonicalDContinuumV1` (a continuum is not application-scoped).
- `project_id`: installation/tenant/deployment-persistence scope, carried by
  `CanonicalDContinuumV1` and `CanonicalDMapV1` (which must match its
  referenced D-Continuum's `project_id` -- `project_mismatch` finding).
  NEVER carried by `CanonicalDGraphV1` (ADR-0009: "The D-Graph itself never
  references a project").
- `dgraph_id` / `dcontinuum_id` / `dmap_id`: each contract's own identity,
  paired with its own `revision` -- never used interchangeably with
  `application_id`/`project_id`, and never used as a placement target
  (`service_id`/`node_id` are the only identifiers ever placed against each
  other).
- `service_id` / `node_id`: the application-plane and continuum-plane
  element identities D-Map actually binds. Distinct from `call_id`/`link_id`
  (edge identities, never placement targets) and from `dserv_id`-shaped
  identifiers the older VS1 fixtures use (this v1's field is spelled
  `service_id`, matching this stage's own naming, not the VS1 sketch's).

No test constructs two of these fields with the same value and expects them
to be treated as aliases; the mismatch tests (`application_mismatch`,
`project_mismatch`, `dmap_for_another_application_graph_is_rejected`,
`graph_and_continuum_from_another_project_are_rejected`) exist specifically
to prove no accidental aliasing occurs.

## Versioning

Exact schema identifiers, exposed as `const` (never a repeated string
literal in validation code): `CANONICAL_DGRAPH_SCHEMA = "datum.dgraph/1"`,
`CANONICAL_DCONTINUUM_SCHEMA = "datum.dcontinuum/1"`,
`CANONICAL_DMAP_SCHEMA = "datum.dmap/1"`, defined once per crate and
referenced everywhere `schema` is compared or constructed. A future breaking
change to any one contract gets a new schema string (`/2`) and, per this
stage's own precedent, a new type name (`CanonicalDGraphV2` or similar) --
never a silent reinterpretation of `/1`'s existing field shape.

## Distinction from `OperationalDGraph` / the legacy composite `DGraph`

Restated precisely (ADR-0001's table, now backed by real, disjoint Rust
types rather than documentation labels alone):

| Rust type | Canonical label | What it is |
|---|---|---|
| `CanonicalDGraphV1` (NEW, Stage 110) | canonical application D-Graph | `G = (V, E)`, `V` = D-Serv, `E` = D-Call. Placement-free, cycles allowed. |
| `OperationalDGraph` (`dserver/src/core/operational_dgraph.rs`) | operational deployment document v1 | A per-node list of desired container services. No edges, no D-Serv/D-Call semantics, unchanged by this stage. |
| `DGraph` (`dserver/src/core/dgraph.rs`) | controlled composite graph | The closed Controlled WASM Deployment phase (C59-I44) structure: application + infrastructure + placement + health. Frozen, unchanged by this stage. |
| observed D-Graph (snapshot-materialized, `model.rs`/`topology.rs`) | observed runtime graph | Evidence-enriched interpretation of runtime state. Unchanged by this stage. |

Stage 110 touches none of `operational_dgraph.rs`, `dgraph.rs`, `model.rs`,
or `topology.rs` -- verified by `git diff` against the Stage 109 baseline
showing zero lines changed in any of those four files (see the review
package's `legacy-graph-collision-audit.txt`). `CanonicalDGraphV1` does not
import from, wrap, or get constructed from any of them anywhere in this
stage's source.

**`OperationalDGraph` is the future consumer, not today's:** once Stage 111
implements the D-Deploy proposal-acceptance workflow, an accepted
`CanonicalDMapV1` is expected to eventually drive what
`OperationalDGraph`/the per-node operational slice materializes for a given
node -- but that projection does not exist yet, is explicitly out of scope
here, and nothing in Stage 110 wires the two together.

## Documentation updates

`documentation/reference/datum-canonical-model.md` is updated minimally to
correct its stale "Stage 108" claims (the typed model did not, in fact,
arrive with a `datum-model` crate in Stage 108) and to name where the real
v1 types now live, without rewriting the rest of the page or its `v2
contract set` table (which still correctly describes the eventual, fuller
shape those schema ids may grow toward).

## Non-goals (explicit)

Not implemented in this stage, deliberately:

- **Stage 111**: `target_nodes` runtime behavior changes, D-Map ->
  operational-projection activation, accepted-deployment state machines,
  D-Deploy proposal endpoints, placement-proposal acceptance, atomic
  running-deployment transitions, host mutation from a D-Map.
- **Stage 112**: any placement algorithm -- bin packing, capability
  scoring, latency scoring, resource OPTIMIZATION, random placement,
  first-fit, a policy engine, GNN, RL, or LLM-driven placement.
  `validate_against` REJECTS an invalid mapping the caller already
  constructed; it never constructs, scores, or selects one. (Independent
  review v3: resource-FEASIBILITY rejection -- does a caller-supplied
  mapping fit the declared static capacity -- is a Stage 110 goal, added
  above; it is not resource optimization, which remains this non-goal.)
- **Stage 113**: D-Monitor -- no periodic collection loop, no process
  CPU/RSS sampling, no live network telemetry, no history database. The
  D-Continuum contract is structured (see above) so Stage 113 can later
  attach observations without mutating this structural identity model, but
  no such attachment mechanism exists yet.
- D-IoT/D-Forward placement machinery, D-Node ABI/host-kind negotiation, and
  the D-Deploy proposal/acceptance decision record -- all present in the
  Stage 107 VS1 sketch, none implemented here (see "Relationship to the
  Stage 107 hand-authored VS1 fixtures" above).
- A shared `datum-model` crate (ADR-0007, reaffirmed).
- Any change to Stage 108/109 (native process runtime) behavior,
  authorization, `RuntimeKind` enum/parser, or `ServiceArtifact
  runtime.kind` dispatch -- all remain exactly as ADR-0015 defined them,
  untouched by this stage. (Independent review v4 correction: an earlier
  version of this ADR had the canonical D-Graph/D-Continuum modules IMPORT
  `RuntimeKind::parse` -- that import has been removed entirely; see
  "D-Graph: application-plane semantics" above. `RuntimeKind` remains a
  real, unmodified, still-used type -- just never inside these three
  canonical contract modules.)

## Validation

Both crates' full test suites are green (canonical-contract-filtered test
counts have grown across three independent review rounds as the P0/P1 fixes
above were added -- see the review package's `test-results.txt` for the
EXACT totals from every command run in the specific package being reviewed;
this ADR deliberately does not restate a specific number here, since a
hardcoded historical count in normative architectural prose goes stale on
every subsequent fix and adds no value once test-results.txt exists as the
authoritative record). `scripts/check_canonical_datum_contracts_v1.py` and
its selftest, and `scripts/check_datum_contract_fixtures.py` and its
selftest, are likewise green -- again, see `test-results.txt` for exact
totals. `mkdocs build --strict`: exit 0.
