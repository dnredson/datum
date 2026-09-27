# Phase 117H — Governed DCode Exact-Revision Client / CLI Closure

**Status:** Phase 117H CLOSED. **Phase 117 remains OPEN.**
**Branch:** `agent/phase117-dcompile`.
**Baseline:** Phase 117G closure, commit `b109bafab8fa8dffc1c406cc2e341b6470ef89e2`.
**NO VM OR RPI LIVE VALIDATION WAS REQUIRED FOR PHASE 117H** (local
deterministic tests, including the existing loopback HTTP mock precedent in
`DATUM/src/dcode/client.rs`, plus DATUM's existing spawned-real-DServer
integration test suites).

Phase 117G hardened the SERVER side of D-Code executable-revision selection:
an immutable, versioned registry; an explicit, digest-covered D-Deploy
binding; acceptance-time revalidation; and an exact-digest authorization
lookup. It intentionally deferred one thing: DATUM's own governed
invocation client still selected the D-Code artifact to execute by LOGICAL
`(application_id, dserv_id)` identity, not by the exact
`descriptor_digest` a fresh authorization actually names. Phase 117H closes
that gap.

## 1. The inherited gap

Before this phase, every governed DATUM caller did:

```
fetch_dcode_authorization(...)          // exact revision named here
fetch_dserv_artifact(app, service)      // but re-resolved LOGICALLY here
invoke_dcode(...)
```

`fetch_dserv_artifact` calls `GET /api/v1/dcode/artifacts/:application_id/:dserv_id`,
which DServer resolves via
`DCodeArtifactRegistryStateV2::get_logical_if_unambiguous` — this succeeds
only while at most one revision is registered, and fails closed (409) the
moment a second revision exists. This made the OLD client **safe** (it
could never silently execute the wrong revision) but **incomplete**: it
could not execute the valid governed scenario Phase 117G explicitly
enabled — a service's active binding naming one specific revision while
another revision of the same logical service also legitimately coexists in
the registry.

## 2. The exact server-side endpoint this phase consumes (unchanged, 117G)

`GET /api/v1/dcode/artifacts/:application_id/:dserv_id/:descriptor_digest`
already existed, unmodified by this phase. No server file was touched —
`dserver/src/core/dcode_registry.rs`, `ddeploy.rs`, `ddeploy_v2.rs`, and
`dcode_authorization.rs` remain exactly as Phase 117G left them.

## 3. Client-side changes (`DATUM/src/dcode/client.rs`)

- **`artifact_revision_endpoint_url(base_url, application_id, dserv_id, descriptor_digest)`**
  — builds the exact-revision URL via `Url::path_segments_mut`, which
  percent-encodes every segment (including the literal `:` in
  `descriptor_digest`'s canonical `sha256:<64 hex>` form). No manual string
  concatenation anywhere.
- **`fetch_dserv_artifact_revision(base_url, application_id, dserv_id, descriptor_digest)`**
  — calls that exact endpoint and parses the response.
- **`fetch_dserv_artifact`/`artifact_endpoint_url`** (the old logical
  functions) are **kept, unrenamed** (to avoid needless churn at their
  remaining legitimate call sites — their own tests), but their doc
  comments now explicitly say LEGACY / LOGICAL / UNAMBIGUOUS-ONLY / NOT
  suitable for governed invocation, and name the replacement.
- **`GovernedDCodePreflight { authorization, artifact }`** — a plain
  internal pairing type (no `serde`, no new wire schema) — plus
  **`fetch_governed_dcode_execution_material(base_url, project_id,
  application_id, service_id, node_id)`**, the ONE shared governed
  resolution sequence: fetch authorization → verify it corresponds to the
  caller's own request → fetch the exact revision it names → verify the
  artifact/authorization relation → return both. Introduced because three
  independent production call sites (below) were each hand-rolling this
  exact sequence; one shared implementation removes the risk of one of them
  silently drifting back to a weaker pattern in the future.

## 4. Governed CLI sequence, before and after

Before:

```rust
let authorization = client::fetch_dcode_authorization(url, project, app, service, node)?;
let artifact = client::fetch_dserv_artifact(url, app, service)?; // LOGICAL, ignores authorization's own digest
```

After:

```rust
let preflight = client::fetch_governed_dcode_execution_material(url, project, app, service, node)?;
// preflight.authorization, preflight.artifact -- artifact is EXACTLY the
// revision preflight.authorization.dserv_artifact_descriptor_digest names.
```

The fresh authorization is the ONLY selector. No CLI argument, local file,
`artifact_id`, or `version` participates in choosing a revision anywhere in
the governed path.

## 5. Authorization-as-selector: cross-checks added (task Sections 10/11)

`fetch_governed_dcode_execution_material` performs two groups of checks,
both documented in its own doc comment as to which are genuinely NEW
defense versus which duplicate an existing downstream check for earlier,
clearer error reporting:

**Group A — authorization vs. the caller's own request (genuinely new; not
checked anywhere else in this crate):**
`authorization.project_id`/`application_id`/`service_id`/`node_id` are
compared against the exact parameters the caller passed in. Nothing else in
DATUM re-derives "does the HTTP response we actually received correspond to
the request we actually sent" — `invoke_dcode` only ever checks *internal*
consistency between the `authorization` and `artifact` values it is handed,
never against the original caller intent. A misrouted proxy/cache, or a
server-side routing bug, that silently returned an authorization for a
DIFFERENT application/service/node would previously go undetected until
(at best) a confusing failure deep inside invocation. This closes that.

