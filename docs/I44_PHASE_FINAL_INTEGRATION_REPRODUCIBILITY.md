# I44/150 — Phase-Final Integration and Reproducibility

- checkpoint: `I44/150`
- predecessor: `I43/149`
- status: `implementation_ready_for_validation`
- target: close the Controlled WASM Deployment phase with one reproducible three-node laboratory campaign

## Purpose

I44 adds no new automatic execution behavior. It packages the already validated contracts into one clean-start campaign that a student or researcher can reproduce without reconstructing the history of I34-I43.

The campaign proves:

- external D-Graph ingestion and validation;
- typed deployment manifest derivation;
- D-Graph-derived dependency stages;
- three logical Sentinel registrations and leases;
- fresh node resource/capability observations;
- readiness evidence before execution;
- direct Sentinel inbox/execution/report path;
- explicit lifecycle attempts bound to real execution IDs;
- canonical observed D-Graph with deployment, resources, lifecycle and readiness evidence;
- persistence of source D-Graph, active manifest and live reports across dserver restart;
- reconstitution of readiness after restart without re-executing artifacts;
- preserved `automatic_execution_allowed=false` and no canonical governed automatic execution.

## Lab topology

```text
Desktop / local-01 — 192.168.10.101
  dserver :8080
  artifact HTTP server :8090
  local Sentinel binary/config

VM1 / fog-01 — 192.168.10.103
  Sentinel

VM2 / cloud-01 — 192.168.10.108
  Sentinel
```

Canonical flow:

```text
External D-Graph fixture
        |
        v
     dserver
        |
        +--> typed manifest
        +--> dependency stages
        +--> node slices
        +--> readiness evidence
        |
        v
Sentinel explicit execution
 fog -> local -> cloud
        |
        v
controlled live reports
        |
        v
observed D-Graph
 resources + lifecycle + readiness
```

## Fixture

Use:

```text
scripts/fixtures/i44-phase-final-three-node-dgraph.json
```

Identity:

```text
project_id=i44-live-project
dgraph_id=i44-phase-final-three-node
manifest_id=manifest:i44-phase-final-three-node
```

Expected assignments:

```text
a-fog-scale        target=fog-01   stage=0
m-local-aggregate  target=local-01 stage=1 depends_on=a-fog-scale
z-cloud-delta      target=cloud-01 stage=2 depends_on=m-local-aggregate
```

Expected scalar results:

```text
fog_scale(12, 2)       = 24
local_aggregate(24, 6) = 30
cloud_delta(30, 10)    = 20
```

## Regression gate

I44 changes only reproducibility fixtures/scripts/docs. The code baseline remains the validated I43 baseline.

Run from the repository root:

```powershell
cargo fmt --manifest-path dserver/Cargo.toml -- --check
cargo test --manifest-path dserver/Cargo.toml
cargo build --manifest-path dserver/Cargo.toml

cargo fmt --manifest-path DATUM/Cargo.toml -- --check
cargo test --manifest-path DATUM/Cargo.toml
cargo build --manifest-path DATUM/Cargo.toml
```

Expected baseline:

```text
dserver: 520 passed; 0 failed
DATUM:   431 passed; 0 failed
```

Existing non-blocking dead-code warnings remain non-claims, not I44 regressions.

## Phase A — clean start

### 1. Fresh controlled state

On the desktop:

```powershell
cd C:\Users\dnred\OneDrive\projetos\PD

New-Item -ItemType Directory -Force .i44-live | Out-Null
$StatePath = Join-Path (Get-Location) ".i44-live\controlled-state.json"

Remove-Item $StatePath -Force -ErrorAction SilentlyContinue
Remove-Item "$StatePath.tmp" -Force -ErrorAction SilentlyContinue
Remove-Item "$StatePath.bak" -Force -ErrorAction SilentlyContinue
```

Start dserver from its own working directory:

```powershell
cd C:\Users\dnred\OneDrive\projetos\PD\dserver
$env:DATUM_CONTROLLED_WASM_STATE_PATH = "C:\Users\dnred\OneDrive\projetos\PD\.i44-live\controlled-state.json"
cargo run
```

