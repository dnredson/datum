# Phase 119D — Live Cross-Node Migration A→B Proof

- Status: Live-infrastructure validation. Zero production source files changed
  by this phase — only this document.
- Branch: `agent/phase119-migration`, exact source commit
  `189de1b3d050a5d22f0ca239eb476278307c1250` used on every host.
- Scope: prove, on two real, physically distinct D-Nodes, that a service's
  placement transitions A→B through the existing, unmodified DDeploy
  acceptance path; that D-Engine membership and DCode-authorized execution
  follow that transition discretely; that content preparation on the pending
  target never changes authority; and that rollback is a new governed
  transition, never a history rewind.

## 1. Executive summary

**The central migration-authority properties designed in Phase 119A and
hardened in 119B/119C were proven live, on two real hosts of different CPU
architectures, using only existing, unmodified production APIs.** A→B and a
subsequent rollback B→A were both demonstrated as ordinary DDeploy
acceptances; D-Engine membership and DCode authorization/execution followed
placement discretely in both directions; the same WASM module executed with
byte-identical output on both nodes, proving genuine execution rather than a
metadata claim.

**Two real findings limited this run's scope, both disclosed honestly rather
than worked around silently:**

1. **A genuine, pre-existing architectural gap** (not introduced by, and out
   of scope for, Phase 119): once a service's placement moves away from a
   node entirely, that node's `operational-reconciliation-context` stops
   existing for it altogether (`"operational D-Graph node slice not found"`)
   — there is no "stopped"/"absent" signal and no orphan-sweep anywhere in
   the reconciliation agent. This was confirmed both by direct source
   tracing and by live behavior against the real running DServer. It means
   the existing `smartsentinel-operational-reconcile` mechanism cannot, as
   currently implemented, ever learn that source A should clean up a
   service that migrated away. Phase 119D does not attempt to fix this
   (fixing it is a Phase 111–115-era OperationalDGraph design change, not a
   migration-authority change).
2. **An environment-specific blocker**, unrelated to (1): `smartsentinel-operational-reconcile`'s
   bootstrap step unconditionally calls a legacy, real-Firestore-backed
   node-snapshot endpoint before it ever reaches reconciliation logic. The
   isolated DServer instance used for this proof (deliberately using a
   placeholder `[firebase]` config, the same pattern Phase 117K used
   successfully) cannot satisfy this call, so this specific binary could
   not be exercised end-to-end in this isolated environment. This blocked
   only the *physical native-process start/stop* portion of the proof — it
   did not block, and is unrelated to, the DDeploy/DCode/D-Engine authority
   properties that are Phase 119's actual subject.

Given both findings, this run's evidence is scoped precisely: **placement
authority transition, D-Engine derivation, and governed DCode
authorization/execution were fully and repeatedly proven live in both
directions (A→B and B→A). Physical Stage107A-governed runtime start/stop on
the real hosts was not exercised in this run** — see §22 for the full
reasoning and §41 for what remains for a future phase.

## 2. Physical topology

| Logical name | Alias | Address | User | Architecture | OS |
|---|---|---|---|---|---|
| Windows control plane | — | 192.168.10.101 | (local) | x86_64 | Windows 11 |
| RPi (source, then rollback target) | `datum-mist` | 192.168.10.109 | `dener` | aarch64 | Linux 6.12.47+rpt-rpi-v8 (Debian) |
| fog-01 (migration target) | `datum-fog` | 192.168.10.103 | `vboxuser` | x86_64 | Linux 6.17.0-41-generic (Xubuntu) |
| cloud-01 | `datum-cloud` | 192.168.10.108 | — | — | **NOT used, NOT started** |

Both RPi and fog-01 reached the isolated DServer directly over the LAN at
`http://192.168.10.101:18080` with **zero firewall changes required** —
confirmed by direct `curl` from both hosts before any other work began.
Mosquitto was never touched, referenced, or depended upon. No DCall route,
grant, or MQTT listener was created anywhere in this phase.

## 3. Logical identity mapping

| Concept | Value |
|---|---|
| SOURCE_NODE_ID | `rpi-119d` (physical: RPi) |
| TARGET_NODE_ID | `fog-119d` (physical: fog-01) |
| PROJECT | `proj:phase119d-live` |
| APPLICATION | `app:phase119d-live` |
| DSCRIPT | `dscript:phase119d-live` |
| DGRAPH | `dgraph:phase119d-live` |
| SERVICE | `probe` |
| FUNCTION | `probe_fn` |
| INPUT_PORT | `trigger` |