**Group B — artifact vs. authorization (duplicates `invoke_dcode`'s own
final defense, deliberately, for early/clear preflight-stage reporting):**
`artifact.application_id == authorization.application_id`,
`artifact.dserv_id == authorization.service_id`, the artifact's
INDEPENDENTLY RECOMPUTED whole-descriptor digest
(`recompute_dserv_artifact_digest`, the same shared function both sides
call — never a hand-rolled second digest rule) equals
`authorization.dserv_artifact_descriptor_digest`, and
`artifact.dcode.sha256 == authorization.dserv_artifact_digest`.

`authorization.node_id == this_node_id` is **not** duplicated in the
preflight helper: `invoke_dcode` already performs exactly this check
against the real execution-time `this_node_id`, and duplicating it here
would check the identical two values the request itself already used, with
no independent information gained (unlike Group A's project/application/
service checks, which validate the HTTP *response* against the request,
not one field of the response against another field the client itself
just sent as part of the same request).

## 6. Module content identity (task Section 12) — unchanged

`authorization.dserv_artifact_digest` remains the module SHA-256; module
loading (`DCodeModuleStore::load(&authorization.dserv_artifact_digest)`)
is untouched. Phase 117H only changes DESCRIPTOR revision selection, never
module-byte addressing.

## 7. Defense-in-depth: `invoke_dcode` unchanged (task Sections 8, 37)

`DATUM/src/dcode/invoke.rs` was not modified. Its existing final
execution-boundary checks remain fully load-bearing and are regression-
tested (`artifact_digest_mismatch_is_rejected`,
`changing_only_limits_is_caught_by_the_real_invocation_path`,
`wrong_declared_size_bytes_is_rejected`, `wrong_node_is_rejected_as_stale_authority`,
27 tests total, all still passing unmodified — see §11). The client
preflight added by this phase is an EARLY check for clear diagnostics; it
is not a replacement for `invoke_dcode`'s own verification, which remains
the real boundary a malicious or buggy executor/module-store cannot bypass.

## 8. Production call-site audit (task Sections 2, 15, 16)

Every production call site of `fetch_dserv_artifact`/`artifact_endpoint_url`/
`/api/v1/dcode/artifacts/` was searched for and classified. **Three**
independent production governed paths existed, not one — all three now use
`fetch_governed_dcode_execution_material`:

1. `DATUM/src/bin/smartsentinel-dcode-invoke.rs` (`Invoke` mode) — the
   primary governed single-invocation CLI.
2. `DATUM/src/bin/smartsentinel-dcall-dispatch.rs` — the governed SOURCE-side
   D-Call dispatch CLI (Phase 113B4B), which invokes source D-Code before
   handing validated emits to `dcall::delivery::dispatch_validated_emit`.
3. `DATUM/src/dcall/delivery.rs::execute_destination_delivery` — the shared
   DESTINATION-side execution helper used by BOTH local (in-process) D-Call
   delivery and the remote D-Call listener's own HTTP handler. This is the
   single most consequential of the three: it is reachable from every real
   inbound D-Call delivery, local or remote.

Fixing only the CLI and leaving `dcall::delivery.rs` on the old logical
fetch would have left the actual D-Call execution path — arguably the more
frequently exercised one in a running SmartSentinel deployment — with the
exact same multi-revision limitation this phase exists to close. All three
were fixed together, in one commit, using the one shared helper.

`smartsentinel-dcode-invoke offline-test-only` mode is unchanged in
authority semantics: it still bypasses DServer entirely by design, still
prints its `WARNING: ... NEVER use this for real governed execution`
banner unmodified, and `invoke_dcode`'s own exact-digest check still
applies to whatever local authorization/artifact files it is given —
mismatched pairs still fail exactly as before.

