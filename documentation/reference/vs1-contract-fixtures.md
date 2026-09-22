# VS1 contract fixtures

Stage 107 establishes the concrete shape of the canonical DATUM v2 contracts and
the v1 F2 support objects as **fixtures**, ahead of any runtime code. They live
under `scripts/fixtures/datum-contracts/vs1/` and are validated by
`scripts/check_datum_contract_fixtures.py`.

```
python scripts/check_datum_contract_fixtures.py            # run all checks
python scripts/check_datum_contract_fixtures.py --list     # list fixtures
python scripts/check_datum_contract_fixtures_selftest.py   # checker self-tests
```

The checker also runs the Stage 107 documentation checks (canonical terms present
in the glossary, ADR index complete, TD-1 in its correct lifecycle state, the
canonical-model page names every v2 contract shape id).

**Stage 110 P0 fix (ADR-0017, independent review v2):** the `dgraph.*`,
`dcontinuum.*` and `dmap.*` fixtures below originally self-declared
`"schema": "datum.dgraph/1"` / `"datum.dcontinuum/1"` / `"datum.dmap/1"` —
the exact strings later assigned to Stage 110's real, compiled, materially
different `CanonicalDGraphV1`/`CanonicalDContinuumV1`/`CanonicalDMapV1`
types. They were migrated to explicit `datum.x-fixture.vs1-dgraph/1` /
`datum.x-fixture.vs1-dcontinuum/1` / `datum.x-fixture.vs1-dmap/1` spellings
so one schema identifier denotes exactly one wire family. This is a label
change only — every validation rule and finding code is unchanged; see
`scripts/fixtures/datum-contracts/vs1/README.md` and ADR-0017 for the full
rationale. The real production contracts and their own fixtures live under
`scripts/fixtures/datum-contracts/canonical-v1/`, validated by
`scripts/check_canonical_datum_contracts_v1.py`.

## Placeholder-digest convention

There is no `datum-model` crate yet (Stage 108), no D-Compile (Stage 110) and no
D-Node runtime (Stage 113). Digests that can only be computed from a real artifact
use the sanctioned placeholder:

```
sha256:0000000000000000000000000000000000000000000000000000000000000000
```

The checker verifies cross-references by **natural key** — a D-Map's
`inputs.dgraph = {dgraph_id, revision}` must match a `datum.x-fixture.vs1-dgraph/1`
fixture's `dgraph_id` + `revision` — not by digest string, and it flags any other
real-looking `sha256:` value in a Stage 107 fixture. Real RFC 8785 JCS + SHA-256
digests (ADR-0006) did not, in the end, arrive with a shared `datum-model` crate
in Stage 108 (that stage became the runtime-kind dispatch seam instead); Stage 110
delivers the first real digests for `datum.dgraph/1`/`datum.dcontinuum/1`/
`datum.dmap/1` as independently-mirrored per-crate types, with their own
fixtures under `canonical-v1/` — these VS1 fixtures keep the placeholder
digest exactly as before.

## Positive fixtures

| File | Schema / role |
|---|---|
| `dgraph.sensor-pipeline.json` | `datum.x-fixture.vs1-dgraph/1` — the VS1 application graph (`decode → alert`, acyclic). |
| `dgraph.dcall-cycle.json` | `datum.x-fixture.vs1-dgraph/1` — a graph with a D-Call cycle, which is **accepted** (D-4). |
| `dcontinuum.monitor-f49c8.json` | `datum.x-fixture.vs1-dcontinuum/1` — S2-Mist / S3-Fog / S5-Cloud, elements `fog-01` and `cloud-01`. |
| `dcontinuum.host-invariance.json` | `datum.x-fixture.vs1-dcontinuum/1` — test-only: `fog-01` supports `container` and `native_process`. |
| `dforward.monitor-f49c8.json` | `datum.x-fixture.vs1-dforward/1` — historical D-IoT elements, interfaces, chains. |
| `dmap.sensor-pipeline.fog-only.json` | `datum.x-fixture.vs1-dmap/1` — VS1a: `decode` + `alert` on `fog-01`. |
| `dmap.sensor-pipeline.fog-cloud.json` | `datum.x-fixture.vs1-dmap/1` — VS1b: `decode@fog-01`, `alert@cloud-01`. |
| `dmap.sensor-pipeline.native-dnode.json` | `datum.x-fixture.vs1-dmap/1` — host-invariance pair of `fog-only`, native D-Node host. |
| `operational-slice.fog-01.container.json` | `datum.operational-slice/2` — VS1a slice, container-hosted D-Node. |
| `operational-slice.fog-01.native.json` | `datum.operational-slice/2` — same slice, native D-Node; `execution_contract` blocks byte-identical. |
| `dserv-artifact.decode.json`, `dserv-artifact.alert.json` | `datum.dserv-artifact/1` — the two D-Code artifacts. |
| `diot-artifact.mosquitto.json` | `ServiceArtifact` + `artifact_class` + the new execution-semantics fields. Standalone; the real catalog is untouched. |
| `artifact-execution-projection.mosquitto.legacy.json` | `datum.artifact-execution-projection/1` — current content; **not `required`-complete**. |
| `artifact-execution-projection.mosquitto.required.json` | `datum.artifact-execution-projection/1` — a `required`-complete variant. |
| `execution-generation-rules.json` | Documented expectations for the D-7a generation model (ADR-0011 §2). |
| `execution-pinning-modes.json` | The two pinning modes, defaults, `f2_status` mapping (N-2). |
| `local-policy-digest.json` | An example effective local policy; no secret values (N-1). |
| `oci-image-identity.json` | Single- and multi-platform OCI image identity (Stage 107B, G.5). |

## Negative fixtures

Every file under `scripts/fixtures/datum-contracts/vs1/negative/` carries an
`x_expect_reject.code`. The checker produces **structured findings** of the form
`{code, message}`, and a negative fixture passes **only when its declared
`x_expect_reject.code` appears among the produced finding codes** — an unrelated
validation error does not prove the intended failure. Additional findings are
allowed. `scripts/check_datum_contract_fixtures_selftest.py` proves this in both
directions (expected code present ⇒ pass; only an unrelated error ⇒ fail).

They cover: a dangling D-Call; a D-Call payload-schema mismatch; a duplicate
`dserv_id`; a D-Serv without D-Code; a `dserv_placement` with no matching
`dnode_runtime_placement`; an unsupported host kind; an ABI mismatch; an
unrealized D-Call; a **cyclic derived deployment-dependency graph** (distinct from
a D-Call cycle); a wildcard MQTT channel; a stage-constraint violation; a
`datum.dserv-artifact/1` carrying a host-kind field; a `DServExecutionContractV1`
carrying a broker address; and a `datum.operational-slice/2` in `legacy_partial`
mode.

## D-Forward chain-hop resolution

In `datum.x-fixture.vs1-dforward/1`, every chain hop's `from`/`to` identifier must resolve to a
declared `diots[].diot_id` or to an entry in the optional `external_endpoints[]`
array. The checker rejects dangling identifiers. `dforward.monitor-f49c8.json`
declares `lora-packet-forwarder` and `chirpstack-gateway-bridge` as D-IoT elements
so its `lorawan-uplink` chain is internally consistent.

## What the fixtures do not do

- They do not carry real cryptographic digests.
- They do not exercise any Rust code, DServer endpoint or agent path.
- They do not modify `artifacts/catalog/` or any laboratory configuration.
- They do not decide TD-1 (ADR-0012).
