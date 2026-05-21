param(
    [string]$ServerUrl = "http://localhost:8080",
    [string]$ProjectId = "smart-city-campinas",
    [string]$DatumDir = ".\DATUM",
    [string]$Node1Id = "fog-01",
    [string]$Node1Config = ".\DATUM\config-fog-01.toml",
    [string]$Node2Id = "fog-02",
    [string]$Node2Config = ".\DATUM\config-fog-02.toml",
    [switch]$SkipPublish,
    [switch]$SkipDatumCommands,
    [switch]$AllowMissingConfigs
)

$ErrorActionPreference = "Stop"

$ScriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$PdRoot = Split-Path -Parent $ScriptRoot

function Resolve-RepoPath {
    param([string]$PathValue)

    if ([System.IO.Path]::IsPathRooted($PathValue)) {
        return [System.IO.Path]::GetFullPath($PathValue)
    }

    return [System.IO.Path]::GetFullPath((Join-Path $PdRoot $PathValue))
}

function Write-Step {
    param([string]$Message)

    Write-Host ""
    Write-Host "==> $Message"
}

function Get-JsonPropertyValue {
    param(
        $Json,
        [string]$PropertyPath,
        [string]$StepName
    )

    $current = $Json
    foreach ($segment in $PropertyPath.Split('.')) {
        if ($null -eq $current) {
            throw "Step '$StepName' expected JSON property '$PropertyPath', but '$segment' was missing."
        }

        if ($current -is [System.Collections.IDictionary]) {
            if (-not $current.Contains($segment)) {
                throw "Step '$StepName' expected JSON property '$PropertyPath', but '$segment' was missing."
            }
            $current = $current[$segment]
            continue
        }

        $property = $current.PSObject.Properties[$segment]
        if ($null -eq $property) {
            throw "Step '$StepName' expected JSON property '$PropertyPath', but '$segment' was missing."
        }
        $current = $property.Value
    }

    return $current
}

function Assert-JsonField {
    param(
        $Json,
        [string]$PropertyPath,
        [object]$ExpectedValue,
        [string]$StepName
    )

    $value = Get-JsonPropertyValue -Json $Json -PropertyPath $PropertyPath -StepName $StepName
    if ($PSBoundParameters.ContainsKey("ExpectedValue") -and $value -ne $ExpectedValue) {
        throw "Step '$StepName' returned '$PropertyPath'='$value', expected '$ExpectedValue'."
    }

    return $value
}

function Assert-FalseFlag {
    param(
        $Json,
        [string]$PropertyPath,
        [string]$StepName
    )

    $value = Get-JsonPropertyValue -Json $Json -PropertyPath $PropertyPath -StepName $StepName
    if ($value -ne $false) {
        throw "Step '$StepName' returned '$PropertyPath'='$value', expected 'False'."
    }
}

function Assert-ArrayField {
    param(
        $Json,
        [string]$PropertyPath,
        [string]$StepName
    )

    $value = Get-JsonPropertyValue -Json $Json -PropertyPath $PropertyPath -StepName $StepName
    if ($value -is [string] -or (-not ($value -is [System.Collections.IEnumerable]))) {
        throw "Step '$StepName' expected '$PropertyPath' to be an array."
    }

    return @($value)
}

function Invoke-JsonGet {
    param(
        [string]$Url,
        [hashtable]$Headers,
        [string]$StepName
    )

    Write-Host "[$StepName] GET $Url"
    try {
        $response = Invoke-WebRequest -Uri $Url -Headers $Headers -Method Get -SkipHttpErrorCheck
    }
    catch {
        throw "Step '$StepName' failed to reach '$Url'. $($_.Exception.Message)"
    }

    $json = $null
    if ($response.Content) {
        try {
            $json = $response.Content | ConvertFrom-Json -Depth 100
        }
        catch {
            throw "Step '$StepName' returned non-JSON content.`n$($response.Content)"
        }
    }

    [pscustomobject]@{
        StatusCode = [int]$response.StatusCode
        Body = $response.Content
        Json = $json
    }
}

function Assert-HttpOk {
    param(
        $Response,
        [string]$StepName
    )

    if ($Response.StatusCode -ne 200) {
        throw "Step '$StepName' expected HTTP 200, got $($Response.StatusCode).`n$($Response.Body)"
    }
}