RPi was chosen as the initial SOURCE (reusing the exact aarch64 host and
governed-invocation pattern already proven live in Phase 117K) and fog-01 as
the migration TARGET, giving a genuine cross-architecture transition
(aarch64 → x86_64) — DCode's architecture-independence (WASM, ADR-0003/
ADR-0005) made this the more informative choice over a same-architecture
pair. This node choice is a test-fixture decision only, not a Phase 120
placement-selection claim.

## 4. Preflight

Both hosts were probed live before any other action:

| Check | RPi | fog-01 |
|---|---|---|
| `hostname` | `rpi` | `Xubuntu` |
| `uname -m` | `aarch64` | `x86_64` |
| CPU cores (`nproc`) | 4 | 4 |
| RAM (`MemTotal`) | 3,981,172,736 bytes | 8,325,046,272 bytes |
| `git --version` | 2.47.3 | 2.51.0 |
| `docker --version` | 26.1.5 (present, **never used**) | 29.1.3 (present, **never used**) |
| `rustc`/`cargo` | 1.94.1 (via rustup, needed `source ~/.cargo/env`) | 1.97.1 |
| Disk free | ~16G | ~2.6G (tight but sufficient) |
| Docker Hub reachable | Yes (HTTP 401, expected unauthenticated response) | Yes | 

Docker was never invoked on either host — the live proof used the
**native_process** Stage 109/ADR-0016 runtime kind exclusively (§10),
deliberately avoiding all OCI image-pull/pinning machinery.

## 5. Exact source distribution

Git HEAD confirmed on Windows: `189de1b3d050a5d22f0ca239eb476278307c1250`.
No HTTPS git credentials exist on either remote host (confirmed once more,
consistent with Phase 117K), so the exact commit was distributed via
`git archive --format=tar.gz -o phase119d-src.tar.gz HEAD` → `scp` → extract
into `~/datum-phase119d-src` on each host — the same disclosed substitution
Phase 117K used, never a plain clone.

## 6. Isolated DServer

- Binary: `dserver/target/release/DServer.exe`, built via `cargo build
  --manifest-path dserver/Cargo.toml --locked --release --bin DServer` from
  the exact commit above.
