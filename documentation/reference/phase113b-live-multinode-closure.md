# Phase 113B — live multinode closure attempt

**Date**: 2026-09-24
**Feature commit**: `890798d15fef35df7b12cbf4cce80e083c21014b` (branch `agent/dforward-diot-runtime-v1`)

**Phase 113B4A**: **PASS** — full governed real-broker MQTT D-Call end-to-end proof, including real grant issuance/redemption and real source/destination WASM execution, entirely on a single machine (see `documentation/reference/governed-mqtt-dcall-e2e.md`).

**Phase 113B4B**: **PARTIAL** — live multinode governance and distinct-host setup demonstrated across two genuinely separate physical VMs; cross-host MQTT delivery did not complete because cloud-01 repeatedly lost the long-lived outbound TCP connection to the broker, regardless of which broker (Docker-hosted on three different ports, and the native Windows broker) was used.

This document does not claim, and nothing below should be read as claiming, that the physical fog-01 → broker → cloud-01 MQTT path completed successfully at any point.

## Topology

| Role | Canonical node ID | Operational host | Notes |
|---|---|---|---|
| Source | `fog-01` | `datum-fog` (VirtualBox VM, `192.168.10.103`) | Confirmed genuinely distinct machine: machine-id `25c55f2abe8f4df496fcd2c585259a66`, NIC MACs `08:00:27:88:ea:f0`/`08:00:27:fa:7d:11`, booted 2026-09-23 14:24:35 UTC. |
| Destination | `cloud-01` | `datum-cloud` (VirtualBox VM, `192.168.10.108`) | Confirmed genuinely distinct machine: machine-id `15553d5fc4c84e8a9f76d7a7dadb27f8`, NIC MACs `08:00:27:78:f0:6c`/`08:00:27:a0:05:ce`. Restarted mid-session (new boot 2026-09-24 16:08:22 UTC) as part of diagnosis. |
| Control plane | (none — operational only) | This Windows host, `192.168.10.101:8080` | Isolated temporary control-state DB; never the developer's real DATUM state. |
| MQTT broker | (none — operational only) | This Windows host, `192.168.10.101:1883` | The exact native Mosquitto instance already used and proven reliable throughout Phase 113B4A/113B4A.1's local real-broker tests. |

Both fog-01 and cloud-01 built production binaries from a local git worktree at the exact feature commit (delivered via a git bundle over the base commit both hosts already shared, to route around a stale/expired GitHub token on the hosts — no code content differs from the reviewed commit).

## Unique namespace

- `project_id = proj:dcall-live-b4ba74cc0a3`
- `application_id = app:pipeline-b4ba74cc0a3`
- `call_id = sensor-to-decoder`
- `dforward_id = dforward:live-b4ba74cc0a3`
- `diot_placement_id = diotplace:mqtt-broker-cloud01-b4ba74cc0a3`

## What was proven live, through real production entrypoints

1. **Real governance registration**, entirely through production HTTP APIs against the isolated control-plane DServer (no direct state injection):
   - DGraph (`dgraph:live-b4ba74cc0a3`, revision 1) with `sensor`→`decoder` D-Call, host-independent.
   - DContinuum with `fog-01`/`cloud-01` canonical nodes.
   - DMap/2 proposal created and **accepted** (`dmap:27:proj:dcall-live-b4ba74cc0a324:app:pipeline-b4ba74cc0a3`, revision 1).
   - DForward + one DIoT (`mqtt-broker`) realizing the call via `via_diot_pubsub`.
   - DIoT runtime binding registered via the production `PUT` API and independently re-fetched via the production `GET .../current` endpoint, confirmed identical.
2. **Real source D-Code invocation on fog-01** — via a new, minimal production CLI (`smartsentinel-dcall-dispatch`, added this phase — see below), fetching real authorization/artifact from the control-plane DServer and executing the real WASM module in the real isolated worker, producing a genuine deterministic emit (`payload_sha256 = 90f27252...`).
3. **Real DCall grant issuance**, correctly resolving `route_kind: "remote"`, source = `fog-01`/`sensor`, destination = `cloud-01`/`decoder` — proving the full DMap/2 → DForward → DIoT realization → runtime binding chain resolves correctly for a genuinely different pair of physical hosts.
4. **Real destination MQTT listener** (`smartsentinel-dcall-mqtt-listen`) running as a subprocess on cloud-01, using only authority-derived startup parameters (dserver URL, project/application/placement IDs, local D-Node root) — no manually-supplied channel, service, DMap reference, or broker URI.

