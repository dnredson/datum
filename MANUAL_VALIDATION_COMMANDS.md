# Manual validation commands — D-Node/D-Continuum + 107A/107B foundation closure, canonical deterministic placement v1, canonical D-Serv artifact / D-Code WASM v0.1, and canonical D-Call realization v0.1

For independent review, after this pass's implementation. These are
commands the USER runs manually; none were run automatically as part of
"closing" a finding in this pass unless explicitly stated otherwise above.

## Canonical D-Serv artifact / D-Code WASM v0.1 (ADR-0021, roadmap phase 111) — v1 + independent review v2 + v3 + v4 + v5 (final ABI conformance, before commit)

Independent review v5 (FINAL, before commit) closed 5 more findings (H1-H5),
none of which change the CLI surface used below: the guest's EXACT packed
`datum_handle` return value `0` now correctly succeeds as canonical "no
output" (never fed into JSON-envelope parsing, distinct from the
non-canonical `(nonzero_ptr << 32) | 0`) — proven through the real isolated
worker too, not just in-process (H1); `datum_dealloc`'s exact typed
signature is now resolved and load-bearing BEFORE `datum_handle` runs
(previously only its NAME was checked, so a wrongly-typed `dealloc` could
silently execute) — a correctly-typed `dealloc` that itself traps AFTER a
successful `datum_handle` still follows this ADR's documented best-effort
semantics, unchanged (H2); the local D-Node identity init flow is now
serialized across concurrent processes/threads via a dedicated exclusive
`fs2` file lock, closing a TOCTOU race that could otherwise let two
concurrent initializations both believe they "won" (H3); a stale comment
claiming `wasm_backtrace(false)` mitigates the known native Wasmtime crash
(it does not — process isolation does) was corrected, along with an
adjacent stale claim about wall-clock timeout enforcement scope (H4); and
the review package's `test-results.txt` is regenerated as one coherent
report rather than a stale heading with newer results appended beneath it
(H5, packaging only).

Independent review v4 closed 3 more findings (G1-G3) plus a documentation
correction (G4): the governed CLI no longer accepts a raw `--node-id`
argument at all — it resolves its own local D-Node identity from a new,
explicitly-initialized (`smartsentinel-dcode-init`) local identity file
(`dcode::local_identity`), deliberately separate from both canonical D-Node
structural authority and Sentinel registration/lease, and uses that SAME
resolved value for both the DServer authorization request and the local
equality check — closing a prior circularity where a caller's own
`--node-id` claim was both the thing requested and the thing checked (G1);
the governed CLI no longer accepts `--worker-binary` either — production
worker resolution is always the trusted sibling `smartsentinel-dcode-worker`
next to the running executable, closing a gap where a caller could inject a
fake worker binary that fabricates a success response without ever running
Wasmtime (G2); execution evidence no longer persists the guest-controlled
output `reason` string verbatim, nor embeds a guest-controlled invalid
`status`/undeclared output `port` value in `error_message` — both are now
digest/size/presence only, proven via adversarial marker-exfiltration tests
across every guest-controlled channel (G3); and a doc comment claiming
"authorization targets THIS node" (true only after G1) is corrected (G4).

Independent review v2 closed 8 findings (R1-R8): the governed CLI now
obtains authorization/artifact directly from DServer (R1, no more
locally-fabricated authorization driving real execution); the DATUM-side
invocation path binds BOTH the module content digest AND the whole-artifact
descriptor digest via a full DATUM-side mirror of `datum.dserv-artifact/1`
(R2A/R2B/R2C); `dcode.size_bytes` is checked against the real loaded module
length (R3); the ADR-0003 `datum-dnode/0` envelope (ports, host-owned
message_id/correlation_id, emit validation) is now actually implemented
(R4); D-Code execution runs in an ISOLATED CHILD PROCESS
(`smartsentinel-dcode-worker`), so the documented Windows/Wasmtime automatic
fuel/epoch crash can no longer take down the parent SmartSentinel process
(R5); the CLI exits non-zero for any non-Success outcome (R6); every
`safety`/`limits` field is now actually enforced or validated (R7); and a
stale registry-comment inaccuracy about `artifact_id`/`version` was
corrected (R8).

