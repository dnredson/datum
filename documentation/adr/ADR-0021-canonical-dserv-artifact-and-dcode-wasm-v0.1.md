# ADR-0021 — Canonical D-Serv artifact and D-Code/WASM v0.1

- Status: **Accepted and implemented, v4 (independent review closure: node-identity and worker-trust hardening, evidence-leakage fix).**
- Decision refs: ADR-0002, ADR-0003, ADR-0004, ADR-0005, ADR-0006, ADR-0008, ADR-0017, ADR-0018, ADR-0019.
- Conceptual roadmap phase: 111 — D-Serv + D-Code/WASM application execution.

## Context

Prior to this ADR, `datum.dserv-artifact/1` existed only as hand-authored
Stage 107 fixtures (`scripts/fixtures/datum-contracts/vs1/dserv-artifact.*.json`)
and ADR prose (ADR-0003/ADR-0004/ADR-0005/ADR-0008): no Rust type, no
authorization chain, no real WASM invocation. The repository's only real
Wasmtime execution path is the closed Controlled WASM Deployment phase
("Management WASM", ADR-0005) — a single scalar-function call with no I/O,
no long-lived instance, and no D-Serv semantics. ADR-0005 is explicit:
"Management WASM is not D-Code."

This ADR closes the gap: it formalizes `datum.dserv-artifact/1` as a real
typed contract, builds a server-owned canonical D-Code artifact registry and
authorization chain from CURRENT accepted D-Deploy authority, and implements
a genuinely separate D-Code invocation engine on the D-Node (SmartSentinel)
side implementing ADR-0003's `datum-dnode/0` calling convention for real.

## Decision

### D-Code vs Management WASM

Two categories, kept strictly separate in source, never aliased:

- **D-Code** (`DATUM::dcode`, `dserver::core::{dserv_artifact,dcode_registry,dcode_authorization}`,
  `dserver::api::dcode`): the executable of a D-Serv. Content-addressed,
  ABI `datum-dnode/0`, zero host imports, fresh instance per invocation.
- **Management WASM** (`DATUM::wasm_runtime`, `dserver::core::controlled_wasm_deployment`):
  unchanged, untouched, not renamed. Nothing in this ADR's code imports from
  `wasm_runtime`; source-guard tests in `dcode::identity` and
  `dcode::runtime` assert this programmatically.

Where a low-level MECHANISM genuinely matches (SHA-256 content identity, a
fuel-accounting Wasmtime `Engine`/`Module`/`Store`, rejecting any module
with a host import), the PATTERN is reused but the code is written fresh —
see `dcode-vs-management-wasm-audit.txt` for the exact list of what was and
was not reused, and why.

### Canonical D-Serv artifact contract

`dserver::core::dserv_artifact::CanonicalDServArtifactV1` is the first Rust
type for `datum.dserv-artifact/1`. It mirrors the existing checked-in
fixtures field-for-field — no required field was renamed or removed. The
outer type uses `#[serde(deny_unknown_fields)]`, which is the direct
enforcement mechanism for ADR-0004 invariant 10 (no host-kind field ever
appears on this contract): `negative/dserv-artifact.host-kind-field.json`
is rejected at deserialization, not by a separate ad-hoc check.
`validate()` additionally enforces: digest identity (`sha256`/`content_id`
agree and are well-formed), the `datum-dnode/0` ABI and its full required-export
set, zero `allowed_imports`, sane limits (v2, R7: including
`max_emits_per_invocation > 0`), `state_model == "stateless"`, and
`instantiation == "fresh_instance_per_invocation"`. v2 (R7) additionally
requires every `safety.*` flag to be `false` — a zero-import v0.1 D-Code
module has no possible mechanism to touch a filesystem, network, host
command, Docker, or any other side effect regardless of what the field
claims, so a declared `true` value is rejected structurally rather than
silently accepted and never actually enforced at the WASM boundary.

No `datum.dserv-artifact/2` was needed — every v0.1 requirement is
expressible in the existing `/1` shape.

### `datum-dnode/0` calling convention

ADR-0003 already defines a concrete convention — this slice implements
THAT one, not an invented alternative:

| Export | Signature | Meaning |
|---|---|---|
| `memory` | memory | Linear memory |
| `datum_abi_version` | `() -> i32` | Must return `0` |
| `datum_alloc` | `(len: i32) -> i32` | Guest allocates an input buffer |
| `datum_dealloc` | `(ptr: i32, len: i32)` | Frees buffers (best-effort; a trap here does not invalidate an already-successful `datum_handle`) |
| `datum_handle` | `(ptr: i32, len: i32) -> i64` | Handles one input; returns `(out_ptr << 32) \| out_len`, where the EXACT packed value `0` means "no output" (independent review v5, H1) -- distinct from a non-canonical `(nonzero_ptr << 32) \| 0`, which is NOT "no output" and is treated as an ordinary (empty) output byte buffer |

