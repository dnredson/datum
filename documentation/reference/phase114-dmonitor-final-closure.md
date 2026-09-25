# Phase 114 — Canonical DMonitor Final Closure and Live Multinode Validation

**Status: PHASE 114 CLOSED. Phase 114E live multinode validation: PASS.**

**Date**: 2026-09-25
**Feature branch**: `agent/dmonitor-v1`
**Final feature commit**: `3e0baa8f415b822f69eef86c0cbfe2a3681e3a65`

This document is the durable closure record for the entire Phase 114 arc
(114A through 114E). It complements, and does not replace, `ADR-0024` (the
architectural decision record) and the individual sub-phase commits, which
remain the authoritative source for design rationale and code-level detail.
This document's job is narrower: state plainly what was built, what was
proven live, and what remains open.

## 1. Scope and intent

Phase 114 introduces **DMonitor**: DATUM's own canonical, independently
verifiable runtime-observation architecture. Its entire purpose is to answer
one question honestly, for both D-Serv and DIoT realizations: *is what is
actually running consistent with what was governed to run, and can that be
proven from fresh evidence right now?*

## 2. DMonitor's relationship to SmartSentinel

SmartSentinel remains the legacy telemetry-collection lineage and its own
compatibility surface (`Snapshot`/`Entity`/`Edge`,
`SmartSentinelTelemetryEnvelope`, `/api/v1/snapshots`, `/api/v1/sentinels/*`)
is untouched by Phase 114 -- nothing was migrated or retired. Phase 114B2
adds a **projection**, not a replacement: SmartSentinel telemetry is
projected into canonical DMonitor observations through
`agent::dmonitor_projection`, alongside SmartSentinel's own existing output
path, never instead of it.

## 3. Authority boundaries (preserved, not renegotiated)

| Layer | Authority over |
|---|---|
| DGraph | Semantic application topology |
| DContinuum | Structural/static node capabilities and capacity |
| DMap | Desired placement / realization (the ONE authority DMonitor is measured against) |
| DForward | DIoT / logical middleware topology, `same_element`/`any_element` dependency semantics |
| DCode / ServiceArtifact | Executable authority |
| DNode | Local execution boundary |
| **DMonitor** | **Observed runtime truth / evidence only** |

DMonitor is strictly detective. Across all of 114A-114E it never gained the
ability to own desired state, select placement, mutate DMap/DForward/runtime-
binding authority, authorize host mutation, or act as a corrective executor.
Every sub-phase's own test suite includes at least one test proving the
relevant "DMonitor cannot become authority" invariant for that layer.

A second, equally load-bearing separation runs through the whole
architecture: **freshness ≠ health ≠ convergence ≠ readiness**. These are
four independent dimensions, never collapsed into one boolean. The Phase
114E live proof (section 12-13 below) is the empirical demonstration of
exactly this separation: a real observation was `health = Healthy` while
simultaneously `freshness = Stale`, and stale-but-Healthy evidence never
authorized readiness.

## 4. Sub-phase history (114A-114D2)

| Phase | Delivered | Commit |
|---|---|---|
| 114A | Canonical `datum.dmonitor-observation/1` model (typed `DMonitorSubjectV1`, independently mirrored DATUM/DServer per ADR-0007) | `124bbd2377dac9c07b12fb2efb6a61c7cf704cbe` |
| 114B1 | Server ingestion, per-stream sequence ordering, idempotent replay, freshness evaluation | `f68c3a01542d7cba0d49bb226ff47ac9da8055e3` |
| 114B2 | SmartSentinel → canonical DMonitor compatibility projection | `e148e0c5a41920f5abbb0d5f16a45adc215059fd` |
| 114C | Derived D-Serv convergence/drift interpretation (`Converged`/`Diverged`/`Unknown`, 13 finding codes) | `371f116a7fac03080fe2fbdaf7526aa827062189` |
| 114D0A | Exact canonical DIoT correlation (DMap/DForward/runtime-binding refs; 19 finding codes total) | `5dfb626e383c087c3d78c799b194174c9f22b2d2` |
| 114D0B | Real governed MQTT listener DMonitor producer (protocol-confirmed ConnAck/SubAck session readiness, non-blocking bounded publication) | `0cfd75752e97b4b0f2de58a882dfd9ab9f6139f8` |
| 114D1 | Server-owned, fail-closed readiness governance (`Ready`/`NotReady`, 9 finding codes) | `c059c3341c7901eeea77ce4881bbd341580d8aef` |
| 114D2 | `any_element` dependency resolution consuming the exact PROVIDER DIoT placement's readiness | `3e0baa8f415b822f69eef86c0cbfe2a3681e3a65` |
| 114E | **Live multinode validation of the entire chain above** | this document |

