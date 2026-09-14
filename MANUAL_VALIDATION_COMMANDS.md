# Manual validation commands — D-Node/D-Continuum + 107A/107B foundation closure

For independent review, after this pass's implementation. These are
commands the USER runs manually; none were run automatically as part of
"closing" a finding in this pass unless explicitly stated otherwise above.

## LOCAL REPOSITORY (already run in this session, repeat to re-verify)

```
cd dserver
cargo build --locked
cargo test --locked
```
Expected: 1053 passed (dserver bin) + 1 passed (control-state-inspect bin), 0 failed.

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