Independent review v3 (FINAL, before commit) closed 7 more findings (F1-F7):
`constraints.allowed_stages` now fails closed as currently-unverifiable when
non-empty, since canonical D-Node authority has no stage identity to check it
against (F1); the isolated worker's timeout is now a HARD, UNGRACED deadline
— the previous 250ms hidden grace on top of `timeout_ms` is gone (F2);
execution evidence now also carries the DMap/DGraph/descriptor digests,
module size, and the ADR-0003 envelope's own digest/size/timestamp — not just
the module digest and raw payload — so a past invocation's exact bound
authority can be reconstructed from evidence alone (F3/F3A); the real
execution path now also runs the mirrored `artifact.validate()` before using
any limits/ports/safety field, not just DServer's registry (F4); a stale doc
comment wrongly implying control-plane authorization needs D-Call was
corrected (F5); two pre-existing, unrelated DATUM test regressions caused by
the already-committed foundation-closure catalog pinning were found and
fixed as baseline hygiene (F6); and `dcode.uri`/`dcode.sha256`/
`dcode.content_id` must now all agree, and `abi.required_exports` must be
exactly the canonical set, not merely a superset (F7).

`dserver` core tests (never rerun as a full suite in this pass per test
policy beyond one final confirmation below):
```
cd dserver
cargo build --locked
cargo test --locked
```
Expected: **1163** passed (DServer bin) + 1 passed (control-state-inspect
bin), 0 failed. (Neither v4 nor v5 made ANY dserver-side changes — G1-G4 and
H1-H5 are all DATUM-side CLI/library concerns — so this count is unchanged
since v3 and was re-confirmed by an actual full-suite run in this v5 pass,
not merely carried forward.)

Focused groups (already run and passing in this session; note `cargo test
<filter>` alone in `dserver/` can select the wrong default binary target —
use `cargo test --locked --bin DServer <filter>` explicitly, as shown):
```
cargo test --locked --bin DServer dserv_artifact        # 22 passed (unaffected by v4, re-confirmed)
cargo test --locked --bin DServer dcode_registry        # 8 passed (unaffected, re-confirmed)
cargo test --locked --bin DServer dcode_authorization   # 15 passed (unaffected, re-confirmed)
cargo test --locked --bin DServer ddeploy               # 148 passed (unaffected, re-confirmed)
cargo test --locked --bin DServer dnode_registry        # 27 passed (unaffected, re-confirmed)
cargo test --locked --bin DServer artifact_reconciliation_context  # 37 passed (unaffected, re-confirmed)
cargo check --locked && cargo check --locked --tests
```

DATUM side — per the standing memory note that DATUM's `[lib]` has
`test = false` in `Cargo.toml`, `cargo test` ALONE never runs its lib tests
— `--lib` (or a filter that also builds the lib target) is required:

```
cd DATUM
cargo build --locked
cargo test --locked --lib
```
Expected: **996 passed, 1 ignored**, 0 failed, for the full DATUM lib
(actually re-run and confirmed in this v5 pass — up from 979 in v4: +17 net
new, all in `dcode` — see below). Filter with `cargo test --locked --lib
dcode` for just the `dcode` module: **110 passed, 1 ignored** (up from 93 in
v4 — v5 added: `runtime`'s 9 new tests [H1 no-output x2, H2 dealloc-signature
x6, 1 missing-dealloc regression], `invoke`'s 3 new H1 no-output tests,
`worker`'s 1 new no-output JSON round-trip test, and `local_identity`'s 4 new
H3 concurrent-race tests = 17 net new).
The 1 ignored test (`infinite_loop_exhausts_fuel_and_fails_closed` in
`dcode::runtime`) remains the documented, reproducible environment-specific
Wasmtime/Windows crash in the IN-PROCESS automatic fuel-exhaustion trap path
— running it standalone (`cargo test --locked --lib -- --ignored
infinite_loop_exhausts_fuel_and_fails_closed`) is expected to CRASH the test
process with `STATUS_STACK_BUFFER_OVERRUN`; this is the documented finding
itself, not a new regression, and R5's whole point is that the REAL
production path no longer goes through this in-process code at all (see the
NEW, non-ignored, GREEN `dcode_worker_isolation` proof below).