function Invoke-DatumCommand {
    param(
        [string[]]$Arguments,
        [string]$StepName
    )

    Write-Host "[$StepName] cargo $($Arguments -join ' ')"
    Push-Location $ResolvedDatumDir
    $previousNativeErrorPreference = $PSNativeCommandUseErrorActionPreference
    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $PSNativeCommandUseErrorActionPreference = $false
        $ErrorActionPreference = "Continue"
        $output = & cargo @Arguments 2>&1 | Out-String
        $exitCode = $LASTEXITCODE
    }
    finally {
        $PSNativeCommandUseErrorActionPreference = $previousNativeErrorPreference
        $ErrorActionPreference = $previousErrorActionPreference
        Pop-Location
    }

    if ($exitCode -ne 0) {
        throw "Step '$StepName' failed with exit code $exitCode.`n$($output.Trim())"
    }

    return $output
}

function Assert-OutputContains {
    param(
        [string]$Output,
        [string]$Pattern,
        [string]$StepName
    )

    if ($Output -notmatch $Pattern) {
        throw "Step '$StepName' did not include expected text matching '$Pattern'."
    }
}

$ResolvedDatumDir = Resolve-RepoPath -PathValue $DatumDir
$ResolvedNode1Config = Resolve-RepoPath -PathValue $Node1Config
$ResolvedNode2Config = Resolve-RepoPath -PathValue $Node2Config
$Headers = @{ "X-Datum-Project" = $ProjectId }

$node1SnapshotChecked = $false
$node2SnapshotChecked = $false
$projectLatestSnapshotsChecked = $false
$projectMaterializedSummaryChecked = $false
$projectPlannerSummaryChecked = $false
$sideEffectsChecked = $false

Write-Step "2-VM metadata-only smoke draft parameters"
Write-Host "ServerUrl: $ServerUrl"
Write-Host "ProjectId: $ProjectId"
Write-Host "DatumDir: $ResolvedDatumDir"
Write-Host "Node1Id: $Node1Id"
Write-Host "Node1Config: $ResolvedNode1Config"
Write-Host "Node2Id: $Node2Id"
Write-Host "Node2Config: $ResolvedNode2Config"
Write-Host "SkipPublish: $SkipPublish"
Write-Host "SkipDatumCommands: $SkipDatumCommands"
Write-Host "AllowMissingConfigs: $AllowMissingConfigs"

if (-not (Test-Path -LiteralPath $ResolvedDatumDir -PathType Container)) {
    throw "DatumDir '$ResolvedDatumDir' does not exist."
}

$missingConfigs = @()
if (-not (Test-Path -LiteralPath $ResolvedNode1Config -PathType Leaf)) {
    $missingConfigs += $ResolvedNode1Config
}
if (-not (Test-Path -LiteralPath $ResolvedNode2Config -PathType Leaf)) {
    $missingConfigs += $ResolvedNode2Config
}

if ($missingConfigs.Count -gt 0) {
    if ($AllowMissingConfigs) {
        Write-Warning "One or more 2-VM DATUM config files are missing:"
        foreach ($path in $missingConfigs) {
            Write-Warning "  - $path"
        }
        Write-Host ""
        Write-Host "2-VM metadata-only smoke draft preflight exited in documentation mode."
        Write-Host "No DATUM commands or server checks were executed."
        Write-Host "This is expected when validating the draft script without live VM configs."
        exit 0
    }

    throw "Missing DATUM config file(s): $($missingConfigs -join ', '). Use -AllowMissingConfigs for documentation/preflight mode."
}

Write-Step "Server preflight"
$health = Invoke-JsonGet -Url "$ServerUrl/health" -Headers $Headers -StepName "Server health"
Assert-HttpOk -Response $health -StepName "Server health"

if (-not $SkipDatumCommands -and -not $SkipPublish) {
    Write-Step "Publish node 1 d-graph snapshot"
    $publishNode1 = Invoke-DatumCommand `
        -Arguments @("run", "--", "dgraph-snapshot", "--config", $ResolvedNode1Config, "--publish") `
        -StepName "DATUM node 1 publish"
    Assert-OutputContains -Output $publishNode1 -Pattern "Published local d-graph snapshot" -StepName "DATUM node 1 publish"
    Assert-OutputContains -Output $publishNode1 -Pattern "side effects performed:\s*false" -StepName "DATUM node 1 publish"

    Write-Step "Publish node 2 d-graph snapshot"
    $publishNode2 = Invoke-DatumCommand `
        -Arguments @("run", "--", "dgraph-snapshot", "--config", $ResolvedNode2Config, "--publish") `
        -StepName "DATUM node 2 publish"
    Assert-OutputContains -Output $publishNode2 -Pattern "Published local d-graph snapshot" -StepName "DATUM node 2 publish"
    Assert-OutputContains -Output $publishNode2 -Pattern "side effects performed:\s*false" -StepName "DATUM node 2 publish"
}
elseif ($SkipPublish) {
    Write-Step "Publish skipped"
    Write-Warning "SkipPublish was set. The script will continue with read-back checks only."
}
elseif ($SkipDatumCommands) {
    Write-Step "DATUM commands skipped"
    Write-Warning "SkipDatumCommands was set. Publish and DATUM viewer checks will be skipped."
}