- Scratch root (outside the Git worktree): `%TEMP%\datum-phase119d-20260928-193029\`.
- Config (`dserver-config.toml`): `bind = "0.0.0.0:18080"`, `debug = true`,
  placeholder `[firebase]` (`project_id = "phase119d-unused"`,
  `credentials_path = "unused.json"`), `[cors] enabled = false` — the exact
  shape `DATUM/tests/dcall_two_node_integration.rs`'s own `spawn_real_dserver`
  uses, and the same pattern Phase 117K used.
- Environment: `DATUM_DSERVER_CONFIG_PATH`/`DATUM_CONTROL_STATE_DB_PATH`/
  `DATUM_OPERATIONAL_DGRAPH_STATE_PATH` all pointed at the scratch root.
- Started as its own process (PID 1825), confirmed listening via direct
  `curl` on `127.0.0.1:18080`, the verified Windows LAN address
  `192.168.10.101:18080`, and from both remote nodes. No other DServer
  process was touched at any point.

## 7. Node binaries built

On each node, from the exact distributed source: `cargo build --release
--locked --bin smartsentinel-dcode-init --bin smartsentinel-dcode-install
--bin smartsentinel-dcode-invoke --bin smartsentinel-operational-reconcile
--bin smartsentinel-observe`, plus (discovered necessary only once the first
invocation attempt failed with `resource_limit`/"failed to spawn dcode
worker process") a follow-up `--bin smartsentinel-dcode-worker` build on
both hosts. fog-01's initial 5-binary build completed in ~4 minutes; RPi's
took approximately 35 minutes (its CPU is dramatically slower for a
`wasmtime`/`cranelift`-heavy dependency tree) — both builds succeeded
cleanly, warnings only (pre-existing, unrelated to this phase).

## 8. Structural D-Node declarations

Declared via the real, unmodified `PUT /api/v1/dnodes/{node_id}` route,
using each node's own real, freshly-probed platform/capacity facts (§4) —
never copied from one node onto the other:

- `rpi-119d`: `{os: "linux", architecture: "aarch64"}`, capacity
  `{cpu_cores: 4, memory_bytes: 3981172736}`, capabilities
  `{dnode_abi_versions: ["datum-dnode/0"]}` → `registry_revision: 1`.
- `fog-119d`: `{os: "linux", architecture: "x86_64"}`, capacity
  `{cpu_cores: 4, memory_bytes: 8325046272}`, capabilities
  `{dnode_abi_versions: ["datum-dnode/0"]}` → `registry_revision: 2`.

## 9. Stage107A resource-registry prerequisites

Confirmed live (and initially unknown to this session — discovered by
iterating against the real DServer's own validation errors) that an
active Resource Registry binding is a **hard prerequisite** for any
reconciliation authorization (`authorize_operational_reconciliation` returns
409 `"no active Resource Registry binding exists"` otherwise). Sequence, via
the real, unmodified production API:

1. `POST /api/v1/resources/discovery` per node — required fields discovered
   empirically: `identity.hardware_identity_confidence` must be `"unknown"`
   when no hardware fingerprint is claimed; `resources.capacity_profile_id`
   must be present in format `"capacity:<64 lowercase hex>"`;
   `platform.os_family`/`platform.arch` (not `os`/`architecture`) are
   required; and a bare ephemeral-instance-scoped resource (no
   `hardware_fingerprint_id` and no `os_installation_id`) is **not**
   planning-eligible and cannot be bound — supplying `os_installation_id`
   (format `"osinst:<64 hex>"`) was required to reach
   `identity_scope: "provisional_os"`/`planning_eligible: true`.
   - `rpi-119d` → `resource:os:a7ee7cd4464efd6a73a7de6834a3f733c6444e90d45d1d2779b21a9bd357ac59`
   - `fog-119d` → `resource:os:427aa887fe61306e118cfc90fe1b742512e7d422343a82f5efa20c2979940f50`
2. `POST /api/v1/resources/{resource_id}/binding` with `{"node_id": ...,
   "transition_kind": "initial_assignment"}` for each:
   - `binding:b45961c2-8e30-4939-aedd-8850333dc506` → `rpi-119d`
   - `binding:5083879b-213e-49b9-8978-ef6ae79a0ee4` → `fog-119d`

## 10. One ABI-compatible DCode module (executable identity held constant)

A single `datum-dnode/0` WASM module was built once, using the exact
`respond_ok_wat` WAT pattern already proven in
`DATUM/tests/dcall_two_node_integration.rs` and Phase 117K, with
`reason = "phase119d-probe"`:

- SHA-256: `98340f04673c0c3064b8a5e56e62377f1768cf7db7192ad4b89c67e63dea2b7d`
- Size: 250 bytes
- Expected reason-payload SHA-256 (independently precomputed before any
  invocation): `bd85e9a4169569e7fb874d5bd8dd9701fb1aa69a7340e79eea6908ea5596f419`

Unlike Phase 117K, executable identity was deliberately held **constant**
throughout this entire phase — the property under test here is "same
DCode, different node," the strongest and most direct migration proof,
never "different DCode, same node" (that was 117K's own, already-closed
property).

## 11. Real production DCompile

`POST /api/v1/dcompile/compile` with a `datum.dcompile-submission/1` body
(one service `probe`, one function `probe_fn`, the SHA-256/size above)
produced, from the real, unmodified production compiler:

- `dgraph:phase119d-live` revision 1, digest
  `sha256:0afa6d8f92d4cb41740dfdf613b3991fcacb7911cbceb422fb12709505aab4cc`
- `descriptor_digest`: `sha256:9e2555cc5ca565d63ffa1bd17d88061938f408a510e5e85173ae2cd1337d6d51`
- A complete `CanonicalDServArtifactV1` (schema `datum.dserv-artifact/1`)

## 12. DCode registration (zero transformation)

The compiler-produced `CanonicalDServArtifactV1` was `POST`ed **unmodified**
to `POST /api/v1/dcode/artifacts`. The registry's own recomputed
`descriptor_digest` matched DCompile's output exactly:
`sha256:9e2555cc5ca565d63ffa1bd17d88061938f408a510e5e85173ae2cd1337d6d51`.

## 13. ServiceArtifact — native_process runtime (not container)

To make placement changes visible via a real, inspectable, governed
runtime primitive while avoiding all Docker Hub/OCI image-pinning
complexity (irrelevant to migration authority itself), this proof used
`runtime.kind = "native_process"` — a fully-implemented, real, Linux-only,
lease-gated Stage 109 (ADR-0016) production path, confirmed by direct
source read to reuse the exact same Stage 107A governance machinery
(`LeaseGuard`, `check_generation_and_projection`, mutation-boundary
context) as the container path, with no parallel authorization mechanism.

Constructing a valid artifact required iterating against the real
DServer's own validation errors (each confirmed directly against source
afterward, not guessed):

- `control_plane.artifact_kind` must be `"container"`, `"git"`, or
  `"command_bundle"` — `native_process` runtime pairs with
  `"command_bundle"`.
- The execution probe kind must be `docker_container_running`,
  `process_running`, or `systemd_active` — `process_running` was used,
  with `params.process_name` matched via `pgrep -x` semantics (Linux
  truncates a script's own `comm` name to 15 characters; `phase119d-probe.sh`
  truncates to exactly `phase119d-probe`, chosen deliberately).
- `control_plane.wasm_decision` is **required** and must be the fixed
  "safe" tuple (`explicit_invocation_required: true`,
  `host_imports_allowed: false`, `side_effects_allowed: false`,
  `safe_to_auto_execute: false`) — confirmed this is the *exact same*
  tuple the server independently hardcodes into every
  `ArtifactReconciliationContext.execution_policy`
  (`artifact_reconciliation_context.rs`), confirming host-mutation
  authority is never delegated to a WASM module's own claimed
  permissions — real mutation happens only via the agent's own governed
  reconciliation code, never "through" a WASM decision.
- `control_plane.acquisition.kind` must be exactly
  `"native_process_executable"`; `acquisition.source` is a plain absolute
  path on the node's **own** filesystem (never fetched over the network by
  the server) — the identical probe script (SHA-256
  `e0939483c51a3729dfcd63c8ea58c5edc4e26600645fedf908e94e21c5fabe05`, 433
  bytes) was placed at the same path, `/tmp/phase119d-source/phase119d-probe.sh`,
  on **both** nodes, since `target_nodes` includes both and `acquisition`
  is one shared field.

Final artifact: `artifact:phase119d-probe`, `target_nodes: ["rpi-119d",
"fog-119d"]`, `execution_generation: 1`, `execution_projection_digest:
sha256:f681dd2843875a26bf00e5969c8dbc6c6ba8de48f2bd59cadf5e981fbfb3a8f1`.

## 14. Initial placement A

`POST /api/v1/ddeploy/proposals` (ordinary production API, `source:
"operator"`, a test-fixture placement decision, not a Phase 120 claim):
DCompile's own D-Graph verbatim, `placements: [{service_id: "probe",
node_id: "rpi-119d"}]`, `dcode_artifact_authority: {"probe":
{"descriptor_digest": ...}}`. A placeholder `dcontinuum` was supplied in
the request; the server **replaced it automatically** with the real,
registry-derived authoritative D-Continuum (confirmed live:
`dcontinuum_ref.revision` came back as `2`, matching the real two-node
registry, not the placeholder's `revision: 1`) — direct live confirmation
of `authoritative_dcontinuum_for_project`'s documented behavior.

- `proposal_id: ddeploy-proposal:ab0012e6-4869-4726-b189-436418e2e51b`,
  digest `sha256:8f45ac397e7bbd2f1cd4e6c0aba8a2b64a625ced851b149a86508d2356991672`
- Accepted (`accepted_by: "operator:phase119d"`):
  `acceptance_id: ddeploy-acceptance:bcfbb75c-3a63-4c53-82fb-f63bc0d669a2`,
  materialized `dmap:19:proj:phase119d-live18:app:phase119d-live` revision
  1, digest `sha256:5f9bada6dc02d07fadd8a3ef400d51a95b8788c3e973e9b052612f00eab8bac1`,
  `currently_active: true`.

## 15. Verified initial authority

`GET /api/v1/ddeploy/active` confirmed exactly
`placements: [{service_id: "probe", node_id: "rpi-119d"}]`. **D-Engine
membership = `{rpi-119d}`**, derived directly from this placement list per
Phase 118's own unmodified definition — no API was created or needed
merely for this test, exactly as Phase 118A/118B/118C established.

## 16. Pre-migration negative proof

Before proposal B ever existed:

- `POST /api/v1/dcode/authorize` for `{probe, fog-119d}` →
  **HTTP 422**, `"dcode_service_not_placed_on_node"`: *"service 'probe' is
  placed on node 'rpi-119d', not the requested node 'fog-119d'"*.
- The same call for `{probe, rpi-119d}` → **HTTP 200**, exact module/
  descriptor digests matching DCompile/registration.

## 17. Governed content installation and execution on A

`smartsentinel-dcode-init --node-id rpi-119d --dnode-root
/home/dener/phase119d-dnode` then `smartsentinel-dcode-install
--module-path .../probe.wasm --declared-sha256 ... --store-root
/home/dener/phase119d-dnode` (hash-verified content-addressed store write,
zero DMap/DDeploy dependency).

Real governed invocation
(`smartsentinel-dcode-invoke invoke --dserver-url http://192.168.10.101:18080
...`) — **first attempt failed** with `outcome=ResourceLimit`,
`"failed to spawn dcode worker process ... No such file or directory"`:
the `smartsentinel-dcode-worker` binary had been omitted from the initial
build list on both hosts. Built it on both (13.7s / 1.5s respectively,
dependencies already cached) and retried successfully:

```
outcome=Success node_id=rpi-119d service_id=probe
output_status=ok output_reason_sha256=68968392bbd3dcfc6c0c9425c178f4da6580ddc654d46c22b2a25c486a0fd08f
```

The `output_reason_sha256` here corresponds to `SHA256('{"status":"ok",
"reason":"phase119d-probe","emits":[]}')`, matching the independently
precomputed expected value exactly (§10) — genuine guest-byte execution,
not a metadata claim.