v0.1 policy: **zero host imports**. No WASI, no clocks, no environment, no
sockets, no filesystem. A module importing anything fails validation before
it is ever instantiated (`dcode::abi::has_forbidden_imports`).

### Module content identity and the D-Node-local module store

Identity is `sha256:<64 lowercase hex>` of the actual bytes, always
recomputed (`dcode::identity`), never trusted from a filename/path/mtime/
caller claim. `dcode::module_store::DCodeModuleStore` implements the
operational (non-canonical) local store: `<root>/dcode/sha256/<64hex>.wasm`,
installed via write-to-temp + atomic rename (never a partial file at the
final path), idempotent re-install of identical content, and digest
re-verification on every load (a corrupted stored file fails to load rather
than being trusted).

### Fresh instance per invocation

`dcode::runtime::DCodeModuleCache` caches `Engine`+`Module` by digest;
`Store`/`Instance`/guest memory/globals are NEVER cached — a brand new
`Store` and `Instance` are created for every single invocation (ADR-0008).
Proven by `stateful_module_proves_fresh_instance_per_invocation`: a module
with a mutable global that increments on each call is invoked three
SEPARATE times and observes the initial-state value every time.

### Isolation and resource limits

Enforced: fuel budget (`Store::set_fuel`), maximum linear-memory growth (a
`ResourceLimiter` denying growth beyond `max_memory_pages`), input/output
byte caps, and full host-side pointer/length bounds validation (including
checked-arithmetic overflow protection) on every guest-memory access. A
malformed return pointer/length fails safely with a descriptive error,
never undefined behavior.

**Known environment limitation, CONTAINED in v2 (see `wasmtime-platform-defect-audit.txt`
and `worker-process-isolation-audit.txt`):** on this development
environment's exact Windows/MSVC/Wasmtime-27.0.0 combination, Wasmtime's
AUTOMATIC interrupt-based traps (fuel exhaustion OR epoch-deadline, both
independently reproduced) crash the process hosting Wasmtime
(`STATUS_STACK_BUFFER_OVERRUN`) instead of returning a normal `Err`, while a
module's own explicit `unreachable` trap does not. Fuel accounting is
correctly configured and does not interfere with legitimate execution (see
`fuel_is_genuinely_consumed_by_a_bounded_computation_that_completes_successfully`);
the crash is specific to a module that actually EXHAUSTS its budget mid-loop.
v1 only DOCUMENTED this; it did not contain it, meaning canonical untrusted
D-Code could crash the SmartSentinel parent process. **v2 (R5) fixes this
for real**: `dcode::runtime::DCodeModuleCache` (the code that can crash) now
runs EXCLUSIVELY inside a short-lived `smartsentinel-dcode-worker` child
process, spawned per invocation by `dcode::worker::invoke_isolated`. The
parent enforces `artifact.limits.timeout_ms` itself via OS-level process
supervision (`Child::kill`/`wait`, not Wasmtime epoch interruption) and
observes only an abnormal exit status or a timeout if the child crashes or
hangs — it never shares an address space with Wasmtime. This is proven by a
real, non-ignored, GREEN integration test,
`DATUM/tests/dcode_worker_isolation.rs::infinite_loop_cannot_crash_the_parent_process`,
which exercises the exact scenario the still-`#[ignore]`d in-process
`dcode::runtime::tests::infinite_loop_exhausts_fuel_and_fails_closed` cannot
safely run, and passes. The in-process test remains, `#[ignore]`d, as
platform evidence of the underlying defect the isolation now routes around
— it is no longer the only evidence of containment.

### Canonical D-Code artifact registry

`dserver::core::dcode_registry::DCodeArtifactRegistryStateV1` is a
server-owned registry of canonical metadata and digest identity ONLY — the
module bytes themselves are never persisted server-side, only on the
D-Node-local module store. Keyed by `(application_id, dserv_id)`
(length-prefixed, collision-free, mirroring `ddeploy::application_key`).
Declaration is idempotent-but-conflict-detecting, mirroring
`dnode_registry::declare_dnode`'s precedent exactly. Persisted under a new
control-state domain (`DOMAIN_DCODE_REGISTRY`), separate from
`ServiceArtifact`'s own registry (ADR-0005: they may coexist, never
conflated). The registry key is `(application_id, dserv_id)` — NOT
`artifact_id`/`version` — so ANY different descriptor at that key conflicts
regardless of what its `artifact_id`/`version` say; v0.1 has no artifact
replacement/version-transition mechanism at all (independent review v2, R8:
an earlier version of this ADR/comment incorrectly implied `artifact_id`/
`version` was itself an escape hatch — corrected).