Three test call sites across two integration-test files explicitly mirror
this same governed production sequence against a real spawned DServer
process, and were updated to the new helper too so these tests continue to
accurately model production behavior rather than silently diverging from
it: `DATUM/tests/dcall_mqtt_governed_e2e.rs::run_real_source_invocation`,
`DATUM/tests/dcall_two_node_integration.rs::run_real_source_invocation`,
and one further inline call site in the same file's message-limit test
(`LIMIT_APP` scenario). One additional `fetch_dcode_authorization` call in
`dcall_two_node_integration.rs` (re-fetching authorization alone, to assert
its digest changed after a D-Graph revision bump) does not fetch an
artifact at all and required no change.

## 9. D-Call path audit (task Section 16)

Result: **one DATUM-side D-Call path DID independently perform a logical
D-Code-artifact fetch** — `dcall::delivery.rs::execute_destination_delivery`,
listed in §8 above, now hardened. No OTHER DCall path (route derivation,
grant issuance/redemption, MQTT publish/subscribe) performs any D-Code
artifact fetch of its own; those stay entirely on the server side
(`dserver::core::dcall.rs`'s own exact-binding-aware port resolution,
already hardened in Phase 117G) or operate purely on already-resolved
route/grant data. No new D-Call topology or derivation logic was added.

## 10. Test results

**Multi-revision governed success** (task Section 38,
`governed_preflight_succeeds_in_the_multi_revision_scenario`): an
authorization names D2 while D1 (different content, same module digest)
also legitimately exists; the mock server never even needs to serve D1 —
the governed preflight succeeds and returns exactly D2. This is precisely
the scenario the old logical fetch could not support.

**Wrong exact-response failure** (task Section 24, critical,
`governed_preflight_fails_closed_on_wrong_descriptor_response`):
authorization names D1; the exact-digest GET (mock) unexpectedly returns D2
instead. The governed preflight fails closed with a
descriptor-digest-mismatch error before any invocation is attempted.

**Same-module/different-policy failure** (task Section 25,
`governed_preflight_fails_closed_on_same_module_different_descriptor`): D1
and D2 share an identical `dcode.sha256` but differ in `limits`;
authorization names D1, server returns D2. Fails closed on the
whole-descriptor digest, proving the client binds to the entire descriptor
revision, not merely the module bytes.

**Inconsistent module-digest failure** (task Section 26,
`governed_preflight_fails_closed_on_inconsistent_module_digest`): an
authorization whose own `dserv_artifact_digest` field does not match the
real module digest embedded in the artifact its own
`dserv_artifact_descriptor_digest` correctly names. Fails closed on the
module-digest relation.

**Authorization-context mismatch** (task Section 10,
`governed_preflight_fails_closed_on_authorization_context_mismatch`): a
mocked authorization response naming a different `service_id` than what was
actually requested. Fails closed before any artifact fetch is attempted.

**URL correctness** (task Section 21,
`artifact_revision_endpoint_url_is_correct`): a `descriptor_digest`
containing `:` round-trips correctly through `Url::path_segments()`.

**Exact fetch success/404** (task Sections 22-23,
`fetch_dserv_artifact_revision_parses_a_real_http_success_response`,
`fetch_dserv_artifact_revision_surfaces_a_real_http_404`): both exercised
over the existing hand-rolled loopback TCP HTTP mock (`serve_one_http_response`),
consistent with this repository's established no-mocking-crate convention.
A new `serve_two_http_responses` helper serves the authorize-then-fetch
two-request sequence the governed preflight actually makes.

**`invoke_dcode` defense-in-depth regression** (task Section 37): all 27
existing `dcode::invoke` tests pass unmodified, confirming the final
execution-boundary digest/ABI/size checks remain fully load-bearing.

**Real end-to-end confirmation:** `dcall_two_node_integration` (6 tests)
and `dcall_mqtt_governed_e2e` (10 tests) — both of which spawn a REAL
`dserver` process and exercise the actual governed CLI/delivery flow over
real HTTP/MQTT — pass unmodified after switching their own governed-flow
test helpers to the new preflight function, positively confirming the fix
against genuine server behavior, not only mocks.

## 11. Distributed TOCTOU (task Sections 29-30) — unchanged, honestly stated

Phase 117H does **not** create distributed atomicity. A control-plane state
change can still theoretically occur between the DServer authorization
response, the descriptor fetch, and the guest actually executing.