## 18. Pending target proposal B

`POST /api/v1/ddeploy/proposals`: identical `dgraph`/`dcode_artifact_authority`
to proposal A, placement changed to `{probe, fog-119d}`. The server
automatically bound `expected_active_dmap_revision: 1` — matching A's
current revision, with zero manual CAS bookkeeping required.

- `proposal_id: ddeploy-proposal:43de88c0-e133-4653-9262-abad9172ac4a`,
  digest `sha256:a201add7f6c7068e42c5f7e46f22ea6cfe96c5506c7680746166d8cac6aa4b67`,
  `state: "pending"`. **Not accepted yet.**

## 19. The migration intent (Phase 119B/C values, not re-invoked as code)

Recorded explicitly, corresponding exactly to a `MigrationIntentV1` value:

```
project_id           = proj:phase119d-live
application_id       = app:phase119d-live
source_dmap_ref      = { id: dmap:19:proj:phase119d-live18:app:phase119d-live,
                          revision: 1,
                          digest: sha256:5f9bada6dc02d07fadd8a3ef400d51a95b8788c3e973e9b052612f00eab8bac1 }
target_proposal_id     = ddeploy-proposal:43de88c0-e133-4653-9262-abad9172ac4a
target_proposal_digest = sha256:a201add7f6c7068e42c5f7e46f22ea6cfe96c5506c7680746166d8cac6aa4b67
```

