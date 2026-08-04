[CmdletBinding()]
param(
    [string]$ServerUrl = "http://localhost:8080",
    [string]$ProjectId = "smart-city-campinas",
    [string]$DatumDir = ".\DATUM",
    [string]$Fog01Config = ".\DATUM\config-fog-01.local.toml",
    [string]$Fog02Config = ".\DATUM\config-fog-02.local.toml",
    [Parameter(Mandatory = $true)]
    [string]$CloudConfig,
    [string]$ObservedSnapshot = ".\DATUM\snapshot-fog-02.json",
    [string]$ObservedModuleId = "mqtt-adapter",
    [string]$Fog01Id = "fog-01",
    [string]$Fog02Id = "fog-02",
    [string]$CloudId = "cloud-01",
    [string]$OutputDir = ".\scripts\.smoke-output\i33-observed-health-matrix",
    [switch]$KeepEvidence
)

# I33 exercises persisted observed-health translation and metadata publication only.
# It never runs a DATUM collection cycle, service checks, planner execution, recovery,
# dispatch, deployment, installation, Docker, WASM, migration, or reconfiguration.
$ErrorActionPreference = "Stop"
$script:RestoreRequired = $false
$script:RestoredBaselineConfirmed = $false

$ScriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$PdRoot = Split-Path -Parent $ScriptRoot
$DGraphApiUrl = "$($ServerUrl.TrimEnd('/'))/api/v1/snapshots"

function Resolve-RepoPath([string]$PathValue) {
    if ([System.IO.Path]::IsPathRooted($PathValue)) {
        return [System.IO.Path]::GetFullPath($PathValue)
    }
    return [System.IO.Path]::GetFullPath((Join-Path $PdRoot $PathValue))
}

function Write-Step([string]$Message) { Write-Host "`n==> $Message" }

function Get-AvailableJsonFields($Json) {
    if ($null -eq $Json) { return "response was null" }
    $fields = @($Json.PSObject.Properties | ForEach-Object { $_.Name })
    if ($fields.Count -gt 0) { return "available fields: " + ($fields -join ", ") }
    try {
        return "JSON: " + ($Json | ConvertTo-Json -Depth 10 -Compress)
    } catch {
        return "response type: $($Json.GetType().FullName)"
    }
}

function Get-Field($Json, [string]$Path, [string]$Step) {
    $value = $Json
    foreach ($part in $Path.Split('.')) {
        if ($null -eq $value) { throw "${Step}: missing JSON field '$Path' ($((Get-AvailableJsonFields $value)))." }
        $property = $value.PSObject.Properties[$part]
        if ($null -eq $property) { throw "${Step}: missing JSON field '$Path' ($((Get-AvailableJsonFields $value)))." }
        $value = $property.Value
    }
    return $value
}

function Assert-Equal($Actual, $Expected, [string]$Step, [string]$Name) {
    if ($Actual -ne $Expected) {
        throw "${Step}: '$Name' was '$Actual'; expected '$Expected'."
    }
}

function Assert-False($Json, [string]$Path, [string]$Step) {
    Assert-Equal (Get-Field $Json $Path $Step) $false $Step $Path
}

function Assert-Number($Json, [string]$Path, [int]$Expected, [string]$Step) {
    Assert-Equal ([int](Get-Field $Json $Path $Step)) $Expected $Step $Path
}

function Assert-CollectionCount($Json, [string]$Path, [int]$Expected, [string]$Step) {
    Assert-Equal (@(Get-Field $Json $Path $Step).Count) $Expected $Step $Path
}

function Invoke-JsonGet([string]$Url, [string]$Step) {
    Write-Host "[$Step] GET $Url"
    try {
        $response = Invoke-WebRequest -Uri $Url -Headers @{ "X-Datum-Project" = $ProjectId } -Method Get -SkipHttpErrorCheck
    } catch {
        throw "${Step}: could not reach '$Url'. $($_.Exception.Message)"
    }
    if ([int]$response.StatusCode -ne 200) {
        throw "${Step}: expected HTTP 200; got $($response.StatusCode).`n$($response.Content)"
    }
    try { return $response.Content | ConvertFrom-Json -Depth 100 }
    catch { throw "${Step}: response was not JSON.`n$($response.Content)" }
}

