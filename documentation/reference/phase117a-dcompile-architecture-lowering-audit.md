# Phase 117A — DCompile Architecture & Lowering Audit

**Status:** Phase 117A CLOSED (audit only). **Phase 117 remains OPEN.** **NO
PRODUCTION DCOMPILE CODE WAS ADDED DURING PHASE 117A.**
**Branch:** `agent/phase117-dcompile`.
**Baseline:** Phase 116 closure, commit `ff72b2b6401b37564a4fe949eb19a9c15c0b4326`.

## 1. Executive conclusion

Given only `datum.dscript/1` as it exists today, DCompile can deterministically
derive almost nothing beyond identity pass-through: `application_id`, a set of
`function_id`s, and their opaque annotation key-value pairs. Everything else
the canonical vocabulary implies DCompile produces — D-Serv grouping, D-Call
edges, resource/ABI requirements, and D-Code content itself — requires
information that does not exist in `CanonicalDScriptV1` and, for D-Code
content specifically, **does not exist anywhere in this repository's current
architecture at all**: no code path, in `dserver` or `DATUM`, ever computes a
content digest from source or produces new build output. Every artifact model
this repository has (`CanonicalDServArtifactV1` for D-Code, `ServiceArtifact`
for the older container/native-process bridge) assumes already-built,
already-digested content supplied by a human or an external pipeline. This is
not a Phase 117 oversight — it is the actual, consistent architecture today,
and Phase 117 inherits it rather than needing to reopen it.

The one genuinely reusable, load-bearing precedent found is `dserver/src/core/canonical_placement.rs`'s
`plan_canonical_placements`: a pure function taking existing canonical
documents plus explicit additional inputs and deterministically producing a
new canonical output, together with `DDeployProposalV1`'s existing
`ProposalSource::Deterministic` + `PlannerProvenanceV1` machinery for
algorithmically-produced proposals. DCompile should be shaped in this same
mold — a pure function over explicit inputs, feeding its output through the
*existing* proposal entry point, not a new one.

