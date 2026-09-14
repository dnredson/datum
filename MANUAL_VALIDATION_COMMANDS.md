# Manual validation commands — D-Node/D-Continuum + 107A/107B foundation closure, and canonical deterministic placement v1

For independent review, after this pass's implementation. These are
commands the USER runs manually; none were run automatically as part of
"closing" a finding in this pass unless explicitly stated otherwise above.

## Canonical deterministic placement v1 (ADR-0020) — v1 + v2 + v3 (independent review fixes)

Per this pass's test policy, the full suite below was deliberately NOT
re-run to produce this count (only the focused groups were, and are
reported in `canonical-deterministic-placement-v1-review-v3.zip`'s
`test-results.txt`) — the total below is a computed expectation from the
previously-verified 1102-passed v2 baseline (itself 1063 + 19 + 20) plus
this v3 pass's own known net-new test count (1 new
`core::canonical_placement` unit test [eligibility-snapshot
absent-vs-present-empty JSON round-trip] + 11 new `core::ddeploy` tests
[provenance input-completeness: commitment/eligibility round-trip,
distinctness-changes-provenance x2, and 7 C1D decision-integrity/tamper
reload-failure tests] + 4 new `api::ddeploy` direct-serde tests = 16), for
the user to confirm directly:

```
cd dserver
cargo build --locked
cargo test --locked
```
Expected: **1118** passed (dserver bin, = 1063 + 19 [v1] + 20 [v2] + 16 [v3])
+ 1 passed (control-state-inspect bin), 0 failed.

Focused groups (already run and passing in this session):
```
cargo test --locked canonical_placement   # 23 passed (16 v1 + 6 v2 + 1 v3)
cargo test --locked ddeploy                # 148 passed (119 v1 + 10 core v2 + 4 api v2 + 11 core v3 + 4 api v3)
cargo test --locked api::ddeploy           # 8 passed (4 v2 + 4 v3)
cargo test --locked dnode_registry         # 27 passed (unaffected, re-confirmed)
cargo test --locked artifact_reconciliation_context   # 37 passed (unaffected, re-confirmed)
cargo check --locked                       # clean
cargo check --locked --tests               # clean
```

### WINDOWS / POWERSHELL — live-server proof of `POST /api/v1/ddeploy/plan`

Requires a running `DServer` instance and at least one declared D-Node (see
the "D-Node/D-Continuum authority transition" section below, steps 1-2, to
declare `fog-01`/`cloud-01` first).

```powershell
$base = "http://127.0.0.1:8080"
$plan = @{
  application_id = "app-planned-demo"
  dgraph = @{
    schema = "datum.dgraph/1"; application_id = "app-planned-demo"
    dgraph_id = "dgraph:planned-demo"; revision = 1
    services = @(
      @{ service_id = "svc-a"; execution_requirement = @{ dnode_abi = "datum-dnode/0" }; resource_requirement = @{ cpu_cores = 1; memory_bytes = 134217728 } }
    )
    calls = @()
  }
} | ConvertTo-Json -Depth 8
Invoke-RestMethod -Method Post -Uri "$base/api/v1/ddeploy/plan" -ContentType "application/json" -Headers @{ "X-Datum-Project" = "monitor-f49c8" } -Body $plan
```
Expected: HTTP 201, a `state: "pending"` proposal body with a populated
`placement_provenance.decisions[0]` naming a declared D-Node, and
`proposal.source == "deterministic"`. Follow with the normal
`POST /api/v1/ddeploy/proposals/{proposal_id}/accept` call (see below) to
prove the planner never auto-accepts — that call remains a SEPARATE,
explicit step with its own `accepted_by` audit label.

### v2 fix — R1 spoofing attempt must be rejected with 400

Add a `source` (or `dcontinuum`/`placements`) field to the `$plan` body
above and resend it:

