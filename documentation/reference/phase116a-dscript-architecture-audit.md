# Phase 116A — DScript Architecture & Contract Audit

**Status:** Phase 116A CLOSED (audit only). **NO PRODUCTION CODE CHANGES WERE
MADE DURING PHASE 116A.**
**Branch:** `agent/phase116-dscript`.
**Baseline:** Phase 115H closure, commit `d1a9883aebf50fe54eaa779fe2281fc3be39ef6c`
(already integrated into `main` at the time this audit began — see Section 2).

This is a read-only architectural and historical audit. It makes no
implementation decisions, invents no syntax, and defines no parser, compiler,
or runtime. Where repository evidence is silent, this document says so
explicitly rather than filling the gap with an architecturally-attractive
guess.

---

## 1. Executive conclusion

**DScript exists in DATUM today exclusively as a documentation-level
vocabulary concept, inherited verbatim from the DATUM reference-model paper
(Kamienski et al.) and recorded in three canonical documents.** No Rust type,
schema, parser, compiler, or runtime path for DScript has ever existed in
this repository's full history, on any branch. The single tangible artifact
resembling DScript — a hand-authored JSON "VS1" specification sketch with
`source.dscript_id`/`provenance.d_functions`/`compiler: {name: "dcompile"}`
fields — is explicitly self-disclaimed by the project's own ADR-0017 as
**never implemented or read by any Rust code**, and was **consciously not
carried forward** when the real `CanonicalDGraphV1` was built (Stage 110):
the production type has no `source`/provenance field at all. Every ADR from
Stage 107A onward that mentions DScript/DCompile/DEngine mentions them only
in an explicit *out-of-scope exclusion list*.

The canonical vocabulary, however, is precise and consistent enough to give
Phase 116 a real starting definition: **DScript is the logically-centralized
SOURCE representation of a D-Application — a set of D-Functions with
non-functional annotations — that D-Compile turns into a D-Graph and D-Code,
mapping D-Function → D-Serv with a cardinality that need not be 1:1.** This
is a compile-time, source-to-topology relationship, not a live orchestration
or workflow-execution language. Nothing in the repository's history supports
retries, timeouts, rollback declarations, loops, or event/trigger semantics
belonging to DScript itself.

## 2. Audit scope and evidence

Investigated: `DATUM/src/`, `dserver/src/`, `documentation/`, `docs/`,
`scripts/` (including `scripts/fixtures/`), all Cargo manifests, and the
full git history across all local and remote branches (`git log --all`,
pickaxe searches `-S`/`-G`, `--diff-filter=D` for deleted files). No
production code was modified. Two focused read-only investigations were run
in parallel (current-tree discovery; git-history archaeology) and their
findings were independently cross-checked against the actual current source
(struct definitions, ADR text, fixture JSON) before being written into this
document — every quote below was re-verified directly, not merely trusted
from a summary.

## 3. Existing DScript artifact inventory