Write-Step "Server node latest read-back for node 1"
$node1Latest = Invoke-JsonGet `
    -Url "$ServerUrl/api/v1/nodes/$Node1Id/dgraph-snapshot/latest" `
    -Headers $Headers `
    -StepName "Server node 1 latest d-graph snapshot"
Assert-HttpOk -Response $node1Latest -StepName "Server node 1 latest d-graph snapshot"
Assert-JsonField -Json $node1Latest.Json -PropertyPath "node_id" -ExpectedValue $Node1Id -StepName "Server node 1 latest d-graph snapshot" | Out-Null
Assert-JsonField -Json $node1Latest.Json -PropertyPath "project_id" -ExpectedValue $ProjectId -StepName "Server node 1 latest d-graph snapshot" | Out-Null
Assert-FalseFlag -Json $node1Latest.Json -PropertyPath "safe_to_auto_execute" -StepName "Server node 1 latest d-graph snapshot"
Assert-FalseFlag -Json $node1Latest.Json -PropertyPath "side_effects_performed" -StepName "Server node 1 latest d-graph snapshot"
$node1SnapshotChecked = $true
$sideEffectsChecked = $true

Write-Step "Server node latest read-back for node 2"
$node2Latest = Invoke-JsonGet `
    -Url "$ServerUrl/api/v1/nodes/$Node2Id/dgraph-snapshot/latest" `
    -Headers $Headers `
    -StepName "Server node 2 latest d-graph snapshot"
Assert-HttpOk -Response $node2Latest -StepName "Server node 2 latest d-graph snapshot"
Assert-JsonField -Json $node2Latest.Json -PropertyPath "node_id" -ExpectedValue $Node2Id -StepName "Server node 2 latest d-graph snapshot" | Out-Null
Assert-JsonField -Json $node2Latest.Json -PropertyPath "project_id" -ExpectedValue $ProjectId -StepName "Server node 2 latest d-graph snapshot" | Out-Null
Assert-FalseFlag -Json $node2Latest.Json -PropertyPath "safe_to_auto_execute" -StepName "Server node 2 latest d-graph snapshot"
Assert-FalseFlag -Json $node2Latest.Json -PropertyPath "side_effects_performed" -StepName "Server node 2 latest d-graph snapshot"
$node2SnapshotChecked = $true
$sideEffectsChecked = $true

Write-Step "Server project latest snapshot list"
$projectLatestSnapshots = Invoke-JsonGet `
    -Url "$ServerUrl/api/v1/dgraph-snapshots/latest" `
    -Headers $Headers `
    -StepName "Server project latest d-graph snapshots"
Assert-HttpOk -Response $projectLatestSnapshots -StepName "Server project latest d-graph snapshots"
Assert-JsonField -Json $projectLatestSnapshots.Json -PropertyPath "project_id" -ExpectedValue $ProjectId -StepName "Server project latest d-graph snapshots" | Out-Null
$snapshotArray = Assert-ArrayField -Json $projectLatestSnapshots.Json -PropertyPath "snapshots" -StepName "Server project latest d-graph snapshots"
$snapshotCount = [int](Assert-JsonField -Json $projectLatestSnapshots.Json -PropertyPath "count" -StepName "Server project latest d-graph snapshots")
if (-not $SkipPublish -and -not $SkipDatumCommands -and $snapshotCount -lt 2) {
    throw "Server project latest d-graph snapshots returned count '$snapshotCount', expected at least 2 after both publishes."
}
if (($SkipPublish -or $SkipDatumCommands) -and $snapshotCount -lt 2) {
    Write-Warning "Project latest snapshot count is '$snapshotCount'. This is tolerated because publish or DATUM commands were skipped."
}
$projectLatestSnapshotsChecked = $true

Write-Step "Project materialized d-graph summary"
$materializedSummary = Invoke-JsonGet `
    -Url "$ServerUrl/api/v1/dgraph/materialized/summary" `
    -Headers $Headers `
    -StepName "Server project materialized d-graph summary"