**Recommendation: Phase 117B should NOT attempt end-to-end compilation.** It
should define a small, explicit, typed compile-input/mapping contract
(closing Gap #2, Section 6) and, separately, prove that a *given* explicit
mapping plus a DScript can deterministically produce a calls-empty candidate
`CanonicalDGraphV1` with an auto-assigned (not invented) D-Node ABI. D-Code
generation, real D-Call derivation, and resource-requirement sourcing all
require additional explicit inputs or policy decisions this audit could not
find evidence for, and must not be guessed at.

## 2. Audit scope and evidence

Investigated: `dserver/src/core/canonical_dscript.rs`, `DATUM/src/agent/canonical_dscript.rs`,
`dserver/src/core/canonical_dgraph.rs`, `DATUM/src/agent/canonical_dgraph.rs`,
`dserver/src/core/dserv_artifact.rs`, `dserver/src/core/dcode_registry.rs`,
`dserver/src/api/dcode.rs`, `DATUM/src/dcode/{module_store,invoke,identity}.rs`,
`dserver/src/core/service_artifact_registry.rs`, `dserver/src/core/ddeploy.rs`,
`dserver/src/api/ddeploy.rs`, `dserver/src/core/canonical_placement.rs`,
`dserver/src/core/wasm_module_registry.rs` (and its DATUM/API mirrors),
ADR-0001, ADR-0003–0009, ADR-0017, ADR-0021, ADR-0025, the Phase 116A/B/C
documents, and the full git history (`git log --all` grep/pickaxe searches
for DCompile/dcompile/mapping_style/d_functions). No production code was
modified. Two focused read-only research passes were run in parallel
(DCode/ServiceArtifact/DDeploy lifecycle trace; DGraph defaults/DCall-ID/
build-provenance investigation) and cross-checked directly against source
before being written here.

## 3. Canonical DCompile definition

The only definition (`ADR-0001-canonical-datum-vocabulary.md:36`, unchanged
since Phase 116A's audit):

> **D-Compile** — Compiles a D-Script into a D-Graph and D-Code; maps
> D-Function → D-Serv (1:1, 1:N, M:1, M:N).

This is the architectural target, not an algorithm. It says nothing about
where D-Code content, resource requirements, D-Node ABI, or D-Call edges come
from, nor what "1:N"/"M:N" mean operationally (Section 21).

## 4. Current DScript input capability

From `ValidatedDScriptInput` alone, DCompile can obtain, and only obtain:

- `document.application_id` — a plain pass-through string.
- `document.dscript_id`, `document.revision` — identity/versioning of the
  source, not of anything DCompile produces.
- A **set** of `function_id`s (order carries no meaning — Phase 116B/C).
- Per function, an opaque `BTreeMap<String, String>` of annotations, whose
  keys and values carry **no defined semantics** (ADR-0025) — DCompile must
  not assign meaning to any key, including ones that look suggestive
  (`"cpu"`, `"memory"`, `"group"`).
- `digest_sha256` — the DScript's own content digest, useful only as
  provenance ("compiled from exactly this source"), not as an input to any
  compilation decision.

That is the complete list. Nothing else can be derived without inventing
information.

## 5. Missing source/code input analysis (Gap #1)

**Does today's architecture already contain an upstream code-input contract
DCompile could consume? No.**

`CanonicalDServArtifactV1` (`dserver/src/core/dserv_artifact.rs:44`,
`datum.dserv-artifact/1`, Stage 111-ext/ADR-0021) is the real, canonical,
already-implemented D-Code identity contract:

```rust
pub struct CanonicalDServArtifactV1 {
    pub schema, artifact_id, version, application_id, dserv_id: String,
    pub dcode: DServArtifactDCodeV1,       // { uri, sha256, content_id, size_bytes }
    pub abi: DServArtifactAbiV1,           // { name, version, required_exports, allowed_imports }
    pub ports: DServArtifactPortsV1,
    pub limits: DServArtifactLimitsV1,
    pub safety: DServArtifactSafetyV1,
    pub state_model, instantiation: String,
    pub constraints: DServArtifactConstraintsV1,
    pub qos, provenance, probes: Value,    // uninterpreted by any Rust code
    pub signature_ref: Option<String>,
}
```

Its `dcode.uri` is validated to be exactly `datum-blob://sha256/<64hex>`, with
that digest required to equal both `dcode.sha256` and `dcode.content_id`
(`dserv_artifact.rs:183-198`) — **this is always a reference to
already-existing bytes in a content-addressed blob store, never a build
recipe.** Registration (`POST /api/v1/.../dserv-artifacts` →
`declare_dserv_artifact`, `dcode_registry.rs:69`) requires the caller to
supply a **fully-formed** `CanonicalDServArtifactV1`; the server only
validates and stores it — it never constructs one from anything. On the
agent side, `DATUM/src/dcode/module_store.rs`'s `install(bytes, declared_sha256)`
*verifies* bytes against a digest the caller already knows; it never computes
one from source.

**No code path anywhere in `dserver` or `DATUM` computes a content digest
from source text, or produces new build output.** `sha256`/`content_id`
values appear only as hardcoded test literals or externally-declared values.
100% of "building" (source → WASM bytes) is assumed to happen entirely
outside this codebase — exactly mirroring `ServiceArtifact`'s container
images, which are always hand-authored/externally-resolved OCI references,
never built in-repo.

The only historical hint of a source/build-provenance concept is the
obsolete VS1 sketch's `provenance.build{rustc,target,cargo_lock_digest,feature}`
(`scripts/fixtures/datum-contracts/vs1/dserv-artifact.*.json`), explicitly
disclaimed by ADR-0017 as a specification sketch "none of which any Rust code
has ever read, written, or validated," and superseded by `CanonicalDServArtifactV1`,
which has no such field.

A separate, unrelated near-miss was checked and ruled out:
`dserver/src/core/wasm_module_registry.rs`'s `WasmModuleDescriptor`/
`WasmFunctionDescriptor` (module kinds `embedded_test`/`diagnostic`/
`remediation_candidate`/`unknown`) superficially resembles a "module with
named functions" registry, but it is never referenced anywhere in
`DATUM/src/agent/*` (the governed reconciliation/DCode-invocation path) —
it is a separate, legacy SmartSentinel diagnostic/remediation WASM catalog,
architecturally disconnected from Stage 107A governance and from
`CanonicalDServArtifactV1`. **Classification: GENERIC — NOT DCODE.**

**Conclusion: `CanonicalDFunctionV1` has no code-body/source field by
design (Phase 116), and no upstream "here is the source/build recipe for
this function" contract exists anywhere today.** DCompile v1 cannot compile
source into WASM bytes — that capability does not exist in this repository
at all and is out of Phase 117's current scope to invent. What DCompile
*can* legitimately do is **wire together already-built, already-digested
D-Code artifacts** supplied as an explicit input alongside the DScript —
closer to "linking," not "compiling," for anything touching actual content.

## 6. DFunction→DServ mapping semantics (Gap #2)

`CanonicalDScriptV1.functions` is an unordered set with no grouping
directive. No annotation-driven, algorithmic, or naming-convention-based
grouping evidence exists anywhere in current code, ADRs, or history.

The only historical hint is the obsolete VS1 sketch: one `dscript:sensor-pipeline`
document mapped to two D-Serv artifacts, `alert` (`d_functions: ["threshold_alert"]`)
and `decode` (`d_functions: ["decode_uplink"]`) — a real 1:N example, with the
partition supplied **externally, by whoever hand-authored the fixture**, not
derived from the DScript itself. This is historical evidence of the *concept*
(mapping is an explicit partition decision) only — its wire shape (`d_functions`
embedded in a `provenance` block) is superseded and not to be revived.

**Function_id-to-service_id-by-naming-convention is explicitly rejected**
(as the task requires re-deriving, not assuming): the vocabulary permits
1:N/M:1/M:N, so a `function_id` cannot double as a `service_id` in general,
and no evidence anywhere suggests it should even in the 1:1 case.

**Conclusion: grouping cannot be inferred from `datum.dscript/1` alone. It
must be an explicit, separate compile-time input** — a typed
"DCompile mapping/request" contract that names, for a given DScript, which
`function_id`s belong to which generated D-Serv. This is a legitimate
117B-or-later deliverable, not a gap 117A can or should close by inventing a
default (e.g., "1:1 by default" has zero evidentiary support and is
explicitly the kind of choice the task warns against making from aesthetics).

## 7. D-Serv identity requirements

Properties a generated `dserv_id`/`service_id` needs (derived from
`CanonicalDGraphV1`'s own invariants plus general system requirements, not
invented): stable for identical compile inputs; collision-resistant across
applications; independent of placement/DMap/runtime realization;
distinguishable when grouping changes.

**Evidence found:** every `service_id`/`call_id`/`artifact_id` in every
existing fixture and test in this repository is an arbitrary,
human/tool-chosen string — there is **no precedent for hashing a canonical
identity into an opaque digest-derived ID anywhere in this codebase.** The
one relevant identity-*composition* (not hashing) precedent is
`dserver/src/core/ddeploy.rs:128`'s `canonical_dmap_id(project_id, application_id)`
— a server-owned, length-prefixed string composition specifically designed
to avoid ambiguous-concatenation collisions, confirmed used instead of a
caller-chosen string for `dmap_id` at proposal acceptance
(`ddeploy.rs:8056`'s test comment: "`accept_v2_proposal` derives `dmap_id`
itself via the server-owned `canonical_dmap_id` composition (never a
caller-chosen string)").

**Conclusion:** since grouping itself must be an explicit input (Section 6),
the most evidence-consistent answer is that **`service_id` is assigned by
whatever supplies the mapping** (a human or policy names each group when
specifying the partition), not algorithmically hashed from content — matching
this repository's universal convention of human/tool-chosen identity strings.
If an automatically-*composed* (not hashed) identity is ever needed (e.g. to
guarantee unwitting collisions can't happen across applications),
`canonical_dmap_id`'s length-prefixed composition scheme is the directly
applicable precedent to reuse the *pattern* of, not its literal code.

## 8. DCall derivation analysis

`CanonicalDScriptV1` carries no function-call, import, export, or invocation
relationship of any kind — and, per Section 5, no source/code body exists
for any static analysis to even operate on. There is nothing to analyze.

Per the task's own outcome list: **outcome A and C together** — DScript v1
genuinely lacks enough information for DCompile to derive any `CanonicalDCallV1`
edge; a calls-empty `CanonicalDGraphV1` is the only thing derivable from a
DScript alone (this is structurally legal per `CanonicalDGraphV1`'s own
`empty_collections_validate_cleanly` precedent). Real edges would require an
explicit additional call-graph input supplied alongside the DScript. **Outcome
D (static source analysis) is ruled out** — there is no source to analyze,
by Phase 116's own deliberate design. **Outcome B (an existing module
representation already contains enough call information) is also ruled
out** — `CanonicalDServArtifactV1`'s `ports.inputs`/`ports.outputs` describe
typed payload *ports*, not a call graph, and nothing connects one artifact's
ports to another's by any existing mechanism.

## 9. DCall call_id generation

If a future explicit call-graph input does yield `CanonicalDCallV1` edges,
`call_id` needs to be deterministic, collision-safe, and stable for the same
logical edge. **No existing precedent generates `call_id` from
`(source_service_id, target_service_id)`** — every fixture/test hand-writes
an arbitrary string (`"bridge-to-adapter"`, `"self"`). This is a genuinely
open decision for whenever DCall derivation is actually implemented, not
resolved by any direct precedent; `canonical_dmap_id`'s composition pattern
(Section 7) is the closest analogy if a deterministic scheme is chosen later.

## 10. Canonical DGraph output requirements

`CanonicalDGraphV1 { schema, application_id, dgraph_id, revision, services, calls }`,
`CanonicalDServV1 { service_id, execution_requirement: { dnode_abi }, resource_requirement: { cpu_cores, memory_bytes } }`
(`dserver/src/core/canonical_dgraph.rs`, unchanged since Phase 115/116).

- `schema` — a constant, trivial.
- `application_id` — direct pass-through from the DScript.
- `dgraph_id`, `revision` — new identity DCompile's caller/policy must
  assign; no evidence dictates how (Section 7's reasoning applies equally
  here — likely caller/policy-supplied, not hashed).
- `services`/`calls` — require the explicit mapping (Section 6) and
  call-graph (Section 8) inputs respectively; not derivable from DScript
  alone.
- `execution_requirement.dnode_abi` and `resource_requirement.{cpu_cores,memory_bytes}`
  — see Sections 11-12; these are genuinely required fields on
  `CanonicalDServV1` with **no default anywhere in the codebase** for the
  resource fields (confirmed: no `Default` impl, no `DEFAULT_CPU_CORES`/
  `DEFAULT_MEMORY_BYTES` constant, every existing call site takes them as
  required explicit parameters — this is REQUIREMENT-side declared data,
  never an observation-side value).

## 11. DNode ABI lowering finding

**Auto-assignment of `dnode_abi = DNODE_ABI_V0` ("datum-dnode/0") by
DCompile is evidence-justified, not an invented default.** Confirmed:
`DNODE_ABI_V0` is the *only* value `CanonicalDServV1::validate()` and
`CanonicalDServArtifactV1::validate()` ever accept (`dserv_artifact.rs:200-206`
hard-rejects any other `abi.name/abi.version` combination); no `Default` impl
exists, but every existing fixture/test writes it explicitly as a literal
because it is architecturally the sole legal choice, not a convenient
shortcut. DCompile writing this same literal explicitly (never silently, and
never as a true "default" mechanism that could later hide a second ABI) is
consistent with, not a departure from, how every human author of a DGraph
document already does it today. This is the one place in the entire audit
where an automatic value assignment is justified by hard evidence rather than
convenience.

This is architecturally distinct from `RuntimeKind` (`container`/`native_process`,
ADR-0015's operational dispatch seam) and from the OCI/host-platform concepts
`ServiceArtifact` uses — `dnode_abi` is the host-independent D-Serv execution
contract; DCompile must never conflate the two.

## 12. Resource requirement lowering finding

**No default exists, and none may be invented.** `cpu_cores`/`memory_bytes`
have zero fallback anywhere in this codebase; DScript's annotations are
opaque and uninterpreted (ADR-0025) — DCompile must never read
`annotations["cpu"]`/`annotations["memory"]` and assign it meaning, no
matter how suggestive such a key looks. **Conclusion: resource requirements
are a hard, currently-unfillable gap.** Phase 117B (or later) needs an
explicit, separately-typed compile-input contract carrying real resource
declarations, supplied by a human/policy — never silently defaulted, and
never sourced from annotation-string-sniffing.

## 13. Current DCode model

Summarized from Section 5: `CanonicalDServArtifactV1` (ADR-0021) is the
canonical, content-addressed D-Code identity. Lifecycle: caller declares a
fully-formed artifact → `declare_dserv_artifact` validates + registers
(keyed by `(application_id, dserv_id)`, immutable per key, conflict-detected
on differing re-declaration) → separate Stage 107A authorization
(`post_authorize_dcode_invocation`) governs whether it may ever be invoked →
DATUM's `DCodeModuleStore` verifies bytes against the declared digest at
install and again at every load → `invoke_dcode*` runs already-verified
bytes. **Registration alone grants no executability** — invocation still
requires the complete current authority chain (Stage 107A, current D-Graph/
D-Map topology), exactly as Phase 115's audits established for the
operational path.

## 14. Exact meaning of "generate DCode" under today's architecture

The reference-paper phrase "D-Compile...creates D-Code" cannot be taken to
mean "compiles source into executable bytes" in this codebase, because that
capability doesn't exist here at all (Section 5). Reconciled meaning: **"generate
DCode" today can only mean constructing a valid `CanonicalDServArtifactV1`
*descriptor* that references already-built content** — i.e., DCompile's
D-Code-facing output, if any, is the wiring/metadata around a WASM module
(`artifact_id`, `dserv_id`, `abi`, `ports`, `limits`, `safety`, `constraints`),
NOT the `dcode.{uri,sha256,content_id}` triple, which can only be filled in
once real bytes exist and are hashed — something outside this repository's
current scope. Forcing the paper's terminology onto today's implementation
without this reconciliation would produce a DCompile that either fabricates
digests (a serious integrity violation) or cannot complete at all.

## 15. ServiceArtifact vs. DServArtifact boundary

These are **two distinct, unrelated artifact models**, confirmed from code,
not merely by name similarity:

- `CanonicalDServArtifactV1` (ADR-0021) — the canonical, WASM/D-Code
  identity for the "Application WASM / D-Code" execution path (ADR-0005).
- `ServiceArtifact` (`service_artifact_registry.rs`, Stage 107/109-era,
  ADR-0011+) — the operational bridge for **container and native-process**
  runtime dispatch (`artifacts/catalog/*.json`, `image_source_kind`,
  `resolved_image_identity`, `execution_projection_digest`, native
  `acquisition.sha256`) — a completely different runtime kind from D-Code/WASM
  invocation.

Per the canonical vocabulary, DCompile's D-Code output target — if it
produces one at all — is `CanonicalDServArtifactV1`, never `ServiceArtifact`.
`ServiceArtifact` belongs to a separate operational lineage this phase must
not touch or duplicate.

## 16. DDeploy boundary

`DDeployProposalV1` carries `dgraph_ref: CanonicalContractReferenceV1`
(`{id, revision, digest}`) — a reference, not an inline graph — but
`create_proposal` (`ddeploy.rs:1862`) **accepts an already-constructed
`CanonicalDGraphV1` value directly as a parameter**, computing the reference
and storing a full `dgraph_snapshot` itself. **There is no separate "DGraph
registry" a caller must populate first** — a candidate DGraph exists only as
part of a proposal-creation call.

Critically, `ProposalSource::{Operator, Deterministic, Ai}` **already
exists**, with `Deterministic` used today by `canonical_placement::plan_canonical_placements`'s
output, tracked via `PlannerProvenanceV1 { planner_version, policy_name,
policy_version, dgraph_ref, dcontinuum_ref, ... }` on `ProposalRecord`. **This
is the exact existing entry point a future DCompile-produced candidate DGraph
should flow through** — supplying `ProposalSource::Deterministic` (or a new
sibling variant) with an analogous provenance record — rather than any new
DDeploy-side plumbing. DCompile must never select nodes, place services,
accept its own output, or create an active DMap; it only ever produces a
candidate DGraph value handed to this already-governed entry point.

## 17. Authority matrix

| Concern | Owner after DCompile |
|---|---|
| Source document | DScript |
| Compilation/lowering (given explicit mapping + resource + ABI + call-graph inputs) | DCompile — pure, produces a *candidate*, never authoritative on its own |
| D-Code content identity (`dcode.{uri,sha256,content_id}`) | Whatever externally built and digested the bytes — **not DCompile** (Section 5) |
| Application topology | DGraph (candidate from DCompile; authoritative only via DDeploy acceptance) |
| Service identity | DGraph/DServ — assigned by the explicit mapping input (Section 6-7), not DCompile's own invention |
| Calls | DCall — requires an explicit call-graph input; DCompile does not derive it from DScript alone |
| Executable identity | DCode/`CanonicalDServArtifactV1` — DCompile may produce the descriptor wiring, never the content digest |
| Operational artifact bridge | Existing `ServiceArtifact` path — untouched, unrelated |
| Placement | DMap — DCompile never selects nodes |
| DIoT provider topology | DForward — DCompile has no role (Section confirmed: canonical definition is DScript→DGraph+DCode only) |
| Runtime observation | DMonitor — DCompile must never consume live health |
| Host mutation | Governed reconciliation (Stage 107A/109) — entirely untouched |
| Proposal/acceptance entry point | Existing DDeploy `create_proposal`/`ProposalSource::Deterministic` — DCompile hands its candidate here, never bypasses it |

## 18. Gap matrix

| Required output/concern | From DScript v1 directly? | Available elsewhere today? | Missing? | Candidate source | Authority owner | 117B implication |
|---|---|---|---|---|---|---|
| `application_id` | Yes | — | No | DScript | DScript | none |
| `dgraph_id` | No | No | Yes | caller/policy | DCompile's caller | needs assignment convention |
| `dgraph_revision` | No | No | Yes | caller/policy | DCompile's caller | needs assignment convention |
| `service_id` (D-Serv IDs) | No | No | Yes | explicit mapping input | whoever supplies the mapping | new typed mapping contract |
| D-Serv grouping | No | Historical VS1 hint only (superseded) | Yes | explicit mapping input | whoever supplies the mapping | new typed mapping contract (Gap #2) |
| D-Node ABI requirement | No | Yes — sole legal constant `DNODE_ABI_V0` | No (auto-fillable) | DCompile itself | DCompile (justified, Section 11) | safe to auto-assign, always explicit |
| CPU requirement | No | No default anywhere | Yes | explicit resource input | whoever supplies resource policy | new typed input, never annotation-sniffed |
| Memory requirement | No | No default anywhere | Yes | explicit resource input | whoever supplies resource policy | new typed input, never annotation-sniffed |
| D-Call edges | No | No | Yes | explicit call-graph input | whoever supplies the call graph | may remain empty in 117B |
| D-Call IDs | No | No precedent | Yes | new deterministic scheme (TBD) | DCompile, once call-graph exists | open, not resolved here |
| D-Code module content (`dcode.{uri,sha256,content_id}`) | No | No — never computed anywhere in this repo | Yes | externally-built content, digested outside this repo | whoever built the content | out of Phase 117's realistic scope entirely |
| Artifact descriptor (wiring around content) | Partially — `dserv_id`/`application_id` derivable once mapping exists | `CanonicalDServArtifactV1` shape exists | Partially | DCompile (given mapping) + externally-supplied content identity | DCompile for wiring, external builder for content | 117D-scale work, needs content identity input first |
| Build provenance (toolchain/rustc/etc.) | No | No — obsolete VS1-only, never implemented | N/A | not required by any current contract | N/A | do not revive; not needed by `CanonicalDServArtifactV1` |
| Compiler identity/version | No | No | Yes, if a provenance record is wanted | new field, mirroring `PlannerProvenanceV1` | DCompile | recommended for auditability, not required for correctness |
| DScript source digest (for provenance) | Yes (`digest_sha256`) | — | No | `ValidatedDScriptInput` | DScript | trivially includable in any output provenance |
| DFunction→DServ mapping provenance | No | Historical VS1 hint only | Yes | explicit mapping input, recorded verbatim | DCompile output provenance | recommended (Section "mapping provenance"), not embedded in DGraph itself |

## 19. End-to-end data flow (current + hypothesized future, corrected from evidence)

```
datum.dscript/1 JSON
    -> decode_and_validate_dscript_json
    -> ValidatedDScriptInput { document, digest_sha256 }
    -> [explicit, separately-typed compile inputs, NOT YET DEFINED:
          - DFunction -> DServ mapping/grouping
          - per-service resource requirement (cpu_cores, memory_bytes)
          - D-Node ABI (auto-fillable today: DNODE_ABI_V0)
          - optional explicit call graph (else calls: [])
          - dgraph_id/revision assignment]
    -> pure, deterministic DCompile lowering (NOT IMPLEMENTED)
    -> candidate CanonicalDGraphV1 (calls likely empty in a first slice)
       + optional mapping/compiler provenance record (mirrors PlannerProvenanceV1)
    -> [D-Code descriptor wiring requires separately-supplied, already-built
        content identity -- NOT produced by DCompile itself]
    -> existing dserver::core::ddeploy::create_proposal
       (ProposalSource::Deterministic, existing mechanism, Section 16)
    -> existing DDeploy proposal/acceptance -> authoritative CanonicalDMapV1
    -> existing reconciliation -> DNode execution
```

Everything from `create_proposal` onward is **already implemented and
unchanged** by Phase 117 — DCompile's only job is to hand this pipeline a
well-formed candidate `CanonicalDGraphV1`, through the entry point that
already exists.

## 20. Historical/prototype findings

Full-history search (`git log --all`, grep and pickaxe on `DCompile`,
`dcompile`, `mapping_style`, `d_functions`) reconfirms Phase 116A's findings
with no new substantive evidence: the only non-noise hits are the VS1 sketch
commits (`07400c8`, `932f434`) and this project's own Phase 116 documentation
commits. Apparent "dcompile" pickaxe hits in unrelated commits (`b7df06b`,
etc.) were checked and are false-positive substring matches inside Windows
API feature-flag metadata (`"dcompanimation"` inside a `.cargo` cache blob),
not architectural evidence. Apparent "d_functions" hits in WASM-module-registry
commits are the same `allowe**d_function**s` false positive Phase 116A's own
audit already identified. **Classification: no new historical evidence
beyond what Phase 116A already found and disclaimed.** No commit, on any
branch, has ever added a parser, compiler, or lowering algorithm for
DScript→DGraph/DCode.

## 21. Open semantic questions

- **What do 1:N/M:1/M:N actually mean at the implementation level?**
  Genuinely undefined by any repository evidence. The VS1 example shows 1:N
  as "one DScript, two D-Servs, each owning a disjoint subset of functions" —
  i.e., **partition**, not duplication. Whether M:N implies a function can
  belong to *multiple* D-Servs simultaneously (shared/duplicated
  implementation) or whether M:N is always reducible to a partition-of-pairs
  is **not defined anywhere** and is marked OPEN, not resolved by this audit.
- **If a function maps to multiple D-Servs, is its implementation
  duplicated, shared, or wrapped?** No evidence. Given Section 5's finding
  that DScript carries no implementation at all, this question is currently
  moot for DScript itself, but becomes real once a content-input contract
  (Section 5's identified future need) exists — deferred, not resolved here.
- **What does an empty DScript (zero functions) compile to?** DGraph's own
  precedent (`empty_collections_validate_cleanly`) suggests a
  structurally-valid, empty `CanonicalDGraphV1` is legitimate; this audit
  recommends that behavior for 117B (a valid compile producing an empty
  DGraph, not a semantic rejection) but does not implement it.
- **Annotations and compilation** — per the task's own framing, this audit's
  finding is **(A): DCompile v1 should ignore opaque annotations entirely.**
  No evidence supports (B) typed-input-outside-annotations already existing,
  and (C) a future `datum.dscript/2` is speculative and out of scope. Any
  future non-functional requirement must come through an explicit new typed
  compile input (Section 12), never through interpreting an annotation
  string.

## 22. Recommended Phase 117B vertical slice

**Not implemented; evaluated only.** The smallest slice that closes ONE real
gap without inventing semantics: **a typed, explicit DCompile mapping/request
contract plus pure validation** — analogous to Phase 116B's own
`CanonicalDScriptV1` step. Concretely: a new type (name TBD, e.g.
`DCompileMappingV1`) that, given a DScript's identity, explicitly declares
which `function_id`s belong to which named service, plus (separately) each
named service's `cpu_cores`/`memory_bytes` — nothing about D-Code content,
D-Call edges, or DDeploy integration yet. This directly closes Gap #2
(Section 6) and the resource-requirement gap (Section 12), the two gaps that
block even the smallest useful DGraph-producing step, without touching the
Gap #1 (D-Code content) problem, which has no realistic evidence-based
solution yet and should not be forced.

A second-order candidate — deterministic DGraph lowering given both a
DScript and a mapping (producing a calls-empty, ABI-auto-filled candidate
`CanonicalDGraphV1`) — is the natural 117C once 117B's input contract exists,
but is explicitly NOT this audit's recommendation for the immediate next
step; 117B should establish the input contract first and prove it in
isolation (pure validation, no lowering) before any lowering code is written.

## 23. Proposed Phase 117 decomposition

Illustrative, derived from the gaps found:

```
117A — architecture/lowering audit                          (this document)
117B — explicit DFunction->DServ mapping + resource-requirement input contract
117C — deterministic, calls-empty candidate DGraph lowering (DScript + 117B mapping -> CanonicalDGraphV1)
117D — explicit call-graph input + DCall derivation (only once real evidence/need exists)
117E — D-Code descriptor wiring (requires a separate, not-yet-scoped content-identity input contract; may itself need its own sub-audit before implementation)
117F — DDeploy handoff (wire 117C's output through the EXISTING create_proposal/ProposalSource::Deterministic path, Section 16 — no new DDeploy mechanism)
117G — closure
```

117E in particular may warrant its own short audit (mirroring 117A's own
approach) before implementation, given Section 5's finding that no
build/content-identity input contract exists anywhere today — this is a
larger gap than a simple "add a field" step.

## 24. Explicit 117B exclusions

- No placement selection or DMap mutation of any kind.
- No DForward/DIoT generation (confirmed out of DCompile's canonical scope).
- No runtime execution, no DMonitor read or mutation.
- No automatic DDeploy proposal creation or acceptance.
- No automatic artifact authorization.
- No AI-based mapping, grouping, or resource-requirement inference (Phase
  123's domain; if AI ever proposes a mapping, it must flow through the
  same deterministic validation as a human-supplied one).
- No interpretation of any `CanonicalDFunctionV1.annotations` key or value
  for any purpose (ADR-0025, reaffirmed).
- No D-Code content generation, no digest fabrication, no `dcode.{uri,sha256,content_id}`
  invention.
- No new DDeploy-side proposal type or bypass of `create_proposal`.
- No `compile()`/`lower()`/`generate_dgraph()`/`generate_dcode()` function —
  117B is a typed input contract and its pure validation only, nothing that
  lowers anything yet.

## 25. Evidence index

- `documentation/adr/ADR-0001-canonical-datum-vocabulary.md:36` (D-Compile definition)
- `documentation/adr/ADR-0017-canonical-datum-contracts-v1.md:62-83` (VS1 sketch disclaimer)
- `documentation/adr/ADR-0021-canonical-dserv-artifact-and-dcode-wasm-v0.1.md`
- `documentation/adr/ADR-0025-dscript-v1-annotation-representation.md`
- `documentation/reference/phase116a-dscript-architecture-audit.md`, `phase116b-canonical-dscript-v1.md`, `phase116c-dscript-input-admission-and-closure.md`
- `dserver/src/core/canonical_dscript.rs`, `DATUM/src/agent/canonical_dscript.rs`
- `dserver/src/core/canonical_dgraph.rs:72-217`, `DATUM/src/agent/canonical_dgraph.rs`
- `dserver/src/core/dserv_artifact.rs:1-220` (full struct + validate() trace)
- `dserver/src/core/dcode_registry.rs:69` (`declare_dserv_artifact`)
- `dserver/src/api/dcode.rs:85,172` (declare/authorize endpoints)
- `DATUM/src/dcode/module_store.rs:38,99`, `DATUM/src/dcode/invoke.rs:201-354`
- `dserver/src/core/service_artifact_registry.rs` (ServiceArtifact, distinct lineage)
- `dserver/src/core/ddeploy.rs:128,149-160,219-234,1862,8056` (`canonical_dmap_id`, `ProposalSource`, `PlannerProvenanceV1`, `create_proposal`)
- `dserver/src/api/ddeploy.rs:308,332` (proposal creation from an inline `CanonicalDGraphV1`)
- `dserver/src/core/canonical_placement.rs:274` (`plan_canonical_placements` — pure deterministic-derivation precedent)
- `dserver/src/core/wasm_module_registry.rs`, `dserver/src/api/wasm_module_registry.rs`, `DATUM/src/wasm_runtime/{client,registry}.rs` (ruled out — unrelated legacy WASM catalog)
- `scripts/fixtures/datum-contracts/vs1/dgraph.sensor-pipeline.json`, `dserv-artifact.{alert,decode}.json` (historical 1:N mapping evidence, superseded)
- Git history: `git log --all` grep/pickaxe for `DCompile`, `dcompile`, `mapping_style`, `d_functions` — no new evidence beyond Phase 116A's findings; apparent hits verified as false positives.
