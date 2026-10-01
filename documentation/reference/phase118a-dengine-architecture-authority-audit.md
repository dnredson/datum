# Phase 118A — D-Engine Architecture, Authority, Identity & Projection Audit

- Status: Audit / design only. No production code was added or changed by this phase.
- Branch: `agent/phase118-dengine`, created from Phase-117 closure HEAD
  `8d2c0af2aee2373af09b08ab9effece32148b21a` (`origin/agent/phase117-dcompile`).
- Scope: determine whether D-Engine is a derived semantic view over existing
  DDeploy/DMap/DNode-registry authority, or whether it needs its own schema,
  identity, or persisted state. Recommend (do not enact) the minimum Phase 118B
  implementation slice.

## 1. Executive conclusion

**D-Engine is a derived semantic view, not an independent canonical authority.**

`D-Engine(project_id, application_id)` is defined as the set of distinct
`node_id`s referenced by the DServ placements of that application's **current
accepted DMap authority**, as already exposed by the single existing production
seam `ddeploy::active_placement_authority`. No new persisted state, no new wire
schema, no new digest/revision identity, and no new API surface is required to
answer "what runs this D-Application" correctly today. Everything a D-Engine
membership view needs already exists in `PlacementAuthorityView.placements`
(`Vec<DServPlacementV2>` / `Vec<CanonicalDServPlacementV1>`), which is itself
already documented in-source as "the ONE shared v1/v2 active-authority
interpretation every production consumer... must use" (`ddeploy_v2.rs`, doc
comment on `PlacementAuthorityView`, verified verbatim this phase).

The correct minimum Phase 118B slice (recommended, not built here) is a single
pure internal function, `derive_active_dengine_nodes`, that projects
`PlacementAuthorityView.placements` down to a deduplicated, deterministically
ordered `Vec<String>` of `node_id`s. No wire type, no API route, and no live
VM/RPi validation are required for that slice (Section 33).

## 2. Method

This audit re-derived every conclusion from current primary source rather than
trusting prior-phase summaries. Sources read in full this phase: ADR-0001,
`datum-canonical-model.md`, `glossary.md` (grep-confirmed identical to
ADR-0001), ADR-0004, ADR-0019, `canonical_dforward.rs` (full file, including
tests), and targeted full-body reads of `active_placement_authority`,
`PlacementAuthorityView`/`PlacementAuthorityV2Extras`/`ActiveDMapV2`/
`DDeployStateV2` in `ddeploy.rs`/`ddeploy_v2.rs`, `CanonicalDMapV1`/
`CanonicalDServPlacementV1` in `canonical_dmap.rs`, `DServPlacementV2`/
`DIoTPlacementV2` in `canonical_dmap_v2.rs`, `CanonicalDNodeV1`/
`DNodeRegistryStateV1`/`remove_dnode`/`check_dcontinuum_authority` in
`dnode_registry.rs`/`canonical_dcontinuum.rs`, and
`resolve_artifact_for_service`/`check_node_eligibility`/
`derive_application_operational_services` in `ddeploy.rs` (re-verified
independently of the Phase 117L citation, not merely re-cited). A full
git-history and working-tree grep for `D-Engine`/`DEngine`/`D-Platform` was run
and every hit outside the three vocabulary documents was read in context.

## 3. Canonical vocabulary: the exact D-Engine definition

ADR-0001's canonical definition table (byte-identical across ADR-0001,
`datum-canonical-model.md`, and `glossary.md`):

> **D-Engine** — The set of D-Nodes that runs a D-Application.

This is the entire canonical definition. There is no elaboration, no field
list, no schema reference, and no separate "D-Engine" row anywhere else in any
of the three vocabulary documents.

**D-Platform** — "D-Platform is composed of D-Engine, D-Forward and D-Monitor."
Both documents carry an explicit prototype-status disclaimer: "the repository
does not yet represent D-Platform as one first-class runtime object; today it
is an architectural grouping of those responsibilities." This disclaimer
applies transitively to D-Engine as one of D-Platform's three named
constituents — D-Engine is explicitly *not yet* claimed to be a first-class
runtime object either, which is consistent with (not merely compatible with)
the derived-view hypothesis this audit confirms.

## 4. The "v2 contract set" — D-Engine has no schema

`datum-canonical-model.md` enumerates exactly six canonical wire schemas plus
one embedded seventh:

| Schema | Owner concept |
|---|---|
| `datum.dgraph/1` | D-Graph |
| `datum.dcontinuum/1` | D-Continuum |
| `datum.dforward/1` | D-Forward |
| `datum.dmap/1` | D-Map |
| `datum.operational-slice/2` | derived operational slice |
| `datum.dserv-artifact/1` | D-Serv artifact |
| `datum.dserv-execution-contract/1` (embedded in the slice) | D-Serv execution contract |

D-Engine appears in none of these. Every other D-Platform constituent named in
Section 3 (D-Forward, via `datum.dforward/1`) has its own schema; D-Monitor's
derived status is out of this audit's scope (Section 30) but is likewise
schema-absent from this table. This is strong primary-source evidence that
D-Engine, as canonically defined, was never intended to be an independently
serialized/persisted contract — its membership is meant to be read off of
whichever contract already carries D-Node placement, i.e. D-Map.

## 5. D-Engine vs. D-Node