Authorization now binds TWO explicitly separate identities (v2, R2A):
`dserv_artifact_digest` (the module's own content digest, `dcode.sha256` —
changes only when the executable bytes change) and
`dserv_artifact_descriptor_digest` (the canonical JCS digest of the WHOLE
artifact descriptor, `recompute_dserv_artifact_digest` — changes if ANYTHING
in the descriptor changes, including `limits`/`ports`/`safety`/`abi`, even
while the module bytes stay identical). Before this fix, a caller could keep
the module SHA-256 constant while silently raising `fuel_per_invocation`,
widening `ports`, or flipping a `safety` flag, and neither the server-side
staleness check NOR the real DATUM-side `invoke_dcode` execution path would
notice. Closing this required a genuine full mirror of
`datum.dserv-artifact/1` on the DATUM side
(`dcode::dserv_artifact::CanonicalDServArtifactV1`, v2, R2B) implementing
the IDENTICAL canonical digest algorithm — proven by a literal cross-crate
digest-parity test over the real checked-in fixture, not merely "should
match" reasoning — since the real execution path could not previously even
compute the descriptor digest it needed to check. The governed CLI now
fetches this authoritative artifact from
`GET /api/v1/dcode/artifacts/:application_id/:dserv_id` (v2, R2C) rather
than trusting an arbitrary local artifact file, and `invoke_dcode`
cross-checks application_id/dserv_id/artifact_id/ABI/module-digest/
descriptor-digest all independently before executing.

### Active-D-Serv -> D-Code authorization chain

`dserver::core::dcode_authorization::authorize_dcode_invocation` proves the
full chain before issuing a `DCodeInvocationAuthorizationV1`:

1. the application has an ACTIVE accepted D-Map;
2. the service exists in that application's immutable D-Graph snapshot and
   requires `datum-dnode/0`;
3. the active D-Map places that exact service on the requested node;
4. the current authoritative D-Node registry declares `datum-dnode/0`
   support for that node;
5. a canonical D-Serv artifact is registered for that service, its ABI
   matches, and it validates cleanly.

`dserver::api::dcode::post_authorize_dcode_invocation` acquires
`DDEPLOY_TRANSACTION -> DNODE_TRANSACTION -> DCODE_TRANSACTION` (extending
the lock order ADR-0019/ADR-0020 already established; `DCODE_TRANSACTION`
is a new leaf lock, never acquired before either of the other two anywhere
in the crate, so no reverse-acquisition path exists) and always derives a
FRESH authorization — nothing is cached server-side across calls.

**Stale authority / TOCTOU:** `verify_dcode_authorization_still_current`
re-derives the identical chain and fails closed
(`stale_dcode_authorization`) the moment ANY of it has drifted — the
service moved, the node's declared ABI changed, or the registered artifact
changed. This is a DServer-side capability, tested directly
(`active_dmap_changing_after_authorization_is_detected_as_stale`,
`artifact_changing_after_authorization_is_detected_as_stale`).

**Documented scope boundary, updated in v2 (R1):** `invoke_dcode` itself
still does NOT call back to DServer before executing — it verifies
everything checkable from purely local data (the authorization actually
targets this node, the artifact's identity/module-digest/descriptor-digest/
ABI all match the authorization exactly, the module's real byte length
matches its declared `size_bytes`). What changed in v2 is which caller
supplies those inputs: the GOVERNED `smartsentinel-dcode-invoke invoke`
subcommand (`dcode::client::fetch_dcode_authorization`/
`fetch_dserv_artifact`) now calls `POST /api/v1/dcode/authorize` and
`GET /api/v1/dcode/artifacts/:application_id/:dserv_id` directly,
IMMEDIATELY before invocation — since `/authorize` always freshly derives
the chain server-side and never caches, this call itself IS the
re-verification step; no separate `/verify` endpoint was needed. A
`smartsentinel-dcode-invoke offline-test-only` subcommand still exists,
clearly marked internal/test-only, for tests that need to exercise the
invocation path without a running DServer — it is never presented as, and
must never be used as, the production path. **Honest residual limitation:**
a control-plane state change can still theoretically occur between that HTTP
response and the guest actually executing (distributed TOCTOU); this ADR
does not claim, and R1 does not achieve, distributed atomicity. What it DOES
close is the v1 gap where a real invocation could be driven purely from a
locally-fabricated authorization/artifact file with no DServer involvement
at all. See `control-plane-preflight-audit.txt` and
`stale-authority-audit.txt`.

