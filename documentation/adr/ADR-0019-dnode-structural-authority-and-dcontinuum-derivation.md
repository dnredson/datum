# ADR-0019 — D-Node structural authority and authoritative D-Continuum derivation

- Status: **Accepted and implemented, v2 (independent review fix).**
- Decision refs: ADR-0002 (single placement authority), ADR-0006 (canonical
  digest), ADR-0013 (OCI catalog image identity resolution), ADR-0017
  (canonical `datum.dcontinuum/1`), ADR-0018 (D-Deploy proposal/acceptance).
- Scope: closes the one open integration gap the conceptual roadmap audit
  (`DATUM_ROADMAP_AUDIT_107A_125_2026-09-14.md`) found in conceptual phase
  109 (canonical D-Deploy) and substantially advances conceptual phase 110
  (D-Node as a canonical execution entity). It does **not** implement
  deterministic placement (conceptual 120), D-Code/WASM execution (111),
  D-Call (112), or any AI/GNN/RL policy (121/123) — none of those are
  touched here.

## Context

Stage 111 (ADR-0018) made D-Deploy's proposal/acceptance/projection
lifecycle internally governed and relationally sound across four
independent-review rounds. But `create_proposal`/`validate_full`
(`core::ddeploy`) accept a caller-supplied `CanonicalDContinuumV1` as a plain
parameter, validated only for internal shape consistency
(`CanonicalDContinuumV1::validate()`), never cross-checked against any
server-owned record of what D-Nodes genuinely exist. A caller could
therefore make a proposal "authoritative" against a fictional node, or
against a real `node_id` with fabricated `platform`/`capacity`/
`dnode_abi_versions` — the exact node-level fields
`CanonicalDMapV1::validate_against`'s resource-feasibility check relies on.

Separately, the roadmap audit found conceptual phase 110 (D-Node as a
canonical execution entity, distinct from a physical host, from
SmartSentinel/D-Agent, and from `ServiceArtifact`) SCAFFOLDED: `node_id` is
used pervasively as a bare string identifier, but nothing in the repository
owned a structural, persisted, canonical `CanonicalDNodeV1` record
independent of Sentinel lease state.

## Decision

### 1. D-Node structural declaration is a NEW, GLOBAL, server-owned registry — completely independent of Sentinel lease/liveness

`core::dnode_registry::DNodeRegistryStateV1` persists a `BTreeMap<node_id,
CanonicalDNodeV1>` (reusing ADR-0017's own `CanonicalDNodeV1` type verbatim
— no new wire schema was introduced) via a new control-state domain,
`DOMAIN_DNODE_REGISTRY` (`storage::control_state`), using the identical
`load_domain_json`/`persist_domain_json` pattern ADR-0018's `ddeploy_state`
domain already established.

**GLOBAL, not project-partitioned.** A structural D-Node declaration is
keyed only by `node_id`, exactly like Sentinel registration
(`SentinelRegistrationRecord`) is keyed only by `node_id` with no
`project_id` at all. Nothing else in this repository models "which project
owns which physical node," and `CanonicalDContinuumV1.project_id` is already
documented (ADR-0001) as a wire-identity SCOPING LABEL, not a claim that
infrastructure is partitioned by project. `build_canonical_dcontinuum`
therefore stamps the requested `project_id` onto the SAME global node set
for every project, rather than inventing a new node-to-project ownership
model this pass found no repository evidence for.

**Why not extend `SentinelRegistrationRequest`?** Audited first
(`core/sentinel_registration.rs`): `SentinelRegistrationRecord` carries
`instance_id`/`lease_seconds`/free-form `capabilities: Vec<String>` tags and
is not even persisted today (in-memory `OnceLock<RwLock<..>>` only, cleared
on restart) — it has no `platform`/`capacity` fields, and its `capabilities`
are unstructured tags (`"wasmtime"`, `"artifact_fetch"`), not the typed
`dnode_abi_versions` set `CanonicalDNodeV1` requires. Extending it would mix
STRUCTURAL declaration into OPERATIONAL/LIVE lease state — the exact
conflation ADR-0017's `CanonicalDContinuumV1` module doc already warns
against. A separate registry is the smaller, cleaner change (this pass's own
explicit "may be introduced" option) and does not foreclose a future
integration: **a Sentinel MAY declare a D-Node descriptor** — nothing prevents an
agent's own bootstrap flow from calling `declare_dnode` for the node it runs
on — but nothing here REQUIRES a 1:1 relationship. There is no `instance_id`
field anywhere in `DNodeRegistryStateV1`, and `node_id` is never derived
from or checked against any `SentinelRegistrationRecord`. A future design
remains free to let one agent instance declare/manage multiple D-Nodes, or
several instances cooperatively declare the same D-Node.

