[CmdletBinding()]
param(
    [string]$ServerUrl = "http://localhost:8080",
    [string]$ProjectId = "smart-city-campinas",
    [string]$DatumDir = ".\DATUM",
    [string]$Fog01Config = ".\DATUM\config-fog-01.local.toml",
    [string]$Fog02Config = ".\DATUM\config-fog-02.local.toml",
    [Parameter(Mandatory = $true)]
    [string]$CloudSshTarget,
    [Parameter(Mandatory = $true)]
    [string]$CloudDatumDir,
    [Parameter(Mandatory = $true)]
    [string]$CloudConfig,
    [string]$Fog01Id = "fog-01",
    [string]$Fog02Id = "fog-02",
    [string]$CloudId = "cloud-01"
)

# I32 is deliberately a metadata-only smoke test. It never invokes `datum once`,
# dispatch, deployment, recovery, Docker, WASM, or a planner execution endpoint.
$ErrorActionPreference = "Stop"
$script:RestoreRequired = $false
$script:RestoreSucceeded = $false

$ScriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$PdRoot = Split-Path -Parent $ScriptRoot

function Resolve-RepoPath([string]$PathValue) {
    if ([System.IO.Path]::IsPathRooted($PathValue)) { return [System.IO.Path]::GetFullPath($PathValue) }
    return [System.IO.Path]::GetFullPath((Join-Path $PdRoot $PathValue))
}

function Write-Step([string]$Message) { Write-Host "`n==> $Message" }

function Get-Field($Json, [string]$Path, [string]$Step) {
    $value = $Json
    foreach ($part in $Path.Split('.')) {
        if ($null -eq $value) { throw "${Step}: missing JSON field '$Path'." }
        $property = $value.PSObject.Properties[$part]
        if ($null -eq $property) { throw "${Step}: missing JSON field '$Path'." }
        $value = $property.Value
    }
    return $value
}

function Assert-Equal($Actual, $Expected, [string]$Step, [string]$Name) {
    if ($Actual -ne $Expected) { throw "${Step}: '$Name' was '$Actual'; expected '$Expected'." }
}

function Assert-False($Json, [string]$Path, [string]$Step) {
    Assert-Equal (Get-Field $Json $Path $Step) $false $Step $Path
}

function Assert-Number($Json, [string]$Path, [int]$Expected, [string]$Step) {
    Assert-Equal ([int](Get-Field $Json $Path $Step)) $Expected $Step $Path
}

function Invoke-JsonGet([string]$Url, [string]$Step) {
    Write-Host "[$Step] GET $Url"
    try { $response = Invoke-WebRequest -Uri $Url -Headers @{ "X-Datum-Project" = $ProjectId } -Method Get -SkipHttpErrorCheck }
    catch { throw "${Step}: could not reach '$Url'. $($_.Exception.Message)" }
    if ([int]$response.StatusCode -ne 200) { throw "${Step}: expected HTTP 200; got $($response.StatusCode).`n$($response.Content)" }
    try { return $response.Content | ConvertFrom-Json -Depth 100 }
    catch { throw "${Step}: response was not JSON.`n$($response.Content)" }
}

function Invoke-LocalDatum([string[]]$Arguments, [string]$Step) {
    Write-Host "[$Step] cargo $($Arguments -join ' ')"
    Push-Location $ResolvedDatumDir
    try {
        $output = & cargo @Arguments 2>&1 | Out-String
        if ($LASTEXITCODE -ne 0) { throw "${Step}: cargo exited with $LASTEXITCODE.`n$($output.Trim())" }
        return $output
    } finally { Pop-Location }
}

function ConvertTo-PosixSingleQuoted([string]$Value) {
    $escapedQuote = [string]::Concat("'", '"', "'", '"', "'")
    return "'" + $Value.Replace("'", $escapedQuote) + "'"
}

function Invoke-CloudDatum([string[]]$Arguments, [string]$Step) {
    $joined = ($Arguments | ForEach-Object { ConvertTo-PosixSingleQuoted $_ }) -join ' '
    $command = "cd $(ConvertTo-PosixSingleQuoted $CloudDatumDir) && cargo $joined"
    Write-Host "[$Step] ssh $CloudSshTarget cargo $($Arguments -join ' ')"
    $output = & ssh $CloudSshTarget $command 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "${Step}: remote cargo exited with $LASTEXITCODE.`n$($output.Trim())" }
    return $output
}

function Assert-Output([string]$Output, [string]$Pattern, [string]$Step) {
    if ($Output -notmatch $Pattern) { throw "${Step}: expected output matching '$Pattern'." }
}