| Artifact | Path / commit | Classification | Current? | DScript relevance | Authority implications | Reusable? | Notes |
|---|---|---|---|---|---|---|---|
| Canonical vocabulary entries for D-Application/D-Script/D-Function/D-Compile | `documentation/adr/ADR-0001-canonical-datum-vocabulary.md:33-36`, mirrored in `documentation/reference/glossary.md:14-17` and `documentation/reference/datum-canonical-model.md:39-42` | DOCUMENTATION-ONLY CONCEPT | Current, unmodified since introduction | Direct — this IS the canonical definition | None (definitions only, no authority claimed) | Yes — this is the authoritative definition Phase 116 must build from | Verbatim quotes in Section 4 |
| D-Engine vocabulary entry + D-Platform prototype-status note | `ADR-0001:46-47` | DOCUMENTATION-ONLY CONCEPT | Current | Indirect (Phase 118 boundary) | None yet; flags D-Platform/D-Engine as not-yet-first-class | Yes | See Section 13 |
| ADR-0022 exclusion-list mention | `documentation/adr/ADR-0022-canonical-dcall-realization-v0.1.md:122-123` | HISTORICAL ROADMAP MENTION ONLY | Current text, but purely a "not built here" disclaimer | Confirms consistent phase-fencing, no definition | None | No new information, but corroborates scope discipline | "No DForward, DIoT, DEngine, GNN/RL/LLM, DScript, DCompile, module distribution, placement generation, or automatic proposal acceptance is added." |
| VS1 fixture `source` block (`dscript_id`, `dscript_digest`, `compiler`, `mapping_style`) | `scripts/fixtures/datum-contracts/vs1/dgraph.sensor-pipeline.json:6-11` (added in commit `07400c8`, 2026-09-10; ADR text in commit `932f434`) | PROTOTYPE / EXPERIMENTAL (self-disclaimed) | Present on disk, structurally disconnected from production types | Direct — the only concrete DScript-shaped data in the repo | None — never validated, never read by any Rust code | Weak — informal sketch, superseded by the real `CanonicalDGraphV1`, but the `mapping_style: "1:1"` hint and D-Function→D-Serv grouping idea are worth remembering | See Section 4 for the disclaimer text |
| VS1 fixture `provenance` block (`dscript_id`, `d_functions`, `compiler`, `build{rustc,target,cargo_lock_digest,feature}`) | `scripts/fixtures/datum-contracts/vs1/dserv-artifact.alert.json:24-30`, `dserv-artifact.decode.json:24-30` | PROTOTYPE / EXPERIMENTAL (self-disclaimed) | Present on disk, inert | Direct | None | Weak, same reasons | Shows one D-Script mapping to multiple D-Servs (`alert`, `decode`), each pinned to a distinct `d_functions` list — concrete evidence for non-1:1 D-Function→D-Serv mapping |
| ADR-0017 explicit disclaimer of the VS1 sketch | `documentation/adr/ADR-0017-canonical-datum-contracts-v1.md:62-83` | DOCUMENTATION (retrospective, authoritative on scope) | Current | Direct — the project's own record that this was a sketch, never implemented | Confirms no production authority was ever granted to this shape | Yes, as historical-intent evidence | Verbatim quote in Section 4 |
| `check_datum_contract_fixtures.py` `CANONICAL_TERMS` vocabulary-presence check | `scripts/check_datum_contract_fixtures.py:73-76` | TEST/FIXTURE (vocabulary-completeness lint only) | Current | Confirms "D-Script" is tracked as a term that must appear in the glossary prose — nothing more | None — checks documentation completeness, not DScript structure | No (not reusable for schema validation) | Lists D-Script alongside every other canonical term |
| Real `CanonicalDGraphV1` (production) | `dserver/src/core/canonical_dgraph.rs:72-84`, `DATUM/src/agent/canonical_dgraph.rs:40-50` | CANONICAL PRODUCTION CONTRACT (for DGraph, not DScript) | Current | Negative evidence — confirms no `source`/provenance/dscript field exists in the real type | Establishes what any future DCompile output must actually match | N/A | Fields: `{schema, application_id, dgraph_id, revision, services, calls}` only |
| `installation_plan/`, `deployment_planning/`, `installation_tasks/` (legacy SmartSentinel HTTP-client models) | `DATUM/src/installation_plan/`, `DATUM/src/deployment_planning/`, `DATUM/src/installation_tasks/` | LEGACY / SUPERSEDED, GENERIC — NOT DSCRIPT | Present, unrelated | None — predates the D-* vocabulary entirely; loosely-typed with `BTreeMap<String, Value>` catch-alls | None | No | Flagged only to explicitly rule it out, per the audit's instruction not to confuse generic planning/scripting code with architectural DScript |
| Parser/DSL crate dependencies (pest, nom, lalrpop, logos, chumsky, tree-sitter) | All Cargo manifests | ABSENT | N/A | None found | N/A | N/A | Zero such dependencies exist anywhere in the workspace |
| Any `dscript*`/`d_script*`/`d-script*` named file, on any branch, ever | Full git history, all branches | DEAD / NEVER EXISTED | N/A | Clean negative finding | N/A | N/A | Confirmed via `git log --all --diff-filter=A --name-only` search and pickaxe search across all history |