**Why not reuse `core::resource_registry`?** Also audited first: that
subsystem (`ResourceRegistryRecord`) solves hardware/OS *identity binding*
(is this the same physical machine across reboots?), not structural
placement capacity. Its `resources` field is an opaque, content-addressed
`capacity_profile_id` digest — the actual numeric `cpu_cores`/`memory_bytes`
behind that digest are never resolved anywhere in this repository. Reusing
it here would have required either inventing numbers behind that digest
(explicitly forbidden by this pass's own instructions) or building a new
capacity-profile catalog — out of scope for this pass.

**Idempotent-but-conflict-detecting declaration.** `declare_dnode` mirrors
`api::sentinel_registration::put_sentinel_registration`'s own precedent
(refuses to silently reassign a registered node's `instance_id`): declaring
an already-known `node_id` with IDENTICAL content (`dnode_abi_versions`
compared as a normalized, order-independent set, per ADR-0017) is an
idempotent no-op; declaring it with DIFFERENT content fails closed with
`dnode_structural_conflict`. There is no in-place structural update in this
v0.1 — `remove_dnode` then redeclare is the correct workflow to change a
node's declared facts. This is a deliberate, documented minimal-scope
choice, not an oversight.

### 2. Authoritative `datum.dcontinuum/1` derivation

`core::dnode_registry::build_canonical_dcontinuum(state, project_id)`
constructs a fresh `CanonicalDContinuumV1` from the registry's current node
set for every call — deterministic, stably ordered regardless of
declaration order (proven by
`authoritative_dcontinuum_is_deterministic_and_stably_ordered_regardless_of_registration_order`),
never trusting a caller-supplied digest, never including any live telemetry
field (proven by `authoritative_dcontinuum_contains_no_live_telemetry_field`,
mirroring `canonical_dcontinuum::tests::no_telemetry_field_exists_on_the_type`).
An empty registry fails closed with `authoritative_dcontinuum_unavailable`
rather than silently returning an empty-but-"valid" continuum. `links` is
always empty in this v1 — declarative topology-link declarations are
explicitly out of scope for this pass (a documented limitation, not a
silent omission); links are not consumed by `CanonicalDMapV1` feasibility
validation today, so this defers no safety property.

### 3. D-Deploy authority transition — layered ABOVE `core::ddeploy`, which is untouched

Per this pass's explicit instruction not to redesign Stage 111's proposal/
acceptance semantics, **`core::ddeploy::create_proposal`/`validate_full`
were not modified at all** (their 110 existing tests are unchanged and still
pass verbatim). The fix is layered in the API handlers
(`api::ddeploy::post_validate_proposal`/`post_create_proposal`), mirroring
the exact precedent `artifact_reconciliation_context::PlacementAuthority`
already established for the `target_nodes` authority transition (core logic
untouched, authority-awareness added as a wrapper):

```
authoritative_dcontinuum_for_project(project_id, claimed) -> Result<CanonicalDContinuumV1, ApiError>
```

1. Builds the current authoritative continuum via `build_canonical_dcontinuum`.
2. Runs `check_dcontinuum_authority(claimed, authoritative)` — SUBSET
   semantics: `claimed` need not enumerate every registered node, only the
   ones it actually references (its placement targets), but every node it
   DOES reference must exist in the registry with EXACTLY matching
   `platform`/`capacity` (else `dnode_structural_conflict`) and
   `dnode_abi_versions` (order-independent set compare; else
   `dnode_capability_invalid`); a referenced node absent from the registry
   fails `dcontinuum_node_unregistered`.
3. On success, returns the AUTHORITATIVE continuum — which REPLACES the
   caller's `dcontinuum` for every downstream use (digest computation,
   `dcontinuum_ref`, `validate_full`'s dcontinuum parameter, and
   `CreateProposalInput.dcontinuum`). A proposal's persisted
   `dcontinuum_snapshot` is therefore always server-derived, never a
   verbatim client payload, even when the client's claim happened to be
   internally consistent.

The caller-supplied `dcontinuum` field is kept on the wire (no v1 API
breakage) but is now advisory input to the authority check only, never
trusted content. `post_validate_proposal` additionally retains its existing
`payload.dcontinuum.validate()` shape check for early, informative feedback
on a malformed submission before the authority check runs.

**Stage 111's acceptance transaction (CAS, idempotency, digest recheck,
persistence atomicity, accepted-history validation, placement authority) is
unchanged in its original v1 form** — the v1 revision of this ADR affected
only what `dcontinuum_snapshot` a NEW proposal is created with.
**Superseded by "Independent review fix v2" below (R1):** acceptance itself
now ALSO revalidates a PENDING proposal's persisted `dcontinuum_snapshot`
against the current authoritative D-Continuum before committing — see that
section for the full rationale; every OTHER acceptance property listed above
(CAS, idempotency, digest recheck, persistence atomicity, accepted-history
validation, placement authority) remains exactly as Stage 111 defined it.