function Invoke-LocalDatum([string[]]$Arguments, [string]$Step) {
    Write-Host "[$Step] cargo $($Arguments -join ' ')"
    Push-Location $ResolvedDatumDir
    try {
        $output = & cargo @Arguments 2>&1 | Out-String
        if ($LASTEXITCODE -ne 0) {
            throw "${Step}: cargo exited with $LASTEXITCODE.`n$($output.Trim())"
        }
        return $output
    } finally { Pop-Location }
}

function Invoke-LocalDatumJson([string[]]$Arguments, [string]$Step) {
    Write-Host "[$Step] cargo $($Arguments -join ' ')"
    $stderrPath = [System.IO.Path]::GetTempFileName()
    Push-Location $ResolvedDatumDir
    try {
        $stdout = & cargo @Arguments 2> $stderrPath | Out-String
        $exitCode = $LASTEXITCODE
        $stderr = if (Test-Path -LiteralPath $stderrPath) {
            Get-Content -LiteralPath $stderrPath -Raw
        } else { "" }
        if ($exitCode -ne 0) {
            throw "${Step}: cargo exited with $exitCode.`nstdout:`n$($stdout.Trim())`nstderr:`n$($stderr.Trim())"
        }
        try { return $stdout | ConvertFrom-Json -Depth 100 }
        catch {
            throw "${Step}: expected JSON from DATUM. $($_.Exception.Message)`nstdout:`n$($stdout.Trim())`nstderr:`n$($stderr.Trim())"
        }
    } finally {
        Pop-Location
        Remove-Item -LiteralPath $stderrPath -Force -ErrorAction SilentlyContinue
    }
}

function Invoke-LocalDatumPublication([string[]]$Arguments, [string]$Step) {
    $publicationArguments = @($Arguments + @("--url", $DGraphApiUrl, "--project-id", $ProjectId))
    return Invoke-LocalDatum $publicationArguments $Step
}

function Publish-Fixture([string]$Config, [string]$Scenario, [string]$Step) {
    $args = @("run", "--quiet", "--", "dgraph-snapshot", "--config", $Config, "--fixture", $Scenario, "--publish")
    $output = Invoke-LocalDatumPublication $args $Step
    if ($output -notmatch "Published local d-graph fixture snapshot") {
        throw "${Step}: fixture publication was not confirmed."
    }
    if ($output -notmatch "safe to auto execute:\s*false" -or $output -notmatch "side effects performed:\s*false") {
        throw "${Step}: required safety flags were not false."
    }
}

function Assert-Baseline([string]$Step) {
    $materialized = Invoke-JsonGet "$ServerUrl/api/v1/dgraph/materialized/summary" "$Step materialized"
    foreach ($item in @(
        @("source_snapshot_count", 3), @("summary.node_count", 3), @("summary.runtime_count", 3),
        @("summary.module_count", 3), @("summary.flow_count", 2), @("summary.relationship_count", 14),
        @("summary.healthy_targets", 0), @("summary.unknown_targets", 3), @("summary.degraded_targets", 0),
        @("summary.unavailable_targets", 0), @("summary.unhealthy_targets", 0)
    )) { Assert-Number $materialized $item[0] $item[1] "$Step materialized" }
    Assert-CollectionCount $materialized "validation_errors" 0 "$Step materialized"
    Assert-False $materialized "side_effects_performed" "$Step materialized"

    $readiness = Invoke-JsonGet "$ServerUrl/api/v1/dgraph/readiness/summary" "$Step readiness"
    Assert-Equal (Get-Field $readiness "readiness_status" "$Step readiness") "ready" "$Step readiness" "readiness_status"
    Assert-Number $readiness "warning_count" 0 "$Step readiness"
    Assert-Number $readiness "blocked_count" 0 "$Step readiness"
    Assert-Number $readiness "degraded_flow_count" 0 "$Step readiness"
    Assert-False $readiness "checks.side_effects_performed" "$Step readiness"
    Assert-False $readiness "side_effects_performed" "$Step readiness"
    Assert-False $readiness "metadata.execution_performed" "$Step readiness"
}