**Two pre-existing, unrelated test regressions FOUND AND FIXED as F6
baseline-regression hygiene (not D-Code logic):** running the full DATUM
integration-test targets (`cargo test --locked --tests`) previously surfaced
`tests/smartsentinel_fog_bundle_contract.rs::fog_core_artifacts_and_local_profile_validate`
and (found during this pass's own final confirmation run)
`tests/smartsentinel_o2_service_artifact.rs::magistrala_bootstrap_db_catalog_contract_is_valid`,
both red because the earlier, already-committed foundation-closure pass
(`95349e1bd2e25aa9246de82092667b6fec5371d1`) correctly moved real catalog
images to digest-pinned identities, but these tests and
`DATUM/config.fog.linux`'s dispatch mappings (a REAL config feeding a
`trusted_image` governance check, not cosmetic) still referenced the old
mutable tags. Both fixed (config + one assertion updated to the correct
pinned identity, digest pinning itself untouched) — see
`foundation-regression-hygiene-audit.txt` in the review package. The full
`cargo test --locked --tests -- --test-threads=1` run remains genuinely
green in v5 (re-confirmed by an actual run, not carried forward): **481
passed, 0 failed**, across every integration test binary (up from 480 in
v4 — the 1 new v5 `dcode_worker_isolation` no-output-through-the-real-worker
test below).

Real end-to-end CLI proof (install a compiled WASM module, then invoke it,
via the actual `smartsentinel-dcode-install`/`smartsentinel-dcode-invoke`
binaries as subprocesses — `smartsentinel-dcode-invoke` now requires the
`offline-test-only` subcommand for this local-file-driven mode; the governed
`invoke` subcommand instead fetches from a live DServer, see below):
```
cargo test --locked --test dcode_cli_end_to_end
```
Expected: **6 passed**, 0 failed (up from 3 in v3 — v4 added
`governed_invoke_subcommand_rejects_raw_node_id_and_worker_binary_arguments`
(G1C-1/G2B-1), `smartsentinel_dcode_init_is_idempotent_and_fails_closed_on_conflict`
(G1C-4, via the real new `smartsentinel-dcode-init` binary), and
`governed_invoke_without_prior_init_fails_closed_before_any_network_attempt`
(G1C, proves the local check runs BEFORE any DServer contact)).

**Real process-isolation proof, with v3's strict-timeout tightening.** This
exercises the REAL `smartsentinel-dcode-worker` subprocess via the exact
same `dcode::worker::invoke_isolated` function the governed CLI path uses,
and proves the CALLING (test) process survives an infinite loop / fuel
exhaustion / a malformed module / a hung worker, none of which it could
survive if D-Code ran in-process on this platform — AND (new in v3) that
`timeout_ms` is enforced as a hard, ungraced deadline, not
`timeout_ms + 250ms`:
```
cargo test --locked --test dcode_worker_isolation
```
Expected: **10 passed**, 0 failed (up from 9 in v3/v4 — v5 added
`no_output_module_succeeds_through_the_isolated_worker_without_being_parsed_as_json`,
H1's proof that the canonical no-output distinction survives the REAL worker
IPC boundary, not just the in-process path), including
`infinite_loop_cannot_crash_the_parent_process` — GREEN and non-ignored,
unlike its in-process analog above.