```powershell
$spoofed = $plan | ConvertFrom-Json
$spoofed | Add-Member -NotePropertyName source -NotePropertyValue "ai"
Invoke-RestMethod -Method Post -Uri "$base/api/v1/ddeploy/plan" -ContentType "application/json" -Headers @{ "X-Datum-Project" = "monitor-f49c8" } -Body ($spoofed | ConvertTo-Json -Depth 8)
```
Expected: HTTP 4xx (a JSON-extraction rejection from `deny_unknown_fields`)
and no proposal created — never a 201 with `proposal.source` reflecting the
spoofed value. This pass did not execute an end-to-end Axum integration test
against the pinned Axum version to confirm the EXACT status code (400 vs
422 vs another 4xx), so an exact code is deliberately not asserted here
(independent review v3, C2 correction) — `api::ddeploy::tests::
plan_request_payload_serde_rejects_an_explicit_source_field` (and its
`dcontinuum`/`placements` siblings) directly prove the deserialization
itself fails via `serde_json::from_str`, which is what actually determines
this behavior regardless of which 4xx status Axum's JSON extractor maps a
`serde` error to.

### v2 fix — R3 project-wide overcommit must fail at the SECOND acceptance

Declare a small node (e.g. `cpu_cores: 4`), submit and accept two Pending
proposals for two DIFFERENT applications each requesting 4 CPU cores on it.
The first `POST .../accept` must succeed; the second must return HTTP 422
with `"error":"project_node_cpu_capacity_exceeded"` — never a silent
double-commit.

```
cd ..
python scripts/check_datum_contract_fixtures.py
python scripts/check_datum_contract_fixtures_selftest.py
python scripts/check_canonical_datum_contracts_v1.py
python scripts/check_canonical_datum_contracts_v1_selftest.py
python scripts/check_ddeploy_contracts_v1.py
python scripts/check_ddeploy_contracts_v1_selftest.py
python scripts/check_real_catalog_oci_pinning.py
python scripts/check_real_catalog_oci_pinning_selftest.py
python -m mkdocs build --strict
```
Expected: every checker exits 0; the new one reports "19 registry, digest-pinned; 0 registry, UNPINNED; 3 local_build; 0 legacy_unclassified".

## WINDOWS / POWERSHELL — end-to-end live-server proof of the D-Node/D-Continuum authority transition

These require a running `DServer` instance (`cargo run` from `dserver/`,
default `http://127.0.0.1:8080`, adjust the base URL/port to your actual
config). None of these were run against a live server during this pass —
only the offline Rust unit tests above were executed for the D-Node/
D-Continuum authority logic itself; this section is the live end-to-end
proof for a human reviewer.

1. Declare the two lab D-Nodes the real fixtures already assume
   (`fog-01`, `cloud-01` — same platform/capacity as
   `dserver/src/core/ddeploy.rs`'s own `sample_dcontinuum` test fixture and
   `canonical-v1/dcontinuum.smart-sentinel-lab.json`):

```powershell
$base = "http://127.0.0.1:8080"

Invoke-RestMethod -Method Put -Uri "$base/api/v1/dnodes/fog-01" -ContentType "application/json" -Body (@{
  node_id = "fog-01"
  platform = @{ os = "linux"; architecture = "amd64" }
  capacity = @{ cpu_cores = 4; memory_bytes = 4294967296 }
  capabilities = @{ dnode_abi_versions = @("datum-dnode/0") }
} | ConvertTo-Json -Depth 5)

Invoke-RestMethod -Method Put -Uri "$base/api/v1/dnodes/cloud-01" -ContentType "application/json" -Body (@{
  node_id = "cloud-01"
  platform = @{ os = "linux"; architecture = "amd64" }
  capacity = @{ cpu_cores = 8; memory_bytes = 17179869184 }
  capabilities = @{ dnode_abi_versions = @("datum-dnode/0") }
} | ConvertTo-Json -Depth 5)
```

2. Fetch the authoritative D-Continuum (this is what a well-behaved client
   should submit back verbatim as `dcontinuum` on a proposal):

```powershell
Invoke-RestMethod -Method Get -Uri "$base/api/v1/dcontinuum/authoritative" -Headers @{ "X-Datum-Project" = "monitor-f49c8" }
```