New stable finding codes (mirroring Stage 111's own vocabulary):
`authoritative_dcontinuum_unavailable`, `dcontinuum_node_unregistered`,
`dnode_structural_conflict`, `dnode_capability_invalid`.

**Operational consequence, stated plainly:** every project must have at
least one structural D-Node declared (`PUT /api/v1/dnodes/:node_id`) before
`POST /api/v1/ddeploy/proposals(/validate)` can succeed at all — this is the
intended fail-closed behavior this ADR exists to introduce, not a
regression. See `MANUAL_VALIDATION_COMMANDS.md` for the exact commands.

### 4. Node-specific canonical operational slice — audited, found ALREADY CORRECT, strengthened with a regression

Traced the full path: accepted `CanonicalDMapV1`
→ `core::ddeploy::union_project_operational_graph`
→ `api::ddeploy::effective_canonical_operational_graph`
→ `api::operational_dgraph::operational_dgraph_snapshot`/
  `graph_and_authorizations_snapshot` (both already check canonical
  authority FIRST, per that module's own `operational_dgraph_snapshot` doc
  comment, unchanged since Stage 111)
→ `operational_dgraph_slice_snapshot`
→ `core::operational_dgraph::build_operational_slice`
→ the real `GET /api/v1/operational-dgraph/slice/:node_id` endpoint
  SmartSentinel/preflight consumes.

This chain was ALREADY correctly wired before this pass — no code change
was needed here. A new regression,
`canonical_authority_node_slice_contains_exactly_that_nodes_placed_services`
(`core::ddeploy` tests), proves it end-to-end: two applications place
services on different nodes via accepted D-Maps, and each node's slice
(built via the SAME `build_operational_slice` function the real endpoint
calls) contains EXACTLY that node's own services, nothing from the other
application's node.

### 5. `local_substitutions[]`/`mutation_boundaries_crossed[]` (107A, F2) — unchanged, not redesigned

Audited only, per instruction; no code changed. See
`f2-evidence-audit.txt` in the review package for the coverage mapping.

### 6. 107B real catalog conversion — real network resolution, not invention

Traced the full pinning chain (`oci_registry_resolver.rs` → real Docker Hub
/ GHCR OCI Distribution v2 API → `oci_image_pinning.rs` →
`ServiceArtifactRegistry::apply_resolved_image_identity`). The mechanism
was already real, tested, and correct — including a genuine, working
bearer-token challenge flow and platform-verified resolution. Confirmed live
network access is available in this environment and used the EXISTING
`resolve_image_identity` function (never a hand-rolled second algorithm) to
resolve all 16 unique, `registry`-classified, real catalog image references
that were still bare tags, then rewrote each affected
`artifacts/catalog/*.json`'s `container.image` to
`"{repository}@{manifest_digest}"` for the `linux/amd64` platform —
following the EXACT precedent the one already-pinned artifact
(`chirpstack-gateway-bridge.json`) established. Of the 22 real
`ServiceArtifact` catalog files (`artifacts/catalog/scenario.json` is a
topology descriptor, not one of the 22), 19 are `registry`-classified and
are now digest-pinned — 1 was already pinned before this pass
(`chirpstack-gateway-bridge`), 18 were newly pinned in this pass (16 unique
images; 3 of the 18 share images across the `fluxmq` node1/2/3 trio). The
remaining 3 (`sma`, `lora-device-simulator`, `lora-packet-forwarder`) are
`local_build`-classified and are, BY DESIGN (ADR-0013 Sec. 3), never
resolvable in this slice — this is not a residual gap this pass could close
without inventing a build-attestation pipeline explicitly out of scope. 0
artifacts are `legacy_unclassified` or non-container. **Correct accounting:
22 total ServiceArtifacts = 19 registry-pinned + 3 local_build — never
phrased as "18 of 22 pinned" (independent review fix v2, R5 correction:
that phrasing under-counted the one artifact already pinned before this
pass).**

A new, focused checker (`scripts/check_real_catalog_oci_pinning.py` +
selftest) proves this holds for the real, checked-in catalog and will catch
a future regression (a newly-added registry-classified artifact that is
never pinned, or a `legacy_unclassified` field).

### 7. `execution_projection_digest: String::new()` — audited, confirmed harmless

Every occurrence of this literal is inside a `#[cfg(test)]` fixture
constructor. The REAL path (`ServiceArtifactRegistry::create`/`update`)
unconditionally overwrites both `execution_generation` and
`execution_projection_digest` with server-computed values regardless of
caller input (`"Server-owned (D-7a, DC-5): any client-supplied generation/
digest is ignored, never trusted"`, existing doc comment), and `validate()`
separately cross-checks that a persisted `execution_projection_digest`
matches its own recomputation, rejecting otherwise. An empty/placeholder
digest can never reach a real authorization or execution request. No code
change; see `execution-projection-digest-audit.txt`.

### 8. Configuration/content identity and secret version binding — mechanism real, real-artifact evidence honestly incomplete

`ConfigurationMountV1.declared_content_sha256` and `SecretBindingV1.version`
both already participate in the same content-addressed
`execution_projection_digest` Sec. 7 describes — a change to either would
already invalidate a stale authorization under the SAME default-`required`
fail-closed gate 107B's image pinning uses (`required_completeness_gaps` reports
`unpinned_configuration_mount:<source>` for a missing content hash; secret
identity is bound the same way once `secret_bindings` — not the simpler,
unversioned `secret_refs` — is used).