### 2. Artifact server

In a second desktop PowerShell:

```powershell
cd C:\Users\dnred\OneDrive\projetos\PD

$Artifact = Get-ChildItem -Path . -Recurse -Filter fog-scale.wasm | Select-Object -First 1
$ServeRoot = Split-Path $Artifact.Directory.FullName -Parent

py -3 -m http.server 8090 --bind 0.0.0.0 --directory "$ServeRoot"
```

Confirm:

```powershell
curl.exe -I http://192.168.10.101:8090/artifacts/fog-scale.wasm
curl.exe -I http://192.168.10.101:8090/artifacts/local-aggregate.wasm
curl.exe -I http://192.168.10.101:8090/artifacts/cloud-delta.wasm
```

Expected: HTTP 200 for all three artifacts.

Python is only the static artifact transport in this laboratory fixture. It does not orchestrate the Sentinel execution path.

### 3. Publish the external D-Graph

In a third desktop PowerShell:

```powershell
cd C:\Users\dnred\OneDrive\projetos\PD
$IP = "192.168.10.101"

$Activation = Invoke-RestMethod `
    -Method Post `
    -Uri "http://${IP}:8080/api/v1/controlled-wasm/dgraph" `
    -ContentType "application/json" `
    -InFile "scripts/fixtures/i44-phase-final-three-node-dgraph.json"

$Activation.manifest.manifest_id
$Activation.project_id
```

Expected:

```text
manifest:i44-phase-final-three-node
i44-live-project
```

### 4. Publish presence and resource prerequisites

Use the I44 helper:

```powershell
.\scripts\i44-publish-prerequisites.ps1
```

Expected:

```text
I44 prerequisites PASS
sentinel_count=3
active_count=3
stale_count=0
execution_performed=false
```

This helper performs no artifact execution.

### 5. Validate pre-execution state

```powershell
.\scripts\i44-validate-controlled-state.ps1 `
  -ExpectedReportCount 0 `
  -ExpectedLifecycleAttemptCount 0 `
  -ExpectedReadyAssignmentCount 1 `
  -ExpectedWaitingAssignmentCount 2 `
  -ExpectedCompletedAssignmentCount 0
```

Expected global state:

```text
readiness=ready_for_explicit_execution
reports=0
lifecycle_attempts=0
ready=1
waiting=2
completed=0
automatic_execution_allowed=false
```

## Phase B — explicit dependency-ordered execution

The execution sequence is deliberately manual and explicit:

```text
fog -> local -> cloud
```

For each assignment:

1. create lifecycle attempt;
2. transition `discovered -> prepared -> executing`;
3. invoke the bound Sentinel explicitly;
4. obtain the real controlled live report execution ID;
5. transition lifecycle attempt to `executed` using that execution ID.

No loop, Python orchestrator or automatic retry is used in the Sentinel success path.

### 6. Fog lifecycle preparation

Desktop:

```powershell
$Body = @{
    attempt_id = "i44-fog-attempt-1"
    assignment_id = "a-fog-scale"
    requested_by = "i44-phase-final"
} | ConvertTo-Json

Invoke-RestMethod `
    -Method Post `
    -Uri "http://${IP}:8080/api/v1/controlled-wasm/deployment-attempts" `
    -ContentType "application/json" `
    -Body $Body | Out-Null

foreach ($Status in @("prepared", "executing")) {
    $Body = @{ status=$Status; reason="i44 phase-final fog $Status" } | ConvertTo-Json
    Invoke-RestMethod `
        -Method Post `
        -Uri "http://${IP}:8080/api/v1/controlled-wasm/deployment-attempts/i44-fog-attempt-1/transition" `
        -ContentType "application/json" `
        -Body $Body | Out-Null
}
```

### 7. Fog explicit Sentinel execution

On VM1 `.103`:

```bash
cd ~/datum-i38