### SmartSentinel invocation path and CLI

`DATUM::dcode::invoke::invoke_dcode(executor, module_store, authorization,
artifact, this_node_id, input_port, input) -> DCodeInvocationEvidenceV1` is
the central function: identity/digest/ABI/size checks, ADR-0003 envelope
construction, execution via a pluggable `DCodeExecutor`, output-envelope
validation, evidence recording. Zero host mutation, zero placement mutation,
zero accept-proposal call anywhere in this path (grep-verified). `artifact`
is now the FULL `dcode::dserv_artifact::CanonicalDServArtifactV1` mirror
(v2, R2B) — not v1's narrow `DServArtifactForInvocation` subset — so the
canonical whole-descriptor digest can actually be recomputed and checked.
`DCodeExecutor` (v2, R5) is either `InProcessExecutor` (fast, test-only, no
crash isolation) or `dcode::worker::IsolatedWorkerExecutor` (the REAL
production executor, spawning a fresh child process per invocation).

Four CLI binaries provide the "minimal CLI for actual human/lab proof"
(111.11): `smartsentinel-dcode-install` (verify + atomically install a
module), (v4, new) `smartsentinel-dcode-init` (explicitly initialize this
installation's LOCAL D-Node identity — see "Independent review v4
corrections" below), `smartsentinel-dcode-invoke` (two subcommands —
`invoke`, the GOVERNED production path fetching from a live DServer and
resolving its own node identity/worker binary locally, never from a
caller-supplied override, and `offline-test-only`, clearly marked
internal/test-only, which retains both overrides for isolated testing), and
`smartsentinel-dcode-worker` (never invoked directly by an operator — the
isolated child process `IsolatedWorkerExecutor` spawns). Proven end-to-end
via real subprocess execution in `DATUM/tests/dcode_cli_end_to_end.rs` and
`DATUM/tests/dcode_worker_isolation.rs` — not merely unit tests of the
library functions they call.

### The ADR-0003 `datum-dnode/0` envelope (v2, R4)

v1 passed a caller's raw payload bytes directly to `datum_handle` and
returned the guest's raw output bytes directly to the caller — no envelope
at all, incorrectly deferring envelope semantics to D-Call. D-Node (this
slice) owns envelope CONSTRUCTION and VALIDATION; D-Call (phase 112) will
later TRANSPORT/ROUTE already-valid envelopes, which does not require it to
exist for this slice to construct/validate one locally. `dcode::envelope`
now implements it: the host constructs the input envelope
(`{abi, port, message_id, correlation_id, received_at_utc, payload_b64}`,
`message_id`/`correlation_id` ALWAYS host-generated, never derived from or
influenced by the guest — the output schema has no slot for either, so a
guest attempting to inject its own is a structurally invalid output, not an
accepted override) and fail-closed-validates the output
(`{status: "ok"|"rejected", reason, emits[]}`): invalid UTF-8/JSON/status,
malformed base64, too many emits (`limits.max_emits_per_invocation`), or an
undeclared port (checked against `artifact.ports.inputs`/`ports.outputs`)
all fail closed. A well-formed envelope is an infrastructure-level
`Success` regardless of whether the guest's own business status is "ok" or
"rejected" — `output_status` carries that business decision separately in
evidence. Emits are recorded as digests/sizes only and never routed
anywhere in this slice.

### Module content identity now also binds size (v2, R3)

`dcode.size_bytes` existed in the schema but was never checked against
anything. `invoke_dcode` now requires the ACTUAL loaded module byte length
to exactly equal the artifact's declared `size_bytes`, failing closed
(`module_size_mismatch`) before execution if it does not.

### Execution evidence

`DCodeInvocationEvidenceV1` records digests and sizes only — never the full
input/output payload. Outcomes: `success`, `validation_failed`,
`resource_limit`, `trap`, `stale_authority`. `fresh_instance_per_invocation`
is always `true`, and this ADR does not claim mathematical determinism
merely because the instance was fresh (a D-Code module can still read a
genuinely nondeterministic host resource within its own sandbox). v2 adds
`input_port`/`message_id`/`correlation_id`/`output_status`/`emits` to record
the ADR-0003 envelope actually used — v4 corrected `output_reason` itself
(see "Independent review v4 corrections" below): the guest-controlled
`reason` string is never persisted verbatim, only `output_reason_present`/
`_sha256`/`_size`. v3 adds `dmap_digest`/`dgraph_digest`/
`dserv_artifact_descriptor_digest`/`module_size_bytes`/`input_envelope_sha256`/
`input_envelope_size`/`received_at_utc`. Guest-output-validation failures'
`error_message` is a STATIC safe classification (v4) — never the guest's
offending value verbatim; `error_offending_value_sha256`/`_size` identify it
without persisting it.