Two honestly-reported, NOT-invented gaps: (a) every real catalog
configuration-mount `source` path (e.g. `mosquitto/mosquitto.conf`,
`chirpstack/gateway-bridge.toml`) references a file that does not exist
anywhere in this repository — it lives only on the real lab hosts — so this
pass could not compute a real `declared_content_sha256` for any of them
without inventing bytes it has never seen; exact `sha256sum` commands for a
human to run on the real lab hosts are in `MANUAL_VALIDATION_COMMANDS.md`.
(b) every real catalog artifact declares secrets via the simpler, unversioned
`secret_refs` (whose own preflight, `secret_preflight::build_secret_preflight`,
always reports `status: "unresolved"`/`material_available: false` — a
declarative dry-run shape, not a live resolution path) rather than the
version-aware `secret_bindings`; migrating them would require deciding where
real secret version identifiers come from, which is a product/operational
decision this pass does not invent an answer to. See
`config-identity-audit.txt` and `secret-version-audit.txt`.

### 9. Required-vs-optional real artifacts

No new `required`/`optional` field was introduced. `artifacts/catalog/scenario.json`'s
`nodes[].artifacts` already enumerates, per real lab node, exactly which
catalog artifacts the reference deployment uses — this is the existing
authoritative membership list, and the default-`required` pinning mode
(Sec. 6) already applies uniformly to every artifact reachable from it.
Inventing a separate required/optional axis on top would duplicate this
without new information.

## Consequences

- Stage 111 D-Deploy is now closed against the client-supplied-D-Continuum
  trust gap the roadmap audit identified as its one open item.
- Conceptual phase 110 (D-Node) now has a genuine structural, persisted,
  canonical registry — advanced from SCAFFOLDED, though still not a full
  first-class runtime/lifecycle object (no capability negotiation beyond the
  existing `dnode_abi_versions` set, no D-Node-local execution context).
- 107B real catalog trust materially improved (1 → 19 of 22 registry-pinned,
  18 newly pinned this pass); the remaining 3 (`local_build`) are a
  structural limit of this slice, not an oversight.