DATUM/target/debug/datum wasm-deploy-live \
  --config .i38-live/fog.toml \
  --deployment http://192.168.10.101:8080 \
  --assignment auto \
  --arg i32:12 \
  --arg i32:2 \
  --execute \
  --invoked-by i44-phase-final \
  --allow-insecure-http \
  --json
```

Expected:

```text
assignment_id=a-fog-scale
node_id=fog-01
status=executed
result=24
report_published=true
```

Desktop — bind lifecycle to the real report:

```powershell
$Reports = Invoke-RestMethod "http://${IP}:8080/api/v1/controlled-wasm/live-reports"
$FogReport = $Reports.reports | Where-Object { $_.assignment_id -eq "a-fog-scale" } | Select-Object -Last 1
$FogExecutionId = $FogReport.execution_id

$Body = @{
    status = "executed"
    execution_id = $FogExecutionId
    reason = "i44 fog execution linked to controlled live report"
} | ConvertTo-Json

Invoke-RestMethod `
    -Method Post `
    -Uri "http://${IP}:8080/api/v1/controlled-wasm/deployment-attempts/i44-fog-attempt-1/transition" `
    -ContentType "application/json" `
    -Body $Body | Out-Null
```

Readiness should now release local:

```powershell
$Observed = Invoke-RestMethod "http://${IP}:8080/api/v1/controlled-wasm/observed-dgraph"
$Observed.metadata.deployment_readiness.assignments |
    Select-Object assignment_id, status, waiting_on_assignments
```

Expected semantic state:

```text
a-fog-scale        completed
m-local-aggregate  ready
z-cloud-delta      waiting_on_dependencies
```

### 8. Local lifecycle and explicit execution

Desktop lifecycle:

```powershell
$Body = @{
    attempt_id = "i44-local-attempt-1"
    assignment_id = "m-local-aggregate"
    requested_by = "i44-phase-final"
} | ConvertTo-Json

Invoke-RestMethod -Method Post `
    -Uri "http://${IP}:8080/api/v1/controlled-wasm/deployment-attempts" `
    -ContentType "application/json" -Body $Body | Out-Null

foreach ($Status in @("prepared", "executing")) {
    $Body = @{ status=$Status; reason="i44 phase-final local $Status" } | ConvertTo-Json
    Invoke-RestMethod -Method Post `
        -Uri "http://${IP}:8080/api/v1/controlled-wasm/deployment-attempts/i44-local-attempt-1/transition" `
        -ContentType "application/json" -Body $Body | Out-Null
}
```

Desktop Sentinel execution:

```powershell
DATUM\target\debug\datum.exe wasm-deploy-live `
  --config .i35-live\local.toml `
  --deployment http://${IP}:8080 `
  --assignment auto `
  --arg i32:24 `
  --arg i32:6 `
  --execute `
  --invoked-by i44-phase-final `
  --allow-insecure-http `
  --json
```

Expected result: `30`.

Bind lifecycle:

```powershell
$Reports = Invoke-RestMethod "http://${IP}:8080/api/v1/controlled-wasm/live-reports"
$LocalReport = $Reports.reports | Where-Object { $_.assignment_id -eq "m-local-aggregate" } | Select-Object -Last 1
$LocalExecutionId = $LocalReport.execution_id

$Body = @{
    status = "executed"
    execution_id = $LocalExecutionId
    reason = "i44 local execution linked to controlled live report"
} | ConvertTo-Json

Invoke-RestMethod -Method Post `
    -Uri "http://${IP}:8080/api/v1/controlled-wasm/deployment-attempts/i44-local-attempt-1/transition" `
    -ContentType "application/json" -Body $Body | Out-Null
```

Expected readiness progression:

```text
a-fog-scale        completed
m-local-aggregate  completed
z-cloud-delta      ready
```

### 9. Cloud lifecycle and explicit execution

Desktop lifecycle:

```powershell
$Body = @{
    attempt_id = "i44-cloud-attempt-1"
    assignment_id = "z-cloud-delta"
    requested_by = "i44-phase-final"
} | ConvertTo-Json

Invoke-RestMethod -Method Post `
    -Uri "http://${IP}:8080/api/v1/controlled-wasm/deployment-attempts" `
    -ContentType "application/json" -Body $Body | Out-Null