function Publish-Fixture([string]$NodeId, [string]$Config, [bool]$Cloud, [string]$Scenario, [string]$Step) {
    $args = @("run", "--quiet", "--", "dgraph-snapshot", "--config", $Config, "--fixture", $Scenario, "--publish")
    $output = if ($Cloud) { Invoke-CloudDatum $args $Step } else { Invoke-LocalDatum $args $Step }
    Assert-Output $output "Published local d-graph fixture snapshot" $Step
    Assert-Output $output "scenario:\s*$([regex]::Escape($Scenario))" $Step
    Assert-Output $output "safe to auto execute:\s*false" $Step
    Assert-Output $output "side effects performed:\s*false" $Step
}

function Assert-LatestSnapshot([string]$NodeId) {
    $step = "Latest snapshot $NodeId"
    $snapshot = Invoke-JsonGet "$ServerUrl/api/v1/nodes/$NodeId/dgraph-snapshot/latest" $step
    Assert-Equal (Get-Field $snapshot "project_id" $step) $ProjectId $step "project_id"
    Assert-Equal (Get-Field $snapshot "node_id" $step) $NodeId $step "node_id"
    Assert-False $snapshot "safe_to_auto_execute" $step
    Assert-False $snapshot "side_effects_performed" $step
}

function Assert-Baseline([string]$Step) {
    $materialized = Invoke-JsonGet "$ServerUrl/api/v1/dgraph/materialized/summary" "$Step materialized"
    Assert-Equal (Get-Field $materialized "project_id" "$Step materialized") $ProjectId "$Step materialized" "project_id"
    Assert-Number $materialized "source_snapshot_count" 3 "$Step materialized"
    foreach ($item in @(@("node_count", 3), @("runtime_count", 3), @("module_count", 3), @("flow_count", 2), @("relationship_count", 14), @("unknown_targets", 3), @("unhealthy_targets", 0), @("unavailable_targets", 0))) {
        Assert-Number $materialized "summary.$($item[0])" $item[1] "$Step materialized"
    }
    if (@(Get-Field $materialized "validation_errors" "$Step materialized").Count -ne 0) { throw "${Step} materialized: expected no validation errors." }
    Assert-False $materialized "side_effects_performed" "$Step materialized"

    $readiness = Invoke-JsonGet "$ServerUrl/api/v1/dgraph/readiness/summary" "$Step readiness"
    Assert-Equal (Get-Field $readiness "readiness_status" "$Step readiness") "ready" "$Step readiness" "readiness_status"
    Assert-Number $readiness "warning_count" 0 "$Step readiness"
    Assert-Number $readiness "blocked_count" 0 "$Step readiness"
    Assert-Number $readiness "degraded_flow_count" 0 "$Step readiness"
    Assert-False $readiness "side_effects_performed" "$Step readiness"
    Assert-False $readiness "metadata.execution_performed" "$Step readiness"
}

function Assert-DegradedFlow() {
    $readiness = Invoke-JsonGet "$ServerUrl/api/v1/dgraph/readiness/summary" "Degraded-flow readiness"
    Assert-Equal (Get-Field $readiness "readiness_status" "Degraded-flow readiness") "ready_with_warnings" "Degraded-flow readiness" "readiness_status"
    Assert-Number $readiness "warning_count" 1 "Degraded-flow readiness"
    Assert-Number $readiness "degraded_flow_count" 1 "Degraded-flow readiness"
    Assert-Number $readiness "blocked_count" 0 "Degraded-flow readiness"
    Assert-False $readiness "side_effects_performed" "Degraded-flow readiness"
    Assert-False $readiness "metadata.execution_performed" "Degraded-flow readiness"

    $planner = Invoke-JsonGet "$ServerUrl/api/v1/dgraph/planner/dry-run/summary" "Degraded-flow planner"
    Assert-Equal (Get-Field $planner "status" "Degraded-flow planner") "planned" "Degraded-flow planner" "status"
    Assert-Number $planner "warning_count" 1 "Degraded-flow planner"
    Assert-Number $planner "blocked_count" 0 "Degraded-flow planner"
    Assert-False $planner "side_effects_performed" "Degraded-flow planner"
}