- Two real, bounded, honestly-disclosed gaps remain (config-content hashing
  requiring real lab-host access; secret version identifiers requiring a
  product decision) — both fail closed under the existing default pinning
  mode rather than silently passing, so neither is a live security hole,
  only an incomplete real-artifact-evidence trail.

## Independent review fix v2 corrections

An independent review of the v1 package (141,387 bytes, SHA-256
`2c2987a4a52abc8ae4b0f1c5157017daac25fe7e59bf04cdf26887986cddc27a`, 46
entries) found two genuine closure defects (R1, R2) and three packaging/
reporting defects (R3, R4, R5). All five are fixed in this revision. No v1
architectural decision listed above was reversed or redesigned.

### R1: acceptance now revalidates CURRENT D-Node structural authority (P1)

**The gap.** Section 3 (v1) layered the D-Node authority check ONLY at
proposal-CREATION time — `create_proposal`'s persisted `dcontinuum_snapshot`
was authoritative when written, but `accept_proposal` never re-checked it
against the CURRENT registry. A proposal created while a node was validly
declared, then accepted AFTER that node was removed or had its platform/
capacity/ABI capabilities changed, would still be accepted against
structural facts DServer no longer considered true — silently defeating the
very trust boundary Section 3 exists to establish.

**The fix.** `core::ddeploy::accept_proposal` gained a new parameter,
`current_authoritative_dcontinuum: Option<&CanonicalDContinuumV1>`. On the
PENDING → ACCEPTED path (never on an idempotent replay of an
ALREADY-accepted proposal — see below), it calls the SAME
`dnode_registry::check_dcontinuum_authority` function Section 3 already
established, comparing the proposal's persisted `dcontinuum_snapshot`
against the caller-supplied CURRENT authoritative continuum. Any mismatch
(node removed → `dcontinuum_node_unregistered`; platform/capacity changed →
`dnode_structural_conflict`; ABI capabilities changed →
`dnode_capability_invalid`) fails closed with the new
`AcceptError::StaleAuthority`/`stale_authoritative_dcontinuum` (HTTP 409,
not 422 — the proposal's own shape was fine; the world it depended on
changed). The historical `dcontinuum_snapshot` is NEVER rewritten, NEVER
auto-updated, and placement is NEVER regenerated — a stale proposal simply
fails; the operator must create a new one against the new authority.

**Immutability of history preserved.** This check applies ONLY to the
PENDING → ACCEPTED transition. The idempotent-replay branch (an
ALREADY-Accepted proposal) returns before ever consulting this parameter —
callers may legitimately pass `None` when replaying, and
`accepted_historical_dmap_remains_valid_after_a_later_registry_change`
proves a replay succeeds byte-identically even after EVERY node the
original proposal depended on is later removed from the registry.

**TOCTOU safety.** `api::ddeploy::post_accept_proposal` now acquires
`api::dnode_registry`'s own transaction lock (exposed as `pub(crate)`),
NESTED inside its existing `DDEPLOY_TRANSACTION`, held for the entire
remainder of the call — through the authority fetch, the `ServiceArtifact`
stable view, the acceptance mutation, and the final SQLite persist. Fixed,
documented, never-reversed lock order:
`DDEPLOY_TRANSACTION` → `DNODE_TRANSACTION` → `ServiceArtifact` stable view
→ SQLite D-Deploy persistence. `put_dnode`/`delete_dnode` acquire ONLY
`DNODE_TRANSACTION` and never `DDEPLOY_TRANSACTION`, so there is no
reverse-order acquisition path and therefore no deadlock — this is a
documented invariant, not merely an untested claim. A concurrent D-Node
`PUT`/`DELETE` cannot commit anywhere in that window, so the structural
facts an in-flight acceptance validates against cannot change out from
under it between the check and the persist.

New tests (`core::ddeploy`):
`pending_proposal_fails_acceptance_after_its_node_is_removed`,
`pending_proposal_fails_acceptance_after_node_capacity_changes`,
`pending_proposal_fails_acceptance_after_platform_changes`,
`pending_proposal_fails_acceptance_after_abi_capability_changes`,
`stale_proposal_snapshot_is_rejected_even_though_it_remains_internally_self_consistent`,
`accepted_historical_dmap_remains_valid_after_a_later_registry_change`. See
`stale-dcontinuum-acceptance-audit.txt`.

