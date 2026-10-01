# Phase 117D — DCode Descriptor & Content-Identity Architecture Audit

**Status:** Phase 117D AUDIT CLOSED (audit only). **Phase 117 remains OPEN.**
**NO PRODUCTION DCODE OR DCOMPILE CODE WAS ADDED DURING PHASE 117D.**
**Branch:** `agent/phase117-dcompile`.
**Baseline:** Phase 117C closure, commit `ec4772f6321e5ed6919f9cb2f5efd52fe83af3e8`.

## 1. Executive conclusion

Under today's implemented architecture, "DCompile produces DCode" cannot
mean "compiles source into executable bytes" — that capability does not
exist anywhere in this repository (reconfirming Phase 117A). It also cannot
honestly mean "DCompile computes the module's content digest" — DCompile
never receives module bytes, and nothing in this codebase ever computes a
`sha256` from source; every digest either comes from a human-run CLI tool
(`smartsentinel-dcode-install`, reading a local `.wasm` file a human already
built and placed) or is asserted directly in a hand-authored descriptor. The
only truthful meaning left, and the one this audit recommends, is
**Option C**: DCompile consumes a narrow, explicit, externally-supplied
content-identity input (module digest + size, at minimum) and deterministically
constructs the remaining descriptor fields — most of which turn out to be
fixed, sole-legal-value constants already established by ADR-0021, not
open policy choices.

The most significant new finding this audit surfaces is a genuine,
previously-undocumented integration gap: **`dserver/src/core/ddeploy.rs`
already has a mature, real eligibility-checking mechanism against the
older `ServiceArtifact` (container/native-process) registry, but has
*zero* equivalent path for `CanonicalDServArtifactV1`/the DCode registry.**
A DCompile-produced candidate D-Graph containing WASM/D-Code services could
be accepted into an active D-Map through the existing proposal/acceptance
flow today *without DDeploy ever checking whether a corresponding D-Code
artifact exists at all*. This is a real, separate blocker from descriptor
construction, and this audit does not attempt to solve it.

A second major finding, equally load-bearing: `dcode_registry.rs`'s own
source comment states explicitly that **v0.1 has no artifact replacement/
version-transition mechanism at all** — one immutable artifact per
`(application_id, dserv_id)`, full stop. This means a future DCompile
cannot yet support recompiling and redeploying the SAME service under a
new executable — that capability is genuinely missing, not merely
unimplemented in DCompile.

## 2. Scope and evidence

Investigated: `dserver/src/core/dserv_artifact.rs` (full struct + validate()
trace), `dserver/src/core/dcode_registry.rs` (full trace),
`dserver/src/core/dcode_authorization.rs` (full trace of both
`authorize_dcode_invocation` and its re-verification sibling),
`dserver/src/core/dcall.rs` (module doc comment + port-resolution logic),
`dserver/src/core/ddeploy.rs`/`ddeploy_v2.rs` (grepped for DCode/ServiceArtifact
references), `DATUM/src/dcode/{module_store,invoke,client}.rs`,
`DATUM/src/bin/smartsentinel-dcode-install.rs`, ADR-0021, ADR-0025, the
Phase 117A/B/C documents, and full git history (`git log --all` pickaxe/
grep searches). No production code was modified. One research fork was
attempted for the DATUM-side byte-lifecycle and history questions; it
failed on a rate limit mid-run, so those questions were answered instead
by direct file reads and `git log` invocations in this same session —
every finding below is sourced from an actual file read or command output,
not from the failed fork's partial state.

## 3. Meaning of "DCompile produces DCode" today

Evaluating the five alternatives the task posed:

- **(A) DCompile constructs complete descriptors around externally-built
  content identities** — closest to correct, but "complete" overstates it:
  most fields are fixed constants (Section 6), only a few need real external
  input.
- **(B) DCompile validates/binds a caller-supplied COMPLETE descriptor** —
  rejected: this would make DCompile a rubber stamp with no actual lowering
  role, and would require the caller to already know every fixed-constant
  field DCompile itself is better positioned to assign deterministically and
  consistently.
- **(C) DCompile consumes a narrow typed external-content input and
  constructs the remaining safe/canonical fields** — **recommended.**
  Matches this project's own established pattern exactly: 117B/117C already
  do precisely this for the D-Graph (explicit mapping/resource input,
  deterministic construction of the rest, including the one fixed-constant
  field `dnode_abi`).
- **(D) DCompile produces a new intermediate descriptor-intent contract
  materialized later** — no evidence requires this extra layer yet; adds
  complexity without a demonstrated need. Revisit only if a real
  multi-step build pipeline emerges.
- **(E) another model** — not found.

**Conclusion: Option C**, mirroring 117B/117C's own already-proven
narrow-input/deterministic-construction shape.

## 4. Current CanonicalDServArtifactV1 contract

Unchanged since Phase 117A: `dserver/src/core/dserv_artifact.rs:44`,
`datum.dserv-artifact/1`, `#[serde(deny_unknown_fields)]`, fields
`{schema, artifact_id, version, application_id, dserv_id, dcode, abi,
ports, limits, safety, state_model, instantiation, constraints, qos,
provenance, probes, signature_ref}`. This document does not restate the
full struct shape (see Phase 117A Section 4 or the file itself); Section 5
below covers every field individually.

## 5. Field-source matrix