function New-ScenarioConfig([string]$ScenarioSnapshotPath, [string]$ScenarioConfigPath) {
    $configText = Get-Content -LiteralPath $ResolvedFog02Config -Raw
    $section = [regex]::Match(
        $configText,
        '(?ms)^\s*\[outputs\.file_json\]\s*$.*?(?=^\s*\[|\z)'
    )
    if (-not $section.Success) {
        throw "Preflight: fog-02 config has no [outputs.file_json] section."
    }
    if ($section.Value -notmatch '(?m)^\s*enabled\s*=\s*true\s*$') {
        throw "Preflight: fog-02 outputs.file_json must be enabled."
    }
    $pathMatches = [regex]::Matches($section.Value, '(?m)^\s*path\s*=.*$')
    if ($pathMatches.Count -ne 1) {
        throw "Preflight: expected exactly one outputs.file_json path in fog-02 config."
    }

    $tomlPath = ([System.IO.Path]::GetFullPath($ScenarioSnapshotPath) -replace '\\', '/')
    $newSection = $section.Value.Remove($pathMatches[0].Index, $pathMatches[0].Length).Insert(
        $pathMatches[0].Index,
        "path = `"$tomlPath`""
    )
    $scenarioConfig = $configText.Remove($section.Index, $section.Length).Insert($section.Index, $newSection)
    Set-Content -LiteralPath $ScenarioConfigPath -Value $scenarioConfig -Encoding utf8NoBOM
}

function New-ScenarioSnapshot([pscustomobject]$Scenario, [string]$ScenarioSnapshotPath) {
    $snapshot = (Get-Content -LiteralPath $ResolvedObservedSnapshot -Raw | ConvertFrom-Json -Depth 100)
    $now = [DateTimeOffset]::UtcNow
    $snapshot.meta.cycle_id = "i33-$($Scenario.Name)-$([guid]::NewGuid().ToString('N'))"
    $snapshot.meta.observed_at_utc = $now.ToString("o")

    $matching = @($snapshot.entities | Where-Object {
        $_.kind -ieq "service" -and $_.attrs.name -ieq $ObservedModuleId
    })
    if ($matching.Count -lt 1) {
        throw "Preflight: persisted snapshot has no service entity named '$ObservedModuleId'."
    }

    if ($Scenario.Name -eq "absent") {
        $snapshot.entities = @($snapshot.entities | Where-Object {
            -not ($_.kind -ieq "service" -and $_.attrs.name -ieq $ObservedModuleId)
        })
    } else {
        $observedAt = switch ($Scenario.Name) {
            "stale" { $now.AddSeconds(-120) }
            "future-skewed" { $now.AddSeconds(10) }
            default { $now }
        }
        foreach ($entity in $matching) {
            $entity.attrs | Add-Member -NotePropertyName "status" -NotePropertyValue $Scenario.RawStatus -Force
            $entity.observed_at_utc = $observedAt.ToString("o")
            $entity.ttl_s = 60
        }
    }

    $snapshot | ConvertTo-Json -Depth 100 | Set-Content -LiteralPath $ScenarioSnapshotPath -Encoding utf8NoBOM
}

function Get-ObservedHealthEntry($Snapshot, [string]$Step) {
    $health = @($Snapshot.health | Where-Object {
        $_.target_kind -ieq "module" -and $_.target_id -ieq $ObservedModuleId
    })
    if ($health.Count -ne 1) {
        throw "${Step}: expected one health entry for module '$ObservedModuleId'; found $($health.Count)."
    }
    return $health[0]
}

function Assert-ScenarioHealth($Snapshot, [pscustomobject]$Scenario, [string]$Step) {
    $health = Get-ObservedHealthEntry $Snapshot $Step
    Assert-Equal $health.status $Scenario.ExpectedStatus $Step "health.status"
    Assert-Equal $health.source "persisted_local_snapshot" $Step "health.source"
    Assert-False $health "metadata.checks_executed" $Step
    Assert-Equal (Get-Field $health "metadata.reporting_only" $Step) $true $Step "metadata.reporting_only"
    Assert-Equal (Get-Field $health "metadata.observation_present" $Step) $Scenario.ObservationPresent $Step "metadata.observation_present"
    Assert-Equal (Get-Field $health "metadata.observation_fresh" $Step) $Scenario.ObservationFresh $Step "metadata.observation_fresh"
    if ($health.reason -notmatch $Scenario.ReasonPattern) {
        throw "${Step}: health.reason '$($health.reason)' did not match '$($Scenario.ReasonPattern)'."
    }

    if ($Scenario.ObservationPresent) {
        Assert-Equal (Get-Field $health "metadata.raw_status" $Step) $Scenario.RawStatus $Step "metadata.raw_status"
        Assert-Equal (Get-Field $health "metadata.clock_skew_detected" $Step) $Scenario.ClockSkewDetected $Step "metadata.clock_skew_detected"
    }
}

function Assert-PublishedScenario([pscustomobject]$Scenario, [string]$ScenarioConfigPath) {
    $step = "I33 $($Scenario.Name) publish"
    $published = Invoke-LocalDatumJson @(
        "run", "--quiet", "--", "dgraph-snapshot", "--config", $ScenarioConfigPath,
        "--observed-health", "--publish", "--json", "--url", $DGraphApiUrl,
        "--project-id", $ProjectId
    ) $step
    Assert-Equal (Get-Field $published "published" $step) $true $step "published"
    Assert-False $published "side_effects_performed" $step
    $localSnapshot = Get-Field $published "local_snapshot" $step
    Assert-ScenarioHealth $localSnapshot $Scenario $step
    Assert-False $localSnapshot "safe_to_auto_execute" $step
    Assert-False $localSnapshot "side_effects_performed" $step

    $materialized = Invoke-JsonGet "$ServerUrl/api/v1/dgraph/materialized/summary" "I33 $($Scenario.Name) materialized"
    Assert-Number $materialized "source_snapshot_count" 3 "I33 $($Scenario.Name) materialized"
    Assert-Number $materialized "summary.node_count" 3 "I33 $($Scenario.Name) materialized"
    Assert-CollectionCount $materialized "validation_errors" 0 "I33 $($Scenario.Name) materialized"
    Assert-Number $materialized "summary.healthy_targets" $Scenario.HealthyTargets "I33 $($Scenario.Name) materialized"
    Assert-Number $materialized "summary.unknown_targets" $Scenario.UnknownTargets "I33 $($Scenario.Name) materialized"
    Assert-Number $materialized "summary.degraded_targets" $Scenario.DegradedTargets "I33 $($Scenario.Name) materialized"
    Assert-Number $materialized "summary.unavailable_targets" 0 "I33 $($Scenario.Name) materialized"
    Assert-Number $materialized "summary.unhealthy_targets" $Scenario.UnhealthyTargets "I33 $($Scenario.Name) materialized"
    Assert-False $materialized "side_effects_performed" "I33 $($Scenario.Name) materialized"

    $readiness = Invoke-JsonGet "$ServerUrl/api/v1/dgraph/readiness/summary" "I33 $($Scenario.Name) readiness"
    Assert-False $readiness "checks.side_effects_performed" "I33 $($Scenario.Name) readiness"
    Assert-False $readiness "side_effects_performed" "I33 $($Scenario.Name) readiness"
    Assert-False $readiness "metadata.execution_performed" "I33 $($Scenario.Name) readiness"
}

function Restore-Fog02Baseline([string]$Step) {
    Publish-Fixture $ResolvedFog02Config "baseline" "$Step publish"
    Assert-Baseline $Step
    $script:RestoredBaselineConfirmed = $true
}

$ResolvedDatumDir = Resolve-RepoPath $DatumDir
$ResolvedFog01Config = Resolve-RepoPath $Fog01Config
$ResolvedFog02Config = Resolve-RepoPath $Fog02Config
$ResolvedCloudConfig = Resolve-RepoPath $CloudConfig
$ResolvedObservedSnapshot = Resolve-RepoPath $ObservedSnapshot
$ResolvedOutputDir = Resolve-RepoPath $OutputDir

Write-Step "I33 observed-health scenario matrix preflight"
if ($PSVersionTable.PSVersion.Major -lt 7) {
    throw "Preflight: PowerShell 7 or later is required; run this script with pwsh."
}
foreach ($path in @(
    $ResolvedDatumDir, (Join-Path $ResolvedDatumDir "Cargo.toml"), $ResolvedFog01Config,
    $ResolvedFog02Config, $ResolvedCloudConfig, $ResolvedObservedSnapshot
)) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Preflight: required local path does not exist: $path"
    }
}
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "Preflight: cargo is not available on this Windows host."
}
foreach ($config in @(
    @($ResolvedFog01Config, $Fog01Id), @($ResolvedFog02Config, $Fog02Id),
    @($ResolvedCloudConfig, $CloudId)
)) {
    $configText = Get-Content -LiteralPath $config[0] -Raw
    if ($configText -notmatch ('(?m)^\s*node_id\s*=\s*"' + [regex]::Escape($config[1]) + '"\s*$')) {
        throw "Preflight: local config '$($config[0])' must declare node_id '$($config[1])'."
    }
}
$null = Invoke-JsonGet "$ServerUrl/health" "Server health"

$originalHashes = @{
    Fog02Config = (Get-FileHash -LiteralPath $ResolvedFog02Config -Algorithm SHA256).Hash
    ObservedSnapshot = (Get-FileHash -LiteralPath $ResolvedObservedSnapshot -Algorithm SHA256).Hash
}
New-Item -ItemType Directory -Path $ResolvedOutputDir -Force | Out-Null

$scenarios = @(
    [pscustomobject]@{ Name = "healthy"; RawStatus = "available"; ExpectedStatus = "healthy"; ReasonPattern = "derived from a fresh persisted service observation"; ObservationPresent = $true; ObservationFresh = $true; ClockSkewDetected = $false; HealthyTargets = 1; UnknownTargets = 2; DegradedTargets = 0; UnhealthyTargets = 0 },
    [pscustomobject]@{ Name = "degraded"; RawStatus = "warning"; ExpectedStatus = "degraded"; ReasonPattern = "derived from a fresh persisted service observation"; ObservationPresent = $true; ObservationFresh = $true; ClockSkewDetected = $false; HealthyTargets = 0; UnknownTargets = 2; DegradedTargets = 1; UnhealthyTargets = 1 },
    [pscustomobject]@{ Name = "stale"; RawStatus = "unavailable"; ExpectedStatus = "unknown"; ReasonPattern = "exceeded its ttl_s"; ObservationPresent = $true; ObservationFresh = $false; ClockSkewDetected = $false; HealthyTargets = 0; UnknownTargets = 3; DegradedTargets = 0; UnhealthyTargets = 0 },
    [pscustomobject]@{ Name = "absent"; RawStatus = $null; ExpectedStatus = "unknown"; ReasonPattern = "no matching service observation"; ObservationPresent = $false; ObservationFresh = $false; ClockSkewDetected = $false; HealthyTargets = 0; UnknownTargets = 3; DegradedTargets = 0; UnhealthyTargets = 0 },
    [pscustomobject]@{ Name = "unrecognized"; RawStatus = "mystery-state"; ExpectedStatus = "unknown"; ReasonPattern = "did not report a recognized explicit health state"; ObservationPresent = $true; ObservationFresh = $true; ClockSkewDetected = $false; HealthyTargets = 0; UnknownTargets = 3; DegradedTargets = 0; UnhealthyTargets = 0 },
    [pscustomobject]@{ Name = "future-skewed"; RawStatus = "available"; ExpectedStatus = "unknown"; ReasonPattern = "in the future beyond allowed clock skew"; ObservationPresent = $true; ObservationFresh = $false; ClockSkewDetected = $true; HealthyTargets = 0; UnknownTargets = 3; DegradedTargets = 0; UnhealthyTargets = 0 }
)

try {
    Write-Step "Publish initial three-node baseline"
    Publish-Fixture $ResolvedFog01Config "baseline" "Publish fog-01 baseline"
    $script:RestoreRequired = $true
    Publish-Fixture $ResolvedFog02Config "baseline" "Publish fog-02 baseline"
    Publish-Fixture $ResolvedCloudConfig "baseline" "Publish cloud-01 baseline"
    Assert-Baseline "Initial baseline"
    $script:RestoredBaselineConfirmed = $true

    foreach ($scenario in $scenarios) {
        $script:RestoredBaselineConfirmed = $false
        $scenarioSnapshotPath = Join-Path $ResolvedOutputDir "$($scenario.Name)-snapshot.json"
        $scenarioConfigPath = Join-Path $ResolvedOutputDir "$($scenario.Name)-fog-02-config.toml"

        try {
            Write-Step "Run I33 scenario: $($scenario.Name)"
            New-ScenarioSnapshot $scenario $scenarioSnapshotPath
            New-ScenarioConfig $scenarioSnapshotPath $scenarioConfigPath

            $local = Invoke-LocalDatumJson @(
                "run", "--quiet", "--", "dgraph-snapshot", "--config", $scenarioConfigPath,
                "--observed-health", "--json"
            ) "I33 $($scenario.Name) local read"
            Assert-ScenarioHealth $local $scenario "I33 $($scenario.Name) local read"
            Assert-False $local "safe_to_auto_execute" "I33 $($scenario.Name) local read"
            Assert-False $local "side_effects_performed" "I33 $($scenario.Name) local read"

            Assert-PublishedScenario $scenario $scenarioConfigPath
        } finally {
            Restore-Fog02Baseline "I33 $($scenario.Name) restored baseline"
        }
    }
} finally {
    if ($script:RestoreRequired -and -not $script:RestoredBaselineConfirmed) {
        Restore-Fog02Baseline "I33 final emergency restoration"
    }

    $finalConfigHash = (Get-FileHash -LiteralPath $ResolvedFog02Config -Algorithm SHA256).Hash
    $finalSnapshotHash = (Get-FileHash -LiteralPath $ResolvedObservedSnapshot -Algorithm SHA256).Hash
    Assert-Equal $finalConfigHash $originalHashes.Fog02Config "I33 source preservation" "fog-02 config SHA256"
    Assert-Equal $finalSnapshotHash $originalHashes.ObservedSnapshot "I33 source preservation" "persisted snapshot SHA256"

    if (-not $KeepEvidence -and (Test-Path -LiteralPath $ResolvedOutputDir)) {
        Remove-Item -LiteralPath $ResolvedOutputDir -Recurse -Force
    }
}

Write-Step "I33 observed-health scenario matrix completed"
Write-Host "Confirmed scenarios: healthy, degraded, stale, absent, unrecognized, future-skewed."
Write-Host "Confirmed after every scenario: three-node baseline restored."
Write-Host "Confirmed: source config and persisted snapshot hashes unchanged."
Write-Host "Confirmed: checks_executed=false, safe_to_auto_execute=false, side_effects_performed=false."
Write-Host "No collection cycle, service check, placement, recovery, dispatch, deployment, installation, Docker, WASM, migration, or reconfiguration was invoked."
