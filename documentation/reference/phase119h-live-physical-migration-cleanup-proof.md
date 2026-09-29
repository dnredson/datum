# Phase 119H — Live Physical A→B→A Migration, Reconciliation and Governed Source-Cleanup Re-Proof

- Status: Live-infrastructure validation. Zero DATUM/DServer production source
  files changed by this phase — only this document. (One real, disclosed host
  configuration change was made to fog-01's Docker daemon — see §11.)
- Branch: `agent/phase119-migration`, exact source commit
  `2f6df9a1a6274c259fa8b6c0cf89bb2b790ebc16` on Windows control plane, RPi,
  and fog-01 alike.
- Scope: the live physical proof Phase 119D could not complete — real
  Stage107A-governed container start/stop on two real, physically distinct
  hosts, a real cutover, real governed source cleanup crossing the M8
  boundary on both sides of an A→B→A cycle, and live proof of Phase 119H0's
  diagnosed Firebase-bootstrap coupling plus Phase 119H1's fix.

## 1. Executive summary

**Every central live claim in this phase's task was proven, on two real
physically distinct D-Nodes, using exclusively the real production
`smartsentinel-operational-reconcile` binary and the unmodified DServer HTTP
API — no manual `docker run`/`docker rm` was ever used as a success path.**

- Real physical container start/stop, previously blocked entirely by
  Phase 119D's Firebase-bootstrap finding, now works — proven identically on
  both nodes.
- The exact required authority/physical sequence (DMap rev1→rev2→rev3,
  `S: A→B→A`) was realized with real Docker containers on both architectures,
  including the required temporary physical overlap at each cutover.
- Every new Phase 119G `datum.dmap_id`/`datum.dmap_revision`/`datum.dmap_digest`
  container label was observed live, on real running containers, matching
  the exact accepted DMap reference at every stage.
- Governed source cleanup was proven live in **both** directions: A's stale
  rev1 runtime was removed after B's cutover, and B's stale rev2 runtime was
  removed after the rollback to A — both crossing the real M8 mutation
  boundary under a distinct `operation="cleanup"` authorization, both
  preceded by a non-mutating dry-run, and both preceded by a live negative
  proof that a directive alone (no valid cleanup authorization) causes zero
  mutation.
- The Phase 119G history algorithm's central correctness property —
  current-return-to-source suppression — was proven live: after the rollback
  acceptance, A (which the rev1 history would naively implicate) received an
  **ordinary desired-service entry, zero cleanup directives**, while B
  correctly received the rev2 cleanup directive.
- Phase 119H0's diagnosis and Phase 119H1's fix were proven live: both
  nodes' bootstrap calls genuinely could not resolve the legacy
  Firebase-backed node/capability endpoints against this isolated
  environment (confirmed via a direct, real `403 CONSUMER_INVALID` from
  Firestore), yet both bootstraps still reached `KnownBoundNode` and
  handed off cleanly to reconciliation — exactly the fix's design.
- Canonical placement authority was single-active throughout — never a union
  of A and B — confirmed by direct `GET /api/v1/ddeploy/active` reads at
  every stage.

**One genuine, real, pre-existing (not Phase 119G/119H1) finding was hit and
resolved via a disclosed host configuration change, not a source
workaround** — see §11.

## 2. Physical topology

| Logical name | Alias | Address | User | Architecture | OS | Docker |
|---|---|---|---|---|---|---|
| Windows control plane | — | 192.168.10.101 | (local) | x86_64 | Windows 11 | — |
| RPi (source A, then rollback target) | `datum-mist` | 192.168.10.109 | `dener` | aarch64 | Debian (Linux 6.12.47) | 26.1.5, `overlay2` graphdriver |
| fog-01 (target B) | `datum-fog` | 192.168.10.103 | `vboxuser` | x86_64 | Ubuntu/Xubuntu (Linux 6.17.0) | 29.1.3, reconfigured to `overlay2` mid-phase (§11) |
| cloud-01 | `datum-cloud` | 192.168.10.108 | — | — | **NOT used, NOT started** |

fog-01 was a VirtualBox VM found powered off at the start of this phase and
started headlessly for this proof, per explicit user confirmation of its
identity. No firewall changes were required on either node; both reached the
isolated DServer directly over the LAN.

## 3. Logical identity mapping

| Concept | Value |
|---|---|
| SOURCE_NODE_ID | `rpi-119d` (physical: RPi) |
| TARGET_NODE_ID | `fog-119d` (physical: fog-01) |
| PROJECT | `proj:phase119h-live` |
| APPLICATION | `app:phase119h-live` |
| DGRAPH | `dgraph:phase119h-live` |
| SERVICE | `probe` |
| CONTAINER_NAME | `datum-phase119h-probe` |
| ARTIFACT | `artifact:phase119h-probe` (`alpine:3.20`, runtime kind `container`) |

## 4. Pre-flight

| Check | RPi | fog-01 |
|---|---|---|
| `uname -m` | aarch64 | x86_64 |
| `nproc` | 4 | 4 |
| RAM | 3,981,172,736 bytes | 8,325,046,272 bytes |
| Disk free (before cleanup) | 15G | **1.7G (97% full)** |
| Docker | 26.1.5, present, real | 29.1.3, present, real |
| rustc/cargo | 1.94.1 (rustup) | 1.97.1 |
| git | 2.47.3 | 2.51.0 |

fog-01's disk was critically low from accumulated scratch directories left
by earlier phases (`~datum-i38-stage104`, `~datum-114e[.bundle]`,
`~datum-b4b`, `~datum-phase119d-src`, `~datum-i35`, `~datum-i38`,
`~chirpstack-provisioner-test` — ~17GB total). Per explicit user
confirmation, these were removed (a stale `~/PD` checkout with uncommitted
local changes was deliberately left untouched), freeing the disk to 17G
available before any build was attempted.

## 5. Exact source distribution

Exact commit `2f6df9a1a6274c259fa8b6c0cf89bb2b790ebc16` distributed via
`git archive --format=tar.gz -o phase119h-src.tar.gz HEAD` → `scp` → extract
into `~/datum-phase119h-src` on each remote node (no HTTPS git credentials
exist on either host — the same disclosed substitution Phase 119D used).

## 6. Binaries built

Per node, from the exact distributed source: `--bin
smartsentinel-operational-reconcile`, `--bin smartsentinel-resource-discovery`,
`--bin smartsentinel-bootstrap` (`cargo build --release --locked`). fog-01:
2m24s for the first binary (all deps), then incremental (~2s) for the other
two. RPi: 30m36s for the first binary (matches Phase 119D's own estimate for
this hardware's `wasmtime`/`cranelift` compile time), then 29s incremental.
On Windows: `dserver`'s `DServer` release binary, already current from this
session's earlier work.

## 7. Isolated DServer

- Scratch root (outside the Git worktree): a session-local temp directory.
- Config: `[server] bind="0.0.0.0:18080", debug=true`; `[firebase]
  project_id="phase119h-unused", credentials_path="unused.json"`; `[cors]
  enabled=false` — the same placeholder-Firebase pattern Phase 119D and
  Phase 117K used.
- `DATUM_DSERVER_CONFIG_PATH`/`DATUM_CONTROL_STATE_DB_PATH` pointed at the
  scratch root (Phase 119H0 confirmed operational-dgraph state now shares
  this same unified SQLite database — no separate env var needed, unlike
  Phase 119D's era).
- Confirmed listening on `127.0.0.1:18080`, `192.168.10.101:18080`, and from
  both remote nodes before any other work began. No other DServer process,
  and no Mosquitto/DCall/MQTT anything, was touched at any point. Stopped
  cleanly at the end of this phase after all evidence was captured.

## 8. Resource Registry — real discovery and binding

Both nodes ran the real `smartsentinel-resource-discovery` binary against
their own local hardware, then the resulting real discovery report was
`POST`ed to `/api/v1/resources/discovery`, then bound via `POST
/api/v1/resources/:resource_id/binding` — every fact locally, genuinely
observed, never invented:

| Node | Resource ID | Identity scope | Binding ID |
|---|---|---|---|
| rpi-119d | `resource:hw:c1bd91343a8f910a044601105b4910360eb96418d672b0ddd54020c98ad3c92d` (real board-serial hardware fingerprint) | `canonical_hardware` | `binding:4f16d2cf-d16a-44ca-ab65-00a03603895f` |
| fog-119d | `resource:os:9d564a4d83804cff3b62e84394a8da53d8610c717cd892623a2242ebaabb6e09` (no hardware fingerprint on this VM; OS-install-id fallback) | `provisional_os` | `binding:322660d6-1f8e-4a9c-9264-4d188b9ece26` |

Structural D-Nodes `rpi-119d`/`fog-119d` were declared via `PUT
/api/v1/dnodes/:node_id` using each node's own real, freshly-probed
platform/capacity facts (never copied from one node onto the other).

## 9. Bootstrap H1 live proof

`smartsentinel-bootstrap` (the exact `run_bootstrap` reconciliation itself
uses) run standalone on each node:

| Node | `bootstrap_state` | `resolved_node_id` | `node_known` |
|---|---|---|---|
| rpi-119d | **KnownBoundNode** | `rpi-119d` (correct) | **false** |
| fog-119d | **KnownBoundNode** | `fog-119d` (correct) | **false** |

`node_known=false` on both is direct, live confirmation that the
legacy-Firebase-backed `query_node_context` call genuinely could not
succeed (§14) — yet bootstrap reached `KnownBoundNode` on both nodes anyway,
purely via the Resource Registry binding, exactly as Phase 119H1's
`resolve_node_context_enrichment` was designed to do. Every subsequent real
`smartsentinel-operational-reconcile --execute`/`--cleanup` invocation on
both nodes (§10 onward) confirms this held for the real, full reconciliation
path too, not merely the standalone bootstrap check.

## 10. Runtime kind, artifact, and one iterative-discovery note

Container runtime (`alpine:3.20`, `command: ["sleep", "infinity"]`,
`container_name: "datum-phase119h-probe"`), registered via
`POST /api/v1/service-artifacts`, matching this session's own confirmed-current
`ServiceArtifact` schema (`0.2.0`). One expected, minor iteration against the
real server's own validation (mirroring Phase 119D's own methodology,
never treated as a defect): an initial registration produced
`unsupported service artifact schema_version` (used the wrong version
string), corrected immediately.

## 11. Real finding: containerd image-store vs. Stage107B `.Id` verification (disclosed, resolved via host config, not source)

Registering the artifact under the server's unconditional
`execution_pinning_mode = required` default (confirmed live: **no
production HTTP route exists to lower a project's pinning mode** — only
test code calls `ServiceArtifactRegistry::set_pinning_mode` anywhere in this
codebase) required real OCI image-identity pinning
(`POST /api/v1/service-artifacts/:id/image-identity/resolve`, real query
against `registry-1.docker.io`, real per-platform manifest/config digests
for both `linux/amd64` and `linux/arm64/v8`).

A's realization on RPi succeeded immediately. B's realization on fog-01
failed with `blocked_image_identity_mismatch: config_mismatch`. Root-caused
live, not guessed: fog-01's Docker Engine used the **containerd image-store
snapshotter** (`docker info` → `driver-type: io.containerd.snapshotter.v1`),
under which `docker image inspect`'s `.Id` field reports the OCI **manifest**
digest rather than the **config** digest —
`DATUM/src/agent/image_identity_verification.rs`'s identity check compares
`.Id` against the resolved `config_digest`, which assumes classic Docker
image-ID semantics. **Independently confirmed DATUM's own OCI resolver was
correct**, by querying `registry-1.docker.io`'s real manifest directly and
confirming its declared `config.digest` matched what DServer had resolved
exactly. RPi's classic `overlay2` graphdriver does not exhibit this, which
is exactly why A worked on the first corrected attempt and B did not.
Reproduced twice (including from a fully clean local image cache) to rule
out any artifact of earlier manual `docker pull` testing.

**Classification: C — a genuine, real, pre-existing (Stage107B/ADR-0013 era)
compatibility gap, entirely unrelated to Phase 119G's cleanup logic or
Phase 119H1's bootstrap fix.** Per explicit user decision, resolved by
reconfiguring fog-01's Docker daemon
(`/etc/docker/daemon.json: {"features": {"containerd-snapshotter": false}}`,
`systemctl restart docker`) back to the classic `overlay2` image store — a
disclosed, reversible **host configuration change**, not a DATUM source
change, workaround, or fake identity of any kind. fog-01 had zero other
running containers at the time. After this change, B's realization
succeeded immediately with a correctly verified image. **No DATUM source
was modified to resolve this finding.**

A second, small iterative-discovery item (same category as Phase 119D's own
methodology, not a defect): `required` mode's `blocked_local_policy_substitution`
required explicitly declaring `network_aliases: ["probe"]` on the
`ContainerDeploymentSpec` (Stage107A finding F2c) rather than relying on the
agent's local `service_name` substitution — corrected by one `PUT` update.

## 12. rev1 — initial placement A

`POST /api/v1/ddeploy/proposals` → `POST .../accept`:

- Proposal `ddeploy-proposal:7ac53481-4fbf-460b-b8af-24703c116ab2`, digest
  `sha256:541d1598ca1f4e77512c7fad5bb143ab2ed606d6de1c8968e4714ae04e9849ce`.
- Acceptance `ddeploy-acceptance:29aec633-0219-44f6-899b-6984e4cf5471`,
  materialized `dmap:19:proj:phase119h-live18:app:phase119h-live` **rev 1**,
  digest `sha256:f31d439059b487bef98f6c8efd06f80e583e9f2239bb1b25d948f1900feec978`.
- `GET /api/v1/ddeploy/active` → `placements: [{probe, rpi-119d}]`.
  **D-Engine = `{rpi-119d}`.**

## 13. rev1 physical realization on A

Authorized (`operation=reconcile`, durable, finite lease, current
binding/revision) → `smartsentinel-operational-reconcile --execute` on RPi
→ `host_mutation=true`, `converged_after=1`. Live `docker inspect`:

```
State.Running: true
Config.Labels:
  datum.artifact_id:               artifact:phase119h-probe
  datum.execution_generation:      3
  datum.execution_projection_digest: sha256:f2489b562c...40f7d
  datum.dmap_id:                   dmap:19:proj:phase119h-live18:app:phase119h-live
  datum.dmap_revision:             1
  datum.dmap_digest:               sha256:f31d439059b487bef98f6c8efd06f80e583e9f2239bb1b25d948f1900feec978
```

`datum.dmap_digest` matches rev1's accepted DMap digest **exactly** — the
first live confirmation of the new Phase 119G container labels anywhere in
this project. B confirmed physically absent (`docker ps -a` empty on fog-01)
at this baseline.

## 14. B pending, pre-cutover proof

`POST /api/v1/ddeploy/proposals` for `{probe, fog-119d}` →
`expected_active_dmap_revision: 1` (auto-bound). **Not accepted yet.**
Re-verified live while pending: active DMap still rev1/A; A's operational
context still `cleanup_directives: []` (no cleanup synthesized from a merely
pending proposal, per M16/M17).

## 15. Cutover — rev2

`POST .../accept` for the pending proposal:

- Acceptance `ddeploy-acceptance:2cfea26e-ed6a-416c-a6f3-e026e165252a`,
  materialized DMap **rev 2**, digest
  `sha256:c9d020838b778c224c9a2766467e62f3b59adcb7ebea62e835412a92940511cd`,
  `placements: [{probe, fog-119d}]`.

Immediately after acceptance, `GET .../active` confirms **D-Engine =
`{fog-119d}`** — a discrete flip, never a union.

## 16. A's cleanup-only context — the central Phase 119G live proof

`GET /api/v1/nodes/rpi-119d/operational-reconciliation-context`
immediately after cutover:

```json
"desired_services": [],
"artifacts": [],
"cleanup_directives": [
  {
    "service_id": "probe",
    "application_id": "app:phase119h-live",
    "previous_dmap_ref": { "revision": 1, "digest": "sha256:f31d439059b4...eec978" }
  }
]
```

rpi-119d has zero current placements (absent from ordinary
`OperationalDGraph.nodes`), yet DServer served a **valid, non-404,
cleanup-only context** — exactly Phase 119F-R1/119G's designed behavior,
confirmed live for the first time anywhere in this project: the cleanup
directive names the exact historical rev1 `dmap_ref`, never a fabricated or
generic signal.

## 17. B's ordinary context

`GET /api/v1/nodes/fog-119d/operational-reconciliation-context` →
`desired_services: [{probe, running, dmap_ref revision 2}]`,
`cleanup_directives: []` — no cleanup ever targets a node's own current
placement.

## 18. B realized — temporary physical overlap

Authorized and realized identically to A (§13; hit and resolved the finding
in §11 along the way). `docker inspect` on fog-01:

```
datum.dmap_revision: 2
datum.dmap_digest:   sha256:c9d020838b778c224c9a2766467e62f3b59adcb7ebea62e835412a92940511cd
```

At this exact point: **A still physically running (rev1 labels, confirmed
live), B now physically running (rev2 labels)** — the required temporary
overlap — while `GET .../active` and D-Engine both confirm **canonical
authority is exclusively B**. This distinction (physical overlap ≠
authority overlap) is the central property this whole phase exists to
prove, and it held exactly as designed.

## 19. Negative proof: a directive alone is not mutation authority

Before any cleanup authorization existed for A:
`smartsentinel-operational-reconcile --cleanup --execute` on RPi →
`cleanup_directives=1` (the directive genuinely observed) but
`cleanup_mutated=0`, `host_mutation=false`. Live `docker inspect` confirms
A's container **still running**. This is real, live proof that
`cleanup_directives` presence alone can never authorize a mutation — the
distinct `operation="cleanup"` authorization is genuinely load-bearing, not
theoretical.

## 20. A cleanup — authorize, dry-run, execute, M8

`POST /api/v1/nodes/rpi-119d/operational-reconciliation/cleanup-authorize`
(`operation: "cleanup"`, durable, finite lease, current rev2) →
`authorization_id: opauth:06b0d75e-9a0f-4d59-89c5-6fa0cfa4cc00`,
`artifact_bindings: []`, `snapshot: []` (exactly Phase 119G's design — no
historical artifact identity fabricated).

**Dry-run** (`--cleanup`, no `--execute`): `status:
"cleanup_observed_match_not_executed"`, exact rev1 `previous_dmap_ref`
match, `local_runtime_kind: "container"`, `host_mutation_performed: false`
— container confirmed still running.

**Execute** (`--cleanup --execute`): `cleanup_mutated=1`.

```json
"status": "rolled_back",
"host_mutation_performed": true,
"mutation_boundaries_crossed": [{
  "boundary": "M8",
  "authorization_id": "opauth:06b0d75e-9a0f-4d59-89c5-6fa0cfa4cc00",
  "expires_at_utc": "2026-09-29T12:33:25...",
  "result": "passed"
}]
```

`docker ps -a` on RPi confirms the container **physically absent**. B
confirmed still physically running.

**First converged state: A physically absent, B physically running,
canonical authority B.**

## 21. Rollback — a new transition, rev3

`POST /api/v1/ddeploy/proposals` for `{probe, rpi-119d}` →
`expected_active_dmap_revision: 2` (auto-bound to B's current revision,
never a reused rev0/rev1 value) → `POST .../accept`:

- Acceptance `ddeploy-acceptance:eb28ea09-1a19-464c-acf7-b84121afb777`,
  materialized DMap **rev 3**, digest
  `sha256:47f3349cb781278d1022bd7393db25ddb7d706195edbf9f8340e0b8e2004b0fd`
  (never rev1's digest — a genuinely new transition, not a history rewind).

## 22. Current-return-to-source suppression — the central algorithm live proof

Immediately after rev3 acceptance:

- **A's context**: `desired_services: [{probe, running, rev3}]`,
  **`cleanup_directives: []`**. Even though rev1's history genuinely placed
  `probe` on `rpi-119d`, **no cleanup directive is emitted** — current
  authority (rev3, also A) wins, exactly as Phase 119F-R1's derivation
  requires, now proven against real accepted DMap history, not only unit
  tests.
- **B's context**: `desired_services: []`,
  `cleanup_directives: [{probe, previous_dmap_ref: revision 2, digest
  matching rev2 exactly}]`.

## 23. A realized (rev3) and B cleanup — dry-run, execute, M8

A authorized/realized identically to §13/§18; `docker inspect` confirms
`datum.dmap_revision: 3`, digest matching rev3 exactly. At this point A and
B are both physically running (transient overlap, authority solely A).

B cleanup: `cleanup-authorize` (`operation=cleanup`, rev3) →
`authorization_id: opauth:69c56280-a51c-4137-ad7e-45570f7c4b14`. Dry-run
(`--cleanup`): exact rev2 match, `host_mutation_performed: false`, container
still running. Execute (`--cleanup --execute`): `cleanup_mutated=1`,
`status: "rolled_back"`, M8 boundary `result: "passed"` under this exact
authorization, `docker ps -a` on fog-01 confirms the container **physically
absent**.

**Final converged state: A physically running (rev3 labels), B physically
absent, `GET .../active` confirms rev3/`{rpi-119d}`, D-Engine =
`{rpi-119d}` — byte-identical placement to the original rev1 baseline.**

## 24. DMap revision sequence

`1 → 2 → 3`, strictly monotonic, never rewound — confirmed by three
distinct materialized digests, no two equal.

## 25. Label timeline

| Stage | Node | `datum.dmap_revision` | `datum.dmap_digest` |
|---|---|---|---|
| rev1 realized | rpi-119d | 1 | `sha256:f31d439059b4...eec978` |
| rev2 realized | fog-119d | 2 | `sha256:c9d02083...0511cd` |
| rev3 realized | rpi-119d | 3 | `sha256:47f3349c...4b0fd` |

Every label observed via live `docker inspect`, matching the corresponding
accepted DMap's own digest exactly, on real running containers, on real
hosts of two different CPU architectures.

## 26. Full authority/physical timeline

| Stage | Active DMap rev | Canonical placement | D-Engine | Physical A | Physical B | A cleanup directive | B cleanup directive | A auth op | B auth op |
|---|---|---|---|---|---|---|---|---|---|
| rev1 accepted | 1 | A | `{A}` | absent | absent | — | — | — | — |
| rev1 converged | 1 | A | `{A}` | **running** | absent | — | — | reconcile | — |
| B pending | 1 | A | `{A}` | running | absent | none | — | — | — |
| rev2 accepted | 2 | B | `{B}` | running (stale) | absent | **rev1** | — | — | — |
| B realized | 2 | B | `{B}` | running (stale) | **running** | rev1 | none | — | reconcile |
| A cleanup unauthorized (negative) | 2 | B | `{B}` | running (unchanged) | running | rev1 (observed, not executed) | — | — | — |
| A cleanup dry-run | 2 | B | `{B}` | running | running | rev1 (matched, not executed) | — | cleanup (issued) | — |
| A cleanup executed | 2 | B | `{B}` | **absent** | running | (satisfied) | — | cleanup | — |
| rollback pending | 2 | B | `{B}` | absent | running | — | — | — | — |
| rev3 accepted | 3 | A | `{A}` | absent | running (stale) | **none (suppressed)** | **rev2** | — | — |
| A realized (rev3) | 3 | A | `{A}` | **running** | running (stale) | none | rev2 | reconcile | — |
| B cleanup dry-run | 3 | A | `{A}` | running | running | none | rev2 (matched, not executed) | — | cleanup (issued) |
| B cleanup executed | 3 | A | `{A}` | running | **absent** | none | (satisfied) | — | cleanup |

## 27. M8 evidence — both destructive cleanups

| Cleanup | authorization_id | expires_at_utc | boundary | result | host_mutation_performed |
|---|---|---|---|---|---|
| A (rev1) | `opauth:06b0d75e-9a0f-4d59-89c5-6fa0cfa4cc00` | `2026-09-29T12:33:25Z` | M8 | passed | true |
| B (rev2) | `opauth:69c56280-a51c-4137-ad7e-45570f7c4b14` | `2026-09-29T12:34:44Z` | M8 | passed | true |

## 28. Negative/safety proofs

- **Directive without authorization** (§19): `cleanup_directives=1`,
  `cleanup_mutated=0`, container unaffected — proven live.
- **Missing/no cleanup obligation elsewhere**: at no point did any
  reconciliation or cleanup cycle on either node report a directive or
  mutation for a service/node combination outside this exact scenario.
- Stale-authorization-after-return-to-source (task §43) was **not**
  additionally exercised live in this run — it complicates the primary
  sequence for no new evidence, is already covered by
  `cleanup_authorization_still_enforces_dgraph_revision_freshness` (Phase
  119G unit regression), and the task's own instruction was to skip it
  rather than risk primary evidence. Documented here as a deliberate,
  disclosed omission, not an oversight.

## 29. Firebase H1 live evidence

Direct, real evidence the legacy endpoints genuinely fail against this
isolated environment: `GET /api/v1/nodes` (unrelated legacy list endpoint,
same `state.firebase` dependency as `query_node_context`) returned a real
`403 PERMISSION_DENIED`/`CONSUMER_INVALID` from `firestore.googleapis.com`
against the placeholder `project_id`. Both nodes' own bootstrap runs (§9)
independently confirm `node_known=false` for the same underlying reason.
Bootstrap succeeded past this on both nodes regardless — direct, live
confirmation of Phase 119H1's fix, not a hypothetical.

## 30. Multi-app non-interference

No unrelated currently-desired DATUM service existed on either node during
this proof (a fresh isolated project/application/container name was used
throughout, per this task's own preference), so the optional live spot
check (task §33) was not applicable — consistent with the task's own
instruction not to manufacture a second application solely for this check,
since it is already covered structurally by Phase 119G's own unit
regression (`process_cleanup_directives` only ever iterates
`cleanup_directives`, never `desired_services`).

## 31. Cleanup / wind-down

- Isolated DServer stopped after all evidence was captured.
- Raw evidence (every request/response JSON, both nodes' reconciliation/
  cleanup reports, build logs) retained in a local, non-Git scratch
  location, per this task's own instruction — only this document was
  committed.
- Remote scratch directories (`~/datum-phase119h-src`, `~/phase119h-live`)
  left in place on both nodes, pending this document's commit, matching
  Phase 119D's own precedent.
- fog-01's Docker daemon remains reconfigured to classic `overlay2` (§11) —
  a deliberate, disclosed, real host state, not reverted, since it fixed a
  genuine compatibility gap rather than being a throwaway test artifact.
- fog-01 (the VirtualBox VM) may be powered down; RPi may be left running
  or powered down per normal operating practice for that device.

## 32. What this run does and does not prove

**Proven live, on two real, physically distinct, different-architecture
hosts, using only the real production CLI and unmodified DServer API:**
- Everything Phase 119D already proved (placement/authority/D-Engine
  transitions, monotonic revision sequence, rollback as a new transition).
- Real Stage107A-governed container start AND stop, on both nodes, in both
  directions of an A→B→A cycle — the exact gap Phase 119D's own §26
  disclosed as unexercised.
- Every new Phase 119G `datum.dmap_*` container label, live, matching exact
  accepted DMap digests, at three distinct revisions.
- The full Phase 119G governed-cleanup mechanism end to end: cleanup-only
  context construction, dry-run observation, authorized execution, M8
  boundary evidence, and — critically — the current-return-to-source
  suppression property that Phase 119F-R1's independent review specifically
  required be provably correct, not merely unit-tested.
- Phase 119H0's diagnosis and Phase 119H1's fix, live, on real infrastructure
  with a genuinely-failing legacy Firebase dependency.

**Not proven, and explicitly out of this run's scope:**
- The optional stale-cleanup-authorization-after-return-to-source
  adversarial check (§28) — deliberately skipped per the task's own
  instruction, already covered by unit regression.
- Multi-application non-interference under live conditions (§30) — no
  second application existed to interfere; covered structurally by existing
  unit regression instead.
- Whether the containerd-image-store `.Id` compatibility gap (§11) affects
  any OTHER DATUM code path beyond this proof's own container-pinning
  verification — not investigated further, as it is explicitly out of this
  phase's scope (no DATUM source was touched to address it).

## 33. Recommendation

Phase 119 should **not** be considered closed by this run alone, consistent
with this task's own instruction. Recommend **Phase 119I — Independent Final
Migration Closure Review**, covering the full chain from 119A through this
119H live run, with explicit judgment on: (a) whether live physical proof in
both migration directions plus governed cleanup in both directions satisfies
Phase 119's original central invariant, (b) the §11 finding's disclosure and
resolution (host config change, zero source change) as an acceptable
resolution path, and (c) whether the deliberately-skipped adversarial checks
(§28, §30) require live exercise before final closure or remain adequately
covered by existing unit regression.