To manually exercise the CLI yourself against a real module (requires a
`.wasm` file implementing the `datum-dnode/0` ABI — the WAT fixtures in
`DATUM/src/dcode/runtime.rs`'s test module can be compiled with `wat2wasm`
or the `wat` crate):
```
# INTERNAL/TEST-ONLY mode (bypasses DServer entirely -- retains --node-id
# and --worker-binary overrides for isolated testing):
cargo run --locked --bin smartsentinel-dcode-install -- \
  --module-path <path-to>.wasm --declared-sha256 <sha256> --store-root ./dcode-store
cargo run --locked --bin smartsentinel-dcode-invoke -- offline-test-only \
  --authorization-path authorization.json --artifact-path artifact.json \
  --node-id <node-id> --input-port <port> --store-root ./dcode-store --input-path input.bin

# GOVERNED PRODUCTION mode (R1, hardened by v4 G1/G2 -- requires a running
# DServer with the service already active + a registered D-Serv artifact).
# One-time step: initialize this installation's LOCAL D-Node identity
# (idempotent; a DIFFERENT node_id at the same --dnode-root fails closed):
cargo run --locked --bin smartsentinel-dcode-init -- \
  --node-id <node-id> --dnode-root ./dcode-store
# Then invoke -- note there is NO --node-id and NO --worker-binary here at
# all: both are resolved from the trusted local installation, never from a
# command-line argument.
cargo run --locked --bin smartsentinel-dcode-invoke -- invoke \
  --dserver-url http://127.0.0.1:8080 --project-id <project> \
  --application-id <app> --service-id <service> \
  --input-port <port> --dnode-root ./dcode-store --input-path input.bin
```

### WINDOWS / POWERSHELL — live-server proof of the governed D-Code invocation path (R1, v4 G1/G2)

Requires a running `DServer` instance, a declared D-Node (see the
D-Continuum authority section below), an accepted D-Map placing a service on
that node, and a registered `datum.dserv-artifact/1` for that service
(`POST /api/v1/dcode/artifacts`). Then, WITHOUT ever writing an authorization
file OR a `--node-id`/`--worker-binary` argument by hand, confirm the CLI's
`invoke` subcommand resolves its own local identity/worker and obtains a
fresh authorization live:
```powershell
cargo run --bin smartsentinel-dcode-init -- `
  --node-id <node-id> --dnode-root ./dcode-store
cargo run --bin smartsentinel-dcode-invoke -- invoke `
  --dserver-url http://127.0.0.1:8080 --project-id monitor-f49c8 `
  --application-id <app> --service-id <service> `
  --input-port <port> --dnode-root ./dcode-store --input-path input.bin
```
Expected: the process resolves `local_node_id` from `./dcode-store`'s
identity file (never from a command-line override), fetches a fresh
authorization + artifact over HTTP for that SAME node_id, recomputes and
checks the descriptor digest itself, and either succeeds or fails closed
with a specific error code — never silently accepts a hand-fabricated
authorization or a caller-chosen worker binary (there is no path to either
in this CLI mode at all).

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

## Canonical D-Call realization v0.1 (ADR-0022, roadmap phase 112) — v7 endpoint transport-authority closure

Continue the existing route/grant/evidence architecture. Production dispatch
accepts only an opaque `GovernedDCodeInvocationResult` and an index into its
own validated emits. The production source producer resolves local identity
and the installed sibling worker; it accepts no executor, node-id, or worker
override. Generic invocation results cannot enter production dispatch.
`dcall-test-support` remains a non-default Cargo feature for synthetic unit
fixtures. Its producer can use an arbitrary executor: those results are test
fixtures, not production execution provenance. Default builds omit those seams.

The real integration suite uses DEFAULT production entrypoints, including
`invoke_dcode_governed_capturing_emits`, `dispatch_validated_emit`, and the
governed receive path. It provisions the exact Cargo-built worker at the
sibling name resolved by the test executable's logical installation. It uses
no executor/worker/node override seam. Each real DServer has isolated config,
SQLite, operational state, and its own executable copy.

### Focused and real proof commands