Every commit above remains exactly as authored; none was rewritten or
squashed for this closure.

## 5. Canonical observation model (114A)

`CanonicalDMonitorObservationV1` carries a typed, tagged `DMonitorSubjectV1`
(`DNode`, `RuntimeProcess`, `DServRealization`, `DCodeExecution`,
`DCallDelivery`, `DIoTRealization`, `NetworkReachability`, `Unbound`). It
structurally cannot carry a `desired_state` field -- the type system makes
"DMonitor as authority" a compile error, not a convention. DATUM and
DServer maintain independent Rust mirrors (never a shared crate, per
ADR-0007), cross-checked via golden-vector digest tests so the two
implementations are provably byte-identical without being the same code.

## 6. Ingestion / stream / freshness semantics (114B1)

Ingestion is per-`(node_id, monitor_instance_id)` stream, strictly
sequence-ordered, idempotent on exact replay, and rejects stale/conflicting
sequences without ever silently overwriting history. Freshness is evaluated
independently against an explicit policy (`max_observed_age_seconds`,
`max_received_age_seconds`, `max_future_clock_skew_seconds`) and produces
`Fresh`/`Stale`/`ClockSkew` -- never folded into health.

## 7. SmartSentinel compatibility projection (114B2)

`agent::dmonitor_projection` derives canonical observations from
`SmartSentinelTelemetryEnvelope`, using a dedicated process-epoch/sequence
context (later generalized in 114D0B into the neutral
`agent::dmonitor_emission` module so the DIoT runtime producer could share
the identical epoch/sequence rule rather than reimplementing it).

## 8. Convergence / drift interpretation (114C)

`evaluate_application_dserv_convergence` compares admitted, fresh,
exact-authority-matching D-Serv evidence against the CURRENT active
`PlacementAuthorityView`, producing `Converged`/`Diverged`/`Unknown` plus a
stable 13-code finding inventory. Unexpected realizations (evidence for a
service with no desired placement at all) are detected as drift in their
own right.

## 9. Exact DIoT correlation (114D0A)

`DIoTRealization`'s canonical shape was extended with required identity
(`diot_id`, `diot_placement_id`, `node_id`) and optional exact-correlation
references (`dmap_ref`, `dforward_ref`, `runtime_binding_ref`).
`evaluate_application_diot_convergence` reuses Phase 113B1's own
`verify_runtime_binding_still_current` unmodified -- DIoT convergence is
never a re-implementation of runtime-binding currentness, only a consumer
of it. The finding inventory grew to 19 entries (13 original + 6 DIoT-only),
never redefining any of the original 13.

## 10. Real MQTT DMonitor producer (114D0B)

`run_mqtt_dcall_listener` gained a genuine protocol-level readiness gate:
`MqttListenerSession::establish_and_confirm` requires an observed
`ConnAck{Success}` followed by an all-`Success` `SubAck` before any
`Healthy` observation may be emitted -- `subscribe()` returning `Ok` alone
was proven, by direct audit of `rumqttc` 0.24, to be insufficient. DMonitor
publication runs on a bounded, non-blocking background worker
(`dcall::dmonitor_publisher`) so a slow/unreachable DServer can never stall
MQTT delivery. A definite transport failure emits one best-effort
`Unhealthy` observation before the listener's existing fail-closed
termination; a graceful stop emits no synthetic `Unhealthy` at all -- the
last `Healthy` evidence is left to age into staleness under the ordinary
freshness rule. Phase 114E's E8 gate is the live proof of exactly this
graceful-stop behavior (section 13).

## 11. Readiness governance (114D1)

`core::dmonitor_readiness::evaluate_application_readiness` derives a
fail-closed, two-state (`Ready`/`NotReady` -- deliberately no third
`Unknown` governance state) decision from the SAME convergence functions
described in sections 8 and 9 above, adding only a health-gating layer
computed exclusively from ADMITTED CURRENT evidence. The policy (`dmonitor-readiness-v1`:
`max_observed_age_seconds=90`, `max_received_age_seconds=90`,
`max_future_clock_skew_seconds=10`, `accepted_health=Healthy`) is entirely
server-owned; `GET /api/v1/dmonitor/readiness/:project_id/:application_id`
accepts no caller override of policy or evaluation time.