### Relationship to ServiceArtifact

`ServiceArtifact` remains the transitional operational bridge for
container/native-process/middleware execution — it is NOT deleted, NOT
migrated, and NOT aliased to D-Code. Canonical D-Serv Artifact / D-Code is
application executable content for a canonical D-Serv. They coexist. A
future integration may let one D-Serv be implemented by a non-WASM runtime,
but this ADR specifically establishes canonical D-Code/WASM v0.1.

### CLI exit-code semantics (v2, R6)

`smartsentinel-dcode-invoke` previously always exited `0` and printed
`status=ok`, even for `StaleAuthority`/`ValidationFailed`/`ResourceLimit`/
`Trap` outcomes — an automated caller could not distinguish real success
from failure without parsing the evidence JSON, and the original wrong-node
end-to-end test even incorrectly asserted `.status.success()` for a failure
case. The CLI now exits `0` ONLY on an actual `Success` outcome and prints
`status=failed` otherwise, while still always writing/printing the full
evidence record — a failure is never silently swallowed.

## Independent review v2 corrections (summary)

Eight findings (R1-R8) from independent review, all fixed in this pass; see
each finding's own subsection above and the named `*-audit.txt` files in the
review package for full detail. Nothing about the APPROVED v1 architecture
was redesigned — every fix here extends or binds existing mechanisms more
tightly, none of them replace one:

- **R1** — the governed CLI path now fetches authorization + artifact
  directly from DServer immediately before execution, closing the gap where
  a real invocation could be driven purely from a locally-fabricated file.
- **R2 (A/B/C)** — authorization now binds the module content digest AND the
  whole-descriptor digest separately, backed by a full DATUM-side mirror of
  `datum.dserv-artifact/1` with a proven cross-crate digest-parity guarantee.
- **R3** — `dcode.size_bytes` is now checked against the real loaded module
  length before execution.
- **R4** — the ADR-0003 `datum-dnode/0` envelope (ports, host-owned
  message/correlation identity, emit validation) is now actually
  implemented, not deferred to D-Call.
- **R5** — D-Code execution runs in an isolated child process
  (`smartsentinel-dcode-worker`), so the documented Windows/Wasmtime
  automatic-interrupt crash can no longer take down the parent
  SmartSentinel process; proven by a real, non-ignored, green subprocess
  test exercising the exact scenario the in-process test cannot safely run.
- **R6** — the CLI exits non-zero for any non-Success outcome.
- **R7** — every `safety`/`limits` field on the artifact contract is now
  either actually enforced or validated to its only meaningful value.
- **R8** — a stale registry comment about `artifact_id`/`version` as an
  update mechanism was corrected to match the registry's real
  `(application_id, dserv_id)`-keyed, no-replacement-in-v0.1 semantics.

## Independent review v3 corrections (final, before commit)

Seven findings (F1-F7) from the final independent review, all fixed in this
pass. Nothing from the APPROVED v2 mechanisms (source spoofing prevention,
target_nodes-as-eligibility-only, project-wide commitment accounting, the
authorization chain, the digest split, the envelope, worker isolation, CLI
exit semantics) was reopened or redesigned — every fix here closes a
specific, narrowly-scoped gap the v2 fixes themselves exposed.

- **F1** — `constraints.allowed_stages` was declared on real fixtures but
  never enforced anywhere. Canonical D-Node structural authority has no
  authoritative stage identity (only node_id/platform/capacity/ABI), so
  rather than inventing one from Sentinel lease state, live telemetry, or
  ServiceArtifact `target_stages`, v0.1 now fails closed
  (`dcode_artifact_stage_constraint_unverifiable`) in SERVER-SIDE
  `authorize_dcode_invocation` whenever `allowed_stages` is non-empty. This
  does not make the descriptor itself invalid — it is simply not yet
  EXECUTABLE under v0.1. See `allowed-stages-constraint-audit.txt`.