```powershell
cd dserver
cargo test --locked --bin DServer dcall
cargo test --locked --bin DServer dcall_endpoint
cargo check --locked
cargo check --locked --tests
cd ../DATUM
cargo test --locked --features dcall-test-support --lib dcall
cargo test --locked --lib dcode
cargo check --locked
cargo check --locked --tests
# SQLite inspection requires Python. If the Windows alias is unavailable,
# set DATUM_PROOF_PYTHON to the real installed python.exe.
cargo test --locked --test dcall_two_node_integration -- --ignored --test-threads=1
cd ..
python -B scripts/check_dcall_default_provenance.py
```

The expensive suite contains SEVEN tests: remote governed delivery, local
governed delivery, payload-tamper rejection, stale-source after DGraph revision
change, stale-source after DMap destination-placement change, destination
message-limit preflight, and source message-limit preflight. All seven are
ignored in ordinary broad runs; the explicit `--ignored` command runs them.
The integration target no longer requires `dcall-test-support`.

Each stale mismatch is independently exercised through real HTTP issuance:
source service/node, DGraph id/revision/digest, DMap id/revision/digest, source
artifact id, module digest, and descriptor digest. Every rejection returns
HTTP 422 `dcall_source_execution_stale`, with no grant-state current entry or
history write. Real stale tests retain the old opaque source result across
actual proposal acceptance. SQLite remains empty; the control-plane observer
records no destination authorization request, which production execution
requires before invoking the destination worker. No retries occur.

Message-limit tests reject `payload_size > max_message_bytes` with the exact
source/destination codes, inspect SQLite current state AND history before any
accepted request, and accept the exact raw-payload boundary. These are
conservative RAW-PAYLOAD preflight bounds. Phase 111 remains the exact authority
for the serialized ADR-0003 envelope's byte size at actual execution; an
accepted boundary grant does not promise that its final envelope will fit.

Persisted-state corruption tests reject invalid local/remote node relationships,
endpoint presence/revision/id/digest, contract references, endpoint identities,
module/descriptor/evidence digests, and key/grant-id disagreement. Reload never
repairs state. The replay test issues, redeems, persists, reloads, and rejects
second redemption with the existing AlreadyRedeemed outcome.

The remote observer captures the REAL destination listener response for an
independent evidence digest and message/correlation comparison. SQLite's grant
record binds the exact source digest. The local suite also observes a complete
real destination evidence record on a second fresh, redeemed local grant and
compares the same public evidence builder's result. Ordinary local dispatch is
separately required to succeed through the same private destination helper.
Persistent grant and delivery evidence contains digests/sizes, never payloads.


## v7 transport authority and response closure

Grant issuance binds endpoint E (node, transport, endpoint id, revision and
JCS descriptor digest). After the production endpoint fetch, the source checks
node == grant destination, transport == HTTP, id/revision/digest == the grant.
It recomputes `sha256:` plus lowercase SHA-256 of existing DATUM JCS bytes for
exactly `{node_id, transport, endpoint_id, revision, base_url}` and requires
recomputed == fetched == grant digest. Only then is the bearer/payload POST
constructed and sent. Failure is `dcall_endpoint_authority_mismatch`, with a
fixed diagnostic hashed into sanitized evidence and no redemption attempt.

The live-observer regression exercises production `deliver_remote` and the
actual endpoint-fetch client: a legitimate revision-2 descriptor at a changed
URL receives ZERO requests from an old revision-1 grant. All control-plane
requests are recorded (exactly the GET, no redeem). Independent node, transport,
id, revision, claimed digest and URL substitutions also fail before delivery.
The unchanged real seven-test suite supplies matching-endpoint delivery.

DServer still checks CURRENT endpoint == E during redemption. This catches a
registry change AFTER source validation and BEFORE redemption and prevents
execution. It does not make the two checks atomic: authority can change in the
small gap before network arrival, and a previously validated address can then
receive bytes. Bare HTTP also provides no confidentiality against an on-path
attacker. This is not distributed atomicity or cryptographic endpoint attestation.

