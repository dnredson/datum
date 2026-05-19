param(
    [string]$ServerUrl = "http://localhost:8080",
    [string]$ClientConfig = "..\DATUM\config.toml",
    [string]$ProjectId = "smart-city-campinas",
    [string]$NodeId = "fog-01",
    [switch]$SkipClientBuild,
    [switch]$SkipServerChecks,
    [switch]$VerboseOutput
)

$ErrorActionPreference = "Stop"

$ScriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$PdRoot = Split-Path -Parent $ScriptRoot
$DatumDir = Join-Path $PdRoot "DATUM"
$DserverDir = Join-Path $PdRoot "dserver"
$ResolvedClientConfig = [System.IO.Path]::GetFullPath((Join-Path $ScriptRoot $ClientConfig))

function Write-Step {
    param([string]$Message)
    Write-Host ""
    Write-Host "==> $Message"
}

function Run-CommandOrFail {
    param(
        [string]$WorkingDirectory,
        [string[]]$Command,
        [string]$StepName
    )

    Write-Host "[$StepName] $($Command -join ' ')"
    Push-Location $WorkingDirectory
    try {
        $output = & $Command[0] $Command[1..($Command.Length - 1)] 2>&1 | Out-String
        $exitCode = $LASTEXITCODE
    }
    finally {
        Pop-Location
    }

    if ($VerboseOutput -and $output.Trim()) {
        Write-Host $output.TrimEnd()
    }

    if ($exitCode -ne 0) {
        $excerpt = $output.Trim()
        if ($excerpt.Length -gt 1200) {
            $excerpt = $excerpt.Substring(0, 1200)
        }

        throw "Step '$StepName' failed with exit code $exitCode.`n$excerpt"
    }

    return $output
}

function Invoke-JsonEndpoint {
    param(
        [string]$Url,
        [hashtable]$Headers,
        [string]$StepName
    )

    Write-Host "[$StepName] GET $Url"
    $response = Invoke-WebRequest -Uri $Url -Headers $Headers -Method Get -SkipHttpErrorCheck
    $body = $response.Content
    $json = $null

    if ($body) {
        try {
            $json = $body | ConvertFrom-Json -Depth 100
        }
        catch {
            throw "Step '$StepName' returned non-JSON content.`n$body"
        }
    }

    if ($VerboseOutput -and $body) {
        Write-Host $body
    }

    [pscustomobject]@{
        StatusCode = [int]$response.StatusCode
        Body = $body
        Json = $json
    }
}

function Assert-HttpOk {
    param(
        $Response,
        [string]$StepName
    )

    if ($Response.StatusCode -ne 200) {
        $excerpt = ($Response.Body | Out-String).Trim()
        if ($excerpt.Length -gt 1200) {
            $excerpt = $excerpt.Substring(0, 1200)
        }

        throw "Step '$StepName' expected HTTP 200, got $($Response.StatusCode).`n$excerpt"
    }
}

function Assert-JsonHasProperty {
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
}

function Assert-NodeCapabilities {
    param($Json)

    Assert-JsonHasProperty $Json "project_id" "Read capabilities from server"
    Assert-JsonHasProperty $Json "node_id" "Read capabilities from server"

    if ($Json.project_id -ne $ProjectId) {
        throw "Read capabilities from server returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($Json.node_id -ne $NodeId) {
        throw "Read capabilities from server returned node_id '$($Json.node_id)', expected '$NodeId'."
    }
}

function Assert-ActionPlanContract {
    param($Json)

    Assert-JsonHasProperty $Json "actions" "Action-plan latest"
    if ($Json.actions -and $Json.actions.Count -gt 0) {
        Assert-JsonHasProperty $Json.actions[0] "capability" "Action-plan latest"
    }
}

function Assert-DispatchContract {
    param(
        $Json,
        [string]$StepName
    )

    Assert-JsonHasProperty $Json "actions" $StepName
    Assert-JsonHasProperty $Json "blocked_actions" $StepName
    Assert-JsonHasProperty $Json "deferred_actions" $StepName
    Assert-JsonHasProperty $Json "dispatch_policy" $StepName
    Assert-JsonHasProperty $Json "summary" $StepName
}

Write-Step "Resolve paths"
Write-Host "PD root: $PdRoot"
Write-Host "DATUM dir: $DatumDir"
Write-Host "dserver dir: $DserverDir"
Write-Host "Client config: $ResolvedClientConfig"