## 12. DForward `any_element` dependency readiness (114D2)

`core::dforward_dependency_readiness::evaluate_dforward_dependency_readiness`
closes ADR-0010's deferred gate: for a consumer's
`requires(interface, scope=any_element)`, it resolves the exact provider
DIoT from the CURRENT accepted DForward chain topology (never by hop
position -- consumer/provider sides are identified by exact
`provides`/`requires` semantics) and asks whether THAT EXACT provider
placement's Phase 114D1 readiness is `Ready`. **It never requires
whole-application readiness to be `Ready`.** Provider or provider-placement
ambiguity (both audited as structurally possible under current canonical
validation, which enforces `placement_id` uniqueness but never `diot_id`
uniqueness) fails closed rather than guessing.

## 13. Phase 114E: live multinode topology

Executed 2026-09-25, entirely through production HTTP APIs and production
binaries -- no direct state injection, no synthetic observation.

| Role | Canonical node | IP | Live process for this proof |
|---|---|---|---|
| Control plane + MQTT broker | (operational only) | `192.168.10.101` | DServer (isolated instance) + Mosquitto (pre-existing service) |
| Provider | `fog-01` | `192.168.10.103` | **Real** `smartsentinel-dcall-mqtt-listen` |
| Consumer | `cloud-01` | `192.168.10.108` | None required -- participates as a real, declared DNode + DIoT placement only |

Namespace: `project_id = proj:dmonitor-live-114e0`,
`application_id = app:pipeline-114e0`.

Topology: provider DIoT `mqtt-broker` (placement `diotplace:mqtt-broker-fog01`
on `fog-01`) provides interface `sensor-bus`; consumer DIoT `diot:consumer`
(placement `diotplace:consumer` on `cloud-01`) requires it with
`scope = any_element`, connected by one canonical DForward chain hop. A
third, deliberately unrelated D-Serv placement (`unrelated-service` on
`fog-01`) was included specifically to prove that whole-application
readiness can remain `NotReady` while the `any_element` dependency itself
is independently `Satisfied` -- direct empirical proof of the 114D2 rule in
section 12.