## 4. Historical intent

The canonical vocabulary (`ADR-0001-canonical-datum-vocabulary.md`, adopted
from the DATUM reference-model paper and mirrored verbatim in the glossary
and canonical-model doc) states:

> **D-Application** — Placement-independent application logic comprising all
> pieces of code needed to implement the required functionality of a smart
> application. Its logically centralized source representation is D-Script.
> D-Code is generated later as the executable of a D-Serv.
>
> **D-Script** — Monolith-distributed source: a set of D-Functions with
> non-functional annotations. The logically centralized source representation
> of a D-Application.
>
> **D-Function** — A function unit inside a D-Script.
>
> **D-Compile** — Compiles a D-Script into a D-Graph and D-Code; maps
> D-Function → D-Serv (1:1, 1:N, M:1, M:N).

This is the *only* substantive definition of DScript's intent anywhere in
the repository. It describes a **source-to-topology compilation
relationship**: a developer (or tool) authors a D-Application as a
monolith-shaped source document (D-Script) containing named units of logic
(D-Functions) plus "non-functional annotations" (unspecified further, but by
convention in this class of reference model these are typically things like
resource/QoS/placement hints, not business logic itself). D-Compile then
splits/maps that source into a distributed D-Graph (topology of D-Servs) and
generates D-Code (the actual executable per D-Serv), with the mapping from
D-Function to D-Serv allowed to be many-to-many.

The only concrete (if inert) illustration of this idea is the VS1 fixture
sketch, where a single `dscript:sensor-pipeline` document maps to two
distinct D-Serv artifacts (`alert` with `d_functions: ["threshold_alert"]`,
`decode` with `d_functions: ["decode_uplink"]`) — a real example of a 1:N
(one D-Script, two D-Servs) mapping, each D-Serv owning a disjoint subset of
D-Functions. ADR-0017 is explicit that this sketch was never wired to
production:

> "The VS1 fixtures are a hand-authored **specification sketch** for a
> considerably larger future vision — D-Compile source lineage
> (`source.dscript_id`, `mapping_style`)... none of which any Rust code has
> ever read, written, or validated."
>
> "Stage 110's explicit brief is a **deliberately smaller v1**... Building
> the VS1 shape now would mean implementing exactly the Stage 111 D-Deploy v0
> lifecycle and the D-Forward contract this ADR is explicitly scoped not to
> touch."

There is **no evidence** anywhere in the repository — current tree or
history — that DScript was ever intended as: a distributed orchestration
language, a live workflow-execution DSL, a control-flow layer with runtime
retries/timeouts/rollback, or a user-facing scripting language distinct from
"the source a D-Application is written in." Every later ADR that mentions it
(ADR-0022) treats it purely as a future, out-of-scope phase name, consistent
with — not contradicting — the reference-model definition above. This audit
does not invent a richer interpretation than the evidence supports.

## 5. Current implementation status

Zero. No Rust module, type, trait, parser, lexer, grammar, AST, IR,
compiler pass, or execution engine for DScript exists anywhere in
`DATUM/src` or `dserver/src`, on the current branch or any other branch, at
any point in this repository's history. This was independently confirmed by
both a current-tree grep sweep (including false-positive-checked substring
matches like `allowe**d_function**s`, which are unrelated) and a full-history
pickaxe/log search (`git log -S"DScript"`, `-S"dscript"`, `-G` regex,
`--diff-filter=A --name-only` for script-named files). The only non-doc
artifact is the inert VS1 JSON sketch (Section 3), which is data, not code,
and is read by nothing.

## 6. Canonical semantic role

Based on Section 4, DScript's canonical role — to the extent evidence
defines one — is: **the human/tool-authored, logically-centralized source
description of a D-Application's functional units (D-Functions) plus their
non-functional annotations, which D-Compile consumes to produce a D-Graph
(topology) and D-Code (executables), with D-Function → D-Serv cardinality
that may be many-to-many.** DScript is a *compile-time source artifact*, not
a runtime object, not an execution plan, and not a placement mechanism.