- **F2** — the isolated worker's parent previously added a hidden 250ms
  `WATCHDOG_MARGIN_MS` grace on top of `limits.timeout_ms` before killing an
  overtime child, meaning the canonical timeout was silently expanded by
  ~25% or more for small values. `timeout_ms` is now a hard parent-observed
  deadline starting no later than `Command::spawn()`, with zero added
  execution grace — deliberately fail-closed (a very small `timeout_ms` may
  reject a fast module whose startup overhead alone exceeds it, which is
  strictly safer than silently granting extra runtime). Any test-assertion
  jitter tolerance now lives only in test files, never in the enforcement
  path. See `strict-timeout-audit.txt`.
- **F3 / F3A** — `DCodeInvocationEvidenceV1` only recorded the module
  content digest and DMap/DGraph id+revision (not their canonical digests),
  so post-execution evidence could not uniquely identify the exact
  limits/ports/safety/constraints/snapshot combination authorization
  actually bound. Evidence now also carries `dmap_digest`, `dgraph_digest`,
  `dserv_artifact_descriptor_digest`, and `module_size_bytes` (populated
  directly from the authorization/artifact actually used, never
  reconstructed). Separately, since R4 wraps input in an ADR-0003 envelope
  before it reaches the guest, evidence now also carries
  `input_envelope_sha256`/`input_envelope_size`/`received_at_utc` — the
  identity of what the guest actually saw — kept deliberately distinct from
  the pre-existing `input_sha256`/`input_size` (the raw caller payload).
  Neither the full payload nor the full envelope is ever persisted; this
  remains execution evidence, not a cryptographic non-repudiation ledger.
  See `execution-evidence-binding-audit.txt`.
- **F4** — `invoke_dcode` trusted a fetched/offline artifact's structural
  validity on faith before using its limits/ports/safety, even though DATUM
  now has a full mirrored `artifact.validate()`. A local validation gate was
  added immediately after the descriptor-digest check and before any of
  those fields are used, making the mirrored validator load-bearing on the
  real execution side too — defense-in-depth, not a replacement for
  DServer's own registry-time validation. See
  `local-artifact-validation-audit.txt`.
- **F5** — a stale doc comment on `post_authorize_dcode_invocation` claimed
  the governed CLI "does not itself call back to DServer... since that would
  require D-Call/distribution machinery." This became false as of v2's R1 —
  the governed CLI now calls this very endpoint (and the artifact GET)
  immediately before execution, and control-plane authorization is not
  D-Call at all. Corrected while preserving the one HONEST remaining
  limitation: distributed TOCTOU between that HTTP response and guest
  execution. See `control-plane-preflight-audit.txt`.
- **F6** — an unrelated, pre-existing DATUM integration test regression
  (`smartsentinel_fog_bundle_contract.rs`, plus a second instance found
  during this pass's final full-suite run,
  `smartsentinel_o2_service_artifact.rs`) was red because the already-
  committed foundation-closure work (`95349e1`) correctly moved real catalog
  images to digest-pinned identities, but these tests and
  `DATUM/config.fog.linux`'s dispatch mappings (which feed a real
  `trusted_image` governance check, not merely cosmetic config) still
  referenced the old mutable tags. Fixed as baseline-regression hygiene, NOT
  as D-Code logic — the pinning itself was correct and is unchanged. See
  `foundation-regression-hygiene-audit.txt`.
- **F7** — two cheap, ADR-uncontradicted hardenings: (A) `dcode.uri`'s
  digest component must now equal both `dcode.sha256` and `dcode.content_id`
  (a descriptor must not name three different module identities); (B)
  `abi.required_exports` must now be the EXACT semantic set for
  `datum-dnode/0` — duplicates and unknown extras are rejected, not merely
  tolerated as a superset. Both checks were verified against every real,
  checked-in fixture before implementation to confirm neither would
  contradict accepted behavior. See `artifact-binding-audit.txt`.

## Independent review v4 corrections (node identity, worker trust, evidence leakage)

Three findings (G1-G3), plus a documentation precision correction (G4) and
one optional cheap hardening, all fixed in this pass. Nothing from the
APPROVED v2/v3 mechanisms (D-Code/Management-WASM separation, DServer
authorization, the canonical artifact registry, the descriptor/module digest
split, the envelope, process isolation, the hard ungraced timeout,
`allowed_stages` fail-closed semantics, execution placement, DDeploy) was
reopened or redesigned.