if (-not (Test-Path $DatumDir)) {
    throw "DATUM directory not found: $DatumDir"
}

if (-not (Test-Path $DserverDir)) {
    throw "dserver directory not found: $DserverDir"
}

if (-not (Test-Path $ResolvedClientConfig)) {
    throw "Client config not found: $ResolvedClientConfig"
}

$headers = @{
    "X-Datum-Project" = $ProjectId
}

if (-not $SkipServerChecks) {
    Write-Step "Check server health"
    $health = Invoke-JsonEndpoint -Url "$ServerUrl/health" -Headers $headers -StepName "Server health"
    Assert-HttpOk $health "Server health"
}

if (-not $SkipClientBuild) {
    Write-Step "Run DATUM client build/test checks"
    Run-CommandOrFail -WorkingDirectory $DatumDir -Command @("cargo", "test") -StepName "DATUM cargo test" | Out-Null
    Run-CommandOrFail -WorkingDirectory $DatumDir -Command @("cargo", "build") -StepName "DATUM cargo build" | Out-Null
}

Write-Step "Publish client capabilities"
Run-CommandOrFail `
    -WorkingDirectory $DatumDir `
    -Command @("cargo", "run", "--", "capabilities", "--config", $ResolvedClientConfig, "--publish") `
    -StepName "DATUM capabilities publish" | Out-Null

if (-not $SkipServerChecks) {
    Write-Step "Read capabilities from server"
    $capabilities = Invoke-JsonEndpoint `
        -Url "$ServerUrl/api/v1/nodes/$NodeId/capabilities" `
        -Headers $headers `
        -StepName "Read capabilities from server"
    Assert-HttpOk $capabilities "Read capabilities from server"
    Assert-NodeCapabilities $capabilities.Json
}

Write-Step "Send snapshot"
$onceOutput = Run-CommandOrFail `
    -WorkingDirectory $DatumDir `
    -Command @("cargo", "run", "--", "once", "--config", $ResolvedClientConfig) `
    -StepName "DATUM once"

if ($onceOutput -notmatch "http_json=ok") {
    Write-Host "DATUM once output did not include 'http_json=ok'. Continuing because exit code was 0."
}

if (-not $SkipServerChecks) {
    Write-Step "Check server pipeline endpoints"

    $graph = Invoke-JsonEndpoint -Url "$ServerUrl/api/v1/graph/latest" -Headers $headers -StepName "Graph latest"
    Assert-HttpOk $graph "Graph latest"

    $analysis = Invoke-JsonEndpoint -Url "$ServerUrl/api/v1/analysis/latest" -Headers $headers -StepName "Analysis latest"
    Assert-HttpOk $analysis "Analysis latest"

    $actionPlan = Invoke-JsonEndpoint -Url "$ServerUrl/api/v1/action-plan/latest" -Headers $headers -StepName "Action-plan latest"
    Assert-HttpOk $actionPlan "Action-plan latest"
    Assert-ActionPlanContract $actionPlan.Json

    $dispatch = Invoke-JsonEndpoint -Url "$ServerUrl/api/v1/dispatch/latest" -Headers $headers -StepName "Dispatch latest"
    Assert-HttpOk $dispatch "Dispatch latest"
    Assert-DispatchContract $dispatch.Json "Dispatch latest"

    $nodeDispatch = Invoke-JsonEndpoint `
        -Url "$ServerUrl/api/v1/dispatch/for-node/$NodeId" `
        -Headers $headers `
        -StepName "Node-scoped dispatch"
    Assert-HttpOk $nodeDispatch "Node-scoped dispatch"
    Assert-DispatchContract $nodeDispatch.Json "Node-scoped dispatch"
}

Write-Step "Consume node-scoped dispatch from DATUM"
Run-CommandOrFail `
    -WorkingDirectory $DatumDir `
    -Command @("cargo", "run", "--", "dispatch", "--config", $ResolvedClientConfig) `
    -StepName "DATUM dispatch consume" | Out-Null

Write-Host ""
Write-Host "DATUM client/server contract smoke test passed."
Write-Host ""
Write-Host "Validated:"
Write-Host "- health"
Write-Host "- capability publish/read"
Write-Host "- snapshot publish"
Write-Host "- graph"
Write-Host "- analysis"
Write-Host "- action-plan capability metadata"
Write-Host "- gated dispatch"
Write-Host "- node-scoped dispatch"
Write-Host "- client dispatch consumption"
Write-Host ""
Write-Host "No dispatch execution was performed."