What Phase 117G's immutable-revision design already eliminated, and Phase
117H's exact-fetch now actually exploits: once an authorization names
`descriptor_digest` D1, D1's own registered CONTENT can never change out
from under this fetch — the registry has no in-place-mutation or deletion
path at all, so registering a new D2 elsewhere cannot retroactively alter
D1. The remaining residual concern is exclusively ACTIVE AUTHORITY moving
from D1 to D2 (a new accepted D-Deploy proposal) between this preflight and
actual execution — not D1's own content changing in place. This
distinction is documented directly in `dcode::client`'s own module doc
comment, not merely in this file.

## 12. Files changed

- `DATUM/src/dcode/client.rs` — new URL builder, new exact fetch, new
  governed preflight helper + type, legacy-function doc corrections, 9 new
  tests.
- `DATUM/src/bin/smartsentinel-dcode-invoke.rs` — governed `Invoke` mode
  now uses the preflight helper.
- `DATUM/src/bin/smartsentinel-dcall-dispatch.rs` — same, source-side.
- `DATUM/src/dcall/delivery.rs` — `execute_destination_delivery` now uses
  the preflight helper.
- `DATUM/tests/dcall_mqtt_governed_e2e.rs`,
  `DATUM/tests/dcall_two_node_integration.rs` — governed-flow test helpers
  updated to match; one now-unused import removed.
- `documentation/adr/ADR-0021-canonical-dserv-artifact-and-dcode-wasm-v0.1.md` —
  two narrow wording corrections (the governed CLI's artifact fetch is now
  described as exact-revision, not logical).
- `documentation/reference/phase117h-governed-exact-dcode-client-closure.md`
  (this file).

No `dserver/` file was changed.

## 13. Authority review (task Section 46)

- D-Deploy remains the active executable-binding authority (unchanged,
  117G).
- DServer authorization chooses the exact descriptor revision (unchanged,
  117G) — DATUM now actually USES that choice instead of re-resolving one
  of its own.
- DATUM's client never chooses a revision independently: the exact digest
  it fetches is always read directly off the fresh authorization it just
  received.
- The D-Code registry remains immutable revision storage (unchanged,
  117G) — no server file touched.
- DATUM invokes only the descriptor matching fresh authorization — enforced
  twice now: once early by the governed preflight, once finally by
  `invoke_dcode`.
- `DCodeModuleStore` still addresses module bytes purely by module digest
  (`authorization.dserv_artifact_digest`) — untouched.
- `invoke_dcode` remains the final execution-boundary validation,
  unmodified and fully regression-tested.

## 14. Remaining Phase 117 gaps (task Section 49)

- **A. DCompile provenance persistence** — DScript digest, DCompile-request
  digest, DGraph digest, per-service module/descriptor digests are still
  not recorded as a first-class provenance chain anywhere.
- **B. DCompile → registry → DDeploy production handoff** — there is still
  no first-class production API/orchestrator that consumes a
  `DCompileCandidateBundle` end-to-end (declare revisions, build a
  proposal's `dcode_artifact_authority`, create/accept it) as one governed
  operation; each step exists but nothing wires them together for a real
  caller yet.
- **C. DCompile itself** (`lower_calls_empty_dgraph_candidate`,
  `lower_dcode_artifact_candidates`) still has no externally-consumable
  HTTP API surface — server-side core functions only.
- **D. No live (VM/RPi) proof yet** of the full H1-active → register H2 →
  H1-stays-active → create/accept P2 → H2-becomes-active sequence running
  against real infrastructure (Phase 117G/117H validated this only with
  local/deterministic tests and DATUM's existing spawned-local-process
  integration suites).
- **E. No independent Phase 117 closure review** has yet been performed
  across 117A-117H as a whole.

## 15. Recommended next phase

Given the evidence above, **Phase 117I should be a live infrastructure
milestone**: a live (VM/RPi) proof of item D — declare D1, activate it via
an accepted proposal, register D2 (confirm D1 stays active), create and
accept a new proposal binding to D2, confirm D2 becomes active, all against
real running D-Nodes with the real isolated-worker executor — since this is
the one Phase 117G/117H claim that has so far been validated only with
local/deterministic tests, never live infrastructure, and it directly
exercises everything both phases just built. Items A/B/C (provenance,
production handoff, DCompile API) remain real but are independent
follow-on scope better suited to a dedicated Phase 117J+ once the live
proof (117I) has either confirmed or surfaced problems with the current
design under real conditions. Item E (independent closure review) is best
performed either alongside or immediately after 117I, once there is live
evidence to review, not before.

**Phase 117H itself required NO VM/RPi.** Phase 117I is the point at which
starting live infrastructure becomes appropriate.
