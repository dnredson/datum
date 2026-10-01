# Phase 117K — Live Production DCompile → DCode → DDeploy → Real DNode Governed H1/H2 Redeploy Proof

**Status:** Phase 117K LIVE PROOF CLOSED. **Phase 117 remains OPEN pending
independent closure review (Phase 117L).**
**Branch:** `agent/phase117-dcompile`.
**Exact commit under test:** `98bf23ceb440ecd8cd1c953a07c7e062b9b0e378`
(Phase 117J closure).
**No production defect was found.** The repository change from this phase
is documentation only.

## 1. Executive summary

This phase live-proved, against a real DServer control plane and a real
Raspberry Pi D-Node, the central Phase 117 invariant:

> H1 active → compile/register H2 → **H1 remains active** → accept a new
> DDeploy proposal explicitly bound to H2 → H2 becomes active → real
> governed execution switches from H1 bytes to H2 bytes.

The proof began with the real production `POST /api/v1/dcompile/compile`
endpoint (Phase 117J) for **both** H1 and H2 — no `CanonicalDServArtifactV1`
was ever hand-authored. Every downstream artifact (D-Graph, D-Code
descriptor, DDeploy binding map) used in this proof was the compiler's own
verbatim output.

## 2. Topology

| Role | Host | Notes |
|---|---|---|
| Control plane (DServer) | Windows development machine | LAN address `192.168.10.101` (verified live, matches historical expectation) |
| Real D-Node | Raspberry Pi, SSH alias `datum-mist` (`dener@192.168.10.109`) | logical identity `rpi-117k`, architecture `aarch64` |
| fog-01 / cloud-01 | **not started** | single-node topology was sufficient; no production requirement ever forced a second node |
| Windows Mosquitto | **untouched** | DCall was not part of this proof |

## 3. Isolated DServer setup

- Binary: `dserver/target/release/DServer.exe`, built via `cargo build
  --manifest-path dserver/Cargo.toml --locked --release --bin DServer`
  from the exact commit above.
- Isolated scratch root: a phase-specific temp directory outside the Git
  worktree (`%TEMP%\datum-phase117k-<timestamp>`), holding
  `dserver-config.toml`, `control-plane.db`,
  `operational-dgraph-state.json`, stdout/stderr logs, both wasm modules,
  every request/response JSON, and both retrieved evidence files.
- Config: `bind = "0.0.0.0:18080"` (port 18080 confirmed free before use),
  `debug = true`, CORS disabled, an unused placeholder `[firebase]` block —
  mirroring `DATUM/tests/dcall_two_node_integration.rs`'s own
  `spawn_real_dserver` config shape.
- Environment: `DATUM_DSERVER_CONFIG_PATH` / `DATUM_CONTROL_STATE_DB_PATH` /
  `DATUM_OPERATIONAL_DGRAPH_STATE_PATH` all pointed at the isolated scratch
  root.
- Started as its own process (confirmed listening via a direct TCP probe
  and `GET /api/v1/dnodes` → `200 {"count":0,...}`). No other DServer
  process was touched.

## 4. RPi preflight

```
hostname: rpi
uname -a: Linux rpi 6.12.47+rpt-rpi-v8 #1 SMP PREEMPT Debian 1:6.12.47-1+rpt1 (2025-09-16) aarch64 GNU/Linux
uname -m: aarch64
rustc: 1.94.1 (source ~/.cargo/env; not on the non-interactive PATH by default)
cargo: 1.94.1
git: 2.47.3
nproc: 4
total memory: ~3.98 GB
```

TCP and HTTP reachability from the RPi to
`http://192.168.10.101:18080` were both confirmed directly (a Python
socket probe, then a real `curl` returning `200`) **before any firewall
change was considered** — none was needed; no Windows firewall rule was
created or modified.

## 5. RPi source tree