foreach ($Status in @("prepared", "executing")) {
    $Body = @{ status=$Status; reason="i44 phase-final cloud $Status" } | ConvertTo-Json
    Invoke-RestMethod -Method Post `
        -Uri "http://${IP}:8080/api/v1/controlled-wasm/deployment-attempts/i44-cloud-attempt-1/transition" `
        -ContentType "application/json" -Body $Body | Out-Null
}
```

On VM2 `.108`:

```bash
cd ~/datum-i38

DATUM/target/debug/datum wasm-deploy-live \
  --config .i38-live/cloud.toml \
  --deployment http://192.168.10.101:8080 \
  --assignment auto \
  --arg i32:30 \
  --arg i32:10 \
  --execute \
  --invoked-by i44-phase-final \
  --allow-insecure-http \
  --json
```

Expected result: `20`.

Desktop lifecycle binding:

```powershell
$Reports = Invoke-RestMethod "http://${IP}:8080/api/v1/controlled-wasm/live-reports"
$CloudReport = $Reports.reports | Where-Object { $_.assignment_id -eq "z-cloud-delta" } | Select-Object -Last 1
$CloudExecutionId = $CloudReport.execution_id

$Body = @{
    status = "executed"
    execution_id = $CloudExecutionId
    reason = "i44 cloud execution linked to controlled live report"
} | ConvertTo-Json

Invoke-RestMethod -Method Post `
    -Uri "http://${IP}:8080/api/v1/controlled-wasm/deployment-attempts/i44-cloud-attempt-1/transition" `
    -ContentType "application/json" -Body $Body | Out-Null
```

### 10. Validate integrated pre-restart state

Run:

```powershell
.\scripts\i44-validate-controlled-state.ps1
```

Expected PASS summary:

```text
report_count=3
active_sentinels=3
resource_observations=3 fresh=3 stale=0
lifecycle_attempts=3
readiness=ready_for_explicit_execution completed=3
validation_errors=0
automatic_execution_allowed=false
governed_execution_performed=false
```

Also confirm the three scalar outcomes:

```powershell
$Reports.reports | Select-Object assignment_id, node_id, status, results
```

Expected semantic results:

```text
a-fog-scale        fog-01   executed 24
m-local-aggregate  local-01 executed 30
z-cloud-delta      cloud-01 executed 20
```

## Phase C — restart persistence boundary

### 11. Record pre-restart report count

```powershell
$BeforeRestart = Invoke-RestMethod "http://${IP}:8080/api/v1/controlled-wasm/live-reports"
$BeforeRestart.count
```

Expected: `3`.

Stop dserver with `Ctrl+C`.

### 12. Restart dserver using the same state file

```powershell
cd C:\Users\dnred\OneDrive\projetos\PD\dserver
$env:DATUM_CONTROLLED_WASM_STATE_PATH = "C:\Users\dnred\OneDrive\projetos\PD\.i44-live\controlled-state.json"
cargo run
```

Expected startup evidence includes restoration of controlled WASM deployment state.

### 13. Prove persisted controlled state

In the test PowerShell:

```powershell
$SourceAfterRestart = Invoke-RestMethod "http://${IP}:8080/api/v1/controlled-wasm/dgraph"
$ManifestAfterRestart = Invoke-RestMethod "http://${IP}:8080/api/v1/controlled-wasm/deployment-manifest"
$ReportsAfterRestart = Invoke-RestMethod "http://${IP}:8080/api/v1/controlled-wasm/live-reports"