A D-Node is a single structurally-registered physical/logical execution host
(`CanonicalDNodeV1`, owned by the global, non-project-partitioned
`DNodeRegistryStateV1`, per ADR-0019). D-Engine is a *set* of D-Nodes, scoped
to one D-Application, and — critically — that set is not "every registered
D-Node"; it is only the subset actually referenced by that application's
current accepted D-Serv placements. D-Node registration is a structural
declaration ("this node exists and has these capabilities"); D-Engine
membership is a placement-authority fact ("this application's current accepted
DMap places a service on this node"). A D-Node can be registered and never be
part of any application's D-Engine (no placements reference it yet).

## 6. D-Engine vs. D-Continuum

D-Continuum (`build_canonical_dcontinuum`, ADR-0019) is the deterministic,
project-scoped-as-a-label, no-live-telemetry projection of the *entire* global
D-Node registry — every declared node's platform/capacity/capabilities,
independent of any application or placement. D-Continuum answers "what
structural capacity exists to place things onto"; D-Engine answers "what is
actually placed, for this one application, right now." D-Continuum is an input
to placement feasibility checking (via `check_dcontinuum_authority` at
proposal-acceptance time); D-Engine is downstream of an *accepted* placement
decision. D-Continuum is never application-scoped; D-Engine always is.

## 7. D-Engine vs. D-Map

D-Map (`datum.dmap/1`/`datum.dmap/2`) is the accepted authority record that
*contains* the placement facts D-Engine is derived from. D-Map is strictly
richer than D-Engine: it carries `dgraph_ref`/`dcontinuum_ref` (v1) or the full
v2 extras (D-Forward ref, D-Call realizations, DIoT placements, external port
bindings) in addition to `placements`. D-Engine is a narrow projection of one
field of one already-accepted D-Map — `placements: Vec<DServPlacementV2>` /
`Vec<CanonicalDServPlacementV1>` — reduced to just the distinct `node_id`
values. D-Map is authority; D-Engine is a read-only view over authority.

## 8. D-Engine vs. D-Monitor

D-Monitor is out of this audit's implementation scope, but the boundary must
be stated because it recurs throughout the closure criteria (health/liveness
questions, Sections 24/32). D-Engine membership is defined purely from
*accepted placement authority* — it answers "which nodes are supposed to be
running this application's services," not "which nodes are currently healthy,
reachable, or observed to be executing." Node health, liveness, and any
observed-runtime signal are D-Monitor's domain and must never be read by, or
mixed into, D-Engine's own derivation logic. A D-Monitor-derived health overlay
may legitimately be joined against D-Engine's membership set for presentation
purposes elsewhere, but that join happens *outside* D-Engine's own derivation
function, never inside it. This mirrors the exact same separation ADR-0019
already enforces for D-Continuum (no live telemetry field, ever, proven by a
compile-time structural test).

## 9. D-Engine vs. D-Forward (and the DIoT-node-membership question)

This is the single most consequential open determination in this audit, and
the evidence gathered this phase resolves it cleanly.

`canonical_dforward.rs`'s own module doc states: `//! Canonical
datum.dforward/1: logical middleware composition only.` The `CanonicalDForwardV1`
schema and its `CanonicalDIoTV1` element (`diot_id`, `diot_kind`,
`artifact_requirement`, `provides`, `requires`) carry **no `node_id` field at
all** — D-Forward's own canonical contract is purely logical/topological. DIoT
elements are placed onto physical D-Nodes only in a structurally separate
place: `DIoTPlacementV2 { placement_id, diot_id, node_id, artifact_binding }`,
which lives exclusively inside `PlacementAuthorityV2Extras.diot_placements` —
itself nested inside the `v2: Option<...>` field of `PlacementAuthorityView`,
architecturally distinct from the top-level, v1/v2-neutral
`placements: Vec<DServPlacementV2>` field that carries DServ placements.

This is direct, in-repository evidence that DATUM's own data model already
treats "nodes running D-Serv (D-Application service) placements" and "nodes
running DIoT (D-Forward middleware) placements" as two separate placement
domains, even though both use a `node_id` and both are real physical
placements onto real D-Nodes.

**Determination: DIoT-only nodes are EXCLUDED from D-Engine membership.**
D-Engine's canonical definition is "the set of D-Nodes that runs a
D-Application," and a D-Application's constituent unit of work is a D-Service
(via the DScript → DCompile → DGraph → DServ chain), not a DIoT middleware
element. ADR-0001 itself defines D-IoT as "one middleware element of
D-Forward," not of D-Engine or D-Application. A node that hosts *only* DIoT
placements (no DServ placement) is doing real, necessary work, but it is
D-Forward's work, not D-Application-service work, and is therefore outside
D-Engine's own membership set as canonically defined. This exclusion is a
definitional/authority-boundary conclusion, not a claim that DIoT nodes are
unimportant — see Section 34 for the corresponding open question this still
leaves for a possible future "extended platform footprint" view.

## 10. D-Engine vs. D-Platform

Per Section 3, D-Platform is explicitly not yet a first-class runtime object;
it is an architectural grouping of D-Engine + D-Forward + D-Monitor. Nothing in
this audit changes that. D-Engine remains one of D-Platform's three named
constituents and continues to have no independent schema of its own, exactly
like D-Platform itself.

## 11. The informal "D-Engine" terminology tension (surfaced, not resolved)

A second, textually distinct usage of "D-Engine" exists in the repository,
outside the three canonical vocabulary documents:

- `DATUM/src/dcall/delivery.rs`'s module doc calls
  `dcall::delivery::dispatch_validated_emit` "the D-Engine-facing library seam"
  and states "graph-wide propagation is explicitly a LATER D-Engine concern,
  not implemented here."
- `DATUM/src/dcall/mod.rs` and `dserver/src/api/dcall.rs` echo the same
  "D-Engine-facing seam" phrasing.
- ADR-0022 (`canonical-dcall-realization-v0.1.md`, line 122) lists "DEngine"
  inside an explicit exclusions list (things not implemented by that ADR),
  alongside DForward/DIoT/GNN/DScript/DCompile.
- `PHASE113A_REVIEW.md` (line 163) lists "recursive D-Engine dispatch" as
  explicitly deferred.

This usage is not the canonical noun ("a set of nodes"); it reads as an
informal placeholder name for a **future, unscoped, not-yet-designed
orchestration or graph-propagation process** — something that would *consume*
D-Engine's membership (to know which nodes to dispatch/propagate to) rather
than *be* D-Engine's membership itself. The two usages are compatible (a
dispatch process operating over D-Engine's node set is a perfectly coherent
future design) but they are not the same concept, and conflating them would be
a mistake: this audit's own task scope is explicitly the membership/authority
question, not the dispatch/orchestration question. This tension is recorded
verbatim as an open question in Section 34 rather than silently resolved one
way, per this audit's own instructions.

## 12. The correct seam: `active_placement_authority`

`ddeploy_v2.rs`'s own doc comment on `PlacementAuthorityView` states (quoted
verbatim, re-confirmed this phase):

> "the ONE shared v1/v2 active-authority interpretation every production
> consumer (D-Code authorization, D-Call route derivation, grant issuance,
> grant redemption, destination reauthorization) must use instead of
> independently reading `DDeployStateV1.active_by_application`/
> `phase113_v2.active` — see `ddeploy::active_placement_authority`, the sole
> real production constructor."

This is the same seam every other Phase 117 production consumer already uses.
D-Engine derivation should be the next consumer in that list, not a parallel
path that re-reads `DDeployStateV1` fields directly. `active_placement_authority`
already performs v1/v2 unification, dual-active-authority fail-closed
detection, missing-proposal fail-closed detection, and D-Graph digest
recomputation — none of that needs to be reimplemented.

## 13. Confirmed function signature and semantics (re-read in full this phase)

```rust
pub fn active_placement_authority(
    state: &DDeployStateV1,
    project_id: &str,
    application_id: &str,
) -> Result<Option<PlacementAuthorityView>, String>
```

- Returns `Ok(None)` when the application has **no active accepted DMap at
  all** (see Section 23: "no active" is `Ok(None)`, distinct from "active but
  empty placements," which would be `Ok(Some(view))` with `placements: vec![]`).
- Returns `Err(...)` when both a v1 and a v2 active record exist simultaneously
  for the same application key ("dual active DMap authority for application
  key ...") — a fail-closed corruption guard, directly relevant to Section 44 /
  scenario S13.
- Returns `Err(...)` when the active record references a proposal id that is
  missing from `state.proposals` / `state.phase113_v2.proposals` — another
  fail-closed guard, never silently degrading to a partial view.
- On success, `placements` is populated directly from the stored active
  record's own `map.placements` (v1) or `a.map.placements` (v2) — a plain
  field copy, with **no re-validation against the current D-Node registry at
  read time**. See Section 40 for why this matters.

## 14. DMap/1 vs. DMap/2 placement parity (proof, not assumption)

```rust
// canonical_dmap.rs (DMap/1)
pub struct CanonicalDServPlacementV1 {
    pub service_id: String,
    pub node_id: String,
}
// canonical_dmap_v2.rs (DMap/2)
pub struct DServPlacementV2 {
    pub service_id: String,
    pub node_id: String,
}
```

Field-for-field identical. A single derivation function operating on
`&[DServPlacementV2]`-shaped data (or, more precisely, on
`PlacementAuthorityView.placements`, which is already `Vec<DServPlacementV2>`
regardless of whether the underlying active authority was v1 or v2 — see the
`active_placement_authority` body, which maps `CanonicalDServPlacementV1`
values into the same `DServPlacementV2`-typed `placements` field for the v1
branch too) produces correct results for both DMap generations with zero
special-casing. D-Engine derivation therefore never needs to know or care
whether the underlying accepted authority was DMap/1 or DMap/2.

## 15. Membership set semantics: dedup, ordering, empty vs. absent

- **Dedup**: multiple D-Serv placements may legitimately target the same
  `node_id` (co-located services). D-Engine membership is a *set* of
  `node_id`s, not a multiset — duplicates must be collapsed.
- **Ordering**: no ordering is canonically meaningful for a membership set.
  For a deterministic, testable derivation, lexicographic sort of `node_id` is
  the correct choice (consistent with the `BTreeMap`-based ordering already
  used pervasively elsewhere in this codebase, e.g. `DNodeRegistryStateV1.nodes`,
  `DDeployStateV2.active`).
- **Empty vs. absent** (Section 23 restated precisely): "no active D-Map
  authority for this application" (`active_placement_authority` returns
  `Ok(None)`) and "an active D-Map authority exists but currently has zero
  placements" (`Ok(Some(view))` with an empty `placements` vec — legal today;
  `CanonicalDMapV1.placements`/`DServPlacementV2` collections are not
  validated non-empty anywhere read this phase) are two genuinely distinct,
  observable states and must be surfaced distinctly by any D-Engine view
  (e.g. `Option<Vec<String>>` or an explicit "no accepted authority" vs.
  "accepted authority, empty membership" variant — a wire-shape decision left
  to Phase 118B, not decided here).

## 16. Project/application scope

D-Engine, exactly like D-Map and unlike D-Continuum/D-Node-registry, is scoped
per `(project_id, application_id)` — there is no meaningful "D-Engine for a
project" independent of an application, because placement authority itself
(`active_by_application`/`phase113_v2.active`, both keyed by
`application_key(project_id, application_id)`) is application-scoped. A
project-wide "all D-Engines in this project" view, if ever needed, would be a
simple fold over every application's own D-Engine derivation — not a new
authority concept.

## 17. Source-authority-reference vs. invented identity

**Determination: D-Engine should carry no independent identity of its own** —
no `dengine_id`, no `revision`, no `digest`. It should instead carry a
reference back to the D-Map authority it was derived from: `dmap_id`,
`revision`, and `digest`, exactly the fields `PlacementAuthorityView` already
exposes. Inventing a parallel `dengine_revision`/`dengine_digest` would create
a second, redundant identity for what is definitionally the same fact
("current accepted placement authority") and would immediately raise a
synchronization question ("what if the D-Engine revision and the DMap revision
disagree?") that has no legitimate answer, since D-Engine has no independent
degrees of freedom from DMap. This mirrors the same reasoning ADR-0004 uses to
keep `DServExecutionContractV1` host-invariant rather than inventing a
parallel host-aware identity.

## 18. Persistence decision

**Determination: P1 — pure on-read derivation, no persistence.** D-Engine
membership changes exactly when the underlying active placement authority
changes (accept/redeploy events already produce exactly this signal today).
Persisting a separate D-Engine snapshot would require either (a) recomputing
it transactionally inside every DDeploy accept path (duplicated logic, a new
place to get out of sync) or (b) accepting eventual inconsistency between
persisted D-Engine state and the DMap authority it is supposed to mirror —
both strictly worse than deriving it on read from data that is already
persisted, transactionally consistent, and already proven correct through
Phase 113–117. This is the same P1 conclusion Phase 117I independently reached
for DCompile provenance, for an analogous reason (nothing here has an
independent lifecycle from its source authority).

## 19. Wire/API decision

**Determination: not needed in Phase 118A; likely a thin read-model view type
in 118B, only if a concrete consumer need exists.** Two options were weighed:

- **Option A — no wire type at all**, purely an internal Rust helper consumed
  by other server-side logic. Correct if no external caller (CLI, dashboard,
  future D-Call dispatch) needs to observe membership directly yet.
- **Option B — a small derived read-model wire type** (e.g.
  `DEngineMembershipViewV1 { project_id, application_id, dmap_id, revision,
  digest, node_ids: Vec<String> }`), explicitly documented as a *read-only
  projection*, never a `datum.*` canonical contract, never independently
  submitted/validated/accepted — analogous to how `datum.operational-slice/2`
  is already a derived, non-authority wire shape.
- **Option C — a full canonical `datum.dengine/*` contract** — rejected. This
  would contradict Section 4's evidence (no such schema exists or is implied)
  and would invent authority where none exists; a canonical contract implies
  submission/validation/acceptance semantics that make no sense for a value
  with zero independent degrees of freedom (Section 17).

118B's actual need should be driven by whichever concrete consumer first
requires it (a CLI command, a dashboard, or a future D-Call/dispatch
mechanism); this audit recommends Option A first, upgrading to Option B only
when such a consumer is identified, and explicitly recommends against Option C
for reasons in Section 4/17.

## 20. "Runs" — accepted-placement-authority, not observed-runtime

The canonical text says D-Engine is the set of nodes that "runs" a
D-Application. Two readings exist:

- **R1 — accepted-placement authority**: "runs" means "is authoritatively
  assigned to run, per the current accepted DMap" — a governance fact, true
  the instant a DMap is accepted, independent of whether the node is currently
  online, healthy, or has even finished installing the relevant D-Code bytes.
- **R2 — observed runtime**: "runs" means "is currently, empirically observed
  to be executing" — a D-Monitor-owned, liveness-dependent fact.

**Determination: R1.** This is consistent with every other accepted-authority
concept in the codebase (D-Map itself is "accepted," not "observed running";
D-Code authorization is "accepted," not "observed serving"; Phase 117K's own
live proof explicitly demonstrated that *installing* D2/H2 bytes on a real
node does not by itself change what a governed invocation executes — only
*accepting* the corresponding proposal does). R2-flavored facts (health,
readiness, convergence, actual observed execution) remain entirely
D-Monitor's domain, joined against D-Engine's membership set only as a
separate overlay, never folded into D-Engine's own derivation (Section 8).

## 21. No-active vs. empty-active-engine

Restated precisely from Section 15/23: "no D-Engine" (no accepted DMap exists
yet for this application) and "D-Engine with zero members" (an accepted DMap
exists but currently places nothing) are different, both legal, and both must
remain distinguishable in any future view. `active_placement_authority`'s
`Result<Option<...>>` return shape already encodes exactly this distinction
today; a D-Engine derivation built on top of it should preserve it rather than
collapsing both to an empty list.

## 22. Pending/rejected-proposal non-effect

`DDeployStateV1`/`DDeployStateV2` separately track `proposals` (all submitted,
regardless of outcome) from `active_by_application`/`active` (only the
currently accepted one per application). `active_placement_authority` reads
exclusively from the latter. A pending or rejected proposal therefore has zero
effect on D-Engine membership by construction — it is simply never consulted
by the seam D-Engine derivation is defined to use. No new logic is needed to
enforce this; it falls out of reusing the existing correct seam (Section 12).

## 23. (Restated for closure-criteria numbering) absent vs. empty authority

See Sections 15 and 21 — covered together since they are the same underlying
distinction, confirmed directly from `active_placement_authority`'s
`Result<Option<PlacementAuthorityView>, String>` signature and body.

## 24. Health/readiness/convergence are explicitly out of scope

Restated from Sections 8/20: no field, signal, or join against D-Monitor data
belongs inside D-Engine's own derivation function. `CanonicalDNodeV1` itself
was directly confirmed this phase to carry **no** telemetry field — its
`capacity` field doc comment states explicitly: "the DECLARED/STRUCTURAL
static capacity of this node, never a live/observed value... no
CPU-utilization/RSS/free-memory/load-average field is or will ever be added
here (Stage 113 D-Monitor territory, attached elsewhere)." D-Engine membership
inherits this same discipline transitively, since it is derived from
placement authority over exactly these node identities.

## 25. Pure-DCode-redeploy invariant: membership is unchanged

Phase 117K's own live evidence already proved this at the infrastructure
level: registering D2 in the DCode registry and physically installing H2 bytes
on the real D-Node, *without* accepting a new DMap proposal, changed neither a
fresh authorization nor a real governed invocation's target — both continued
to resolve D1/H1. Structurally, this is because `active_placement_authority`
is derived solely from `DDeployStateV1`/`DDeployStateV2` (DMap acceptance
state); DCode registry and DCode authorization are entirely separate state
(`dcode_registry.rs`, `dcode_authorization.rs`) that `active_placement_authority`
never reads. A D-Engine derivation built on top of `active_placement_authority`
therefore automatically inherits the same invariant: **a pure D-Code-revision
redeploy (new descriptor accepted for an already-placed service, same
`service_id`→`node_id` placement) changes zero D-Engine membership**, because
the placements themselves (`service_id`, `node_id` pairs) do not change — only
which D-Code descriptor is bound to that already-existing placement changes,
and that binding lives entirely outside the DMap/placement data D-Engine reads.

## 26. Placement-changing redeploy semantics (definitional only)

A redeploy that *does* change which `node_id` a `service_id` is placed on (as
opposed to Section 25's pure D-Code-revision case) necessarily changes D-Engine
membership at the moment the new DMap proposal is *accepted* — not before
(pending proposals have no effect, Section 22) and not gradually (there is no
partial-membership state; `active_placement_authority` reads a single active
record atomically). Any migration semantics (e.g. "old node stays a member
until new node finishes installing," draining, staged cutover) are explicitly
out of this audit's scope and are Phase 119-or-later territory (Section 32);
this audit states only the before/after definitional boundary, not any
transition mechanism.

## 27. DNode structural removal

Re-verified directly this phase (not assumed): `remove_dnode` in
`dnode_registry.rs` is **unconditional** — it does not check whether the node
is referenced by any active DMap before removing it:

```rust
pub fn remove_dnode(state: &mut DNodeRegistryStateV1, node_id: &str) -> bool {
    let removed = state.nodes.remove(node_id).is_some();
    if removed { state.revision = state.revision.saturating_add(1); }
    removed
}
```

Safety instead comes entirely from **acceptance-time** revalidation
(ADR-0019's R1 fix): if a *new* proposal is accepted after a node it depends
on has been removed, `check_dcontinuum_authority` fails that acceptance
closed with `dcontinuum_node_unregistered`. Critically, **this does not
retroactively invalidate an already-active DMap**: `active_placement_authority`
performs no live cross-check against the current D-Node registry on every
read (Section 13) — it just returns the stored active record's placements
verbatim. Therefore: **a D-Node that is later structurally removed from the
registry can remain listed in an application's already-accepted D-Engine
membership** until that application's DMap is next re-accepted. This is a
real, previously-unstated consequence of composing two already-correct,
independently-designed behaviors (unconditional removal + accept-time-only
revalidation), not a new bug — ADR-0019 deliberately chose accept-time-only
revalidation over continuous revalidation, and D-Engine derivation should
inherit that same design rather than attempting to "fix" it by silently
filtering out removed nodes (which would make D-Engine's view diverge from
the DMap authority it is supposed to mirror exactly). Any future consumer
that wants "current registry-live members only" should perform that join
explicitly against the current D-Node registry as a presentation-layer
concern, not inside D-Engine's core derivation.

## 28. DNode capability change

`check_dcontinuum_authority` treats a platform/capacity/ABI mismatch between a
claimed and the currently-registered node declaration as a structural
conflict finding at *acceptance* time only (same mechanism as Section 27).
Changing a node's capabilities after a DMap is already active does not by
itself alter that DMap's placements or, therefore, D-Engine membership — it
can only affect the *next* proposal's ability to be accepted against that
node. Capability changes are never a D-Engine-membership mutation; they are
purely an eligibility fact for future placement decisions.

## 29. Offline/unhealthy node membership

**Determination: an offline or unhealthy node REMAINS a D-Engine member.**
This follows directly from Section 20 (R1, accepted-authority reading of
"runs") and Section 24 (no telemetry in the node registry, no health signal
anywhere in the DMap/DDeploy authority chain D-Engine derives from). Liveness
is not a field this audit found anywhere in `CanonicalDNodeV1`,
`DServPlacementV2`, `CanonicalDServPlacementV1`, or `PlacementAuthorityView` —
there is nothing to read even if D-Engine wanted to exclude unhealthy nodes.
Health becomes a separate DMonitor-derived overlay that a future consumer may
choose to join against D-Engine's membership set for presentation (e.g. "N of
M D-Engine members currently healthy"), but that join is explicitly outside
D-Engine's own derivation function.

## 30. ServiceArtifact/DCode exclusion (re-verified directly this phase)

Phase 117L's §26 finding was independently re-verified against current source
in this phase, not merely re-cited. Direct reads of
`resolve_artifact_for_service`, `check_node_eligibility`, and
`derive_application_operational_services` (`ddeploy.rs`) confirm all three
operate exclusively on `ServiceArtifact` values keyed by `artifact_id`/
`service_id` and `control_plane.target_nodes` — none references
`DCodeArtifactAuthorityBindingV1`, `dcode_registry`, `descriptor_digest`, or
`dcode_authorization` in any form. D-Engine derivation, reading only
`PlacementAuthorityView.placements` (`service_id`, `node_id` pairs), likewise
never needs to touch `ServiceArtifact` or DCode registry/authorization state.
Both exclusions are independently confirmed, current, and consistent with
Section 25's redeploy-invariant reasoning.

## 31. Authority matrix

| Concept | Owns | Does NOT own |
|---|---|---|
| D-Node registry (`dnode_registry.rs`) | Structural node existence, platform, capacity, ABI capabilities (global, non-project-partitioned) | Placement, membership, health |
| D-Continuum (`canonical_dcontinuum.rs`) | Deterministic projection of the whole registry | Any single application's placement |
| D-Map / DDeploy (`ddeploy.rs`/`ddeploy_v2.rs`) | Accepted `(service_id, node_id)` placement authority per application; DIoT placement authority (v2) | D-Code descriptor binding; node health |
| D-Code registry/authorization | Which descriptor digest a placement's D-Code binding currently authorizes | Placement itself; node identity |
| D-Forward (`canonical_dforward.rs`) | Logical DIoT/middleware topology (no node_id) | Physical node placement (that's `DIoTPlacementV2`, a DDeploy/v2 concept) |
| **D-Engine (derived, no owner state)** | **Nothing new — a pure read projection of DMap placement authority's `node_id`s** | Health, D-Code binding, DIoT membership, structural registry facts |
| D-Monitor (out of scope) | Health/liveness/observed-runtime | Placement authority itself |

## 32. Phase 119/120 non-encroachment

This audit defines D-Engine membership *derivation* only. It explicitly does
not design: placement-changing redeploy migration/drain semantics (Section
26), any dispatch/graph-propagation orchestration mechanism (Section 11's
informal "D-Engine dispatch" usage), any D-Monitor health-overlay join
mechanism, or any D-Platform first-class runtime object. All of these remain
open for Phase 119/120 or later, unconstrained by this audit beyond the
boundaries stated above.

## 33. Recommended minimum Phase 118B slice

1. A single pure function, e.g.
   `derive_active_dengine_nodes(view: &PlacementAuthorityView) -> Vec<String>`,
   returning the deduplicated, lexicographically sorted `node_id` set from
   `view.placements`. No new struct beyond the existing `PlacementAuthorityView`
   input.
2. Call it from wherever `active_placement_authority` is already called, if
   and when a concrete consumer need appears (Section 19, Option A first).
3. **No VM or RPi live validation is required for this slice.** It is a pure,
   deterministic, in-memory projection over already-thoroughly-tested data
   (`PlacementAuthorityView` is already exercised by the full Phase 113–117
   test suites); nothing about deduplicating and sorting a list of strings
   has any live-infrastructure dependency, network behavior, or WASM
   execution surface to validate. This mirrors ADR-0019's own
   `build_canonical_dcontinuum`, which is likewise pure/deterministic and was
   never live-validated on real hardware for that reason.

## 34. Open questions (explicitly unresolved, not silently decided)

1. **The "D-Engine dispatch" terminology tension (Section 11).** Should the
   informal `dcall`/ADR-0022/`PHASE113A_REVIEW.md` usage of "D-Engine" as a
   placeholder for future graph-propagation/orchestration be renamed to avoid
   colliding with the canonical noun-definition confirmed by this audit, or
   should it be formally scoped as "a future process that consumes D-Engine's
   membership"? Left for whichever phase actually designs that mechanism.
2. **Extended-platform-footprint view.** Section 9 excludes DIoT-only nodes
   from D-Engine's canonical membership. Should a future *separate* "full
   platform footprint" view (D-Engine ∪ DIoT-node-set) exist for
   operational/observability purposes, distinct from and never confused with
   canonical D-Engine membership itself? Not decided here.
3. **Section 27's removed-node residue.** Is a D-Engine view that can list a
   structurally-removed `node_id` (until the next DMap re-acceptance)
   acceptable as permanent behavior, or should a future presentation layer be
   required to always join against the live D-Node registry before display?
   This audit recommends leaving core derivation unfiltered (matching DMap
   authority exactly) and treating any filtering as an explicit, separate,
   documented presentation choice — but does not mandate that a consumer ever
   build such a presentation layer.
4. **ADR update.** Should ADR-0001's D-Engine entry gain a short clarifying
   note (e.g. "derived from DMap placement authority; see
   `active_placement_authority`") the next time ADR-0001 is otherwise revised,
   or is a standalone reference document (like this one) sufficient
   indefinitely? This audit recommends a minimal future ADR-0001 clarifying
   note over a new dedicated ADR, given D-Engine's own canonical definition is
   already a single, complete sentence with no schema — but this is a
   recommendation only, not enacted in Phase 118A (no ADR was changed by this
   phase).

## 35. Historical evidence classification

| Source | Classification |
|---|---|
| ADR-0001 / `datum-canonical-model.md` / `glossary.md` | Canonical, authoritative — D-Engine is a passive membership-set noun |
| `dcall/delivery.rs`, `dcall/mod.rs`, `dserver/src/api/dcall.rs` | Informal, forward-looking placeholder usage — "D-Engine-facing seam," not a definition |
| ADR-0022 line 122, `PHASE113A_REVIEW.md` line 163 | Explicit exclusion-list mentions of a future, unscoped "D-Engine dispatch" concern — confirms it was never implemented anywhere in Phase 113/ADR-0022 either |
| `scripts/check_datum_contract_fixtures.py` line 76 | A canonical-term validation list entry, not an implementation |

## 36. Security invariants (E1–E12)

- **E1.** D-Engine derivation must never read or depend on D-Code registry,
  D-Code authorization, or `descriptor_digest` state (Section 25/30 — verified
  by direct source read, no such dependency exists in any candidate seam).
- **E2.** D-Engine derivation must never read or depend on `ServiceArtifact`
  state (Section 30 — verified).
- **E3.** D-Engine derivation must never read D-Monitor/health/liveness state
  (Section 8/24/29 — no such field exists in any input type it would read).
- **E4.** D-Engine derivation must use `active_placement_authority` (or an
  equivalent future accepted-authority-only seam), never read
  `DDeployStateV1.proposals`/`DDeployStateV2.proposals` directly (Section 22 —
  pending/rejected proposals must have zero effect).
- **E5.** D-Engine derivation must fail closed (propagate, not swallow) any
  error `active_placement_authority` itself returns (dual-active corruption,
  missing-proposal corruption) — it must never silently produce a partial or
  best-effort membership set from a corrupted authority state.
- **E6.** D-Engine derivation must distinguish "no active authority" from
  "active authority, empty membership" (Section 15/21/23) and must never
  collapse the two.
- **E7.** D-Engine derivation must exclude DIoT-only placements from core
  membership (Section 9), consistent with D-Forward's own logical-only schema
  boundary.
- **E8.** D-Engine derivation must be pure and deterministic (same
  `PlacementAuthorityView` input always yields the same output set/order) —
  no wall-clock, no randomness, no I/O.
- **E9.** D-Engine derivation must deduplicate `node_id`s (co-located services
  must not produce duplicate membership entries) — Section 15.
- **E10.** D-Engine derivation must carry no independent identity
  (`dengine_id`/`revision`/`digest`) that could desynchronize from the DMap
  authority it mirrors (Section 17).
- **E11.** D-Engine derivation must be application-scoped
  (`project_id`+`application_id`), never silently merge memberships across
  applications (Section 16).
- **E12.** If Phase 118B introduces a wire view (Option B, Section 19), it
  must be explicitly documented as a non-canonical, non-submittable derived
  read model — never given `datum.*`-style submission/acceptance semantics
  (Section 4/19).

## 37. Scenario matrix (S1–S14)

| # | Scenario | Expected D-Engine membership behavior |
|---|---|---|
| S1 | No DMap ever accepted for an application | `active_placement_authority` returns `Ok(None)` → D-Engine view is "absent," not empty |
| S2 | DMap accepted with zero placements | `Ok(Some(view))`, `placements` empty → D-Engine view is "present, zero members" |
| S3 | DMap accepted with 3 DServ placements on 2 distinct nodes | Membership = 2 distinct `node_id`s (dedup applied) |
| S4 | A new proposal is submitted but not yet accepted | No effect on current membership (Section 22) |
| S5 | A proposal is rejected | No effect on current membership (Section 22) |
| S6 | A pure D-Code-revision redeploy (same placements, new descriptor accepted) | Membership unchanged (Section 25, matches Phase 117K live proof) |
| S7 | A placement-changing redeploy is accepted | Membership updates atomically the instant acceptance completes (Section 26) |
| S8 | A D-Node referenced by an active DMap is later structurally removed from the registry | Membership still lists that `node_id` until the DMap is next re-accepted (Section 27) |
| S9 | A D-Node's capability/platform declaration changes after placement | No effect on current membership; only affects future proposal acceptance (Section 28) |
| S10 | A D-Node goes offline/becomes unhealthy | No effect on membership — remains a member (Section 29) |
| S11 | A node hosts only DIoT placements, no DServ placements | Excluded from D-Engine membership (Section 9) |
| S12 | A node hosts both a DServ placement and a DIoT placement | Included in D-Engine membership (via its DServ placement), regardless of its DIoT role |
| S13 | Corrupted dual-active v1+v2 authority state for one application | `active_placement_authority` returns `Err(...)`; D-Engine derivation must propagate the error, never guess (E5) |
| S14 | Active authority references a missing proposal record | Same as S13 — `Err(...)` propagated, fail closed |

## 38. Negative-space ownership list (explicitly NOT owned by D-Engine)

- D-Node structural existence/capabilities — D-Node registry.
- D-Node health/liveness/readiness/convergence — D-Monitor.
- Which D-Code descriptor a placement executes — D-Code registry/authorization.
- Placement *eligibility* (`ServiceArtifact`/`control_plane.target_nodes`) —
  DDeploy's pre-existing ServiceArtifact bridge, orthogonal to D-Engine.
- DIoT/middleware topology and placement — D-Forward / DDeploy v2's
  `diot_placements`.
- Any dispatch, routing, or graph-propagation mechanism — future, unscoped
  work informally referred to as "D-Engine" elsewhere (Section 11), not this
  audit's D-Engine.
- Operational-slice generation, host-lifecycle representation — ADR-0004's
  `DServExecutionContractV1`/`dnode_runtime` machinery, a further-derived
  layer downstream of DMap, not D-Engine's concern.

## 39. Why not derive D-Engine from the operational slice instead of DMap

The operational slice (`datum.operational-slice/2`) is itself a *further*
derived artifact downstream of DMap (ADR-0004), additionally carrying
host-lifecycle representation (`dnode_runtime`, hosting artifact, supervision,
resolved endpoint addresses) that legitimately differs between otherwise
node-equivalent deployments (container vs. native host, per ADR-0004 Decision
2). Deriving D-Engine from the slice would couple a pure membership fact to
host-lifecycle detail that has nothing to do with membership, and would make
D-Engine's value depend on a strictly larger, more volatile input than
necessary. `PlacementAuthorityView` (DMap authority itself) is the more
primitive, correct, and already-canonically-endorsed source (Section 12).

## 40. Live-infrastructure decision for Phase 118B

**No VM or RPi validation is needed for the recommended 118B slice** — see
Section 33, point 3, for the full reasoning (pure deterministic projection
over already-tested data, no network/WASM/live-node surface).

## 41. ADR recommendation (not enacted)

See Open Question 4 (Section 34). No ADR was created or modified during Phase
118A.

## 42. No production code changed

Confirmed via `git status`/`git diff --check` before commit (Section 44): this
phase added exactly one new file, this document. No `dengine.rs`, no API
route, no struct, no persistence, no test, and no ADR file was added or
modified.

## 43. Relationship to Phase 117's closure

Nothing in this audit contradicts, reopens, or redesigns any Phase 117
authority. `dcompile`, `dgraph`, `dcode_registry`/`dcode_authorization`,
`ddeploy`/`ddeploy_v2`, `canonical_dmap`/`canonical_dmap_v2`, `dnode_registry`,
and `canonical_dforward` are all read in this audit exactly as Phase 117
closed them (Phase 117L, verdict B), and are treated as frozen inputs D-Engine
derivation reads from, never as things this phase redesigns.

## 44. Verification performed this phase

- Full reads: ADR-0001, `datum-canonical-model.md`, ADR-0004, ADR-0019,
  `canonical_dforward.rs` (including its test module).
- Grep-confirmed: `glossary.md` identical to ADR-0001's definition table.
- Direct source reads (function bodies, not summaries): `active_placement_authority`
  (full body), `PlacementAuthorityView`/`PlacementAuthorityV2Extras`/
  `ActiveDMapV2`/`DDeployStateV2`, `CanonicalDMapV1`/`CanonicalDServPlacementV1`,
  `DServPlacementV2`/`DIoTPlacementV2`, `CanonicalDNodeV1`/
  `DNodeRegistryStateV1`/`remove_dnode`/`check_dcontinuum_authority`,
  `resolve_artifact_for_service`/`check_node_eligibility`/
  `derive_application_operational_services`.
- Full git-history and working-tree grep for `D-Engine`/`DEngine`/`D-Platform`,
  every hit outside the three vocabulary documents read in context.
- `git status` / `git diff --check` confirmed only this document changed
  (see Section 42).

## 45. Closure statement

```
PHASE 118A AUDIT CLOSED.
PHASE 118 — D-ENGINE — REMAINS OPEN.

NO PRODUCTION D-ENGINE TYPE, SCHEMA, STATE, API, OR AUTHORITY
WAS CREATED DURING PHASE 118A.

DNODE REGISTRATION, PLACEMENT, OBSERVATION, AND EXECUTABLE
CURRENTNESS REMAIN OWNED BY THEIR EXISTING AUTHORITIES.

NO VM OR RPI LIVE VALIDATION WAS REQUIRED FOR PHASE 118A.
```

## 46. Phase 118B — production derivation (implemented)

Phase 118B implemented exactly the minimum slice recommended in Section 33:
one pure function,

```rust
pub fn derive_active_dengine_nodes(
    view: &crate::core::ddeploy_v2::PlacementAuthorityView,
) -> Vec<String>
```

added to `dserver/src/core/ddeploy.rs` immediately after
`active_placement_authority`, the seam it consumes. **D-Engine remains a
derived view, not an authority**: this function introduces no identity,
revision, digest, persistence, wire schema, or API. It reads only
`view.placements` (`Vec<DServPlacementV2>`, the top-level, v1/v2-neutral
D-Serv placement list) and returns the deduplicated, lexicographically
sorted set of `node_id` values found there, via a `BTreeSet<String>`
intermediate (idiomatic, no new dependency).

DIoT placements (`view.v2.diot_placements`) are never read by this function
— confirmed both structurally (the function's only parameter access is
`view.placements`) and by two dedicated tests (`e7_diot_only_node_is_excluded_from_membership`,
`e8_diot_only_active_authority_yields_empty_membership`) proving a node that
appears only in `diot_placements` never appears in the returned membership
set, including when `placements` is otherwise empty.

Runtime health, D-Node capability state, and D-Code executable currentness
play no role: `DServPlacementV2` carries only `service_id`/`node_id`, and
`PlacementAuthorityView` carries no health/liveness/capability/D-Code field
for this function to read even if it wanted to — proven by tests E9–E11 and
restated in the function's own doc comment.

`None` vs. "active authority, zero placements" (Section 15/21/23) remains an
authority-level distinction owned entirely by `active_placement_authority`'s
own `Result<Option<PlacementAuthorityView>, String>` return type, outside
this function: receiving a `&PlacementAuthorityView` at all already implies
active authority exists, so `derive_active_dengine_nodes` itself has nothing
to distinguish — callers must branch on `active_placement_authority`'s
`Option` before calling it. No wrapper over `active_placement_authority` was
added, per Section 19/Option A and the absence of any concrete existing
consumer requiring one.

Twelve focused tests (`E1`–`E12`) were added in a `dengine_derivation` test
submodule of `dserver/src/core/ddeploy.rs`'s existing `mod tests`, built
against the real `PlacementAuthorityView`/`DServPlacementV2`/
`PlacementAuthorityV2Extras`/`DIoTPlacementV2`/`DIoTArtifactBindingV2`
production types (no shadow structs). No wire type, API route, persistence,
or ADR change was introduced. `dserver`'s full test suite (1871 tests) and
the `ddeploy` module's own 240 tests pass unchanged.