**Honest disclosure (per this phase's own Section 33 instruction): the
Phase 119B/C pure Rust function `evaluate_migration_authority_relation`
was NOT directly invoked against this live control-state database** — doing
so would have required a new debug binary/API, which this phase correctly
declined to add. Given these exact recorded values and the live facts
below, that function's own already-unit-tested logic (32/32 passing,
Phase 119C) would evaluate this intent as `ReadyForAcceptance` at this
point, and — after acceptance (§21) — as `TargetAcceptedAndCurrent`. This
is stated as a logical consequence of already-proven unit behavior applied
to these real values, never mis-described as a live invocation of that
function.

## 20. Central pre-cutover proof

With B's proposal existing and DCode content installed on B (next section),
but **before** acceptance, every one of the following was independently
re-verified live:

| # | Property | Live result |
|---|---|---|
| A | Active DMap is still A | `placements: [{probe, rpi-119d}]`, revision 1 |
| B | D-Engine membership remains `{rpi-119d}` | Confirmed (derived from A above) |
| C | B proposal remains `Pending` | Confirmed via `GET .../proposals/{id}` |
| D | Fresh DCode authorization still names A | `node_id: rpi-119d`, descriptor matches |
| E | Real governed invocation still succeeds on A | `outcome=Success`, `output_reason_sha256` matches |
| F | TARGET B authorization still fails closed | HTTP 422 `dcode_service_not_placed_on_node` |
| G | B has no migrated runtime from proposal creation/byte install alone | True by construction — no runtime mechanism was invoked for B at all at this point |
| H | No host mutation occurred from the intent itself | True — only the explicit, independent content-install command mutated anything, and only B's local content store |

## 21. Content preparation on B while pending

`smartsentinel-dcode-install` (same module, same declared SHA-256) run
against `fog-119d`'s own store root — succeeded, hash-verified — **while
proposal B remained `Pending`** and every §20 check above still held. This
is the live, cross-node generalization of Phase 117K's own central finding
(content presence ≠ active executable authority), now proven for a
node-placement change rather than a same-node descriptor change.

## 22. Cutover — the sole event

`POST /api/v1/ddeploy/proposals/{B}/accept` — **the only mutation of
canonical placement authority performed anywhere in this phase.** No
special migration endpoint exists or was created.