## What was not achieved: real MQTT delivery

Every dispatch attempt from fog-01 to cloud-01 failed with `dcall_transport_timeout` (bounded at the grant's own ~30s cap — never an unbounded wait), across **three independently configured brokers**:

1. A Docker bridge-mode Mosquitto container on cloud-01 (host port 1884).
2. A Docker host-networking Mosquitto container on cloud-01 (host port 1883, later found to collide with a pre-existing, unrelated user-owned Magistrala/FluxMQ deployment already running on that host — not a DATUM issue).
3. A fresh Docker bridge-mode container on cloud-01 (host port 1885, avoiding the above collision).
4. The native Windows Mosquitto broker already proven reliable in Phase 113B4A/113B4A.1 (after adding a firewall exception, mirroring the existing DServer firewall rule precedent) — **also failed**, even after a full reboot of cloud-01.

### Root-cause evidence

Mosquitto's own connection log (captured directly from the Docker-hosted attempts) showed the destination listener's subscriber connection being disconnected by the broker for **"exceeded timeout"** (a genuine MQTT keepalive violation) at a moment that, on two independent occurrences, coincided exactly with the arrival of a new, unrelated connection — not a fixed elapsed duration (27s the first time, 119s the second). On the final attempt (against the Windows broker), real-time monitoring of the broker's own TCP connection table showed cloud-01's listener had **no established connection to the broker at all** during the dispatch window, despite the listener process being confirmed alive (`ps`) with a clean startup log moments earlier — i.e. its transport silently died without the process observing or reporting an error.

This pattern is consistent with everything else observed about cloud-01 throughout this session: SSH sessions to this specific VM stalled and dropped without clean termination dozens of times, independent of MQTT entirely.

**The collected evidence is consistent with an environment-specific cloud-01 networking failure; the same DATUM MQTT implementation and broker passed the governed B4A E2E.** This is offered as the best-supported explanation, not a mathematical proof that the VM/network layer is the only possible cause — no code path inside DATUM was instrumented deeply enough during these live attempts to exhaustively rule out every alternative. What the evidence does establish directly: the identical `RumqttcPubSub`/keepalive code, exercised against the exact same broker instance used in the failed live attempts, passed cleanly in a single-machine run of the Phase 113B4A primary governed E2E test (`governed_mqtt_dcall_delivery_end_to_end`), re-run immediately after this session's live attempts — so the defect, whatever its precise mechanism, is not reproducible on that code path without cloud-01 specifically in the loop.

## New production code added this phase

- `DATUM/src/bin/smartsentinel-dcall-dispatch.rs` — a thin CLI wrapper around the already-proven `dcall::delivery::dispatch_validated_emit` and `dcode::invoke::invoke_dcode_governed_capturing_emits` functions (both already exercised end-to-end by Phase 113B4A). No new dispatch or authorization logic; it exists because no standalone production binary previously combined "invoke governed source D-Code" with "dispatch its emit through the real D-Call path" (the existing `smartsentinel-dcode-invoke` explicitly documents itself as local-invocation-only, "no D-Call").

## Exact live commands (secrets removed — no bearer token appears in any command below)

```bash
# Governance registration (run once, from the control-plane host, against
# the isolated B4B DServer):
curl -X PUT http://127.0.0.1:8080/api/v1/diot/runtime-bindings/proj:dcall-live-b4ba74cc0a3/app:pipeline-b4ba74cc0a3/diotplace:mqtt-broker-cloud01-b4ba74cc0a3 \
  -H 'X-Datum-Project: proj:dcall-live-b4ba74cc0a3' \
  -d '{"broker_uri": "mqtt://192.168.10.101:1883"}'

# Destination (cloud-01):
./target/debug/smartsentinel-dcall-mqtt-listen \
  --dserver-url http://192.168.10.101:8080 \
  --dnode-root /home/vboxuser/datum-b4b-runtime/cloud-01 \
  --project-id proj:dcall-live-b4ba74cc0a3 \
  --application-id app:pipeline-b4ba74cc0a3 \
  --placement-id diotplace:mqtt-broker-cloud01-b4ba74cc0a3

# Source (fog-01):
./target/debug/smartsentinel-dcall-dispatch \
  --dserver-url http://192.168.10.101:8080 \
  --project-id proj:dcall-live-b4ba74cc0a3 \
  --application-id app:pipeline-b4ba74cc0a3 \
  --service-id sensor \
  --input-port trigger \
  --dnode-root /home/vboxuser/datum-b4b-runtime/fog-01 \
  --input-path /dev/null
```

## Limitations and trust boundary

- No bearer token, password, private key, MQTT credential, or full raw payload appears anywhere in this document or its accompanying transcript — only IDs, digests, and outcomes.
- The plaintext `mqtt://` trust boundary already documented for Phase 113B3/113B4A applies identically here; nothing about this attempt changes that.
- The destination VM (`cloud-01`) was rebooted once during this session as part of diagnosis, at the user's explicit direction.
- `datum-b4b-runtime/` directories and the `datum-b4b`/`b4b-fog`/`b4b-cloud` local git worktrees created on both VMs during this session were **not** removed (left in place; they are inert, disk-only artifacts posing no ongoing resource cost). The B4B-owned MQTT listener process and Docker container were removed on a best-effort basis given the destination host's ongoing instability.
- The pre-existing, unrelated user-owned Magistrala/FluxMQ Docker deployment on cloud-01 was identified during diagnosis and was never touched.

## Phase 113 architectural closure

Confirmed unchanged/unviolated by both 113B4A and 113B4B:

- DGraph remained semantic and host-independent (no IP/hostname/broker/container/process identity ever appeared in a DGraph document).
- DMap/2 remained the sole placement + D-Call realization authority.
- DForward remained logical middleware authority only.
- DIoTRuntimeBinding remained the sole *operational* realization/broker-endpoint authority — never promoted into canonical DMap/DForward/DGraph state.
- `DCallDeliveryGrantV1` remained the sole delivery authorization primitive; no parallel authorization model was introduced.
- Grant redemption gated destination execution in every case exercised (B4A) and was never bypassed in any B4B attempt (no delivery ever reached redemption without a valid grant, and no destination execution ever occurred without redemption).
- `via_diot_pubsub` never fell back to local or HTTP delivery, in any attempt across either phase.
- MQTT publishes used QoS1, `retain=false`, in every attempt.
- No exactly-once delivery claim was made or implied anywhere in this work.
- No DMonitor implementation was added.
- `any_element` dependencies remain fail-closed without fresh DMonitor evidence; nothing in B4A or B4B fixtures relied on `any_element`.

## What Phase 113 proves, and what it does not

**PROVEN** (through real production entrypoints, no shortcuts):

- Governed DMap/2 + DForward + DIoT runtime-binding authority chain, end to end.
- Real MQTT broker transport, in Phase 113B4A (single machine).
- Real source and destination WASM execution, in Phase 113B4A.
- Real grant issuance and redemption, in Phase 113B4A.
- Duplicate-delivery protection via the single-use grant, in Phase 113B4A.1.
- Stale-binding failure, broker-unavailable failure, and listener-absent failure, all in Phase 113B4A.1.
- Live two-host governance setup (Phase 113B4B): real DGraph/DContinuum/DMap-2 proposal-and-acceptance/DForward/DIoT-realization/runtime-binding registration, and a real grant correctly resolving `route_kind: "remote"` between two genuinely distinct physical hosts.

**NOT PROVEN**:

- A successful, sustained MQTT request/reply exchange across the specific fog-01/cloud-01 VM network. Every live cross-host delivery attempt in Phase 113B4B timed out; this remains an open item, attributed to that VM pairing's networking rather than to the DATUM implementation (see "Root-cause evidence" above), but not verified as resolved by anything in this session.

## Conclusion

Phase 113B's governance/authority chain (DGraph → DMap/2 → DForward → DIoT realization → DIoT runtime binding → DCall grant) is proven correct for a genuinely distinct pair of physical hosts, through production APIs and production binaries, with no shortcuts. The real MQTT transport leg is proven correct in a single-machine environment (Phase 113B4A) but was not successfully demonstrated live across these two specific VMs; the evidence gathered is consistent with a network-layer issue isolated to the destination VM, not a defect in the reviewed DATUM commit, but this session did not resolve or re-verify that boundary. Closing this specific gap requires attention to that VM's networking outside the scope of this session (see "Root-cause evidence" above).