## 7. Authority boundary matrix

| Concern | Canonical owner | Evidence |
|---|---|---|
| Application topology | DGraph | `CanonicalDGraphV1` (production, Stage 110/ADR-0009) |
| Service identity | DGraph / DServ contract | `CanonicalDServV1` within DGraph |
| Call relationship | DCall | `CanonicalDCallV1`, ADR-0022 |
| Placement / realization | DMap | Phase 115 closure + 115H; sole placement authority, hardened |
| DIoT provider topology | DForward | Phase 113 closure |
| Executable identity | DCode / ServiceArtifact | ADR-0011/0013/0014 pinning model |
| Observed runtime reality | DMonitor | Phase 114/115; never desired-state authority |
| Host mutation | Governed reconciliation (Stage 107A/109) | Phase 115/115H |
| Application *source* description | **DScript (proposed, unimplemented)** | This audit, Section 4 |
| Compilation of source into topology/executables | **DCompile (proposed, unimplemented)** | ADR-0001 D-Compile definition |
| Execution-node grouping for a running D-Application | **DEngine (proposed, unimplemented; possibly a grouping over DNode, not a new runtime)** | `ADR-0001:47`, Section 13 |

DScript must not duplicate any existing canonical owner above. In
particular: it must never become a second application-topology authority
(that is DGraph's role, produced only via DCompile, Section 9.1); it must
never express placement (that is DMap's role exclusively, Section 9.5); it
must never fabricate or bypass observed runtime reality (DMonitor's role,
Section 9.7); and it must never itself perform or gate host mutation
(governed reconciliation's role, Section 16).

## 8. Relationship to DGraph / DServ / DCall

**DGraph:** DScript is a *pre-image* of a DGraph, not a DGraph itself and
not a second source of DGraph truth. The only evidence of an intended
mapping (`mapping_style: "1:1"` in the VS1 sketch, alongside the 1:N example
actually shown by the two D-Serv artifacts sharing one `dscript_id`) suggests
DCompile may support more than one mapping cardinality, matching the
vocabulary's explicit "1:1, 1:N, M:1, M:N." Whatever DCompile eventually
produces must be validated/accepted through **the same authority path any
DGraph already goes through today** — currently, DGraph does not have a
freestanding "propose DGraph alone" endpoint; it arrives bundled inside a
D-Deploy proposal (`dserver/src/core/ddeploy.rs`'s `ProposalRecord.dgraph_snapshot`,
accepted atomically via `accept_proposal`). Any future DCompile output would
need to integrate with that existing proposal/acceptance mechanism, not
invent a parallel one — but working out exactly how is Phase 117's job, not
116A's or 116's.

**DServ:** No repository evidence supports DScript inventing service
identity heuristically. The VS1 sketch's `d_functions` list stays entirely
DScript-internal (function names within the source); the D-Serv identifiers
themselves (`artifact_id`, `service_id`) are assigned by the compilation
step's mapping decision, not derived from DScript function names by
convention. Phase 116 should treat any DScript-side service reference as an
explicit, typed reference to an already-existing (or explicitly-proposed)
DServ identity — never inferred from string matching.

**DCall:** DScript's D-Functions and DCall's typed call edges are distinct
concepts operating at different layers. DCall already fully owns "which
D-Serv calls which D-Serv" topology (Phase 112 closure, ADR-0022). If DScript
ever expresses inter-function invocation, that is D-Compile's job to lower
into DCall edges — DScript itself should not duplicate DCall's edge model.
No evidence found suggests DScript needs its own independent call-topology
representation; it needs, at most, enough intra-source structure for
DCompile to derive DCall edges from.

## 9. Relationship to DForward / DIoT

No repository evidence describes any DScript–DForward relationship. ADR-0022
explicitly lists DForward and DIoT alongside DScript/DCompile/DEngine as
jointly out of scope for the phase that introduced DCall — i.e., these
concepts have consistently been deferred together, not because they are
architecturally linked, but because none of them existed yet at that point.
If a future DScript construct expresses a dependency on DIoT readiness
("when service A is ready", per the audit's own hypothesis list in Section
13 below), any such construct must consume DForward's existing readiness
authority (`same_element`/`any_element`, Phase 113/114) rather than invent
provider selection itself. This is a requirement to observe *if and when*
such a construct is added — no evidence today confirms DScript needs it.

## 10. Relationship to DMap

**DScript must not become placement authority.** No historical evidence
(current tree or full git history) shows any DScript-related artifact ever
containing a "run service X on node Y" style placement statement — the VS1
sketch's `source` block carries no placement information at all; placement
in that sketch lived entirely in the (separately-scoped, also-unimplemented)
D-Map fixture shapes ADR-0017 also disclaims. There is therefore no obsolete
placement syntax to explicitly retire — the negative space is already clean.
Going forward: if DScript ever expresses non-functional *requirements* or
*constraints* (resource needs, co-location preferences), Phase 116 must not
assume these translate directly into placement. Any such construct would
need an explicit, governed translation path into a DMap *proposal*, subject
to DMap's existing acceptance authority — DScript itself performing or
implying placement is out of bounds.

## 11. Relationship to DCode / ServiceArtifact

DCode remains the canonical executable identity/authority
(`documentation/reference/datum-canonical-model.md`, Phase 111 closure).
D-Compile's definition ("compiles a D-Script into a D-Graph **and D-Code**")
places DCode generation on the *far side* of compilation — i.e., DScript
itself contains D-Functions (logical units), not resolved executable
identity. The VS1 sketch's `provenance.build{rustc, target,
cargo_lock_digest}` fields hint that a future DCompile might record *how* a
D-Function became compiled Wasm/native content, but this is exactly the kind
of provenance metadata that belongs downstream of DScript, attached to the
resulting ServiceArtifact/execution-projection identity (which already has a
rigorous, production digesting model — `execution_projection_digest`, JCS
canonical-JSON, Stage 107A/107B) — not invented as a parallel identity
scheme inside DScript. **DScript should never embed inline executable
logic or executable-content identity directly; it should reference
D-Functions by name/annotation only, leaving compilation and pinning to
DCompile and the existing ServiceArtifact machinery.**

## 12. Relationship to DMonitor / readiness

No evidence suggests DScript should or does manufacture observed runtime
reality. Any future DScript condition resembling "when service A is ready"
or "after B becomes healthy" must, per the established
`freshness != health != convergence != readiness` invariant (Phase 114/115),
consume DMonitor-derived readiness state exactly as it exists today — never
compute or assert its own notion of health. This is consistent with, not in
tension with, Section 9's DForward finding: readiness/health evaluation
already has a single owner, and nothing found in this audit suggests
DScript needs or historically had its own competing notion.

## 13. DScript vs DCompile vs DEngine

This is the most consequential open architectural question this audit was
asked to investigate, and the evidence changes the naive hypothesis.

The naive pipeline hypothesis offered in the task brief was:

```
DScript source -> parse/validate -> canonical DScript model -> DCompile
   -> compiled representation/execution plan -> DEngine -> governed execution
```

Repository evidence **partially supports and partially revises** this:

- **DScript → DCompile → (DGraph + DCode)** is directly supported by the
  ADR-0001 D-Compile definition and the VS1 sketch's concrete illustration.
  This part of the hypothesis holds.
- **DEngine as "an execution plan interpreter/runtime"** is *not* what the
  canonical vocabulary says. `ADR-0001:47` defines D-Engine as **"The set of
  D-Nodes that runs a D-Application"** — a *grouping/membership* concept over
  already-existing DNode/DMap-placed infrastructure, not a new
  interpreter component. `ADR-0001:46` additionally notes D-Platform
  (D-Engine + D-Forward + D-Monitor) is, today, "an architectural grouping of
  responsibilities," explicitly **not yet a first-class runtime object.**
  This suggests Phase 118's job may be substantially about *formalizing a
  queryable grouping* (which D-Nodes, per an accepted D-Map, collectively
  constitute "the D-Engine running application X") rather than building a
  brand-new execution runtime from scratch. This is a hypothesis for Phase
  118 to validate on its own evidence, not a conclusion this audit can prove
  — but it materially changes what "DEngine" should be assumed to mean, and
  Phase 116 must not design DScript around the naive "DEngine executes a
  compiled plan" assumption without Phase 118 confirming it.

**Explicit boundary, as far as evidence supports:**

- **Belongs in DScript (116):** the source-level, typed representation of
  D-Functions + non-functional annotations; validation that the document is
  internally well-formed and references real things (existing DServ/service
  identities, where referenced); identity/versioning of the DScript document
  itself. DScript does **not** produce a DGraph, does **not** generate
  DCode, and does **not** touch placement.
- **Must wait for DCompile (117):** turning an *accepted* DScript document
  into a DGraph proposal and a D-Function→D-Serv mapping; anything that
  crosses into DGraph/DCode/ServiceArtifact territory; integrating with the
  existing D-Deploy proposal/acceptance flow (Section 8).
- **Must wait for DEngine (118):** anything about which D-Nodes actually
  execute a compiled application, and any formalization of that grouping as
  a queryable object. Given the "set of D-Nodes" definition, this may turn
  out to consume DMap's already-accepted placement rather than compile or
  execute anything new — Phase 118's own audit should confirm or reject this
  before implementation, exactly as this document does for Phase 116.

## 14. Identity / revision / digest requirements

No DScript-specific identity scheme exists or is defined by evidence. By
consistency with every other canonical DATUM contract this repository has
actually shipped (`CanonicalDGraphV1`, `CanonicalDMapV1`,
`OperationalReconciliationAuthorization`, `execution_projection_digest`),
the established pattern is: a typed Rust model, a `schema` version string, a
stable id scoped under `application_id` (or equivalent), a `revision`
counter, and a digest computed via RFC 8785 JCS canonicalization over the
**typed canonical form**, never over raw source text directly, together with
an explicit proposal/acceptance separation for anything that becomes
"current." **This is an inference from sibling-contract convention, not a
DScript-specific citation** — flagged honestly as "likely but not yet
specified" per the audit's own required rigor. If a future DScript needs a
human-authored textual source at all (Section 15/18 discuss this is not
even confirmed necessary), the established pattern would suggest the source
text is parsed into the typed canonical model first, and that model — not
the text — is what is digested and versioned, mirroring
`execution_projection::recompute_digest`'s own precedent of recomputing
from canonical structure rather than trusting an externally-supplied digest.
Who assigns `dscript_id`/owns acceptance is undetermined by evidence and
should be decided in 116B/117 against the existing D-Deploy proposal model
found in Section 8, not invented fresh.

## 15. Semantic constructs supported by evidence

| Candidate construct | Classification |
|---|---|
| Named D-Functions with non-functional annotations | Supported by repository evidence (ADR-0001 definition) |
| D-Function → D-Serv mapping (1:1, 1:N, M:1, M:N) | Supported by repository evidence (ADR-0001 definition + VS1 1:N example) |
| Sequencing / control flow (A then B) | Not supported by any direct evidence; likely but not yet specified |
| Parallelism (A and B) | Not supported by any direct evidence; likely but not yet specified |
| Dependency declarations ("B requires A ready") | Not supported by direct DScript evidence; conceptually adjacent to Phase 115's own scope (dependency ordering remains within Phase 115's broader macro scope, per the roadmap) — likely belongs to a DScript *declaration* consumed by Phase 115-family *evaluation*, not DScript performing evaluation itself; unconfirmed |
| Conditions ("when X...") | Not supported by direct evidence; if added, must consume DMonitor/DForward readiness (Sections 9, 12), never invent its own |
| Service invocation / DCall interaction | Partially supported — D-Compile is defined to derive a D-Graph (which includes DCall edges); DScript's own representation of this is unspecified |
| DIoT dependency | No evidence; would need to consume DForward authority if ever added (Section 9) |
| Variables / data binding | No evidence found either way |
| Retries / timeout / failure branches / rollback declaration | **No evidence DScript was ever intended to own these.** These read as governance/corrective-action concerns (Phase 115's own scope, or a later phase), not source-language constructs. Explicitly flagged: do not add these to DScript merely because they are convenient constructs for a DSL to have |
| Loops | No evidence found either way; likely belongs to a later phase if added at all, given no evidence of any control-flow runtime intent |
| Events / triggers | No evidence found |

## 16. Explicit negative-space / prohibited responsibilities

DScript (and any future DCompile/DEngine work building on it) MUST NOT:

- Perform direct host mutation.
- Perform direct DMap mutation, or imply placement (`run X on Y`) itself
  (Section 10).
- Perform direct DForward mutation or invent DIoT provider selection
  (Section 9).
- Fabricate DMonitor evidence or its own notion of readiness/health/
  convergence (Section 12).
- Bypass Phase 115/115H governed reconciliation authorization or the Stage
  107A lease-cycle gate.
- Invent DServ identity heuristically (Section 8).
- Bypass ServiceArtifact/execution-projection pinning (Section 11) —
  compilation output must still flow through the existing
  required-completeness/digest model, not a parallel one.
- Execute arbitrary native commands merely because source text says so.
- Grant itself privileges, or resolve/inject secrets directly, bypassing the
  existing governed secret-resolution path.

No evidence found in this repository ever attempted any of the above under
the DScript name — this section is a forward-looking contract for Phase 116
onward, not a report of a violation found.

## 17. Historical conflicts and superseded assumptions

The only concrete historical DScript-adjacent design — the VS1 sketch's
`source`/`provenance` field shapes — is **entirely superseded**, by the
project's own explicit self-assessment (ADR-0017), not merely by this
audit's opinion. Specifically:

- **Superseded (concept still plausible, shape obsolete):** embedding
  `dscript_id`/`dscript_digest` directly as a field inside the D-Graph
  document, and embedding `provenance.build{rustc,...}` directly inside a
  D-Serv artifact. The real `CanonicalDGraphV1` and the real
  ServiceArtifact/execution-projection model both took different, more
  rigorous, already-shipped approaches (JCS digesting, `image_source_kind`/
  `resolved_image_identity` pinning) that any future DScript/DCompile design
  should integrate with rather than reintroduce the old ad hoc fields.
- **Not obsolete, still valid:** the underlying D-Function→D-Serv mapping
  *idea*, and the 1:N cardinality example, remain fully consistent with
  today's architecture and are the strongest piece of concrete evidence this
  audit has for what DCompile eventually needs to do.
- **No entire-concept obsolescence found:** nothing in Phases 107–115
  contradicts DScript's core definition (source representation of a
  D-Application); Phase 115's placement/reconciliation hardening if anything
  *clarifies* the boundary DScript must respect (Sections 10, 16), rather
  than invalidating the concept.

## 18. Candidate Phase 116B vertical slices

Not implemented; evaluated only.

1. **Typed canonical DScript document + validation only (no parser, no
   DCompile output).** A Rust type representing a minimal DScript document:
   `schema_version`, an id scoped under `application_id`, a `revision`, and
   a list of D-Functions (name + non-functional annotations, structurally
   typed rather than free-form), with pure structural + reference-resolution
   validation (e.g., any referenced existing DServ id must actually exist).
   No I/O, no host mutation, no DGraph generation. **Scores well** on every
   criterion in Section 23 of the task brief: smallest semantic surface,
   independently unit-testable, deterministic, no host mutation, no
   placement authority, directly useful as DCompile's future input,
   compatible with the existing DGraph/DServ/DCall model, and extensible
   without breaking schema (new annotation kinds can be added later).
2. **Minimal source syntax/parser → typed canonical DScript.** Adds a
   human-authored textual front-end. Rejected as the *first* slice: no
   evidence establishes that DScript needs bespoke textual syntax at all —
   every other DATUM canonical contract (DGraph, DMap proposals) is
   authored directly as JSON today, with no custom parser. Introducing
   grammar/parser machinery before confirming JSON-authored documents are
   insufficient would risk exactly the "invent syntax prematurely" outcome
   the task brief warns against.
3. **Typed sequencing/dependency declarations referencing existing DServ
   IDs.** Rejected as the first slice: Section 15 shows no direct evidence
   dependency/sequencing constructs are even confirmed as DScript's
   responsibility versus Phase 115's declaration/evaluation boundary; adding
   this before Option 1 exists would guess at semantics ahead of the
   evidence.

**Recommendation: Option 1** is the smallest useful, evidence-grounded,
canonical Phase 116B vertical slice.

## 19. Recommended Phase 116 decomposition

Illustrative, derived from the evidence above, not a commitment:

```
116A — architecture/contract audit                         (this document)
116B — canonical typed DScript model + structural/reference validation
116C — source syntax/parser (ONLY if 116B proves JSON-authoring insufficient)
116D — deeper semantic validation (dependency/annotation semantics, once Section 15's open constructs are resolved with evidence from actual DCompile-direction work)
116E — persistence/versioning/API (identity, digesting, proposal/acceptance wiring against the existing D-Deploy flow, Section 8)
116F — integration/closure
```

Phase 117 (DCompile) and Phase 118 (DEngine) remain fully separate phases,
per Section 13's boundary — 116 does not produce a DGraph, DCode, or any
execution grouping.

## 20. Open questions

- What exactly are "non-functional annotations" meant to carry (resource
  hints? QoS? something else)? No repository evidence defines this beyond
  the phrase itself.
- Does DScript need a human-authored textual syntax at all, or is a
  JSON-authored typed document (matching every other DATUM contract's
  authoring convention) sufficient indefinitely? Evidence does not resolve
  this; Section 18 recommends deferring the decision past 116B.
- Is D-Engine (Phase 118) actually just a formalized query over
  DMap-accepted placement + DNode registry, or does it need genuinely new
  runtime machinery? `ADR-0001:46-47`'s "set of D-Nodes" / "architectural
  grouping, not yet first-class" language leans toward the former, but this
  is Phase 118's own question to answer with its own audit.
- Where, precisely, should dependency/sequencing semantics live — as a
  DScript-side declaration, a DCompile-side derivation into existing DCall/
  DForward structures, or neither? Section 15 flags this unresolved rather
  than guessing.
- Who owns DScript identity assignment and acceptance, and does it reuse the
  existing D-Deploy proposal/acceptance flow (Section 8) or need its own?
  Not resolved by evidence; recommended as a 116E question once 116B's typed
  model exists to anchor the discussion concretely.

## 21. Evidence index

- `documentation/adr/ADR-0001-canonical-datum-vocabulary.md:33-36,41,46-47`
- `documentation/reference/glossary.md:14-17,29`
- `documentation/reference/datum-canonical-model.md:39-42`
- `documentation/adr/ADR-0017-canonical-datum-contracts-v1.md:55-90`
- `documentation/adr/ADR-0022-canonical-dcall-realization-v0.1.md:118-124`
- `scripts/fixtures/datum-contracts/vs1/dgraph.sensor-pipeline.json:6-11`
- `scripts/fixtures/datum-contracts/vs1/dserv-artifact.alert.json:24-33`
- `scripts/fixtures/datum-contracts/vs1/dserv-artifact.decode.json:24-33`
- `scripts/check_datum_contract_fixtures.py:73-76`
- `dserver/src/core/canonical_dgraph.rs:72-84`
- `DATUM/src/agent/canonical_dgraph.rs:40-50`
- `dserver/src/core/ddeploy.rs` (`ProposalRecord.dgraph_snapshot`, `accept_proposal`)
- `DATUM/src/installation_plan/`, `DATUM/src/deployment_planning/`, `DATUM/src/installation_tasks/` (ruled out as unrelated legacy code)
- Git history: `git log --all -S"DScript"` → commits `d1a9883`, `1d7ec64`; `git log --all -S"dscript"` → commits `932f434`, `07400c8`; full-history search for any `dscript*`-named file addition → zero results (clean negative finding); full-history search for any commit message mentioning DCompile/DEngine → zero results.

---

**NO PRODUCTION CODE CHANGES WERE MADE DURING PHASE 116A.**