- `acceptance_id: ddeploy-acceptance:7a7988f4-0f6a-4fa6-9b6d-640073b34282`
- Materialized: `dmap:19:proj:phase119d-live18:app:phase119d-live` revision
  **2** (> A's revision 1, monotonic), digest
  `sha256:39e9e3d9e871813a3f4961849acc54ad2f18e64b43b60d3ef6a49f2d413974f9`
- `placements: [{service_id: "probe", node_id: "fog-119d"}]`,
  `currently_active: true`

## 23. D-Engine flip and authorization flip

Immediately after acceptance:

- `GET /api/v1/ddeploy/active` → `placements: [{probe, fog-119d}]`,
  revision 2. **D-Engine membership discretely became `{fog-119d}`** — at
  no point was it, or could it have been, `{rpi-119d, fog-119d}`.
- `POST .../authorize` for `{probe, rpi-119d}` → **HTTP 422**, same
  `dcode_service_not_placed_on_node` finding, symmetric to §16.
- `POST .../authorize` for `{probe, fog-119d}` → **HTTP 200**,
  `authorization_id: dcode-auth:proj:phase119d-live:app:phase119d-live:probe:fog-119d:2`,
  **the same** module digest and descriptor digest as every prior
  authorization — only the authorized node and `dmap_ref` changed.

## 24. Real governed execution on B

`smartsentinel-dcode-invoke invoke` on `fog-119d` → `outcome=Success`.
Direct comparison against A's own evidence file:

| Field | A (rpi-119d) | B (fog-119d) | Match |
|---|---|---|---|
| `dserv_artifact_digest` (module SHA-256) | `98340f04...` | `98340f04...` | **Yes** |
| `dserv_artifact_descriptor_digest` | `sha256:9e2555cc...` | `sha256:9e2555cc...` | **Yes** |
| `output_reason_sha256` | `68968392bb...` | `68968392bb...` | **Yes** |
| `node_id` | `rpi-119d` | `fog-119d` | **No — the only thing that changed** |

This is the core, unambiguous proof: **the exact same executable produced
byte-identical output on two physically distinct hosts of different CPU
architectures; only the authorized/executing node followed the accepted
placement.**

## 25. Operational projection after cutover — and the honest limitation

`GET /api/v1/nodes/fog-119d/operational-reconciliation-context` → real,
non-empty `desired_services: [{service_id: "probe", ..., desired_state:
"running", dmap_ref: {revision: 2, ...}}]` — B is now correctly desired.

`GET /api/v1/nodes/rpi-119d/operational-reconciliation-context` → **not**
an empty `desired_services` list, but an outright error:
`"operational D-Graph node slice not found for project 'proj:phase119d-live'
and node 'rpi-119d'"`. This is the live, empirical confirmation of the
architectural finding disclosed in §1 and traced directly in source
(`dserver/src/core/ddeploy.rs`'s `union_project_operational_graph`, which
iterates *only* over the active DMap's own placements — a node that drops
out of every application's placements simply never appears in the
project-wide operational graph at all, with no "stopped"/"absent" sentinel
ever synthesized for it). There is no way, through this existing mechanism,
for rpi-119d to ever be told "you no longer need to run probe."

## 26. Target/source reconciliation — not exercised in this run

Per §1's second finding: `smartsentinel-operational-reconcile`'s
unconditional bootstrap step (`query_node_context`,
`DATUM/src/agent/bootstrap.rs`) calls `GET /api/v1/nodes/{id}/latest`, a
legacy endpoint backed by real Google Firestore
(`dserver/src/storage/firebase.rs`, `base_url` hardcoded to
`firestore.googleapis.com`, no emulator/override support). Against this
phase's deliberately isolated, placeholder-Firebase DServer, that call
returns `403 CONSUMER_INVALID`, and the CLI aborts before any reconciliation
logic runs — confirmed identically on both nodes. No real Firebase
credentials were available or appropriate to supply for a throwaway
isolated test, and patching the legacy bootstrap/Firebase coupling is
outside Phase 119's declared scope. **Physical native-process start on B,
and any removal attempt on A, were therefore not exercised in this run** —
disclosed here rather than fabricated or silently skipped.

## 27. Rollback — a new governed transition, not history rewind

Content sanity check first: `find /home/dener/phase119d-dnode` confirmed
the original module bytes were still present on RPi from §17 — verified,
not assumed.

A **new** proposal was submitted, identical `dgraph`/`dcode_artifact_authority`,
placement `{probe, rpi-119d}`:

- `proposal_id: ddeploy-proposal:ad7ccd51-afb0-4d9e-8f2d-de63391b777c`,
  `expected_active_dmap_revision: 2` (auto-bound to B's current revision,
  **not** a reused old value).
- Accepted: `acceptance_id: ddeploy-acceptance:5d15fb6a-01f8-49cf-9f6d-3c9028ade1ff`,
  materialized **revision 3** (never a rewind to revision 1), digest
  `sha256:715014ee01e3231dfe9411123826b08c84b33756c3dbb136f9ddeb3da4d9dd5c`.

## 28. Governed execution returns to A

`smartsentinel-dcode-invoke invoke` on `rpi-119d` → `outcome=Success`,
`dmap_revision: 3`. Compared against §17's original evidence: **module
digest, descriptor digest, and `output_reason_sha256` all match exactly** —
the same executable, now re-authorized under a genuinely new DMap revision,
never the old one. `POST .../authorize` for `{probe, fog-119d}` →
HTTP 422, symmetric to §16/§23.

## 29. Final live authority timeline

| Stage | Active DMap (id, rev) | D-Engine | Target proposal | Authorized node | Runtime A | Runtime B |
|---|---|---|---|---|---|---|
| Initial A accepted | rev 1 | `{rpi-119d}` | — | rpi-119d | not exercised (§26) | absent |
| B proposal pending | rev 1 | `{rpi-119d}` | B pending | rpi-119d | not exercised | absent |
| B bytes prepared | rev 1 | `{rpi-119d}` | B pending | rpi-119d | not exercised | content only, no runtime |
| Immediately after B acceptance | rev 2 | `{fog-119d}` | B accepted | fog-119d | not exercised | not exercised (§26) |
| Rollback proposal pending | rev 2 | `{fog-119d}` | rollback pending | fog-119d | content present (verified) | not exercised |
| After rollback acceptance | rev 3 | `{rpi-119d}` | rollback accepted | rpi-119d | not exercised | not exercised |

At every single stage there is exactly one unambiguous canonical placement
authority — never a union of A and B.

## 30. Migration identity/currentness table

| Fact | Value |
|---|---|
| Initial A | `dmap` rev 1, digest `sha256:5f9bada6dc02d07fadd8a3ef400d51a95b8788c3e973e9b052612f00eab8bac1` |
| Target proposal B | `ddeploy-proposal:43de88c0-...`, digest `sha256:a201add7f6c7068e42c5f7e46f22ea6cfe96c5506c7680746166d8cac6aa4b67`, `expected_active_dmap_revision: 1` |
| Accepted B | acceptance `ddeploy-acceptance:7a7988f4-...`, materialized rev **2**, digest `sha256:39e9e3d9e871813a3f4961849acc54ad2f18e64b43b60d3ef6a49f2d413974f9` |
| Rollback proposal | `ddeploy-proposal:ad7ccd51-...`, digest `sha256:8a232b738de5fcfe18c509576d858a2ed098ccda6d73805ed57838c6371a6abc`, `expected_active_dmap_revision: 2` |
| Rollback accepted DMap | rev **3**, digest `sha256:715014ee01e3231dfe9411123826b08c84b33756c3dbb136f9ddeb3da4d9dd5c` |

Revision sequence: **1 → 2 → 3**, strictly monotonic, never rewound.

## 31. DCode table

| Stage | Placement node | Module digest | Descriptor digest | Output reason digest |
|---|---|---|---|---|
| A, pre-migration | rpi-119d | `98340f04...dea2b7d` | `sha256:9e2555cc...7d6d51` | `68968392...86a0fd08f` |
| A, pre-cutover recheck | rpi-119d | `98340f04...dea2b7d` | `sha256:9e2555cc...7d6d51` | `68968392...86a0fd08f` |
| B, post-cutover | fog-119d | `98340f04...dea2b7d` | `sha256:9e2555cc...7d6d51` | `68968392...86a0fd08f` |
| Rollback, A again | rpi-119d | `98340f04...dea2b7d` | `sha256:9e2555cc...7d6d51` | `68968392...86a0fd08f` |

Module digest and descriptor digest are **identical in every row** —
executable identity never changed. Only `node_id` follows accepted
placement, exactly and exclusively.

## 32. Reconciliation desired-vs-actual table

| Stage | Desired on A | Desired on B | Actual on A | Actual on B |
|---|---|---|---|---|
| Before cutover | probe (rev 1) | — | not exercised | absent |
| After cutover | **node slice not found (§25/26)** | probe (rev 2) | not exercised | not exercised |
| After rollback | probe (rev 3) | node slice not found | not exercised | not exercised |

The "Actual" columns are honestly marked "not exercised" throughout (§26) —
this table does not claim physical convergence evidence this run did not
produce. The "Desired on A after cutover" cell records the real, literal
server response (a missing node slice, not an empty list) — see §25 for why
this is itself a significant, disclosed finding rather than an oversight in
this table.

## 33. DMonitor

Not used anywhere in this phase — consistent with the task's own "optional,
supplementary" framing. Migration authority does not depend on it.

## 34. cloud-01 / Mosquitto / DCall

`cloud-01` was never started or referenced. Mosquitto was never stopped,
reconfigured, or depended upon. No DCall route, grant, or MQTT listener was
created.

## 35. Failures encountered and classification

| # | Failure | Classification | Resolution |
|---|---|---|---|
| 1 | `smartsentinel-dcode-invoke` failed with `resource_limit`: missing `smartsentinel-dcode-worker` binary | B — incorrect test setup (omitted from initial build list) | Built the missing binary on both hosts (13.7s / 1.5s); retried successfully |
| 2 | `smartsentinel-operational-reconcile` config rejected: missing `[agent]`/`[outputs]` tables | B — incorrect test setup (config assembled from partial source reading) | Iterated against real error messages to the exact minimal valid config |
| 3 | `ServiceArtifact` registration rejected 4 times in sequence (`artifact_kind`, probe `kind`, `wasm_decision` required, then `wasm_decision` must be the safe tuple) | B — incorrect test setup (constructed from partial source reading, corrected empirically against real validation) | Each corrected in turn against the real server's own error messages |
| 4 | Resource discovery rejected 3 times (`hardware_identity_confidence`, `capacity_profile_id` format, `platform.os_family`/`arch` field names) | B — incorrect test setup | Corrected iteratively |
| 5 | Resources not `planning_eligible`, binding refused | B — incorrect test setup (no `os_installation_id` supplied) | Added `os_installation_id` in the required format |
| 6 | `union_project_operational_graph` produces no signal for a node dropped from placement (§25) | **D — genuine, pre-existing (Phase 111–115-era) architectural gap**, confirmed by both source tracing and live behavior; out of Phase 119's declared scope | Not fixed; disclosed explicitly |
| 7 | `smartsentinel-operational-reconcile`'s bootstrap step hard-requires real Firestore, incompatible with an isolated placeholder-Firebase DServer (§26) | **A — infrastructure/environment constraint** of this isolated test setup, unrelated to Phase 119's own logic | Not fixed (out of scope, no real Firebase credentials available); physical reconciliation steps skipped and disclosed explicitly |

No production code was modified anywhere in this phase. No fix was applied
to items 6 or 7 — both are recorded as findings for a future phase, exactly
per this task's own failure policy (never patch around a real gap silently).

## 36. Cleanup

- Isolated DServer (PID 1825) stopped after all evidence was captured.
- No firewall rule was created at any point (§2), so none needed removal.
- Mosquitto, any other DServer process, and all unrelated containers/services
  were left untouched throughout.
- Scratch directories on both remote nodes
  (`~/datum-phase119d-src`, `/tmp/phase119d-source`, `~/phase119d-dnode`,
  `~/phase119d-native`) were left in place, per this task's own instruction,
  pending this document's commit.
- Raw evidence (every JSON request/response, both evidence files, build
  logs, the WASM module, the probe script) was retained in the local
  Windows scratch root, outside Git, per this task's own instruction — only
  this concise document was committed.

## 37. What this run does and does not prove

**Proven live, on two real, physically distinct, different-architecture
hosts, using only existing unmodified production APIs:**
- Real DCompile → real DCode registration, zero transformation.
- Real initial placement (A), real D-Engine derivation from it.
- Real content preparation on a pending target with zero effect on
  authority, D-Engine membership, or governed authorization (the direct
  cross-node generalization of Phase 117K's own central finding).
- Real governed DCode execution on A, with an independently-verified output
  hash proving genuine guest-byte execution.
- Real DDeploy acceptance as the sole cutover event, producing a
  monotonically-increasing DMap revision.
- Real, discrete D-Engine and DCode-authorization flips — never a union of
  A and B.
- Real governed DCode execution on B, byte-identical to A's own output,
  proving the same executable followed the accepted placement across a
  real architecture change.
- Real rollback as a brand-new governed proposal/acceptance (revision 3,
  never a rewind to revision 1), with governed execution correctly
  returning to the original node.

**Not proven in this run** (both disclosed as findings, not gaps papered
over):
- Physical Stage107A-governed native-process start/stop on either real
  host — blocked by an environment-specific Firebase-bootstrap dependency
  in `smartsentinel-operational-reconcile`, unrelated to migration logic
  itself (§26).
- Full desired-vs-actual runtime convergence after cutover (depends on the
  above).
- Any evidence bearing on the genuine, pre-existing gap in how (or whether)
  a node ever learns it should stop running a service that migrated away
  (§25) — this is a real architectural question for a future phase, not
  something this run could resolve or was in scope to fix.

## 38. Recommendation

Phase 119 should **not** be considered closed by this run alone. Recommend
**Phase 119E — Independent Migration Closure Review**, explicitly covering
119A (architecture), 119B (pure intent core), 119C (currentness hardening),
and this 119D live run — including independent judgment on: (a) whether the
proven subset (placement/authority/D-Engine/DCode) is sufficient for Phase
119's own closure given the task's original central invariant, or whether
full physical convergence proof is required before Phase 120 may begin, and
(b) how to scope the two disclosed findings (§25/§26) relative to Phase 119
vs. a dedicated future hardening phase.