- **G1** — the governed CLI accepted a raw `--node-id` argument and used the
  SAME caller-supplied value both as the node DServer was asked to authorize
  AND as the local identity checked against that authorization inside
  `invoke_dcode` — circular: it proved "DServer authorized the node ID the
  caller requested," never "this process actually IS that node." A process
  physically running as `cloud-01` could pass `--node-id fog-01`, be
  correctly authorized for `fog-01`, and execute a module DMap placed on
  `fog-01` while physically running elsewhere. Fixed by introducing
  `dcode::local_identity` (`DNodeLocalIdentityV1 { schema, node_id }`) — a
  small, OPERATIONAL, per-installation identity file, deliberately NOT
  canonical D-Node structural authority and NOT Sentinel registration/lease/
  liveness (see `local-dnode-identity-audit.txt` for why neither existing
  source — `config::AgentConfig.node_id`, `agent::bootstrap`'s
  `resolved_node_id` — is safe to reuse). Explicitly initialized once via the
  new `smartsentinel-dcode-init` CLI (idempotent on identical re-init, fails
  closed on a conflicting one, atomic write, validated on every read, never
  silently repaired). The governed `invoke` subcommand no longer accepts
  `--node-id` at all — it resolves `local_node_id` from this file and uses
  that SAME value for both the authorization request and the local equality
  check. One physical host MAY represent multiple logical D-Nodes: each
  distinct `--dnode-root` carries its own independent identity file. See
  `local-dnode-identity-audit.txt`.
- **G2** — the governed CLI accepted a `--worker-binary` override and the
  production path executed `Command::new(worker_binary)`, then trusted any
  syntactically valid `WorkerResponse` on stdout — a caller could supply a
  fake executable that never runs Wasmtime and simply prints a fabricated
  success response, defeating execution integrity entirely. Fixed: the
  governed `invoke` subcommand no longer accepts `--worker-binary` at all;
  production worker location is always the trusted sibling
  `smartsentinel-dcode-worker` next to the running executable in the same
  installation directory. `offline-test-only` retains the override for test
  injection; library tests may still pass explicit worker paths directly to
  `invoke_isolated`. See `governed-worker-identity-audit.txt`.
- **G3** — this ADR's own evidence section claimed "digests/sizes only,
  never full payload," but `output_reason` stored the GUEST-controlled
  `reason` string verbatim (a malicious/buggy guest could smuggle arbitrary
  sensitive content, including the entire input payload, through it), and
  guest-output-validation failures (`InvalidStatus`, `UndeclaredOutputPort`)
  copied their guest-controlled offending value directly into
  `error_message` via `Display`. Fixed: `output_reason` replaced with
  `output_reason_present`/`_sha256`/`_size` (never the string); guest-output-
  validation `error_message` is now a STATIC safe classification
  (`envelope::safe_error_message`), with the offending value (if any)
  available only as `error_offending_value_sha256`/`_size`. Proven with
  adversarial exfiltration tests using a unique marker string across every
  guest-controlled channel (`reason`, invalid `status`, undeclared output
  `port`, emit payload, raw input payload, base64 form) — none leak the
  marker into evidence JSON. See `evidence-content-leakage-audit.txt`.
- **G4** — corrected `invoke_dcode`'s own doc comment, which claimed the
  local equality check proves "authorization targets THIS node" back when
  `this_node_id` was still raw caller input (a circular claim at the time).
  After G1 this is now honestly true, and the comment explicitly documents
  the resulting local trust boundary: DServer proves canonical placement/
  authorization; the local D-Node identity file proves which logical node
  this process is; local installation integrity is TRUSTED — v0.1 does not
  defend against an attacker who can already replace the SmartSentinel/
  D-Code binaries or rewrite the local identity file (a stronger threat model
  requiring TPM/secure-boot/remote-attestation, not claimed here).
- **Optional hardening** — `worker.rs` previously wrote the (bounded)
  worker-request JSON to the child's stdin synchronously on the main thread
  before entering the deadline-wait loop; a hung/misbehaving child could in
  principle delay when deadline enforcement even started. The write is now
  done on a background thread (mirroring the existing stdout/stderr reader
  threads), so the main thread reaches `wait_with_timeout` immediately
  regardless of how long the write takes or whether the child ever reads at
  all.

## Independent review v5 corrections (final ABI conformance, before commit)

Five findings (H1-H5), all fixed in this pass. Nothing from the APPROVED
v1-v4 mechanisms (DServer authorization, local D-Node identity, worker-
process isolation, governed worker resolution, the D-Code artifact registry,
the descriptor/module digest binding, envelope semantics, evidence
hardening, placement, DDeploy) was reopened or redesigned.