function Assert-ObservedHealth() {
    $args = @("run", "--quiet", "--", "dgraph-snapshot", "--config", $ResolvedFog02Config, "--observed-health", "--json")
    $local = Invoke-LocalDatum $args "Observed-health local read"
    try { $payload = $local | ConvertFrom-Json -Depth 100 }
    catch { throw "Observed-health local read: expected JSON from DATUM. $($_.Exception.Message)" }
    Assert-False $payload "safe_to_auto_execute" "Observed-health local read"
    Assert-False $payload "side_effects_performed" "Observed-health local read"
    $health = @(Get-Field $payload "health" "Observed-health local read")
    if ($health.Count -lt 1) { throw "Observed-health local read: expected at least one persisted health observation." }
    foreach ($item in $health) { Assert-False $item "metadata.checks_executed" "Observed-health local read" }

    $published = Invoke-LocalDatum @("run", "--quiet", "--", "dgraph-snapshot", "--config", $ResolvedFog02Config, "--observed-health", "--publish") "Observed-health publish"
    Assert-Output $published "Published local d-graph snapshot" "Observed-health publish"
    Assert-Output $published "published health derived read-only from the persisted local snapshot" "Observed-health publish"
    Assert-Output $published "safe to auto execute:\s*false" "Observed-health publish"
    Assert-Output $published "side effects performed:\s*false" "Observed-health publish"
}

$ResolvedDatumDir = Resolve-RepoPath $DatumDir
$ResolvedFog01Config = Resolve-RepoPath $Fog01Config
$ResolvedFog02Config = Resolve-RepoPath $Fog02Config

Write-Step "I32 live three-node metadata-only smoke preflight"
foreach ($path in @($ResolvedDatumDir, $ResolvedFog01Config, $ResolvedFog02Config)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Preflight: required local path does not exist: $path" }
}
if ([string]::IsNullOrWhiteSpace($CloudDatumDir) -or [string]::IsNullOrWhiteSpace($CloudConfig)) { throw "Preflight: CloudDatumDir and CloudConfig are required." }
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { throw "Preflight: cargo is not available on this Windows host." }
if (-not (Get-Command ssh -ErrorAction SilentlyContinue)) { throw "Preflight: ssh is not available on this Windows host." }
& ssh -o BatchMode=yes -o ConnectTimeout=10 $CloudSshTarget "test -d $(ConvertTo-PosixSingleQuoted $CloudDatumDir) -a -f $(ConvertTo-PosixSingleQuoted $CloudConfig) -a -x `$(command -v cargo)" 2>&1 | Out-Null
if ($LASTEXITCODE -ne 0) { throw "Preflight: cloud SSH/config/cargo check failed for '$CloudSshTarget'." }
$null = Invoke-JsonGet "$ServerUrl/health" "Server health"

try {
    Write-Step "Publish three-node baseline"
    Publish-Fixture $Fog01Id $ResolvedFog01Config $false "baseline" "Publish fog-01 baseline"
    $script:RestoreRequired = $true
    Publish-Fixture $Fog02Id $ResolvedFog02Config $false "baseline" "Publish fog-02 baseline"
    Publish-Fixture $CloudId $CloudConfig $true "baseline" "Publish cloud-01 baseline"
    Assert-LatestSnapshot $Fog01Id; Assert-LatestSnapshot $Fog02Id; Assert-LatestSnapshot $CloudId
    Assert-Baseline "Baseline"

    Write-Step "Run degraded-flow advisory scenario"
    Publish-Fixture $Fog02Id $ResolvedFog02Config $false "degraded-flow" "Publish fog-02 degraded-flow"
    Assert-DegradedFlow

    Write-Step "Read and publish persisted observed health"
    Assert-ObservedHealth
} finally {
    if ($script:RestoreRequired) {
        Write-Step "Mandatory fog-02 baseline restoration"
        try {
            Publish-Fixture $Fog02Id $ResolvedFog02Config $false "baseline" "Restore fog-02 baseline"
            Assert-LatestSnapshot $Fog02Id
            Assert-Baseline "Restored baseline"
            $script:RestoreSucceeded = $true
            Write-Host "Baseline restoration succeeded."
        } catch {
            Write-Error "MANDATORY RESTORATION FAILED: $($_.Exception.Message)"
            throw
        }
    }
}

if (-not $script:RestoreSucceeded) { throw "Smoke did not confirm baseline restoration." }
Write-Step "I32 smoke completed"
Write-Host "Confirmed: 3 nodes, 3 runtimes, 3 modules, 2 flows, 14 relationships, 3 unknown, 0 unhealthy."
Write-Host "Confirmed: checks_executed=false, safe_to_auto_execute=false, side_effects_performed=false."
Write-Host "No recovery, dispatch, deployment, installation, Docker, WASM, or planner execution was invoked."