### R2: persisted D-Node registry integrity now fails closed on reload (P1/P2)

**The gap.** `dnode_registry::load_state` validated only `state_version`.
Retained structural corruption (a `BTreeMap` key not matching its own
stored `node_id`; a stored node that no longer passes its own structural
validation; a non-empty node set with `revision < 1`) would load silently.
Worse, `build_canonical_dcontinuum`'s `state.revision.max(1)` would silently
turn a corrupt `revision=0` (with non-empty `nodes`) into a plausible-looking
`revision=1` authoritative continuum — masking exactly the corruption a
relational-integrity check should catch.

**The fix.** A new `validate_persisted_dnode_registry_integrity` function,
invoked from `load_state` after the `state_version` check, validates: (1)
every map key equals its own `CanonicalDNodeV1.node_id`
(`dnode_registry_key_mismatch` on violation); (2) every stored declaration
independently re-validates via the SAME `validate_declaration` function
`declare_dnode` already uses (never a second, independently-maintained
copy) — this transitively re-checks platform/capacity validity and ABI-set
validity, including duplicate/unrecognised ABI entries; (3) a non-empty
node set has `revision >= 1` (`dnode_registry_revision_invalid` on
violation — structurally guaranteed by every legitimate `declare_dnode`
call, so only reachable through direct file corruption). NONE of these are
auto-repaired — corruption fails the load, full stop. The `.max(1)`
corruption mask in `build_canonical_dcontinuum` was removed now that
`load_state` fails closed instead; every legitimate in-memory construction
path (`declare_dnode`) already guarantees the invariant by construction, so
`state.revision` is used verbatim. A legitimate empty bootstrap
(`nodes = {}`, `revision = 0`) remains valid and is not flagged.

New tests (`core::dnode_registry`):
`persisted_map_key_node_id_mismatch_fails_closed_on_reload`,
`persisted_invalid_node_fails_closed_on_reload`,
`persisted_non_empty_registry_with_revision_zero_fails_closed_on_reload`,
`valid_multi_node_registry_reload_remains_green` (positive control). See
`dnode-persisted-integrity-audit.txt`.

### R3/R4: review package is now genuinely self-contained (packaging only, no code change)

The v1 package's `artifacts/catalog/` contained only the 18 files this pass
modified, so a fresh extraction's checker run under-reported the real
catalog (18/18 instead of the true 22 total / 19 pinned / 3 `local_build`).
The v2 package includes the COMPLETE real `artifacts/catalog/` directory (all
22 `ServiceArtifact` files plus `scenario.json`) so a fresh extraction
reproduces the TRUE inventory the checker computes dynamically — the checker
itself was not weakened or hard-coded to force any particular count. The v2
package also includes the key unchanged dependency source (`canonical_dcontinuum.rs`,
`canonical_dmap.rs`, `canonical_dgraph.rs`, `sentinel_registration.rs`,
`service_artifact_registry.rs`, `artifact_reconciliation_context.rs`,
`operational_dgraph.rs`, `oci_registry_resolver.rs`, `oci_image_pinning.rs`,
`execution_projection.rs`, `secret_preflight.rs`,
`secret_resolver_contract.rs`) needed to trace the acceptance lock/stable-view
path without access to the live worktree. See
`fresh-extraction-catalog-audit.txt`.

### R5: reporting language corrected (packaging/documentation only, no code change)

Every "18 of 22 pinned" phrasing in this ADR, the audits, and the MANIFEST is
corrected to the precise accounting: **22 total `ServiceArtifact` catalog
entries (`scenario.json` is a separate topology descriptor, not one of the
22) = 19 registry-classified and digest-pinned (1 already pinned before this
pass + 18 newly pinned this pass) + 3 `local_build` + 0
`legacy_unclassified`.**

## Superseded statements

- v1's "Stage 111's acceptance transaction... is completely unchanged" (Sec.
  3) is superseded by R1 above: acceptance now ALSO revalidates current
  D-Node structural authority for a pending proposal. Every OTHER acceptance
  property that sentence listed remains true and unchanged.
- v1's "18 of 22 real catalog artifacts are now digest-pinned (up from 1)"
  (Sec. 6) and "1 → 18 of 22 pinned" (Consequences) are superseded by R5's
  corrected accounting: 19 of 22 (1 pre-existing + 18 newly pinned).