By design (see ADR-0010's own "Phase 114D2: implementation" section and
this document's section 19), this proof does **not** require a live
consumer-side process: D2's gate is the provider placement's own readiness,
never the consumer's liveness. cloud-01's role in this proof is a real,
independently registered DNode and DIoT placement, not a live runtime.

## 14. Acceptance matrix: E1-E9, all PASS

| Gate | Result | Evidence |
|---|---|---|
| E1 — Authority | **PASS** | `datum.dmap/2` active, exact DMap/DForward, runtime binding current (`11-active-authority.json`, `10-runtime-binding-current.json`) |
| E2 — Initial fail-closed | **PASS** | Before any provider evidence: readiness `not_ready`, dependency `blocked` (`12-readiness-before-listener.json`, `13-dependency-before-listener.json`) |
| E3 — Real provider producer | **PASS** | Real `dcall_mqtt_listener`-collected `diot_realization` observation, `health=healthy`, `runtime_state=subscribed` (`14-provider-first-live-head.json`) |
| E4 — Exact correlation | **PASS** | `freshness=fresh`, `dmap_correlation=exact_match`, `dforward_correlation=exact_match`, `runtime_binding_correlation=exact_match`, `runtime_binding_current=true`, `convergence=converged` (`15-readiness-live-provider.json`) |
| E5 — Provider readiness | **PASS** | `current_health=healthy`, `readiness=ready` (`15-readiness-live-provider.json`) |
| E6 — Cross-node any_element | **PASS** | Application `readiness=not_ready`; dependency (consumer `cloud-01` / provider `fog-01`) `state=satisfied` (`16-dependency-live-provider.json`) |
| E7 — Sustained heartbeat | **PASS** | Same `monitor_instance_id`; admitted evidence sequence progressed (7 sampled → 9 admitted in the same convergence result), still `healthy`, `heartbeat=true`, runtime binding ref unchanged |
| E8 — Graceful stop / staleness | **PASS** | See section 15 |
| E9 — Authority immutability | **PASS** | See section 16 |

**All nine gates PASS. Phase 114E is CLOSED.**

## 15. Graceful-stop and staleness result (E8) — the central empirical proof

The provider listener was stopped gracefully (no synthetic `Unhealthy` was
emitted, matching 114D0B's documented design). The last captured stream
head (`19-head-after-graceful-stop.json`) still read:

```
sequence 21, health = healthy, runtime_state = subscribed, heartbeat = true
observed_at_utc  = 2026-09-25T14:40:35.688045133+00:00
received_at_utc  = 2026-09-25T14:40:34.102025300+00:00
```

Polling continued past the 90-second freshness window. At
`evaluated_at_utc = 2026-09-25T14:42:06.727122300+00:00` (≈91s after that
same observation's `observed_at_utc`, ≈93s after its `received_at_utc` --
both past the 90s policy window), `20-readiness-after-stale.json` shows the
EXACT SAME observation (`sequence 21`, same digest) now:

```
freshness            = stale
health               = healthy        <-- unchanged
admitted             = false
exclusion_reason     = stale_evidence
DIoT convergence     = unknown
provider readiness   = not_ready
dependency (21-*)    = blocked
```

This is the direct, machine-recorded proof of the architectural rule stated
in section 3: **the SAME observation's own recorded `health` never
changed** -- only its relationship to the current evaluation time did.
`Healthy` never overrode `Stale`; `Stale` never authorized readiness. No
timestamp was rounded or assumed for this document -- the exact recorded
`observed_at_utc`/`received_at_utc`/`evaluated_at_utc` values above are
copied verbatim from the proof files.

## 16. Authority immutability result (E9)

Compared across the entire live session (`11-active-authority.json` before
the listener started, `17-authority-before-stop.json` immediately before
graceful stop, `22-authority-after-stale.json` after the staleness
transition -- and the same three-point comparison for the runtime binding
via `10-`/`18-`/`23-runtime-binding-current.json`):

```
dmap_id   dmap:24:proj:dmonitor-live-114e018:app:pipeline-114e0   (unchanged)
revision  1                                                        (unchanged)
digest    sha256:c7eac434eb593f2b866ae7cf6347f66fe2d0b6c96f32fe290b4939ea15212122  (unchanged)
binding_id        diot-runtime-binding:proj:dmonitor-live-114e0:app:pipeline-114e0:diotplace:mqtt-broker-fog01  (unchanged)
binding_revision  1                                                (unchanged)
descriptor_digest sha256:729b5d7b791b532ad993ef02f26866163551b8ca42e359e604bd9e74915967b8  (unchanged)
```

All three snapshots are byte-identical. The entire E1→E9 transition was
produced exclusively by evidence aging and DMonitor re-evaluation -- no
DMap, DForward, or runtime-binding mutation occurred at any point.

## 17. What Phase 114 DOES prove

- A canonical, independently-mirrored DMonitor observation model, ingested,
  ordered, and idempotently replayed by a real DServer.
- Derived D-Serv and DIoT convergence, exactly correlated against current
  DMap/2 + DForward + DIoT runtime-binding authority, reusing (never
  reimplementing) the one production runtime-binding-currentness check.
- A real governed MQTT listener that only claims `Healthy` after genuine
  protocol-level session confirmation, with a non-blocking evidence-
  publication path.
- Server-owned, fail-closed readiness governance, cleanly separated from
  health and convergence.
- `any_element` dependency resolution gated on the exact provider
  placement's readiness, never on whole-application readiness.
- **All of the above running live, for real, across two genuinely distinct
  physical/virtual hosts plus a separate Windows control-plane host**, with
  the graceful-stop → staleness → fail-closed transition captured exactly
  as it happened.

## 18. What Phase 114 does NOT prove

- **No full application data-plane end-to-end exchange.** This proof did
  not send or receive a DCall payload between fog-01 and cloud-01; it
  validated DMonitor observation, governance, and dependency-readiness
  derivation only. `unrelated-service` was deliberately left without
  DMonitor evidence and never became `Ready` -- application-level readiness
  was `NotReady` throughout this proof, by design, not by defect.
- **Phase 113B4B's historical cross-host MQTT *payload* transport gap is
  neither re-attempted nor resolved by this document.** That gap concerned
  a live D-Call request/reply exchange across the fog-01/cloud-01 VM pair;
  Phase 114E's required proof structurally never depends on that leg (see
  section 13) and this closure makes no claim about it one way or the
  other.
- No reconciliation, automatic recovery, restart/redeploy, or corrective
  host mutation exists anywhere in Phase 114 -- DMonitor remains purely
  observational, exactly as designed.
- No `Degraded`/multi-monitor-conflict scenario, no provider ambiguity, and
  no consumer-side live producer were exercised in the LIVE topology (all
  of these are already covered by the extensive unit/integration test
  suites built during 114C/114D0A/114D1/114D2 -- see those phases' own
  commits).

## 19. Known limitations / deferred work

- A production canonical `DServRealization` emitter (equivalent to
  114D0B's DIoT producer) does not yet exist -- the generic SmartSentinel
  bridge deliberately does not manufacture D-Serv realization identity from
  container/process names (114B2's own documented rule). Any application
  with required D-Serv placements will read `NotReady` until such a
  producer exists. This proof deliberately did not paper over that by
  injecting synthetic D-Serv evidence.
- `any_element` provider selection when genuinely ambiguous (multiple
  distinct logical providers, or multiple placements of one provider
  `diot_id`) remains fail-closed by design, not resolved -- this is
  correct behavior, not a gap, but it means no automatic provider
  selection/ranking exists for a future phase that might want one.
- Reconciliation, restart/redeploy, and any automatic corrective action
  driven by DMonitor evidence remain entirely unimplemented and out of
  scope until a phase explicitly authorized to introduce them.
- No TLS/`mqtts://` support; the plaintext `mqtt://` trust boundary already
  documented in ADR-0023/Phase 113B3 applies unchanged.

## 20. Live evidence manifest

Raw proof artifacts are held locally at
`C:\Users\dnred\datum-114e-live\proof\` (not committed -- see rationale
below) and are referenced here by filename so the claims above remain
independently checkable by whoever ran the live session.

| File | Gate | Moment |
|---|---|---|
| `01-dnode-fog.json` / `02-dnode-cloud.json` | E1 | Authority setup — DNode registration |
| `03-dcontinuum-authoritative.json` | E1 | Authority setup — accepted DContinuum |
| `04-artifact-unrelated.json` / `05-artifact-consumer.json` / `06-artifact-provider.json` | E1 | Authority setup — ServiceArtifact declarations |
| `07-ddeploy-v2-proposal.json` / `07a-ddeploy-v2-validation.json` / `08-ddeploy-v2-acceptance.json` | E1 | Authority setup — DMap/2 proposal and acceptance |
| `09-runtime-binding-register.json` / `10-runtime-binding-current.json` | E1 | Authority setup — DIoT runtime binding |
| `11-active-authority.json` | E1, E9 (baseline) | Pre-listener authority snapshot |
| `12-readiness-before-listener.json` / `13-dependency-before-listener.json` | E2 | Pre-listener (fail-closed baseline) |
| `14-provider-first-live-head.json` | E3 | Live — first captured real provider observation |
| `15-readiness-live-provider.json` | E4, E5 | Live — readiness with real evidence admitted |
| `16-dependency-live-provider.json` | E6 | Live — cross-node dependency Satisfied |
| `17-authority-before-stop.json` / `18-binding-before-stop.json` | E9 (mid-point) | Authority snapshot immediately before graceful stop |
| `19-head-after-graceful-stop.json` | E8 | Post-stop — last observation, still Healthy |
| `20-readiness-after-stale.json` / `21-dependency-after-stale.json` | E8 | Post-staleness — fail-closed transition |
| `22-authority-after-stale.json` / `23-binding-after-stale.json` | E9 (final) | Authority snapshot after the whole session — proves immutability |

No bearer token, password, MQTT credential, or Windows username/host-private
configuration appears in any of the 24 files above; each contains only IDs,
digests, timestamps, and governance state. Consistent with this
repository's existing precedent (`reference/phase113b-live-multinode-closure.md`
committed a condensed *summary* rather than raw session output), the raw
proof JSON is kept as a local validation artifact rather than committed
verbatim; this document embeds every value needed to independently verify
the E1-E9 claims above without requiring access to that local directory.

## 21. Final status

```
PHASE 114: CLOSED
LIVE MULTINODE VALIDATION (114E): PASS
NEXT: PHASE 115
```

See also: `ADR-0010` (`any_element` readiness decision, now implemented),
`ADR-0023` (canonical DForward/DIoT/DMap-v2), `ADR-0024` (canonical DMonitor
observation model — the full architectural record for every sub-phase
listed in section 4).