`git clone` over HTTPS from the RPi failed (`fatal: could not read
Username for 'https://github.com'` — no GitHub credentials configured on
this host, a pre-existing environment fact unrelated to this phase).
Following the task's own explicit fallback ("if cloning is unavailable...
use a separate clean scratch copy"), the exact HEAD commit was instead
transferred as a `git archive --format=tar.gz HEAD` produced on the
Windows control host immediately after confirming `git rev-parse HEAD ==
98bf23c...`, copied via `scp`, and extracted into a fresh
`~/datum-phase117k-src` on the RPi. This is a byte-exact snapshot of the
same commit a `git clone --branch agent/phase117-dcompile` at that moment
would have produced; it is disclosed here explicitly rather than
mis-described as a plain clone.

## 6. RPi binaries

Built via `cargo build --manifest-path DATUM/Cargo.toml --locked --release
--bin smartsentinel-dcode-init --bin smartsentinel-dcode-install --bin
smartsentinel-dcode-invoke --bin smartsentinel-dcode-worker` directly on
the RPi (first full build of the `wasmtime`/`cranelift` dependency tree on
this host; took roughly 30 minutes). All four binaries confirmed present
in `DATUM/target/release/`. `smartsentinel-dcode-invoke`'s sibling-worker
resolution (`trusted_sibling_worker_binary_path`, based on
`current_exe()`'s own directory) resolved correctly with no extra
provisioning, since all four binaries live in the same `target/release/`
directory.

## 7. Live identities

```
PROJECT     = proj:phase117k-live
APPLICATION = app:phase117k-live
DSCRIPT     = dscript:phase117k-live   (revision 1, for both H1 and H2)
DGRAPH      = dgraph:phase117k-live    (revision 1, for both H1 and H2)
SERVICE     = probe
FUNCTION    = probe_fn
DNODE       = rpi-117k
INPUT_PORT  = trigger
```

## 8. H1/H2 module generation

Generated via a temporary, standalone Rust helper (`wat` crate, matching
the version DATUM itself depends on) built OUTSIDE the repository — reusing
the exact WAT pattern already proven in
`DATUM/tests/dcall_two_node_integration.rs::respond_ok_wat`, never
hand-written binary bytes. Neither module, nor the helper project, nor any
generated artifact was committed.

| | SHA-256 | Size |
|---|---|---|
| H1 (`{"status":"ok","reason":"phase117k-v1","emits":[]}`) | `ee9fab274a18dc8ae009878e2d895c90b562b7c07d9c31ea4286a57f05a1ff51` | 247 bytes |
| H2 (`{"status":"ok","reason":"phase117k-v2","emits":[]}`) | `4a0fef0fad66cf58a893e7d9d9510ca94a75c972e2fcd6de2cd29d229210dc41` | 247 bytes |

`H1 SHA != H2 SHA` — confirmed. Sizes are legitimately equal (permitted by
the task).

Expected raw-reason digests (independently computed, later compared
against live evidence):

```
SHA256("phase117k-v1") = ba89a8ee805715f2cf8ba7562549d52d40023047f2500d08ea6b5486177e3877
SHA256("phase117k-v2") = 78efe14bd9166ebff1f52768d476b5729e185832ab48f67e10c06f3f5adc799c
```

## 9. DNode declaration and ServiceArtifact bridge

`PUT /api/v1/dnodes/rpi-117k` — `platform.architecture = "aarch64"` (the
RPi's real, verified architecture), `capacity = {cpu_cores: 4,
memory_bytes: 3_000_000_000}` (conservatively under the RPi's real ~3.98 GB
and comfortably above the 64 MiB compile requirement),
`capabilities.dnode_abi_versions = ["datum-dnode/0"]`. Response:
`{"inserted":true,"registry_revision":1}`.

`POST /api/v1/service-artifacts` (header `X-Datum-Project:
proj:phase117k-live`) — a minimal, inert `example.invalid` container
`ServiceArtifact` for `service_id = probe`, `target_nodes = ["rpi-117k"]`,
following `DATUM/tests/dcall_two_node_integration.rs`'s own
`minimal_service_artifact` precedent exactly. **This exists ONLY to
satisfy DDeploy v1's current, separate `ServiceArtifact`-eligibility
lineage** (`resolve_artifact_for_service`, ADR-0002) — actual execution in
this proof is canonical D-Code on the real RPi; this container image is
never pulled, run, or referenced by the invocation path at all. Declared
successfully.

## 10. H1 — real production DCompile call

`POST /api/v1/dcompile/compile`, submission built entirely from true
external facts (DScript, mapping, H1's real declared `sha256`/`size_bytes`)
— no `dscript_ref`/`dgraph_ref`/digest supplied by the caller anywhere.
Response `200`, `schema: "datum.dcompile-candidate/1"`. All Section-20
assertions confirmed directly against the live response:

```
application_id            == app:phase117k-live                (response + dgraph, both)
dgraph.calls               == []
dgraph.services             == [probe] only
dcode_artifacts             == [probe] only
artifact.dcode.sha256       == H1 SHA (verbatim)
artifact.dcode.size_bytes   == 247 (verbatim)
dcode_artifact_authority.probe.descriptor_digest == dcode_artifacts[0].descriptor_digest
```

Recorded identities:

```
dscript_ref.digest          = sha256:5e09ee787f25910a56d587bb95f8d6619546b385bd056f0a270ab5756179eaf1
dcompile_request_digest     = sha256:683c66c0cc75743a5cc5f3c8673911f141b89aed8e36c51a287df2751ec5905d
dgraph_ref.digest           = sha256:40c77cdbb56bf9ef8053fb048cbbf30240eb5de2d3a89903e6e4a34c4a6abb00
D1 descriptor_digest        = sha256:4d13055e1791fd8528e5173cece84f409d21df7c9a61b5cb0c0d8cb9c85f81c1
```

## 11. D1 registration (zero-transformation)

`compile-h1.json.dcode_artifacts[0].artifact`, extracted and POSTed
**completely unmodified** to `POST /api/v1/dcode/artifacts`. Response:
`{"inserted":true,"descriptor_digest":"sha256:4d13055e...81c1"}` — an
EXACT byte-for-byte match with the compiler's own `descriptor_digest`. No
translation layer of any kind existed between the compiler response and
the registration call.

## 12. P1 creation and acceptance

`POST /api/v1/ddeploy/proposals` (header `X-Datum-Project:
proj:phase117k-live`): `dgraph` = `compile-h1.json.dgraph` verbatim,
`dcode_artifact_authority` = `compile-h1.json.dcode_artifact_authority`
verbatim, plus a schema-valid one-node `dcontinuum` placeholder (correctly
overwritten server-side by the real D-Node-registry-derived authoritative
D-Continuum, per ADR-0019 — exactly the documented, expected behavior) and
`placements: [{service_id: probe, node_id: rpi-117k}]`. Response `201`,
`state: "pending"`, `dgraph_ref.digest` matching the compiler's own
`dgraph_ref.digest` exactly.

`POST /api/v1/ddeploy/proposals/:id/accept` (`accepted_by:
"phase117k-live-proof"`) → `200`, `currently_active: true`,
`materialized_dmap_ref = {revision: 1, digest:
sha256:33d583de...becceb}`.

## 13. H1 authorization

`POST /api/v1/dcode/authorize` → `200`:

```
dserv_artifact_digest            == ee9fab27...a1ff51   (== H1 SHA, exact)
dserv_artifact_descriptor_digest == sha256:4d13055e...81c1  (== D1, exact)
authorization_id                 == dcode-auth:...:probe:rpi-117k:1
```

## 14. Real governed H1 execution

`smartsentinel-dcode-install --module-path h1.wasm --declared-sha256
<H1_SHA> --store-root ~/datum-phase117k/dnode-root` → byte-verified,
content-addressed install succeeded (this is the manual, documented
provisioning step — no distribution mechanism was implemented, per Section
45 of the task).

`smartsentinel-dcode-invoke invoke --dserver-url
http://192.168.10.101:18080 ... --dnode-root ~/datum-phase117k/dnode-root
--evidence-out evidence-h1.json` → `mode=smartsentinel-dcode-invoke
status=ok outcome=Success node_id=rpi-117k service_id=probe
host_mutation=false`.

Evidence (`evidence-h1.json`), every required field confirmed:

```
outcome                          = success
node_id                          = rpi-117k
dserv_artifact_digest            = ee9fab27...a1ff51   (H1, exact)
dserv_artifact_descriptor_digest = sha256:4d13055e...81c1  (D1, exact)
output_status                    = ok
output_reason_present            = true
output_reason_sha256             = ba89a8ee805715f2cf8ba7562549d52d40023047f2500d08ea6b5486177e3877  (exact match to independently-computed SHA256("phase117k-v1"))
```

## 15. H2 — real production DCompile call

Same endpoint, submission identical except `dcode_services[0].sha256`/
`size_bytes` now name H2's real facts. Response `200`.

**Pure-recompile invariant, verified programmatically against the two live
responses:**

| Field | H1 == H2? |
|---|---|
| `dscript_ref` | **equal** |
| `mapping` | **equal** |
| `dcompile_request_digest` | **equal** |
| `dgraph` | **equal** |
| `dgraph_ref` | **equal** |
| module `sha256` | **differs** |
| `descriptor_digest` | **differs** |
| `dcode_artifact_authority` binding | **differs** |

This is the load-bearing Phase 117 proof (task Section 28), confirmed
against real, live compiler output, not a unit test.

```
D2 descriptor_digest = sha256:8fbd19c57771027a47d44e9129126a3092df03f86c2e407d055aed68df4dd7f8
```

## 16. D2 registration and D1/D2 coexistence

`compile-h2.json.dcode_artifacts[0].artifact`, unmodified, POSTed to
`/api/v1/dcode/artifacts` → `{"inserted":true,"descriptor_digest":
"sha256:8fbd19c5...4dd7f8"}` — exact match.

Immediately afterward, three live registry checks (Section 30):

```
GET .../probe/sha256:4d13055e...81c1  (exact D1)  -> 200  (D1 still retrievable)
GET .../probe/sha256:8fbd19c5...4dd7f8 (exact D2)  -> 200  (D2 retrievable)
GET .../probe                (legacy logical)      -> 409 {"error":"dserv_artifact_lookup_ambiguous",
                                                             "message":"... 2 revisions are registered ..."}
```

The `409` is EXPECTED SECURITY BEHAVIOR (Phase 117G), not a defect.

## 17. H2 bytes installed before activation (still D1 active)

`smartsentinel-dcode-install` for H2 succeeded — both `h1.wasm` and
`h2.wasm` now physically resident in the same content-addressed store.

**Central P0 proof (Section 32 of the task), the strongest single result
of this phase:**

- A fresh `POST /api/v1/dcode/authorize` call, taken AFTER D2 was both
  registered AND its bytes installed, still returned
  `authorization_id: dcode-auth:...:probe:rpi-117k:1` — literally the
  SAME authorization identity as §13's H1 authorization —
  `dserv_artifact_digest == H1`, `dserv_artifact_descriptor_digest == D1`.
- A SECOND real governed invocation, run at this exact point (H2
  registered, H2 bytes present, P2 not yet even created), produced
  `evidence-before-p2.json` with `dserv_artifact_digest == H1`,
  `descriptor_digest == D1`, and `output_reason_sha256 ==
  SHA256("phase117k-v1")` — the RPi genuinely re-executed H1's real bytes,
  not merely "would have."

**Registering AND installing D2/H2 provably changed nothing about live
active execution.**

## 18. P2 — the governed redeploy

`POST /api/v1/ddeploy/proposals`: identical `dgraph`/`dcontinuum`/
`placements` to P1, `dcode_artifact_authority` =
`compile-h2.json.dcode_artifact_authority` verbatim (the ONLY semantic
difference from P1). Response `201`, `pending`,
`expected_active_dmap_revision: 1` (correctly detected P1's D-Map as the
current baseline).

`POST .../accept` (`accepted_by: "phase117k-live-proof"`) → `200`,
`currently_active: true`, `materialized_dmap_ref = {revision: 2, digest:
sha256:3cc80a33...7003d8}` — same `dmap_id`, revision `1 → 2`.

## 19. Post-P2 authorization: now names D2/H2

```
authorization_id                 = dcode-auth:...:probe:rpi-117k:2   (a NEW authorization identity)
dserv_artifact_digest            = 4a0fef0f...9dc41   (== H2, exact)
dserv_artifact_descriptor_digest = sha256:8fbd19c5...4dd7f8   (== D2, exact)
dgraph_ref.digest                = sha256:40c77cdb...a6abb00  (UNCHANGED from H1's own dgraph_ref)
dmap_ref: revision 1 (digest 33d583de...becceb) -> revision 2 (digest 3cc80a33...7003d8)
```

## 20. Honest old-authorization-staleness claim

There is intentionally **no** public `/verify-old-authorization` HTTP
endpoint, and none was added for this proof. The live evidence for
staleness is therefore, precisely and only:

1. §13's authorization was issued while `active_by_application` bound
   proposal P1/D1 (confirmed by its own `dmap_ref` = revision 1).
2. §18/§19 show a LATER acceptance (P2) changed the current active binding
   to D2, and a freshly-issued authorization now carries a DIFFERENT
   `authorization_id`, `dmap_ref` (revision 2), and descriptor digest.
3. Therefore the §13 authorization no longer describes current active
   authority — by direct comparison of two real, live server responses at
   two points in time, not by an HTTP "verify" call this session invoked.

The actual re-derivation-and-comparison logic this observation rests on
(`dcode_authorization::verify_dcode_authorization_still_current`) already
has focused, deterministic unit tests proving it returns exactly this
"stale" condition (`core::dcode_authorization`'s own test suite, unchanged
by this phase). This phase does not claim to have invoked that function
live over HTTP, because no such endpoint exists; it claims only the two
real authorization snapshots above, which is what a real caller genuinely
observes.

## 21. Real governed H2 execution

`smartsentinel-dcode-invoke invoke` (identical command to §14) →
`status=ok outcome=Success`. Evidence (`evidence-h2.json`):

```
outcome                          = success
service_id / node_id             = probe / rpi-117k
dserv_artifact_digest            = 4a0fef0f...9dc41  (H2, exact)
dserv_artifact_descriptor_digest = sha256:8fbd19c5...4dd7f8  (D2, exact)
dgraph_digest                    = sha256:40c77cdb...a6abb00  (SAME as H1's evidence)
output_status                    = ok
output_reason_sha256             = 78efe14bd9166ebff1f52768d476b5729e185832ab48f67e10c06f3f5adc799c  (exact match to SHA256("phase117k-v2"))
```

Cross-checked against §14's H1 evidence:

```
evidence-h1.output_reason_sha256 != evidence-h2.output_reason_sha256   -- CONFIRMED DIFFERENT
evidence-h1.dserv_artifact_digest != evidence-h2.dserv_artifact_digest -- CONFIRMED DIFFERENT
```

**Real WASM execution on the real RPi produced the exact, independently
predicted reason string for each revision.** This is proof of an actual
guest-behavior change, not merely a metadata change.

## 22. D1 remains historical, not destroyed

After P2/H2 became active, `GET
/api/v1/dcode/artifacts/app:phase117k-live/probe/sha256:4d13055e...81c1`
(the exact D1 digest) still returned `200`. Registry existence and active
authority remain fully independent facts, exactly as designed in 117G —
now confirmed live, not only by unit test.

## 23. Final live invariant table

| Stage | Registered revisions | Installed bytes | Active descriptor | Executed module |
|---|---|---|---|---|
| After P1 | D1 | H1 | D1 | H1 |
| After D2 registration | D1, D2 | H1 | D1 | H1 |
| After D2 installation | D1, D2 | H1, H2 | D1 | H1 |
| Pending P2 (before acceptance) | D1, D2 | H1, H2 | D1 | H1 |
| Accepted P2 | D1, D2 | H1, H2 | D2 | H2 |

Every row through "After D2 installation" is backed by a real, live HTTP
call or a real governed invocation taken at exactly that point (§16-§17).
The "Pending P2" row was not probed by a SEPARATE live call strictly
between P2's creation and its acceptance (this proof created and accepted
P2 in immediate succession) — its state is inferred from combining §17's
already-live "before P2 existed at all" observation with Phase 117G's own
already-established, independently unit-tested guarantee that
`create_proposal` never touches `active_by_application` (e.g.
`j23_pending_proposal_with_binding_activates_nothing`, 117G's own test
suite, unchanged by this phase) — disclosed here explicitly rather than
implied to be a fourth live HTTP probe.

## 24. Five-identity provenance, live

| Identity | H1 | H2 | Changed? |
|---|---|---|---|
| D-Script digest | `sha256:5e09ee78...79eaf1` | same | **No** |
| D-Compile-request digest | `sha256:683c66c0...5905d` | same | **No** |
| D-Graph digest | `sha256:40c77cdb...a6abb00` | same | **No** |
| Module SHA-256 | `ee9fab27...a1ff51` | `4a0fef0f...9dc41` | **Yes** |
| Descriptor digest | `sha256:4d13055e...81c1` | `sha256:8fbd19c5...4dd7f8` | **Yes** |

Exactly matching the Phase 117D/E model: source/mapping/topology identity
is invariant under a pure D-Code-only recompile; module and descriptor
identity are the only things that change.

## 25. Explicit exclusions honored

- No D-Call of any kind (`dgraph.calls == []` throughout; confirmed
  directly in both compile responses).
- No `smartsentinel-dcall-listen`, no MQTT DCall transport, no DCall
  grants.
- No `--dmonitor-publish` used — not required for closure; the
  authoritative proof is DCompile output + registry + accepted DDeploy
  authority + fresh authorization + real invocation evidence, all of which
  this phase captured directly.
- fog-01/cloud-01 were never started — the single real RPi D-Node was
  sufficient for every step; no production requirement ever forced a
  second node.
- No automated module-distribution mechanism was added — `scp` +
  `smartsentinel-dcode-install` (both pre-existing, real production
  binaries) were the only provisioning tools used.
- No repository source file was modified by this phase (documentation
  only) — no production defect was discovered during live validation.

## 26. Cleanup

- The isolated DServer process (PID recorded in
  `%TEMP%\datum-phase117k-<timestamp>\dserver.pid`) was stopped after all
  evidence above was captured and copied back to the control host.
- No Windows firewall rule was ever created (reachability worked without
  one), so none needed removal.
- Mosquitto and every unrelated DServer/process were left completely
  untouched throughout.
- RPi scratch directories (`~/datum-phase117k-src`,
  `~/datum-phase117k/`) were left in place per the task's own instruction
  ("RPi scratch files may remain until the documentation is committed").
- No database file, module binary, full log, or secret was committed —
  this document quotes only the specific field values needed to prove each
  invariant.

## 27. Recommendation

All 30 Phase 117K stop conditions are satisfied. Recommend **Phase 117L —
Independent Phase 117 Closure Review**, auditing the complete chain
(117A-117K) and deciding whether Phase 117 can finally close before Phase
118 (D-Engine). Phase 118 was not started during this phase.