Registry URLs reject query/fragment components (including empty components)
with `dcall_endpoint_url_query_rejected` and
`dcall_endpoint_url_fragment_rejected`; path prefixes remain supported.
Malformed HTTP-2xx JSON, non-object/missing/non-string/unknown outcome, or invalid
success evidence fails `dcall_remote_response_malformed`. Structured `failed`
remains destination-execution failure. The existing 1-MiB body bound remains.

P2 decision: a small typed grant-request rejection preserves ONLY the three
whitelisted stale-source/source-limit/destination-limit codes in existing
error_code evidence. Arbitrary server code/message/body is never persisted as
text; the client retains only status and a body SHA-256 diagnostic. Unknown
codes retain `dcall_grant_request_failed`. No evidence schema expansion.

### Final broad commands

```powershell
cd dserver
cargo test --locked
cd ../DATUM
cargo test --locked --lib
cargo test --locked --tests -- --test-threads=1
cargo check --locked
cargo check --locked --tests
cargo check --locked --tests --features dcall-test-support
cd ..
python -B scripts/check_canonical_datum_contracts_v1.py
python -B scripts/check_canonical_datum_contracts_v1_selftest.py
python -B scripts/check_ddeploy_contracts_v1.py
python -B scripts/check_ddeploy_contracts_v1_selftest.py
python -B scripts/check_datum_contract_fixtures.py
python -B scripts/check_datum_contract_fixtures_selftest.py
python -m mkdocs build --strict
git diff --check
```

### Actual v7 results

The table below is generated from this pass's final command records, not prior
review totals. Zero Rust tests for compiler/checker commands means they are
command checks; their exit codes and output are recorded in test-results.txt.

| Check | Passed | Failed | Ignored |
|---|---:|---:|---:|
| server-focused | 60 | 0 | 0 |
| server-endpoint | 10 | 0 | 0 |
| server-check | 0 | 0 | 0 |
| server-check-tests | 0 | 0 | 0 |
| datum-dcall | 34 | 0 | 0 |
| datum-dcode | 113 | 0 | 1 |
| real-integration | 7 | 0 | 0 |
| server-broad | 1223 | 0 | 0 |
| datum-lib | 1010 | 0 | 1 |
| datum-tests | 481 | 0 | 7 |
| datum-check | 0 | 0 | 0 |
| datum-check-tests | 0 | 0 | 0 |
| datum-check-feature-tests | 0 | 0 | 0 |
| default-provenance | 0 | 0 | 0 |
| canonical | 0 | 0 | 0 |
| canonical-selftest | 0 | 0 | 0 |
| ddeploy | 0 | 0 | 0 |
| ddeploy-selftest | 0 | 0 | 0 |
| fixtures | 0 | 0 | 0 |
| fixtures-selftest | 0 | 0 | 0 |

MkDocs and final `git diff --check` results are recorded in the review package
after this documentation is generated and validated. All required commands
must exit zero before packaging.

### Remaining boundaries

Bare HTTP is for the trusted/lab network; grants do not provide TLS or remote
attestation. DServer compares caller-declared continuity and stores the opaque
source evidence digest; the governed installed runtime supplies execution
provenance. Local installation integrity is trusted. Distributed state can
change between preflight/redemption and guest execution; there is no distributed
atomicity. Lost responses remain ambiguous; grants provide at-most-once
redemption and there are no automatic retries or recursive dispatch.
The pre-existing ignored in-process Wasmtime interrupt-crash test remains;
production invocation uses child-process isolation. Earlier failed attempts
are retained in the review logs. The proof-observer native abort was traced
to inherited nonblocking sockets and double panic during cleanup; both were
fixed. Production listener sockets also explicitly enter blocking mode, with
a delayed-input regression. Existing native-runtime broad tests require
drive-root `/tmp` writes on Windows and passed outside the restricted sandbox.
Final results refer only to the current successful candidate.

No shipped dispatch demonstration CLI was added. D-Forward/D-IoT/D-Engine and
placement generation remain outside this slice. Existing B5/B6 lab-host
configuration and secret-version gaps remain outside Phase 112.