3. Prove the negative case — a proposal claiming a NODE THE REGISTRY DOES
   NOT KNOW ABOUT must be rejected with `dcontinuum_node_unregistered`
   (adjust `dgraph`/`placements` to a real, valid D-Graph for your project;
   the exact 422 body is what matters here):

```powershell
$fabricated = @{
  application_id = "app-x"
  dgraph = $realDgraph            # substitute a real, valid datum.dgraph/1
  dcontinuum = @{
    schema = "datum.dcontinuum/1"; project_id = "monitor-f49c8"
    dcontinuum_id = "fake"; revision = 1
    nodes = @(@{ node_id = "fabricated-node"; platform = @{ os = "linux"; architecture = "amd64" }; capacity = @{ cpu_cores = 999; memory_bytes = 999999999999 }; capabilities = @{ dnode_abi_versions = @("datum-dnode/0") } })
    links = @()
  }
  placements = @(@{ service_id = "svc-a"; node_id = "fabricated-node" })
} | ConvertTo-Json -Depth 8
try {
  Invoke-RestMethod -Method Post -Uri "$base/api/v1/ddeploy/proposals" -ContentType "application/json" -Headers @{ "X-Datum-Project" = "monitor-f49c8" } -Body $fabricated
} catch {
  $_.Exception.Response | Select-Object -ExpandProperty StatusCode
  $_.ErrorDetails.Message
}
```
Expected: HTTP 422, `"error":"dnode_declaration_invalid"` shaped body (from
`unprocessable`) containing a finding with code `dcontinuum_node_unregistered`.

4. Prove a genuine proposal against the now-declared nodes succeeds (use the
   authoritative continuum fetched in step 2, or the real fixtures under
   `scripts/fixtures/datum-contracts/ddeploy-v1/`).

## LOCAL LAB HOSTS — real configuration-content hashing (B5)

Every real catalog configuration-mount `source` path (e.g.
`mosquitto/mosquitto.conf`) references a file that lives only on the actual
lab hosts, not in this repository — this pass could not compute these
hashes without inventing bytes it has never seen. Run on the host(s) that
actually hold these files, then set the resulting hash as
`declared_content_sha256` on the matching catalog artifact's
`configuration_mounts[]` entry:

```
sha256sum mosquitto/mosquitto.conf
sha256sum chirpstack/gateway-bridge.toml
sha256sum chirpstack/postgres/initdb          # NOTE: this is a directory in the real deployment layout; a single sha256sum does not apply to a directory tree -- a tree-hash convention would need to be decided first (out of scope for this pass, not invented here)
sha256sum chirpstack-provisioner
sha256sum chirpstack/server
sha256sum magistrala/atom-bootstrap.yaml
sha256sum magistrala/ssl/certs               # directory -- same caveat as above
sha256sum magistrala/fluxmq/node1.yaml
sha256sum magistrala/fluxmq/node2.yaml
sha256sum magistrala/fluxmq/node3.yaml
sha256sum magistrala/nginx                   # directory -- same caveat as above
sha256sum magistrala/addons/timescale-writer/config.toml
```

## Optional: re-run the live-registry catalog resolution yourself

The 16 real digests this pass wrote into `artifacts/catalog/*.json` were
obtained by temporarily adding an `#[ignore]`d test to
`dserver/src/core/oci_registry_resolver.rs` that called the module's own
real `resolve_image_identity` against the live Docker Hub/GHCR registries,
then deleting that test immediately afterward (`git status`/`git diff` for
that file is empty — confirm with `git log -p --follow` if you want to see
it never landed). To independently re-verify any one digest without
modifying source, e.g. for `eclipse-mosquitto:2`:

```
curl -sS -H "Accept: application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.index.v1+json" \
  "https://auth.docker.io/token?service=registry.docker.io&scope=repository:library/eclipse-mosquitto:pull" | python3 -c "import sys,json;print(json.load(sys.stdin)['token'])" > /tmp/tok.txt
curl -sS -D - -o /dev/null -H "Authorization: Bearer $(cat /tmp/tok.txt)" \
  -H "Accept: application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.index.v1+json" \
  "https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/2" | grep -i docker-content-digest
```