| Field | Validation rule | Available from 117C output? | Fixed legal value? | External input required? | Current production consumer | Proposed future owner | 117E implication |
|---|---|---|---|---|---|---|---|
| `schema` | exact match `datum.dserv-artifact/1` | No (constant) | **Yes** | No | `validate()` | DCompile (literal) | assign directly |
| `artifact_id` | non-empty | No | No | **Yes** — human/tool-chosen | registry lookup does NOT key on it | external input (Section 9) | pass through only |
| `version` | non-empty | No | No | **Yes**, but see Section 9 — no operational authority | none (display/provenance only) | external input | pass through only, never treated as a replacement key |
| `application_id` | non-empty, matches D-Graph | **Yes** (from D-Script via 117C's candidate) | No | No — derivable | `dcode_authorization` (registry key) | DCompile (from D-Graph/D-Script) | assign from 117C context |
| `dserv_id` | non-empty, matches D-Graph | **Yes** (= the 117C-generated `service_id`) | No | No — derivable | `dcode_authorization` (registry key) | DCompile | assign from 117C's generated `service_id` |
| `dcode.uri` | `datum-blob://sha256/<64hex>`, digest-consistent | No | No (syntax fixed, value not) | **Yes** — the digest itself | `validate()`, invocation load | external input (Section 10) | derive syntactically from the supplied digest |
| `dcode.sha256` | 64 lowercase hex | No | No | **Yes**, mandatory | invocation byte verification | external input | never fabricated |
| `dcode.content_id` | `sha256:<64hex>`, must equal `sha256` | No | No (derivable FROM sha256) | No — derivable | `validate()` cross-check | DCompile (derive from supplied `sha256`) | derive, don't require twice |
| `dcode.size_bytes` | any `u64`, checked against real bytes at invoke time | No | No | **Yes** — DCompile never sees bytes | `invoke.rs` R3 check | external input | never fabricated |
| `abi.name` | must combine to `datum-dnode/0` | No | **Yes** (`"datum-dnode"`) | No | `validate()` | DCompile (literal) | assign directly |
| `abi.version` | must combine to `datum-dnode/0` | No | **Yes** (`"0"`) | No | `validate()` | DCompile (literal) | assign directly |
| `abi.required_exports` | must equal the exact canonical 5-export set, no more/fewer/dupes | No | **Yes** (`DNODE_ABI_V0_REQUIRED_EXPORTS`) | No | `validate()` | DCompile (literal) | assign directly |
| `abi.allowed_imports` | must be exactly empty | No | **Yes** (`[]`) | No | `validate()` | DCompile (literal) | assign directly |
| `ports.inputs`/`ports.outputs` | each port non-empty `port`/`payload_schema` | No | No | **Yes**, if any exist | `dcall.rs`'s port RESOLUTION (Section 13) | external input (content-author-declared interface) | pass through; NOT derivable from D-Script |
| `limits.fuel_per_invocation` | `> 0` | No | No | **Yes** — no canonical default | `invoke.rs` fuel accounting (implied) | external input | never inferred from D-Graph resources |
| `limits.timeout_ms` | `> 0` | No | No | **Yes** | invocation timeout enforcement (implied) | external input | never inferred |
| `limits.max_memory_pages` | `> 0` | No | No | **Yes** | Wasm instance sizing (implied) | external input | never derived from `resource_requirement.memory_bytes` (Section 17) |
| `limits.max_message_bytes` | `> 0` | No | No | **Yes** | message-size enforcement (implied) | external input | never inferred |
| `limits.max_emits_per_invocation` | `> 0` | No | No | **Yes** | emit-count enforcement (implied) | external input | never inferred |
| `safety.filesystem_access` | must be `false` | No | **Yes** | No | `validate()` | DCompile (literal `false`) | assign directly |
| `safety.network_access` | must be `false` | No | **Yes** | No | `validate()` | DCompile (literal `false`) | assign directly |
| `safety.host_command_access` | must be `false` | No | **Yes** | No | `validate()` | DCompile (literal `false`) | assign directly |
| `safety.docker_access` | must be `false` | No | **Yes** | No | `validate()` | DCompile (literal `false`) | assign directly |
| `safety.side_effects_allowed` | must be `false` | No | **Yes** | No | `validate()` | DCompile (literal `false`) | assign directly |
| `state_model` | must be `"stateless"` | No | **Yes** | No | `validate()` | DCompile (literal) | assign directly |
| `instantiation` | must be `"fresh_instance_per_invocation"` (ADR-0008) | No | **Yes** | No | `validate()` | DCompile (literal) | assign directly |
| `constraints.allowed_stages` | structurally any `Vec<String>`, but non-empty is UNVERIFIABLE (Section 8 below) | No | **Yes, effectively** (`[]` — only value that is currently ever executable) | No | `dcode_authorization::F_DCODE_ARTIFACT_STAGE_CONSTRAINT_UNVERIFIABLE` | DCompile (literal `[]`) | never generate a non-empty value |
| `qos` | none (`Value`, uninterpreted) | No | No | Optional | none found | leave empty/caller-supplied only if ever needed | never used for DCompile provenance |
| `provenance` | none (`Value`, uninterpreted) | No | No | Optional | none found | leave empty/caller-supplied only if ever needed | never smuggle `dscript_digest`/`dcompile_request_digest` in here (Section 16) |
| `probes` | none (`Value`, uninterpreted) | No | No | Optional | none found (not readiness/DMonitor-integrated) | leave empty | not DCompile's concern |
| `signature_ref` | none (`Option<String>`, unenforced) | No | No | Optional | none found — not verified anywhere | leave `None` unless a real signing system exists | do not invent |

## 6. Fixed canonical values vs explicit inputs

**True fixed constants** (DCompile may assign explicitly and literally,
never framed as a "default" since no second legal value exists,
mirroring 117C's `DNODE_ABI_V0` precedent exactly): `schema`, `abi.name`,
`abi.version`, `abi.required_exports`, `abi.allowed_imports`, every
`safety.*` flag, `state_model`, `instantiation`, and — for anything
DCompile itself would ever generate — `constraints.allowed_stages = []`
(Section 8). Each is confirmed by a direct `validate()` rule rejecting
every other value, not by inference.

**Genuinely required explicit external input, with no canonical default
anywhere:** `dcode.sha256`, `dcode.size_bytes` (DCompile never sees bytes,
so it cannot compute either), every `limits.*` field (Section 7), and
`ports.*` if the module declares any (Section 9). `artifact_id`/`version`
are also externally supplied but carry no operational authority (Section
10).

**Derivable from other explicit input, not separately required:**
`dcode.uri` and `dcode.content_id` are syntactically fixed transformations
of `dcode.sha256` (`datum-blob://sha256/<hex>` and `sha256:<hex>`
respectively) — a future narrow input contract need only supply the raw
digest once; DCompile can construct both wire fields from it without
becoming content-identity authority (it is not inventing a digest, only
formatting an already-supplied one).

## 7. DCode content identity

The four `dcode.*` fields form one enforced invariant
(`dserv_artifact.rs:165-198`, read directly): `sha256`, `content_id`, and
the digest embedded in `uri` must all name the **identical** digest. The
minimal external input is therefore just the raw `sha256` digest plus
`size_bytes` — `uri` and `content_id` are syntactic derivations, not
independent facts. A future narrow content-identity input contract should
accept exactly `{sha256, size_bytes}` (matching the module's real content)
and let DCompile construct `uri`/`content_id` deterministically — this is
formatting, not identity authority, exactly as recommended in Section 6.

## 8. Module-byte lifecycle and datum-blob URI

Traced directly: `DATUM/src/bin/smartsentinel-dcode-install.rs` is the
**only** production caller of `DCodeModuleStore::install` outside test
code. It is a manual CLI tool: `fs::read(&args.module_path)` reads a
**local file path a human operator has already placed on the D-Node's
filesystem**, verifies it against a `--declared-sha256` argument, and
writes it into the local content-addressed store. There is no network
fetch anywhere in this call chain.

`datum-blob://` was grepped across the entire `dserver`/`DATUM` source
tree: every occurrence outside `dserv_artifact.rs`/`dcode_registry.rs`/
`dcode_authorization.rs`'s own test fixtures is itself test code
constructing a fixture string. **No HTTP handler, resolver, or client
anywhere in this codebase ever dereferences a `datum-blob://` URI as a
network location.** It is purely a canonical, structurally-validated
*logical* identity string — never an actually-retrievable resource.
**Conclusion: `datum-blob://` construction, and by extension a future
DCompile assigning it, does not and cannot "publish" bytes anywhere. Byte
distribution from an external build to a D-Node remains an entirely
separate, today-manual, operational concern this phase must not silently
absorb into DCompile's scope** (Section 20 of the task; see also Section
21 below on the ADR-0021 disclosure this was always known).

`size_bytes` IS verified — but only at invocation load time, on the D-Node,
against bytes ALREADY locally present (`DATUM/src/dcode/invoke.rs:585`,
"R3: declared size_bytes must match the actual loaded module length"). It
is never computed by DCompile, which never possesses bytes at all.

## 9. Artifact ID and version semantics

Both traced directly against `dcode_registry.rs`'s own explicit source
comment (line 58-68, quoted verbatim): "the registry key is
`(application_id, dserv_id)` — NOT `artifact_id`/`version` — so a caller
cannot 'declare a real content change' merely by changing `artifact_id`/
`version` while the `(application_id, dserv_id)` pair stays the same; ANY
different descriptor at that key conflicts, regardless of what its
`artifact_id`/`version` say." **Conclusion: `artifact_id` and `version`
carry zero lookup, replacement, or authority semantics today — they are
descriptive/provenance-only strings.** No automatic composition precedent
exists for `artifact_id`, and no evidence supports inventing a
digest-derived one (matching this project's universal human/tool-chosen-
identity convention, already established in Phase 117A/B).

## 10. Registry lifecycle and replacement problem (P0)

**This is the single most consequential finding of this audit.** Given
`application_id = A`, `dserv_id = S` already registered with module digest
`H1`, and a later recompilation of the same logical service producing
`H2`:

`declare_dserv_artifact` (`dcode_registry.rs:69-91`, read directly):
looks up the existing entry by `(A, S)`; if the new descriptor is
byte-identical (after sorting `abi.required_exports`/`allowed_imports` for
order-insensitivity) it is a no-op; **if it differs in ANY way — including
only `dcode.sha256` changing from H1 to H2 — it returns
`DeclareDServArtifactError::Conflict`, unconditionally.** The function's own
doc comment states this outright: "v0.1 has no artifact replacement/
version-transition mechanism at all — one immutable registered canonical
artifact per `(application_id, dserv_id)`, full stop; redeployment/migration
to a new artifact is an explicitly future concern, not implemented here."

**This is not a bug or an oversight this audit discovered — it is an
already-known, already-documented v0.1 scope boundary, confirmed by direct
git history search** (`git log --all --grep="artifact.*replac|artifact.*version|artifact.*upgrade|redeploy|recompile" -i` surfaced only two unrelated hits — `d4af4ee` about an unrelated legacy `artifact_reconciliation_context` schema version field, and `1b3cb56` about native-PROCESS respawn evidence, neither about D-Code artifact replacement). **No historical evidence anywhere shows this limitation was ever intended to be solved incidentally by DCompile.**

Evidence-supported alternatives, per the task's own list, assessed without
selecting one:

- **(A) service_id must change on every executable revision** — would work
  today with zero registry changes (a new `(A, S')` key registers cleanly),
  but conflicts with Section 12's finding that D-Graph revision cannot
  otherwise distinguish old/new D-Code for a logically-stable service — it
  would force D-Graph's D-Serv identity to encode executable revision,
  which 117B/117C never intended (`service_id` is explicit, human-chosen,
  logical identity, not a content-derived label).
- **(B) the registry needs an explicit revision/replacement lifecycle** —
  the most architecturally consistent option with how D-Map/D-Deploy
  already handle revision transitions (proposal → acceptance → new active
  revision), but is a real, nontrivial design task of its own, not a
  byproduct of descriptor construction.
- **(C) active D-Graph revision somehow selects artifact version** — not
  supported by any current evidence; `CanonicalDGraphV1` carries no artifact
  reference of any kind (confirmed, Section 3 of Phase 117A/C), so this
  would require a genuinely new cross-contract mechanism.
- **(D) immutable services are intentional** — cannot be ruled out from
  evidence alone; the doc comment's own phrasing ("an explicitly future
  concern") suggests immutability was accepted as a real v0.1 constraint,
  not asserted as a permanent design position either way.
- **(E) another design** — not identified.

**This audit does not choose among these.** It flags the choice as a
necessary prerequisite decision for any DCompile work that assumes a
service can be recompiled and redeployed — which is a completely ordinary,
expected operational scenario, not an edge case.

## 11. DGraph revision vs DCode revision

Direct consequence of Sections 3 and 10: `CanonicalDGraphV1` contains no
artifact reference, artifact version, or D-Code digest of any kind
(unchanged fact, reconfirmed). Therefore **a new D-Graph revision
referencing the same `service_id` cannot, by any mechanism in this
architecture today, express "this revision expects different D-Code than
the last one."** The registry's `(application_id, dserv_id)` key means
executable identity is currently, functionally, tied to the SERVICE's
identity, permanently, for as long as that service exists — not to any
D-Graph revision. This is the same fact Section 10 already surfaces from
the registry side; here it is confirmed from the D-Graph side too, closing
the loop: neither contract offers an escape hatch.

## 12. Service-ID stability

Given Section 11, the current registry architecture **forces** the
interpretation that `service_id` is logical, stable identity that outlives
individual executable revisions — the ALTERNATIVE interpretation (`service_id`
must encode executable revision/content identity) would make every
recompilation require a brand-new service identity, which 117A/117B never
intended and which would make D-Map/D-Deploy placement history nonsensical
(a "new service" on every rebuild). **This is a major, previously-unstated
cross-contract tension: 117B/117C treat `service_id` as stable logical
identity; the DCode registry's own immutability makes that stability
impossible to honor across a real recompilation today.** Resolving it is
exactly the P0 problem in Section 10 — not something this audit can settle
by declaring an interpretation; it can only name the tension precisely.

## 13. Ports

Direct, decisive evidence found in `dserver/src/core/dcall.rs`'s own module
doc comment (lines 15-24, quoted): "`datum.dgraph/1`'s `CanonicalDCallV1`
... carries NO port field ... ports are exclusively a `datum.dserv-artifact/1`
(ADR-0021) concept. The destination input port is therefore **resolved, not
invented**: it is the destination artifact's declared input port whose
`payload_schema` matches the source's emitting output port's `payload_schema`,
and v0.1 REQUIRES this match to be unique (ambiguous or absent match fails
closed)."

**This proves ports are consumed by real production code (`derive_dcall_route`),
but strictly downstream of an ALREADY-EXISTING `CanonicalDCallV1` edge**
(source/target service already known from the D-Graph's `calls` list).
Ports resolve WHICH declared port on an already-determined destination
receives a message — they are never used to determine WHETHER two services
are connected in the first place, and no code anywhere infers a `calls`
edge from matching `payload_schema` values across two artifacts. **Answering
the task's Q19 directly: ports are NOT a source of D-Call topology/edge
existence. They are local interface metadata that becomes relevant only
after a D-Call edge already exists via some other authority** (today,
hand-authored; Phase 117A/C already established DScript/DCompile v1 supply
no such edges). Whether `ports.inputs`/`ports.outputs` are required at all
for a service to be structurally valid: no — `DServArtifactPortsV1` derives
`Default` and both lists may legitimately be empty.

## 14. Limits

`limits.*` (`fuel_per_invocation`, `timeout_ms`, `max_memory_pages`,
`max_message_bytes`, `max_emits_per_invocation`) each require only `> 0`
(`dserv_artifact.rs:253-266`, read directly) — no canonical default value
is defined by any ADR or accepted fixture convention; the one checked-in
production-shaped fixture uses specific concrete numbers
(`fuel_per_invocation: 5_000_000`, `timeout_ms: 200`, etc.) that are
example/test values, not declared canonical defaults anywhere. Different
services legitimately need different limits (a heavier D-Function needs
more fuel/memory than a trivial one) — **there is no evidence any code
derives these from `CanonicalDGraphV1.resource_requirement`, and doing so
would conflate two different concepts** (Section 17). **Conclusion: limits
must be explicit, per-service content/policy input — never defaulted,
never derived.**

## 15. Safety / ABI / state / instantiation

All confirmed, by direct `validate()` inspection, to be sole-legal-value
fixed constants (Section 6's table) — not policy defaults, because a
zero-import v0.1 module (itself a hard-enforced invariant:
`abi.allowed_imports` must be exactly empty) has, per the validator's own
message, "no possible mechanism to exercise" any of the five safety
capabilities. `state_model = "stateless"` and
`instantiation = "fresh_instance_per_invocation"` are likewise the sole
values `validate()` accepts, the latter citing ADR-0008 by name in the
error message itself.

## 16. Constraints / qos / provenance / probes / signature

- **`constraints.allowed_stages`**: structurally legal non-empty, but
  `dcode_authorization.rs`'s own `F_DCODE_ARTIFACT_STAGE_CONSTRAINT_UNVERIFIABLE`
  finding (with an extensive, explicit doc comment, lines 42-55, quoted in
  part: "canonical D-Node structural authority ... carries NO authoritative
  stage identity ... v0.1 fails closed: a non-empty `allowed_stages` cannot
  currently be verified at all, so authorization is refused") means a
  non-empty value makes an otherwise-valid artifact permanently
  unexecutable under today's architecture. **DCompile must never generate
  a non-empty value** — confirmed as the task instructed, and additionally
  confirmed this is not merely a style preference but a hard
  never-executable consequence.
- **`qos`/`provenance`/`probes`**: all `serde_json::Value`, and a grep
  across all of `dserver`/`DATUM` production code found **zero** reads,
  validations, interpretations, or uses of any of the three beyond their
  own type declaration. `probes` in particular has no connection to
  DMonitor/readiness/reconciliation anywhere (confirmed by absence, not
  merely unchecked). **Conclusion: none should be used to smuggle DCompile
  provenance.** The task's own explicit warning is directly validated:
  `provenance`'s name is coincidental to Phase 117's own provenance
  concerns (D-Script digest, D-Compile-request digest) — using it for that
  purpose would create an uninterpreted, unenforced, easily-desynchronized
  shadow record of exactly the kind ADR-0025 already warned against for
  DScript annotations. If compiler provenance deserves a real, verified
  contract, it needs its own typed field or sibling structure, not this
  catch-all.
- **`signature_ref`**: `Option<String>`, never read or enforced anywhere in
  production code (confirmed by grep). Honestly classified as
  **unused/unenforced optional metadata** — no signing system exists; a
  future DCompile should leave it `None` rather than inventing a signature
  reference.

## 17. DGraph-resource vs artifact-limit relationship

No canonical relationship exists, confirmed by absence: `dserv_artifact.rs`'s
own module doc comment states its validation is "structural, self-contained
... never touches the D-Graph" (already known from Phase 117A), and direct
inspection of every `limits.*` validation rule confirms none references
`resource_requirement` in any way. These describe genuinely different
concepts: `resource_requirement.{cpu_cores, memory_bytes}` is a
**deployment capacity requirement** (how much host capacity a service needs
to be placed, D-Map's concern) — `limits.*` is an **invocation sandbox
bound** (how much a single WASM call may consume before being cut off,
enforced per-invocation at the D-Code runtime layer). A service needing
many CPU cores for concurrent invocations could still have a tiny
per-invocation `fuel_per_invocation`; the two numbers answer unrelated
questions. **No conversion should ever be invented between them.**

## 18. Registry declaration semantics (reconfirmed)

Reconfirms Phase 117A: `declare_dserv_artifact` validates the descriptor's
own structural correctness and registry-key conflict status only — nothing
about registration checks D-Graph membership, D-Map acceptance, or compile
provenance. An artifact may be declared before OR after a D-Graph/D-Map
referencing its `(application_id, dserv_id)` exists — the registry and
D-Deploy are, today, entirely uncoordinated (Section 20). Registration
alone never makes anything executable (Section 19 reconfirms this from the
authorization side).

## 19. Invocation authorization chain

Traced directly and completely (`dcode_authorization.rs:109-277`,
`authorize_dcode_invocation`), in exact order:

1. Application has an active, accepted canonical D-Map (`active_placement_authority`).
2. The active authority's D-Graph snapshot exists.
3. `service_id` exists in that D-Graph snapshot, and its `execution_requirement.dnode_abi == DNODE_ABI_V0`.
4. The active D-Map places that `service_id` on the requesting `node_id`.
5. The requested D-Node is a currently-declared structural D-Node that supports `DNODE_ABI_V0`.
6. A `CanonicalDServArtifactV1` is registered for `(application_id, service_id)`.
7. That artifact independently validates cleanly (its own `validate()`, re-run every time — never trusted from prior registration).
8. The artifact's ABI matches `DNODE_ABI_V0`, and `constraints.allowed_stages` is empty (else fails closed, Section 16).

The result, `DCodeInvocationAuthorizationV1`, binds
`dmap_ref`/`dgraph_ref` (each `{id, revision, digest}`),
`dserv_artifact_id`, `dserv_artifact_digest` (module content digest only)
and — per an explicit "independent review v2" correction in the source —
a SEPARATE `dserv_artifact_descriptor_digest` (the whole descriptor's own
JCS digest, so a change to `limits`/`ports`/`safety`/`abi` is caught even
if the module bytes stay identical), and `dnode_abi`.

**Directly answering the task's Section 20/Q21: neither a D-Script digest
nor a D-Compile-request digest appears anywhere in this authorization
chain today.** The chain is built entirely from D-Map/D-Graph/D-Node/
artifact-registry state — it has no concept of "what compiled this" at
all. Whether that provenance should ever be added to the EXECUTION
authorization chain itself (vs. kept as a separate, non-authorizing audit
record) is an open question this audit does not answer — adding it would
be a new authority-chain change, out of scope for a pure audit, and no
evidence yet establishes it is required for correctness (only for
after-the-fact auditability).

`verify_dcode_authorization_still_current` (not fully re-quoted here,
confirmed to exist at `dcode_authorization.rs:283`) re-derives this exact
same chain and fails closed the moment anything has moved — the same
"recompute and compare, never trust a cached snapshot" discipline this
whole project applies everywhere (Stage 107A, 117C's own wrapper
revalidation).

## 20. DDeploy integration reality (major finding)

Grepped directly and exhaustively: **`dserver/src/core/ddeploy.rs`,
`ddeploy_v2.rs`, and `dserver/src/api/ddeploy.rs` contain ZERO references
to `dcode_registry`, `CanonicalDServArtifactV1`, or `dserv_artifact` of any
kind.** DDeploy's proposal/acceptance flow can accept a candidate D-Graph
containing WASM/D-Code services with no knowledge of, or dependency on,
whether ANY corresponding D-Code artifact is registered, valid, or even
exists. This is a completely separate, currently-real integration gap from
descriptor construction — accepting a D-Graph and being able to actually
invoke its services are, today, unconnected authorities for the D-Code/WASM
runtime kind specifically (Section 21 explains why this is different for
the older `ServiceArtifact` lineage).

## 21. ServiceArtifact separation (reconfirmed, with a new contrast)

Reconfirms Phase 117A's finding that `ServiceArtifact` (container/
native-process, `service_artifact_registry.rs`) and `CanonicalDServArtifactV1`
(D-Code/WASM, ADR-0021) are separate, unrelated lineages. **New finding
this audit adds:** unlike D-Code, `ServiceArtifact` DOES have a real DDeploy
integration — `ddeploy.rs` explicitly implements "the `ServiceArtifact`-
derived eligibility view" at proposal-planning time, and re-validates
"current `ServiceArtifact` authority" at acceptance time
(`F_DDEPLOY_V2_ARTIFACT_AUTHORITY_STALE`, confirmed by direct grep and
reading of the surrounding comments). **This asymmetry is itself evidence
that D-Code/WASM's DDeploy integration (Section 20) is a genuine gap, not
an intentional permanent design** — the container/native-process runtime
kind already proves this project's architecture is capable of, and expects,
exactly this kind of artifact-eligibility integration at the D-Deploy
layer; D-Code/WASM simply does not have its equivalent yet.

## 22. Provenance identity matrix

| Identity | Meaning | Changes when... | Independent of... |
|---|---|---|---|
| D-Script digest | logical source (function set + opaque annotations) | any function added/removed/renamed, or any annotation key/value changes | compile mapping, D-Graph, D-Code |
| D-Compile-request digest | mapping/resource/output-identity intent | grouping changes, resource values change, or output D-Graph identity changes | D-Script's own annotations (117C, C11); D-Code |
| D-Graph digest | compiled application topology | service set, resources, or output identity changes | which D-Function mapped to which service (117C, C12); D-Code entirely |
| D-Code module digest (`dcode.sha256`) | exact executable bytes | the module is rebuilt with different content | descriptor metadata (`limits`/`ports`/`safety`/`abi`), which service it's bound to |
| D-Serv artifact descriptor digest (`recompute_dserv_artifact_digest`) | exact executable metadata/policy, whole descriptor | ANYTHING in the descriptor changes, including `limits`/`ports`/`safety`/`abi`, even with the SAME module bytes | nothing — it is the most granular of the five |

## 23. Independence cases (Section 38 of the task)

- **Case A (same bytes, different limits):** module digest unchanged;
  descriptor digest changes — directly confirmed by
  `dcode_authorization.rs`'s own "independent review v2 (R2A)" comment,
  which exists specifically to prevent this exact scenario from going
  undetected.
- **Case B (same D-Graph, different D-Function mapping):** already proven
  by 117C's own mapping-provenance collision test — D-Graph digest
  identical, D-Compile-request digest differs.
- **Case C (same D-Graph/service identity, different D-Code bytes):**
  this is exactly the Section 10 P0 problem — today, `declare_dserv_artifact`
  refuses the new bytes outright as a `Conflict`; there is no lifecycle
  path for this to succeed at all.
- **Case D (same D-Code bytes, different service binding):** the descriptor
  digest would differ because `application_id`/`dserv_id` are themselves
  part of the descriptor and its digest — binding the same bytes to a
  different service produces a structurally different, differently-keyed
  descriptor, registered independently. No conflict, no special handling
  needed — the registry key already handles this correctly today.
- **Case E (D-Script annotation change only):** proven directly by 117C's
  own `c11_annotation_changes_do_not_affect_generated_dgraph` — D-Script
  digest changes, D-Graph and (by extension, since D-Code has no
  dependency on D-Script at all today) D-Code both remain unaffected.

## 24. Lifecycle matrix

| Entity | Creator/producer | Storage location | Identity | Mutability/replacement | Authority role | Currentness rule |
|---|---|---|---|---|---|---|
| D-Code bytes | external build pipeline / human (never this repo) | D-Node-local filesystem, content-addressed by digest | `sha256` of actual bytes | immutable once installed at a given digest; a NEW digest is a NEW, unrelated store entry | none by itself | "current" is undefined — no registry of "the" bytes for a service, only of registered digests |
| Module digest | whoever built the bytes; verified (not computed) by `smartsentinel-dcode-install`/`invoke.rs` at every use | embedded in `dcode.sha256`/`content_id`/`uri` | itself | immutable | proves content identity, never trusted without recomputation from real bytes | always current relative to whatever bytes were actually loaded |
| Artifact descriptor | human/tool author today; a future DCompile per Section 3's Option C | `DCodeArtifactRegistryStateV1` (dserver, persisted) | `(application_id, dserv_id)` key + own descriptor digest | **immutable per key in v0.1 — Section 10's P0** | none by itself (Section 18) | "current" = whatever is registered at that key; no revision concept |
| Descriptor digest | `recompute_dserv_artifact_digest`, always freshly computed | not persisted as a field; recomputed on demand | whole-descriptor JCS digest | changes whenever the descriptor does | binds `DCodeInvocationAuthorizationV1` to an exact descriptor state | recomputed and compared at every authorization/verification |
| Registry record | `declare_dserv_artifact` | `DCodeArtifactRegistryStateV1` persisted via `storage::control_state` | registry key | immutable per key | required precondition for authorization (Section 19 step 6) | exists or doesn't; no staleness concept of its own |
| Active D-Graph | D-Deploy acceptance (unrelated to DCode) | `DDeployStateV1`'s active authority snapshot | `{dgraph_id, revision, digest}` | replaced wholesale on a new accepted proposal | defines which `service_id`s exist and their ABI requirement | current = whatever D-Deploy currently has active |
| Active D-Map | D-Deploy acceptance | `DDeployStateV1` | `{dmap_id, revision, digest}` | replaced wholesale on acceptance | defines placement | current = whatever is active |
| Invocation authorization | `authorize_dcode_invocation`, computed fresh per call | not persisted as a standing record (confirmed: no evidence of a stored authorization table beyond momentary use) | derived from the full chain | re-derived every time, never cached across a mutation | proves ONE invocation is currently permitted | `verify_dcode_authorization_still_current` re-derives and fails closed on drift |
| D-Node module-store copy | `smartsentinel-dcode-install` (manual) or (in principle) any future automated installer | D-Node-local filesystem | digest-keyed path | one file per digest, never mutated in place | makes bytes locally loadable | verified against the declared digest at every load, not merely at install |

## 25. Authority matrix (refined)

| Concern | Current owner |
|---|---|
| D-Function source identity | D-Script |
| Mapping/resource compile input | D-Compile-request (117B) |
| Generated topology (candidate) | D-Compile lowering (117C) — a candidate, never authority |
| Module byte identity | content SHA-256, established externally, verified (never computed) by DATUM |
| D-Code descriptor | `CanonicalDServArtifactV1` |
| Descriptor registry | `DCodeArtifactRegistryStateV1` (immutable per key, Section 10) |
| Placement | D-Map (via D-Deploy) — **not currently coordinated with the D-Code registry at all** (Section 20) |
| Invocation authorization | the D-Code authorization chain (Section 19) — the only place all of D-Map/D-Graph/D-Node/artifact identity are proven together |
| Installed module bytes | D-Node-local `DCodeModuleStore`, populated manually today (Section 8) |
| Runtime observation | D-Monitor |
| Host mutation | governed reconciliation |

A future DCompile may PRODUCE a candidate descriptor (Section 3, Option C)
without becoming authority for any of: content identity (it never computes
a digest), registration (declaring is a separate, explicit step, exactly
like 117C never calls `create_proposal`), placement, or invocation
authorization.

## 26. Current data flow

```
human/CI builds a .wasm module externally (NOT in this repo)
    -> human/tool computes its sha256 + knows its size
    -> human/tool hand-authors a full CanonicalDServArtifactV1 JSON
       (including all the "fixed constant" fields, today typed by hand)
    -> POST .../dserv-artifacts -> declare_dserv_artifact
       (validates + registers at (application_id, dserv_id), immutable)
    -> [SEPARATELY, uncoordinated: DDeploy accepts a D-Graph/D-Map
        containing the same service_id, with NO cross-check against
        the DCode registry -- Section 20]
    -> operator manually runs smartsentinel-dcode-install on the D-Node
       (reads the local .wasm file, verifies against the declared digest,
       writes it into the local DCodeModuleStore)
    -> a D-Node requests invocation
    -> authorize_dcode_invocation proves the full chain (Section 19)
    -> DATUM invoke_dcode loads bytes from the local store, re-verifies
       sha256 AND size_bytes AND the whole descriptor digest, then executes
```

## 27. Future DCompile-integrated data flow (smallest plausible, evidence-bounded)

```
externally-built Wasm module (still built entirely outside this repo)
    -> external tool supplies: {sha256, size_bytes} + any real ports/limits
       policy the module actually needs
    -> [Phase 117E, NOT implemented here] a narrow, explicit, typed
       DCode-content/policy input, one per generated service
    -> pure descriptor CANDIDATE construction:
         fixed constants assigned literally (schema/abi/safety/state_model/
         instantiation/empty allowed_stages)
         + application_id/dserv_id from the 117C candidate D-Graph
         + content identity/limits/ports from the explicit input
    -> CanonicalDServArtifactV1::validate() (existing validator, unchanged)
    -> [separate, explicit step, NOT automatic] declare_dserv_artifact
    -> [Phase 117D's OWN finding, Section 20 -- still unresolved]
       some future, currently-nonexistent D-Deploy <-> DCode-registry
       eligibility check, mirroring ServiceArtifact's existing one
    -> existing D-Deploy proposal/acceptance (unchanged)
    -> existing D-Code authorization chain (unchanged)
    -> existing, still-manual D-Node byte installation (Section 8 --
       Phase 117 does not solve this)
    -> invocation (unchanged)
```

This flow is corrected against actual code, not idealized: it explicitly
shows the Section 10 replacement problem and the Section 20 DDeploy gap as
still-open holes, not resolved steps.

## 28. Candidate Phase 117E slices

- **E1 — typed external DCode-content/policy input contract only** (mirrors
  117B's own pattern exactly: `{sha256, size_bytes, limits, ports?}` per
  service, pure validation, no descriptor construction yet). Smallest
  possible slice; closes zero of the actual generation gap by itself but
  establishes the input boundary cleanly, matching 117B's own precedent of
  separating "define the input contract" from "use it to construct
  something."
- **E2 — pure `CanonicalDServArtifactV1` candidate construction** from E1's
  input plus 117C's candidate D-Graph (fixed constants assigned, content
  identity/limits/ports copied verbatim, `application_id`/`dserv_id` bound
  from the matching generated service). Directly analogous to 117C's own
  `lower_calls_empty_dgraph_candidate`. Does not register or touch DDeploy.
  **This is the evidence-grounded continuation of DCompile's own pure-lowering
  precedent (117C) into the D-Code dimension.**
- **E3 — bind/validate a complete, externally-authored
  `CanonicalDServArtifactV1` against 117C's D-Graph** (Option B from Section
  3, already rejected there as adding no real value over what a caller
  could already do by hand).
- **E4 — registry lifecycle/replacement hardening** (solving Section 10's
  P0 before any DCompile artifact work). Real, necessary eventually, but
  is a DCode-registry-lifecycle decision independent of DCompile itself —
  DCompile can be built against the CURRENT immutable-registry reality
  (E1+E2) without first solving replacement, as long as E2 never assumes
  replacement works.

**Evaluated against the task's own criteria (evidence support, scope,
risk, prerequisite gaps, authority creation, material Phase 117 progress):**
E1+E2 together are the smallest slice with real evidence support, no
authority creation (pure candidate construction, exactly like 117C), and
material progress (DCompile can now produce something D-Code-shaped, not
merely D-Graph-shaped). E3 adds no value. E4 is important but is a
registry-lifecycle decision, not a DCompile-lowering one, and forcing it
into "Phase 117" scope would conflate two different kinds of work; it is
better tracked as its own explicit prerequisite, addressed when redeployment/
recompilation is actually attempted, not before.

## 29. Recommended Phase 117E vertical slice

**E1 followed immediately by E2**, kept as two small, separately-testable
steps but implemented in the same phase (mirroring how 117B and 117C were
sequenced as separate phases only because the request contract itself was
substantial — here, E1 alone is small enough that splitting it into its
own phase would under-deliver). Concretely: a narrow typed input
(`{sha256, size_bytes}` mandatory, `limits`/`ports` explicit and required
whenever a service needs anything beyond the sole-legal-value defaults —
never inferred), plus a pure function analogous to
`lower_calls_empty_dgraph_candidate` that constructs one
`CanonicalDServArtifactV1` candidate per generated service, assigns every
fixed constant from Section 6 literally, and validates the result via the
existing `CanonicalDServArtifactV1::validate()` before returning it.
**Registration (`declare_dserv_artifact`) is explicitly NOT called by this
function** — exactly mirroring 117C never calling `create_proposal`.

## 30. Explicit Phase 117E exclusions (recorded now, for that phase to honor)

No `declare_dserv_artifact` call. No DDeploy interaction of any kind
(Section 20's gap is documented, not solved). No solution to Section 10's
replacement problem (Phase 117E must not silently assume it works, and
must not attempt to fix it). No byte building, no byte upload, no
`datum-blob://` network resolution (none exists to use). No derivation of
`limits`/`ports` from D-Graph `resource_requirement` or from D-Script
annotations. No use of `qos`/`provenance`/`probes` for DCompile's own
metadata. No D-Call derivation from ports. No AI-assisted content-identity
inference.

## 31. Whether DCall work remains deferred

**Yes, unambiguously.** This audit found real, production evidence that
artifact ports are consumed by `dcall.rs`'s route-resolution logic — but
strictly downstream of an already-existing `CanonicalDCallV1` edge (Section
13). Nothing in this audit's findings creates new evidence that D-Script,
D-Compile's request, or D-Code artifacts can supply the missing edge-existence
information Phase 117A already found absent. A call-graph input phase
remains correctly deferred until a concrete integration need supplies real
evidence — not manufactured to fill an old illustrative letter slot.

## 32. Evidence index

- `dserver/src/core/dserv_artifact.rs:1-450` (full struct, `validate()`, fixtures)
- `dserver/src/core/dcode_registry.rs:1-91` (registry key, `declare_dserv_artifact`, its own explicit "no replacement mechanism" comment)
- `dserver/src/core/dcode_authorization.rs:1-277` (`authorize_dcode_invocation`'s full 8-step chain, `DCodeInvocationAuthorizationV1`'s exact fields, the R2A independent-descriptor-digest correction)
- `dserver/src/core/dcall.rs:1-45` (module doc comment: ports are resolved not invented, `CanonicalDCallV1` carries no port field)
- `dserver/src/core/ddeploy.rs` (grepped: `ServiceArtifact`-derived eligibility view exists; zero DCode-registry references)
- `DATUM/src/dcode/module_store.rs`, `DATUM/src/dcode/invoke.rs:585` (R3 size_bytes check)
- `DATUM/src/bin/smartsentinel-dcode-install.rs` (the sole production byte-installation path — local file, manual)
- `documentation/adr/ADR-0021-canonical-dserv-artifact-and-dcode-wasm-v0.1.md`, ADR-0008 (cited by `instantiation`'s validate() message), ADR-0025
- `documentation/reference/phase117a-dcompile-architecture-lowering-audit.md`, `phase117b-...md`, `phase117c-...md`
- Git history: `git log --all -S"dserv-artifact"`, `-S"datum-blob"`, `-S"cargo_lock_digest"`, `-S"rustc"`, and `--grep` searches for replacement/version/upgrade/redeploy/recompile — no evidence beyond the already-known VS1 sketch (superseded) and two unrelated hits (`d4af4ee`, `1b3cb56`) checked and ruled irrelevant.