- **H1** — ADR-0003 defines the packed `datum_handle` return value `0` as
  "no output," but the runtime collapsed `out_len == 0` (even from a
  non-canonical nonzero pointer) into an empty `Vec<u8>` and then
  unconditionally fed it into JSON-envelope parsing — so a perfectly
  ABI-compliant guest returning exactly `0` incorrectly became
  `ValidationFailed`/`output_invalid_json`. Fixed: `dcode::runtime` now
  distinguishes `ExecutionOutput::NoOutput` (checked BEFORE unpacking
  ptr/len, matched only against the EXACT packed value `0`) from
  `ExecutionOutput::Bytes` (everything else, including the non-canonical
  `(nonzero_ptr << 32) | 0`, unchanged from before). `NoOutput` never reaches
  envelope parsing; `invoke_dcode` returns an infrastructure `Success` with
  no output envelope, no `output_status`, no emits, and no output
  digest/size. The distinction round-trips through the real isolated worker
  IPC boundary too (`WorkerResponse::Success { output_b64: Option<String> }`,
  `None` for `NoOutput`). See `dnode-abi-no-output-audit.txt`.
- **H2** — `datum_dealloc` is in the canonical required-exports set, but only
  its NAME was checked before instantiation; the later typed lookup silently
  ignored a signature mismatch via `if let Ok(...)`, so a module could export
  a wrongly-typed `datum_dealloc` and still execute successfully. Fixed:
  `datum_dealloc`'s exact typed signature (`(i32,i32) -> ()`) is now resolved
  BEFORE `datum_handle` runs, equally load-bearing as every other required
  export — a wrong/missing signature fails closed as `ValidationFailed`
  before any invocation happens. This is distinct from a CORRECTLY-typed
  `dealloc` that itself traps during best-effort cleanup after a successful
  `datum_handle` — per this ADR's own calling-convention table, that trap
  does NOT invalidate the already-successful result (unchanged, and now
  explicitly regression-tested). See `dnode-abi-export-signature-audit.txt`.
- **H3** — the local D-Node identity init flow (load/check-absent ->
  write-temp -> atomic rename) had a TOCTOU race between concurrent
  processes/threads targeting the same root, potentially violating the
  "conflicting re-init fails closed" guarantee. Fixed: `init_local_identity`
  now serializes the entire read-check-write-rename-verify sequence via a
  dedicated exclusive lock file (`dnode-local-identity.lock`), reusing this
  codebase's existing `fs2::FileExt` blocking-lock pattern (the same shape as
  `agent::operational_evidence::append_operational_evidence`). The lock is
  intentionally BLOCKING so a race resolves deterministically — exactly one
  `node_id` wins, every other concurrent caller either matches it or fails
  closed against it. See `local-dnode-identity-audit.txt`.
- **H4** — `dcode::runtime`'s own comment on `wasm_backtrace(false)` claimed
  it "avoids" the known Windows automatic-interrupt crash and that a trap is
  "still returned as a normal `Err` either way" — directly contradicted by
  the still-`#[ignore]`d `infinite_loop_exhausts_fuel_and_fails_closed`
  regression in the very same file. Corrected: `wasm_backtrace(false)` only
  reduces diagnostic/backtrace-capture overhead; it is not a mitigation for
  this crash. The actual containment boundary, unchanged since v2 (R5), is
  process isolation (`dcode::worker`'s disposable child process). The
  module-level doc comment's claim that wall-clock timeout is "not enforced
  in v0.1" was also clarified to specify that this applies to the in-process
  runtime LAYER only — the overall system's real wall-clock enforcement is
  `dcode::worker::invoke_isolated`'s OS-level process supervision, unchanged
  since v2. See `wasmtime-platform-defect-audit.txt`.
- **H5** — packaging/documentation hygiene only: the review package's
  `test-results.txt` previously carried a stale "v3 (final)" heading with v4
  results appended beneath it; this package's copy is freshly regenerated as
  one coherent, non-contradictory current report. No source behavior change.

## Exclusions (explicitly out of scope for this branch)

- D-Call (service-to-service routing, address resolution, channels,
  brokers, backpressure, retries, D-Call realization in D-Map) — phase 112.
- Deterministic placement / node scoring changes (untouched; this slice
  only consumes accepted D-Map placement, never re-derives it).
- GNN / RL / LLM.
- Remote module distribution (a module is installed locally and explicitly
  in v0.1; distribution is a later integration slice).
- Auto-acceptance of anything; this slice never calls `accept_proposal`.

## Future D-Call relationship

Phase 111 establishes that an executable endpoint exists. Phase 112's
D-Call will become ONE producer of the byte-string input `invoke_dcode`
currently receives directly from a CLI/test/file — it does not change the
authorization chain, the module store, or the fresh-instance execution
engine this ADR establishes.