$SourceAfterRestart.project_id
$ManifestAfterRestart.manifest_id
$ReportsAfterRestart.count
$BeforeRestart.count -eq $ReportsAfterRestart.count
```

Expected:

```text
i44-live-project
manifest:i44-phase-final-three-node
3
True
```

No new artifact execution occurred during restart.

### 14. Explicit in-memory non-claims after restart

Sentinel registrations are intentionally in-memory in this phase:

```powershell
$SentinelsAfterRestart = Invoke-RestMethod "http://${IP}:8080/api/v1/sentinels"
$SentinelsAfterRestart.count
```

Expected: `0`.

Lifecycle attempts are also intentionally in-memory:

```powershell
$AttemptsAfterRestart = Invoke-RestMethod "http://${IP}:8080/api/v1/controlled-wasm/deployment-attempts"
$AttemptsAfterRestart.count
```

Expected: `0`.

This is not data-loss in the persisted controlled envelope. The persisted I36 contract is specifically source D-Graph + active manifest + accepted live reports. I44 documents the boundary rather than silently claiming persistence that does not exist.

### 15. Reconstitute presence/resources without re-execution

Run the prerequisite helper again:

```powershell
cd C:\Users\dnred\OneDrive\projetos\PD
.\scripts\i44-publish-prerequisites.ps1
```

This creates fresh Sentinel registrations and fresh resource observations but performs no artifact execution.

Now validate the restored state. Lifecycle count is expected to remain zero, while the three persisted reports keep all assignments completed:

```powershell
.\scripts\i44-validate-controlled-state.ps1 `
  -ExpectedLifecycleAttemptCount 0
```

Expected:

```text
report_count=3
active_sentinels=3
resource_observations=3 fresh=3 stale=0
lifecycle_attempts=0
readiness=ready_for_explicit_execution completed=3
validation_errors=0
automatic_execution_allowed=false
governed_execution_performed=false
```

Confirm report count did not change:

```powershell
$FinalReports = Invoke-RestMethod "http://${IP}:8080/api/v1/controlled-wasm/live-reports"
$BeforeRestart.count
$ReportsAfterRestart.count
$FinalReports.count
```

Expected:

```text
3
3
3
```

## Final PASS criteria

I44 closes only when all of the following are observed:

```text
dserver regression 520/520 PASS
DATUM regression 431/431 PASS
artifact HTTP transport reachable
external D-Graph activation PASS
three Sentinels active before execution
three fresh resource observations
initial readiness ready_for_explicit_execution
fog explicit execution result 24
local explicit execution result 30
cloud explicit execution result 20
three controlled live reports
three lifecycle attempts executed and bound to real execution IDs
observed D-Graph validation_errors=0
all three readiness assignments completed
restart restores source D-Graph + manifest + three reports
restart does not create another report
Sentinel registration non-persistence explicitly observed
lifecycle-attempt non-persistence explicitly observed
re-registration/resource refresh restores healthy readiness
final report count remains exactly 3
automatic_execution_allowed=false
governed_execution_performed=false
```

## Safety/non-claims

I44 does **not** claim or introduce:

- automatic governed deployment;
- cryptographic Sentinel identity attestation;
- persisted Sentinel registration/lease state;
- persisted deployment lifecycle attempts;
- rollback automation;
- placement decisions driven automatically by resources;
- Kubernetes, Ansible or AWS integration;
- Python orchestration of Sentinel execution;
- guest filesystem/network/host-command/Docker access from the controlled WASM sandbox.

The artifact HTTP server may be implemented with Python's static `http.server` in the lab. This is transport only and is outside the Sentinel decision/execution orchestration path.

## Extension points after 150/150

```text
D-Graph research
  visualization, queries, GNN-ready export, relationship learning

Deployment-continuum research
  placement policies, scheduling experiments, dynamic trigger policies,
  stronger identity, lifecycle persistence, rollback/audit

AI / what-if research
  consume observed graph/resource/lifecycle snapshots to compare candidate
  placements without authorizing execution directly

Infrastructure expansion
  Raspberry Pi, physical fog nodes, AWS/cloud targets, Ansible/Kubernetes
  as later experiments rather than phase-closing dependencies
```

## Final architecture boundary

```text
Observed evidence may inform a future policy engine
                 |
                 v
        readiness / what-if
                 |
                 X  no automatic transition in this phase
                 |
                 v
explicit operator/Sentinel invocation
                 |
                 v
controlled WASM execution + report
```

I44's goal is reproducibility and closure, not removal of that `X` boundary.