Assert-HttpOk -Response $materializedSummary -StepName "Server project materialized d-graph summary"
Assert-JsonField -Json $materializedSummary.Json -PropertyPath "project_id" -ExpectedValue $ProjectId -StepName "Server project materialized d-graph summary" | Out-Null
$sourceSnapshotCount = [int](Assert-JsonField -Json $materializedSummary.Json -PropertyPath "source_snapshot_count" -StepName "Server project materialized d-graph summary")
Assert-JsonField -Json $materializedSummary.Json -PropertyPath "summary" -StepName "Server project materialized d-graph summary" | Out-Null
Assert-FalseFlag -Json $materializedSummary.Json -PropertyPath "side_effects_performed" -StepName "Server project materialized d-graph summary"
if (-not $SkipPublish -and -not $SkipDatumCommands -and $sourceSnapshotCount -lt 2) {
    throw "Server project materialized summary returned source_snapshot_count '$sourceSnapshotCount', expected at least 2 after both publishes."
}
if (($SkipPublish -or $SkipDatumCommands) -and $sourceSnapshotCount -lt 2) {
    Write-Warning "Materialized source_snapshot_count is '$sourceSnapshotCount'. This is tolerated because publish or DATUM commands were skipped."
}
$projectMaterializedSummaryChecked = $true
$sideEffectsChecked = $true

Write-Step "Project planner dry-run summary"
$plannerSummary = Invoke-JsonGet `
    -Url "$ServerUrl/api/v1/dgraph/planner/dry-run/summary" `
    -Headers $Headers `
    -StepName "Server project planner dry-run summary"
Assert-HttpOk -Response $plannerSummary -StepName "Server project planner dry-run summary"
Assert-JsonField -Json $plannerSummary.Json -PropertyPath "project_id" -ExpectedValue $ProjectId -StepName "Server project planner dry-run summary" | Out-Null
Assert-JsonField -Json $plannerSummary.Json -PropertyPath "summary" -StepName "Server project planner dry-run summary" | Out-Null
Assert-JsonField -Json $plannerSummary.Json -PropertyPath "status" -StepName "Server project planner dry-run summary" | Out-Null
Assert-FalseFlag -Json $plannerSummary.Json -PropertyPath "side_effects_performed" -StepName "Server project planner dry-run summary"
$projectPlannerSummaryChecked = $true
$sideEffectsChecked = $true

if (-not $SkipDatumCommands) {
    Write-Step "DATUM materialized summary viewer"
    $materializedViewer = Invoke-DatumCommand `
        -Arguments @("run", "--", "dgraph-snapshot", "--config", $ResolvedNode1Config, "--materialized-summary") `
        -StepName "DATUM materialized summary viewer"
    Assert-OutputContains -Output $materializedViewer -Pattern "Project d-graph summary" -StepName "DATUM materialized summary viewer"
    Assert-OutputContains -Output $materializedViewer -Pattern "Side effects performed:\s*false" -StepName "DATUM materialized summary viewer"

    Write-Step "DATUM planner summary viewer"
    $plannerViewer = Invoke-DatumCommand `
        -Arguments @("run", "--", "dgraph-snapshot", "--config", $ResolvedNode1Config, "--planner-summary") `
        -StepName "DATUM planner summary viewer"
    Assert-OutputContains -Output $plannerViewer -Pattern "D-graph planner dry-run summary" -StepName "DATUM planner summary viewer"
    Assert-OutputContains -Output $plannerViewer -Pattern "Side effects performed:\s*false" -StepName "DATUM planner summary viewer"
    Assert-OutputContains -Output $plannerViewer -Pattern "did not deploy modules|did not execute installation|did not .*reconfigure flows" -StepName "DATUM planner summary viewer"
}

Write-Step "2-VM metadata-only smoke draft summary"
Write-Host "- 2-VM metadata-only smoke draft completed"
if ($node1SnapshotChecked) {
    Write-Host "- node 1 snapshot path checked"
}
if ($node2SnapshotChecked) {
    Write-Host "- node 2 snapshot path checked"
}
if ($projectLatestSnapshotsChecked) {
    Write-Host "- project latest snapshot list checked"
}
if ($projectMaterializedSummaryChecked) {
    Write-Host "- project materialized summary checked"
}
if ($projectPlannerSummaryChecked) {
    Write-Host "- planner dry-run summary checked"
}
if ($sideEffectsChecked) {
    Write-Host "- side_effects_performed=false checked"
}
Write-Host ""
Write-Host "No dispatch execution was performed."
Write-Host "No deployment or installation execution was performed."
Write-Host "No Docker/container/WASM-host/migration side effects were performed."
Write-Host "D-graph snapshots were metadata-only."
Write-Host "D-graph materialization was read-only."
Write-Host "D-graph planner dry-run was advisory/read-only."
Write-Host "D-graph planner decisions were not executed."
Write-Host "No local config or mapping files were modified by this script."
Write-Host "No flow reconfiguration was performed."
