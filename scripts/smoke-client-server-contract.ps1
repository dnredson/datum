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
    $previousNativeErrorPreference = $PSNativeCommandUseErrorActionPreference
    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $PSNativeCommandUseErrorActionPreference = $false
        $ErrorActionPreference = "Continue"
        $output = & $Command[0] $Command[1..($Command.Length - 1)] 2>&1 | Out-String
        $exitCode = $LASTEXITCODE
    }
    finally {
        $PSNativeCommandUseErrorActionPreference = $previousNativeErrorPreference
        $ErrorActionPreference = $previousErrorActionPreference
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

function Get-FileFingerprint {
    param([string]$Path)

    try {
        $hash = Get-FileHash -Path $Path -Algorithm SHA256
        return "sha256:$($hash.Hash)"
    }
    catch {
        $content = Get-Content -LiteralPath $Path -Raw
        return "content:$content"
    }
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

function Invoke-JsonPostEndpoint {
    param(
        [string]$Url,
        [hashtable]$Headers,
        [object]$Body,
        [string]$StepName
    )

    Write-Host "[$StepName] POST $Url"
    $jsonBody = $Body | ConvertTo-Json -Depth 100
    $response = Invoke-WebRequest `
        -Uri $Url `
        -Headers $Headers `
        -Method Post `
        -ContentType "application/json" `
        -Body $jsonBody `
        -SkipHttpErrorCheck
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

function Convert-CommandOutputToJson {
    param(
        [string]$Output,
        [string]$StepName
    )

    $trimmed = $Output.Trim()
    if ([string]::IsNullOrWhiteSpace($trimmed)) {
        throw "Step '$StepName' did not return JSON output."
    }

    if (-not $trimmed.StartsWith("{") -or -not $trimmed.EndsWith("}")) {
        throw "Step '$StepName' did not return clean JSON output.`n$trimmed"
    }

    try {
        return $trimmed | ConvertFrom-Json -Depth 100
    }
    catch {
        throw "Step '$StepName' returned invalid JSON.`n$trimmed"
    }
}

function Assert-WasmRuntimeSafetyContract {
    param(
        $Json,
        [string]$StepName
    )

    foreach ($propertyName in @(
        "safe_to_auto_execute",
        "side_effects_performed",
        "filesystem_access",
        "network_access",
        "host_command_access",
        "docker_access",
        "artifact_access",
        "installation_task_access",
        "assisted_execution_access"
    )) {
        Assert-JsonHasProperty $Json $propertyName $StepName
        if ($Json.$propertyName -ne $false) {
            throw "$StepName returned $propertyName='$($Json.$propertyName)', expected 'False'."
        }
    }
}

function Assert-WasmRuntimeReportContract {
    param(
        $Json,
        [string]$StepName,
        [switch]$RequireProjectId,
        [switch]$RequireNodeId
    )

    Assert-JsonHasProperty $Json "module_name" $StepName
    Assert-JsonHasProperty $Json "function_name" $StepName
    Assert-JsonHasProperty $Json "execution_mode" $StepName
    Assert-JsonHasProperty $Json "status" $StepName
    Assert-JsonHasProperty $Json "result_i32" $StepName

    if ($RequireProjectId) {
        Assert-JsonHasProperty $Json "project_id" $StepName
        if ($Json.project_id -ne $ProjectId) {
            throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
        }
    }

    if ($RequireNodeId) {
        Assert-JsonHasProperty $Json "node_id" $StepName
        if ($Json.node_id -ne $NodeId) {
            throw "$StepName returned node_id '$($Json.node_id)', expected '$NodeId'."
        }
    }

    if ([string]::IsNullOrWhiteSpace([string]$Json.module_name)) {
        throw "$StepName did not include a non-empty module_name."
    }

    if ($Json.function_name -ne "add") {
        throw "$StepName returned function_name '$($Json.function_name)', expected 'add'."
    }

    if ($Json.execution_mode -ne "embedded_test") {
        throw "$StepName returned execution_mode '$($Json.execution_mode)', expected 'embedded_test'."
    }

    if ($Json.status -ne "executed") {
        throw "$StepName returned status '$($Json.status)', expected 'executed'."
    }

    if ($Json.result_i32 -ne 5) {
        throw "$StepName returned result_i32 '$($Json.result_i32)', expected '5'."
    }

    Assert-WasmRuntimeSafetyContract -Json $Json -StepName $StepName
}

function Assert-WasmRuntimePublishedReportContract {
    param(
        $Json,
        [string]$StepName
    )

    Assert-WasmRuntimeReportContract `
        -Json $Json `
        -StepName $StepName `
        -RequireProjectId `
        -RequireNodeId
    Assert-JsonHasProperty $Json "report_id" $StepName
    Assert-JsonHasProperty $Json "reported_at_utc" $StepName

    if ([string]::IsNullOrWhiteSpace([string]$Json.report_id)) {
        throw "$StepName did not include a non-empty report_id."
    }

    if ([string]::IsNullOrWhiteSpace([string]$Json.reported_at_utc)) {
        throw "$StepName did not include a non-empty reported_at_utc."
    }
}

function Assert-WasmRuntimeReportListContract {
    param(
        $Json,
        [string]$StepName
    )

    Assert-JsonHasProperty $Json "project_id" $StepName
    Assert-JsonHasProperty $Json "count" $StepName
    Assert-JsonHasProperty $Json "reports" $StepName

    if ($Json.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($Json.reports -isnot [System.Array] -and $Json.reports -isnot [System.Collections.IEnumerable]) {
        throw "$StepName did not return 'reports' as an array."
    }
}

function Assert-WasmModuleSafetyContract {
    param(
        $Json,
        [string]$StepName
    )

    foreach ($propertyName in @(
        "filesystem_access",
        "network_access",
        "host_command_access",
        "docker_access",
        "artifact_access",
        "installation_task_access",
        "assisted_execution_access",
        "side_effects_allowed",
        "safe_to_auto_execute"
    )) {
        Assert-JsonHasProperty $Json $propertyName $StepName
        if ($Json.$propertyName -ne $false) {
            throw "$StepName returned $propertyName='$($Json.$propertyName)', expected 'False'."
        }
    }
}

function Find-WasmModuleDescriptorById {
    param(
        $Modules,
        [string]$ModuleId
    )

    return @($Modules | Where-Object { $_.module_id -eq $ModuleId } | Select-Object -First 1)
}

function Assert-WasmModuleDescriptorContract {
    param(
        $Json,
        [string]$StepName
    )

    Assert-JsonHasProperty $Json "project_id" $StepName
    Assert-JsonHasProperty $Json "module_id" $StepName
    Assert-JsonHasProperty $Json "module_name" $StepName
    Assert-JsonHasProperty $Json "module_kind" $StepName
    Assert-JsonHasProperty $Json "status" $StepName
    Assert-JsonHasProperty $Json "allowed_functions" $StepName
    Assert-JsonHasProperty $Json "safety" $StepName

    if ($Json.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($Json.module_id -ne "embedded-add") {
        throw "$StepName returned module_id '$($Json.module_id)', expected 'embedded-add'."
    }

    if ($Json.module_name -ne "embedded:add") {
        throw "$StepName returned module_name '$($Json.module_name)', expected 'embedded:add'."
    }

    if ($Json.module_kind -ne "embedded_test") {
        throw "$StepName returned module_kind '$($Json.module_kind)', expected 'embedded_test'."
    }

    if ($Json.status -ne "registered") {
        throw "$StepName returned status '$($Json.status)', expected 'registered'."
    }

    $matchingFunction = @($Json.allowed_functions | Where-Object { $_.function_name -eq "add" } | Select-Object -First 1)
    if ($matchingFunction.Count -eq 0) {
        throw "$StepName did not return allowed_functions containing function_name 'add'."
    }

    if ($matchingFunction[0].allowed_execution_modes -notcontains "embedded_test") {
        throw "$StepName did not return allowed_execution_modes containing 'embedded_test' for function 'add'."
    }

    if ($null -ne $matchingFunction[0].max_fuel -and $matchingFunction[0].max_fuel -le 0) {
        throw "$StepName returned non-positive max_fuel '$($matchingFunction[0].max_fuel)'."
    }

    if ($null -ne $matchingFunction[0].timeout_ms -and $matchingFunction[0].timeout_ms -le 0) {
        throw "$StepName returned non-positive timeout_ms '$($matchingFunction[0].timeout_ms)'."
    }

    Assert-WasmModuleSafetyContract -Json $Json.safety -StepName $StepName
}

function Assert-WasmModuleDescriptorListContract {
    param(
        $Json,
        [string]$StepName
    )

    Assert-JsonHasProperty $Json "project_id" $StepName
    Assert-JsonHasProperty $Json "count" $StepName
    Assert-JsonHasProperty $Json "modules" $StepName

    if ($Json.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($Json.modules -isnot [System.Array] -and $Json.modules -isnot [System.Collections.IEnumerable]) {
        throw "$StepName did not return 'modules' as an array."
    }
}

function Assert-WasmModuleArtifactMetadataOnly {
    param(
        $Json,
        [string]$StepName
    )

    Assert-JsonHasProperty $Json "artifact_ref" $StepName
    if ($null -eq $Json.artifact_ref) {
        throw "$StepName did not include artifact_ref metadata."
    }

    Assert-JsonHasProperty $Json "artifact_ref.artifact_id" $StepName
    Assert-JsonHasProperty $Json "artifact_ref.artifact_uri" $StepName
    Assert-JsonHasProperty $Json "artifact_ref.provenance" $StepName

    if ($Json.artifact_ref.artifact_id -ne "embedded-add-metadata-only") {
        throw "$StepName returned artifact_id '$($Json.artifact_ref.artifact_id)', expected 'embedded-add-metadata-only'."
    }

    if ($Json.artifact_ref.artifact_uri -ne "metadata-only://embedded-add") {
        throw "$StepName returned artifact_uri '$($Json.artifact_ref.artifact_uri)', expected 'metadata-only://embedded-add'."
    }

    if ($Json.artifact_ref.provenance -ne "embedded test descriptor only") {
        throw "$StepName returned provenance '$($Json.artifact_ref.provenance)', expected 'embedded test descriptor only'."
    }
}

function Assert-LocalDGraphSnapshotContract {
    param(
        $Json,
        [string]$StepName,
        [switch]$RequireProjectId,
        [switch]$RequireNodeId,
        [switch]$RequireSnapshotMetadata
    )

    if ($RequireProjectId) {
        Assert-JsonHasProperty $Json "project_id" $StepName
        if ($Json.project_id -ne $ProjectId) {
            throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
        }
    }

    if ($RequireNodeId) {
        Assert-JsonHasProperty $Json "node_id" $StepName
        if ($Json.node_id -ne $NodeId) {
            throw "$StepName returned node_id '$($Json.node_id)', expected '$NodeId'."
        }
    }

    if ($RequireSnapshotMetadata) {
        Assert-JsonHasProperty $Json "snapshot_id" $StepName
        Assert-JsonHasProperty $Json "reported_at_utc" $StepName

        if ([string]::IsNullOrWhiteSpace([string]$Json.snapshot_id)) {
            throw "$StepName did not include a non-empty snapshot_id."
        }

        if ([string]::IsNullOrWhiteSpace([string]$Json.reported_at_utc)) {
            throw "$StepName did not include a non-empty reported_at_utc."
        }
    }

    foreach ($propertyName in @(
        "runtimes",
        "modules",
        "flows",
        "health",
        "capabilities"
    )) {
        Assert-JsonHasProperty $Json $propertyName $StepName
        if ($Json.$propertyName -isnot [System.Array] -and $Json.$propertyName -isnot [System.Collections.IEnumerable]) {
            throw "$StepName did not return '$propertyName' as an array."
        }
    }

    Assert-JsonHasProperty $Json "safe_to_auto_execute" $StepName
    Assert-JsonHasProperty $Json "side_effects_performed" $StepName

    if ($Json.safe_to_auto_execute -ne $false) {
        throw "$StepName returned safe_to_auto_execute='$($Json.safe_to_auto_execute)', expected 'False'."
    }

    if ($Json.side_effects_performed -ne $false) {
        throw "$StepName returned side_effects_performed='$($Json.side_effects_performed)', expected 'False'."
    }
}

function Assert-LocalDGraphFixtureSnapshotContract {
    param(
        $Json,
        [string]$StepName,
        [string]$Scenario
    )

    Assert-LocalDGraphSnapshotContract `
        -Json $Json `
        -StepName $StepName `
        -RequireNodeId

    Assert-JsonHasProperty $Json "metadata" $StepName
    Assert-JsonHasProperty $Json "metadata.fixture" $StepName
    Assert-JsonHasProperty $Json "metadata.fixture_scenario" $StepName

    if ($Json.metadata.fixture -ne $true) {
        throw "$StepName returned metadata.fixture='$($Json.metadata.fixture)', expected 'True'."
    }

    if ($Json.metadata.fixture_scenario -ne $Scenario) {
        throw "$StepName returned metadata.fixture_scenario='$($Json.metadata.fixture_scenario)', expected '$Scenario'."
    }

    switch ($Scenario) {
        "degraded-node" {
            $matchingHealth = @($Json.health | Where-Object {
                    $_.target_kind -eq "node" -and
                    $_.target_id -eq $NodeId -and
                    $_.status -eq "degraded"
                })
            if ($matchingHealth.Count -lt 1) {
                throw "$StepName did not include a degraded node health entry for '$NodeId'."
            }
        }
        "unavailable-node" {
            $matchingHealth = @($Json.health | Where-Object {
                    $_.target_kind -eq "node" -and
                    $_.target_id -eq $NodeId -and
                    $_.status -eq "unavailable"
                })
            if ($matchingHealth.Count -lt 1) {
                throw "$StepName did not include an unavailable node health entry for '$NodeId'."
            }
        }
        "missing-module" {
            $missingModule = @($Json.modules | Where-Object { $_.observed_state -eq "missing" })
            $missingModuleHealth = @($Json.health | Where-Object {
                    $_.target_kind -eq "module" -and $_.status -eq "missing"
                })
            if ($missingModule.Count -lt 1 -and $missingModuleHealth.Count -lt 1) {
                throw "$StepName did not include missing-module metadata in modules or health entries."
            }
        }
        "degraded-flow" {
            $degradedFlow = @($Json.flows | Where-Object { $_.status -eq "degraded" })
            $degradedEdge = @(
                $Json.flows |
                ForEach-Object { @($_.edges) } |
                Where-Object { $_.status -eq "degraded" }
            )
            $degradedFlowHealth = @($Json.health | Where-Object {
                    $_.target_kind -eq "flow" -and $_.status -eq "degraded"
                })
            if ($degradedFlow.Count -lt 1 -and $degradedEdge.Count -lt 1 -and $degradedFlowHealth.Count -lt 1) {
                throw "$StepName did not include degraded flow metadata in flows, edges, or health entries."
            }
        }
        "no-candidate" {
            $unavailableRuntime = @($Json.runtimes | Where-Object { $_.status -eq "unavailable" })
            $unsupportedCapability = @($Json.capabilities | Where-Object {
                    $_.capability_kind -eq "container_runtime_configured" -and $_.supported -eq $false
                })
            $runtimeHealth = @($Json.health | Where-Object {
                    $_.target_kind -eq "runtime" -and $_.status -eq "unavailable"
                })
            if ($unavailableRuntime.Count -lt 1 -and $unsupportedCapability.Count -lt 1 -and $runtimeHealth.Count -lt 1) {
                throw "$StepName did not include unavailable runtime or unsupported capability metadata for the no-candidate scenario."
            }
        }
    }
}

function Assert-LocalDGraphSnapshotListContract {
    param(
        $Json,
        [string]$StepName
    )

    Assert-JsonHasProperty $Json "project_id" $StepName
    Assert-JsonHasProperty $Json "count" $StepName
    Assert-JsonHasProperty $Json "snapshots" $StepName

    if ($Json.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($Json.snapshots -isnot [System.Array] -and $Json.snapshots -isnot [System.Collections.IEnumerable]) {
        throw "$StepName did not return 'snapshots' as an array."
    }
}

function Assert-ProjectDGraphMaterializationContract {
    param(
        $Json,
        [string]$StepName
    )

    Assert-JsonHasProperty $Json "project_id" $StepName
    Assert-JsonHasProperty $Json "source_snapshot_count" $StepName
    Assert-JsonHasProperty $Json "graph" $StepName
    Assert-JsonHasProperty $Json "summary" $StepName
    Assert-JsonHasProperty $Json "validation_errors" $StepName
    Assert-JsonHasProperty $Json "side_effects_performed" $StepName

    if ($Json.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($Json.source_snapshot_count -lt 0) {
        throw "$StepName returned source_snapshot_count '$($Json.source_snapshot_count)', expected a non-negative value."
    }

    if ($null -eq $Json.graph) {
        throw "$StepName returned null graph, expected an object."
    }

    if ($null -eq $Json.summary) {
        throw "$StepName returned null summary, expected an object."
    }

    foreach ($requiredHealthField in @(
        "healthy_targets",
        "unknown_targets",
        "degraded_targets",
        "unavailable_targets",
        "unhealthy_targets"
    )) {
        Assert-JsonHasProperty $Json.summary $requiredHealthField "$StepName summary"
    }

    if ($Json.validation_errors -isnot [System.Array] -and $Json.validation_errors -isnot [System.Collections.IEnumerable]) {
        throw "$StepName did not return 'validation_errors' as an array."
    }

    if ($Json.side_effects_performed -ne $false) {
        throw "$StepName returned side_effects_performed='$($Json.side_effects_performed)', expected 'False'."
    }

    foreach ($graphArrayField in @("nodes", "modules", "relationships")) {
        if ($Json.graph.PSObject.Properties.Name -contains $graphArrayField) {
            $value = $Json.graph.$graphArrayField
            if ($value -isnot [System.Array] -and $value -isnot [System.Collections.IEnumerable]) {
                throw "$StepName returned graph.$graphArrayField that was not an array."
            }
        }
    }
}

function Assert-ProjectDGraphMaterializationSummaryContract {
    param(
        $Json,
        [string]$StepName
    )

    Assert-JsonHasProperty $Json "project_id" $StepName
    Assert-JsonHasProperty $Json "source_snapshot_count" $StepName
    Assert-JsonHasProperty $Json "summary" $StepName
    Assert-JsonHasProperty $Json "validation_errors" $StepName
    Assert-JsonHasProperty $Json "side_effects_performed" $StepName

    if ($Json.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($Json.source_snapshot_count -lt 0) {
        throw "$StepName returned source_snapshot_count '$($Json.source_snapshot_count)', expected a non-negative value."
    }

    if ($null -eq $Json.summary) {
        throw "$StepName returned null summary, expected an object."
    }

    if ($Json.validation_errors -isnot [System.Array] -and $Json.validation_errors -isnot [System.Collections.IEnumerable]) {
        throw "$StepName did not return 'validation_errors' as an array."
    }

    if ($Json.side_effects_performed -ne $false) {
        throw "$StepName returned side_effects_performed='$($Json.side_effects_performed)', expected 'False'."
    }

    foreach ($summaryField in @(
        "node_count",
        "relationship_count",
        "healthy_targets",
        "unknown_targets",
        "degraded_targets",
        "unavailable_targets",
        "unhealthy_targets"
    )) {
        if ($Json.summary.PSObject.Properties.Name -contains $summaryField) {
            $value = $Json.summary.$summaryField
            if ($null -ne $value -and $value -lt 0) {
                throw "$StepName returned summary.$summaryField '$value', expected a non-negative value when present."
            }
        }
    }

    $expectedUnhealthyTargets = $Json.summary.degraded_targets + $Json.summary.unavailable_targets
    if ($Json.summary.unhealthy_targets -ne $expectedUnhealthyTargets) {
        throw "$StepName returned summary.unhealthy_targets '$($Json.summary.unhealthy_targets)', expected degraded_targets + unavailable_targets '$expectedUnhealthyTargets'."
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

function Assert-ActionResultNodeContract {
    param($Json)

    Assert-JsonHasProperty $Json "project_id" "Node latest action-results"
    Assert-JsonHasProperty $Json "node_id" "Node latest action-results"
    Assert-JsonHasProperty $Json "report_id" "Node latest action-results"
    Assert-JsonHasProperty $Json "reported_at_utc" "Node latest action-results"
    Assert-JsonHasProperty $Json "results" "Node latest action-results"
    Assert-JsonHasProperty $Json "summary" "Node latest action-results"

    if ($Json.project_id -ne $ProjectId) {
        throw "Node latest action-results returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($Json.node_id -ne $NodeId) {
        throw "Node latest action-results returned node_id '$($Json.node_id)', expected '$NodeId'."
    }

    if ([string]::IsNullOrWhiteSpace([string]$Json.report_id)) {
        throw "Node latest action-results did not include a non-empty report_id."
    }

    if ([string]::IsNullOrWhiteSpace([string]$Json.reported_at_utc)) {
        throw "Node latest action-results did not include a non-empty reported_at_utc."
    }
}

function Assert-ActionResultProjectContract {
    param($Json)

    Assert-JsonHasProperty $Json "project_id" "Project latest action-results"
    Assert-JsonHasProperty $Json "count" "Project latest action-results"
    Assert-JsonHasProperty $Json "reports" "Project latest action-results"

    if ($Json.project_id -ne $ProjectId) {
        throw "Project latest action-results returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($Json.reports -isnot [System.Array] -and $Json.reports -isnot [System.Collections.IEnumerable]) {
        throw "Project latest action-results did not return 'reports' as an array."
    }
}

function Assert-PlanningContract {
    param(
        $Json,
        [string]$PlanningProperty,
        [string]$StepName,
        [switch]$RequireNodeId
    )

    Assert-JsonHasProperty $Json "project_id" $StepName
    if ($Json.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($RequireNodeId) {
        Assert-JsonHasProperty $Json "node_id" $StepName
        if ($Json.node_id -ne $NodeId) {
            throw "$StepName returned node_id '$($Json.node_id)', expected '$NodeId'."
        }
    }

    Assert-JsonHasProperty $Json $PlanningProperty $StepName
    Assert-JsonHasProperty $Json "$PlanningProperty.items" $StepName
    Assert-JsonHasProperty $Json "$PlanningProperty.summary" $StepName
}

function Assert-InstallationTaskListContract {
    param(
        $Json,
        [string]$StepName,
        [switch]$RequireNodeId
    )

    Assert-JsonHasProperty $Json "project_id" $StepName
    Assert-JsonHasProperty $Json "count" $StepName
    Assert-JsonHasProperty $Json "tasks" $StepName

    if ($Json.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($RequireNodeId) {
        Assert-JsonHasProperty $Json "node_id" $StepName
        if ($Json.node_id -ne $NodeId) {
            throw "$StepName returned node_id '$($Json.node_id)', expected '$NodeId'."
        }
    }

    if ($Json.tasks -isnot [System.Array] -and $Json.tasks -isnot [System.Collections.IEnumerable]) {
        throw "$StepName did not return 'tasks' as an array."
    }
}

function Assert-InstallationTaskContract {
    param(
        $Json,
        [string]$StepName
    )

    Assert-JsonHasProperty $Json "task_id" $StepName
    Assert-JsonHasProperty $Json "status" $StepName

    if ([string]::IsNullOrWhiteSpace([string]$Json.task_id)) {
        throw "$StepName did not include a non-empty task_id."
    }

    if ([string]::IsNullOrWhiteSpace([string]$Json.status)) {
        throw "$StepName did not include a non-empty status."
    }
}

function Assert-InstallationPolicyContract {
    param(
        $Json,
        [string]$StepName
    )

    if ($null -eq $Json.installation_policy) {
        Write-Warning "Installation task does not include installation_policy; this may indicate an old stored record."
        return $false
    }

    Assert-JsonHasProperty $Json "installation_policy.decision" $StepName
    Assert-JsonHasProperty $Json "installation_policy.safe_to_auto_execute" $StepName

    if ($Json.installation_policy.safe_to_auto_execute -ne $false) {
        throw "$StepName returned installation_policy.safe_to_auto_execute='$($Json.installation_policy.safe_to_auto_execute)', expected 'False'."
    }

    return $true
}

function Assert-InstallationTaskApprovalContract {
    param(
        $Json,
        [string]$StepName,
        [string]$ExpectedTaskId
    )

    Assert-JsonHasProperty $Json "project_id" $StepName
    Assert-JsonHasProperty $Json "node_id" $StepName
    Assert-JsonHasProperty $Json "task_id" $StepName
    Assert-JsonHasProperty $Json "approved" $StepName
    Assert-JsonHasProperty $Json "approval_mode" $StepName
    Assert-JsonHasProperty $Json "approval_id" $StepName
    Assert-JsonHasProperty $Json "approved_at_utc" $StepName

    if ($Json.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($Json.node_id -ne $NodeId) {
        throw "$StepName returned node_id '$($Json.node_id)', expected '$NodeId'."
    }

    if ($Json.task_id -ne $ExpectedTaskId) {
        throw "$StepName returned task_id '$($Json.task_id)', expected '$ExpectedTaskId'."
    }

    if ($Json.approved -ne $true) {
        throw "$StepName returned approved='$($Json.approved)', expected 'True'."
    }

    if ($Json.approval_mode -ne "operator_approved") {
        throw "$StepName returned approval_mode '$($Json.approval_mode)', expected 'operator_approved'."
    }

    if ([string]::IsNullOrWhiteSpace([string]$Json.approval_id)) {
        throw "$StepName did not include a non-empty approval_id."
    }

    if ([string]::IsNullOrWhiteSpace([string]$Json.approved_at_utc)) {
        throw "$StepName did not include a non-empty approved_at_utc."
    }

    if (-not $Json.PSObject.Properties.Name.Contains("approved_by")) {
        throw "$StepName did not include approved_by."
    }
}

function Assert-InstallationTaskApprovalListContract {
    param(
        $Json,
        [string]$StepName
    )

    Assert-JsonHasProperty $Json "project_id" $StepName
    Assert-JsonHasProperty $Json "node_id" $StepName
    Assert-JsonHasProperty $Json "count" $StepName
    Assert-JsonHasProperty $Json "approvals" $StepName

    if ($Json.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($Json.node_id -ne $NodeId) {
        throw "$StepName returned node_id '$($Json.node_id)', expected '$NodeId'."
    }

    if ($Json.approvals -isnot [System.Array] -and $Json.approvals -isnot [System.Collections.IEnumerable]) {
        throw "$StepName did not return 'approvals' as an array."
    }
}

function Assert-InstallationTaskReadinessContract {
    param(
        $Json,
        [string]$StepName,
        [string]$ExpectedTaskId
    )

    Assert-JsonHasProperty $Json "task_id" $StepName
    Assert-JsonHasProperty $Json "readiness" $StepName
    Assert-JsonHasProperty $Json "reason" $StepName
    Assert-JsonHasProperty $Json "safe_to_auto_execute" $StepName

    if ($Json.task_id -ne $ExpectedTaskId) {
        throw "$StepName returned task_id '$($Json.task_id)', expected '$ExpectedTaskId'."
    }

    if ($Json.safe_to_auto_execute -ne $false) {
        throw "$StepName returned safe_to_auto_execute='$($Json.safe_to_auto_execute)', expected 'False'."
    }
}

function Assert-InstallationTaskReadinessListContract {
    param(
        $Json,
        [string]$StepName
    )

    Assert-JsonHasProperty $Json "project_id" $StepName
    Assert-JsonHasProperty $Json "node_id" $StepName
    Assert-JsonHasProperty $Json "count" $StepName
    Assert-JsonHasProperty $Json "readiness" $StepName

    if ($Json.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($Json.node_id -ne $NodeId) {
        throw "$StepName returned node_id '$($Json.node_id)', expected '$NodeId'."
    }

    if ($Json.readiness -isnot [System.Array] -and $Json.readiness -isnot [System.Collections.IEnumerable]) {
        throw "$StepName did not return 'readiness' as an array."
    }
}

function Assert-LocalMappingRecommendationListContract {
    param(
        $Json,
        [string]$StepName,
        [switch]$RequireNodeId
    )

    Assert-JsonHasProperty $Json "project_id" $StepName
    Assert-JsonHasProperty $Json "count" $StepName
    Assert-JsonHasProperty $Json "recommendations" $StepName

    if ($Json.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($RequireNodeId) {
        Assert-JsonHasProperty $Json "node_id" $StepName
        if ($Json.node_id -ne $NodeId) {
            throw "$StepName returned node_id '$($Json.node_id)', expected '$NodeId'."
        }
    }

    if ($Json.recommendations -isnot [System.Array] -and $Json.recommendations -isnot [System.Collections.IEnumerable]) {
        throw "$StepName did not return 'recommendations' as an array."
    }
}

function Assert-LocalMappingRecommendationSafetyContract {
    param(
        $Recommendation,
        [string]$StepName
    )

    Assert-JsonHasProperty $Recommendation "safe_to_auto_apply" $StepName
    Assert-JsonHasProperty $Recommendation "requires_operator" $StepName

    if ($Recommendation.safe_to_auto_apply -ne $false) {
        throw "$StepName returned safe_to_auto_apply='$($Recommendation.safe_to_auto_apply)', expected 'False'."
    }

    if ($Recommendation.requires_operator -ne $true) {
        throw "$StepName returned requires_operator='$($Recommendation.requires_operator)', expected 'True'."
    }

    if ($Recommendation.PSObject.Properties.Name.Contains("recommendation_id") -and
        [string]::IsNullOrWhiteSpace([string]$Recommendation.recommendation_id)) {
        throw "$StepName exposed recommendation_id but it was empty."
    }

    if ($Recommendation.PSObject.Properties.Name.Contains("recommendation_status") -and
        [string]::IsNullOrWhiteSpace([string]$Recommendation.recommendation_status)) {
        throw "$StepName exposed recommendation_status but it was empty."
    }
}

function Assert-LocalMappingPatchProposalListContract {
    param(
        $Json,
        [string]$StepName,
        [switch]$RequireNodeId
    )

    Assert-JsonHasProperty $Json "project_id" $StepName
    Assert-JsonHasProperty $Json "count" $StepName
    Assert-JsonHasProperty $Json "proposals" $StepName

    if ($Json.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($RequireNodeId) {
        Assert-JsonHasProperty $Json "node_id" $StepName
        if ($Json.node_id -ne $NodeId) {
            throw "$StepName returned node_id '$($Json.node_id)', expected '$NodeId'."
        }
    }

    if ($Json.proposals -isnot [System.Array] -and $Json.proposals -isnot [System.Collections.IEnumerable]) {
        throw "$StepName did not return 'proposals' as an array."
    }
}

function Assert-LocalMappingPatchProposalSafetyContract {
    param(
        $Proposal,
        [string]$StepName,
        [switch]$RequireNodeId,
        [string[]]$AllowedProposalStatuses,
        [string[]]$AllowedProposalKinds
    )

    Assert-JsonHasProperty $Proposal "project_id" $StepName
    Assert-JsonHasProperty $Proposal "safe_to_auto_apply" $StepName
    Assert-JsonHasProperty $Proposal "side_effects_performed" $StepName
    Assert-JsonHasProperty $Proposal "requires_operator" $StepName

    if ($Proposal.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Proposal.project_id)', expected '$ProjectId'."
    }

    if ($RequireNodeId) {
        Assert-JsonHasProperty $Proposal "node_id" $StepName
        if ($Proposal.node_id -ne $NodeId) {
            throw "$StepName returned node_id '$($Proposal.node_id)', expected '$NodeId'."
        }
    }

    if ($Proposal.safe_to_auto_apply -ne $false) {
        throw "$StepName returned safe_to_auto_apply='$($Proposal.safe_to_auto_apply)', expected 'False'."
    }

    if ($Proposal.side_effects_performed -ne $false) {
        throw "$StepName returned side_effects_performed='$($Proposal.side_effects_performed)', expected 'False'."
    }

    if ($Proposal.requires_operator -ne $true) {
        throw "$StepName returned requires_operator='$($Proposal.requires_operator)', expected 'True'."
    }

    if ($Proposal.PSObject.Properties.Name.Contains("proposal_id") -and
        [string]::IsNullOrWhiteSpace([string]$Proposal.proposal_id)) {
        throw "$StepName exposed proposal_id but it was empty."
    }

    if ($Proposal.PSObject.Properties.Name.Contains("proposal_status") -and
        [string]::IsNullOrWhiteSpace([string]$Proposal.proposal_status)) {
        throw "$StepName exposed proposal_status but it was empty."
    }

    if ($AllowedProposalStatuses.Count -gt 0 -and
        $Proposal.PSObject.Properties.Name.Contains("proposal_status") -and
        -not [string]::IsNullOrWhiteSpace([string]$Proposal.proposal_status) -and
        $AllowedProposalStatuses -notcontains [string]$Proposal.proposal_status) {
        $allowedStatusesText = $AllowedProposalStatuses -join ", "
        throw "$StepName returned proposal_status '$($Proposal.proposal_status)', expected one of: $allowedStatusesText."
    }

    if ($Proposal.PSObject.Properties.Name.Contains("proposal_kind") -and
        [string]::IsNullOrWhiteSpace([string]$Proposal.proposal_kind)) {
        throw "$StepName exposed proposal_kind but it was empty."
    }

    if ($AllowedProposalKinds.Count -gt 0 -and
        $Proposal.PSObject.Properties.Name.Contains("proposal_kind") -and
        -not [string]::IsNullOrWhiteSpace([string]$Proposal.proposal_kind) -and
        $AllowedProposalKinds -notcontains [string]$Proposal.proposal_kind) {
        $allowedKindsText = $AllowedProposalKinds -join ", "
        throw "$StepName returned proposal_kind '$($Proposal.proposal_kind)', expected one of: $allowedKindsText."
    }

    if ($Proposal.PSObject.Properties.Name.Contains("proposed_config_section") -and
        $null -ne $Proposal.proposed_config_section -and
        [string]::IsNullOrWhiteSpace([string]$Proposal.proposed_config_section)) {
        throw "$StepName exposed proposed_config_section but it was empty."
    }

    if ($Proposal.PSObject.Properties.Name.Contains("proposed_config_snippet") -and
        $null -ne $Proposal.proposed_config_snippet -and
        [string]::IsNullOrWhiteSpace([string]$Proposal.proposed_config_snippet)) {
        throw "$StepName exposed proposed_config_snippet but it was empty."
    }
}

function Assert-LocalMappingPreflightReportContract {
    param(
        $Json,
        [string]$StepName,
        [switch]$RequireNodeId
    )

    Assert-JsonHasProperty $Json "project_id" $StepName
    Assert-JsonHasProperty $Json "node_id" $StepName
    Assert-JsonHasProperty $Json "report_id" $StepName
    Assert-JsonHasProperty $Json "reported_at_utc" $StepName
    Assert-JsonHasProperty $Json "status" $StepName
    Assert-JsonHasProperty $Json "items" $StepName
    Assert-JsonHasProperty $Json "summary" $StepName
    Assert-JsonHasProperty $Json "safe_to_auto_execute" $StepName
    Assert-JsonHasProperty $Json "side_effects_performed" $StepName

    if ($Json.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($RequireNodeId -and $Json.node_id -ne $NodeId) {
        throw "$StepName returned node_id '$($Json.node_id)', expected '$NodeId'."
    }

    if ([string]::IsNullOrWhiteSpace([string]$Json.report_id)) {
        throw "$StepName did not include a non-empty report_id."
    }

    if ([string]::IsNullOrWhiteSpace([string]$Json.reported_at_utc)) {
        throw "$StepName did not include a non-empty reported_at_utc."
    }

    if ($Json.items -isnot [System.Array] -and $Json.items -isnot [System.Collections.IEnumerable]) {
        throw "$StepName did not return 'items' as an array."
    }

    if ($Json.safe_to_auto_execute -ne $false) {
        throw "$StepName returned safe_to_auto_execute='$($Json.safe_to_auto_execute)', expected 'False'."
    }

    if ($Json.side_effects_performed -ne $false) {
        throw "$StepName returned side_effects_performed='$($Json.side_effects_performed)', expected 'False'."
    }
}

function Assert-ProjectDGraphPlannerDryRunContract {
    param(
        $Json,
        [string]$StepName
    )

    Assert-JsonHasProperty $Json "project_id" $StepName
    Assert-JsonHasProperty $Json "source_snapshot_count" $StepName
    Assert-JsonHasProperty $Json "materialization_validation_errors" $StepName
    Assert-JsonHasProperty $Json "planner" $StepName
    Assert-JsonHasProperty $Json "side_effects_performed" $StepName

    if ($Json.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($Json.source_snapshot_count -lt 0) {
        throw "$StepName returned source_snapshot_count '$($Json.source_snapshot_count)', expected a non-negative value."
    }

    if ($Json.materialization_validation_errors -isnot [System.Array] -and $Json.materialization_validation_errors -isnot [System.Collections.IEnumerable]) {
        throw "$StepName did not return 'materialization_validation_errors' as an array."
    }

    if ($null -eq $Json.planner) {
        throw "$StepName returned null planner, expected an object."
    }

    if ($Json.side_effects_performed -ne $false) {
        throw "$StepName returned side_effects_performed='$($Json.side_effects_performed)', expected 'False'."
    }

    foreach ($plannerProperty in @("placements", "reconfigurations", "blocked", "warnings")) {
        Assert-JsonHasProperty $Json "planner.$plannerProperty" $StepName
        $plannerValue = $Json.planner.$plannerProperty
        if ($plannerValue -isnot [System.Array] -and $plannerValue -isnot [System.Collections.IEnumerable]) {
            throw "$StepName returned planner.$plannerProperty that was not an array."
        }
    }

    Assert-JsonHasProperty $Json "planner.side_effects_performed" $StepName
    if ($Json.planner.side_effects_performed -ne $false) {
        throw "$StepName returned planner.side_effects_performed='$($Json.planner.side_effects_performed)', expected 'False'."
    }

    if ($Json.planner.PSObject.Properties.Name -notcontains "status") {
        throw "$StepName did not return planner.status."
    }

    if ($Json.planner.PSObject.Properties.Name -notcontains "summary" -or $null -eq $Json.planner.summary) {
        throw "$StepName did not return planner.summary as an object."
    }
}

function Assert-ProjectDGraphPlannerDryRunSummaryContract {
    param(
        $Json,
        [string]$StepName
    )

    Assert-JsonHasProperty $Json "project_id" $StepName
    Assert-JsonHasProperty $Json "source_snapshot_count" $StepName
    Assert-JsonHasProperty $Json "summary" $StepName
    Assert-JsonHasProperty $Json "side_effects_performed" $StepName

    if ($Json.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($Json.source_snapshot_count -lt 0) {
        throw "$StepName returned source_snapshot_count '$($Json.source_snapshot_count)', expected a non-negative value."
    }

    if ($null -eq $Json.summary) {
        throw "$StepName returned null summary, expected an object."
    }

    if ($Json.side_effects_performed -ne $false) {
        throw "$StepName returned side_effects_performed='$($Json.side_effects_performed)', expected 'False'."
    }

    if ($Json.PSObject.Properties.Name -contains "status" -and
        [string]::IsNullOrWhiteSpace([string]$Json.status)) {
        throw "$StepName returned empty status."
    }

    foreach ($countProperty in @("placement_count", "reconfiguration_count", "blocked_count", "warning_count")) {
        if ($Json.PSObject.Properties.Name -contains $countProperty) {
            if ($null -eq $Json.$countProperty) {
                throw "$StepName returned null $countProperty."
            }
        }
        elseif ($Json.summary.PSObject.Properties.Name -notcontains $countProperty) {
            throw "$StepName did not return $countProperty either at the top level or inside summary."
        }
    }
}

function Assert-LocalMappingPreflightReportListContract {
    param(
        $Json,
        [string]$StepName
    )

    Assert-JsonHasProperty $Json "project_id" $StepName
    Assert-JsonHasProperty $Json "count" $StepName
    Assert-JsonHasProperty $Json "reports" $StepName

    if ($Json.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($Json.reports -isnot [System.Array] -and $Json.reports -isnot [System.Collections.IEnumerable]) {
        throw "$StepName did not return 'reports' as an array."
    }
}

function Assert-AssistedExecutionRequestContract {
    param(
        $Json,
        [string]$StepName,
        [string]$ExpectedTaskId
    )

    Assert-JsonHasProperty $Json "project_id" $StepName
    Assert-JsonHasProperty $Json "node_id" $StepName
    Assert-JsonHasProperty $Json "task_id" $StepName
    Assert-JsonHasProperty $Json "request_id" $StepName
    Assert-JsonHasProperty $Json "request_status" $StepName
    Assert-JsonHasProperty $Json "execution_mode" $StepName
    Assert-JsonHasProperty $Json "requested_at_utc" $StepName
    Assert-JsonHasProperty $Json "safe_to_auto_execute" $StepName

    if ($Json.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($Json.node_id -ne $NodeId) {
        throw "$StepName returned node_id '$($Json.node_id)', expected '$NodeId'."
    }

    if ($Json.task_id -ne $ExpectedTaskId) {
        throw "$StepName returned task_id '$($Json.task_id)', expected '$ExpectedTaskId'."
    }

    if ([string]::IsNullOrWhiteSpace([string]$Json.request_id)) {
        throw "$StepName did not include a non-empty request_id."
    }

    if ($Json.request_status -ne "requested") {
        throw "$StepName returned request_status '$($Json.request_status)', expected 'requested'."
    }

    if ($Json.execution_mode -ne "assisted_operator_approved") {
        throw "$StepName returned execution_mode '$($Json.execution_mode)', expected 'assisted_operator_approved'."
    }

    if ([string]::IsNullOrWhiteSpace([string]$Json.requested_at_utc)) {
        throw "$StepName did not include a non-empty requested_at_utc."
    }

    if ($Json.safe_to_auto_execute -ne $false) {
        throw "$StepName returned safe_to_auto_execute='$($Json.safe_to_auto_execute)', expected 'False'."
    }
}

function Assert-AssistedExecutionRequestListContract {
    param(
        $Json,
        [string]$StepName
    )

    Assert-JsonHasProperty $Json "project_id" $StepName
    Assert-JsonHasProperty $Json "node_id" $StepName
    Assert-JsonHasProperty $Json "count" $StepName
    Assert-JsonHasProperty $Json "requests" $StepName

    if ($Json.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($Json.node_id -ne $NodeId) {
        throw "$StepName returned node_id '$($Json.node_id)', expected '$NodeId'."
    }

    if ($Json.requests -isnot [System.Array] -and $Json.requests -isnot [System.Collections.IEnumerable]) {
        throw "$StepName did not return 'requests' as an array."
    }
}

function Assert-AssistedExecutionClaimContract {
    param(
        $Json,
        [string]$StepName,
        [string]$ExpectedTaskId,
        [string]$ExpectedRequestId
    )

    Assert-JsonHasProperty $Json "project_id" $StepName
    Assert-JsonHasProperty $Json "node_id" $StepName
    Assert-JsonHasProperty $Json "task_id" $StepName
    Assert-JsonHasProperty $Json "request_id" $StepName
    Assert-JsonHasProperty $Json "claim_id" $StepName
    Assert-JsonHasProperty $Json "claim_status" $StepName
    Assert-JsonHasProperty $Json "claimed_at_utc" $StepName
    Assert-JsonHasProperty $Json "safe_to_auto_execute" $StepName

    if ($Json.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($Json.node_id -ne $NodeId) {
        throw "$StepName returned node_id '$($Json.node_id)', expected '$NodeId'."
    }

    if ($Json.task_id -ne $ExpectedTaskId) {
        throw "$StepName returned task_id '$($Json.task_id)', expected '$ExpectedTaskId'."
    }

    if ($Json.request_id -ne $ExpectedRequestId) {
        throw "$StepName returned request_id '$($Json.request_id)', expected '$ExpectedRequestId'."
    }

    if ([string]::IsNullOrWhiteSpace([string]$Json.claim_id)) {
        throw "$StepName did not include a non-empty claim_id."
    }

    if ([string]::IsNullOrWhiteSpace([string]$Json.claim_status)) {
        throw "$StepName did not include a non-empty claim_status."
    }

    if ([string]::IsNullOrWhiteSpace([string]$Json.claimed_at_utc)) {
        throw "$StepName did not include a non-empty claimed_at_utc."
    }

    if ($Json.safe_to_auto_execute -ne $false) {
        throw "$StepName returned safe_to_auto_execute='$($Json.safe_to_auto_execute)', expected 'False'."
    }
}

function Assert-AssistedExecutionClaimListContract {
    param(
        $Json,
        [string]$StepName
    )

    Assert-JsonHasProperty $Json "project_id" $StepName
    Assert-JsonHasProperty $Json "node_id" $StepName
    Assert-JsonHasProperty $Json "count" $StepName
    Assert-JsonHasProperty $Json "claims" $StepName

    if ($Json.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($Json.node_id -ne $NodeId) {
        throw "$StepName returned node_id '$($Json.node_id)', expected '$NodeId'."
    }

    if ($Json.claims -isnot [System.Array] -and $Json.claims -isnot [System.Collections.IEnumerable]) {
        throw "$StepName did not return 'claims' as an array."
    }
}

function Assert-AssistedExecutionAttemptContract {
    param(
        $Json,
        [string]$StepName,
        [string]$ExpectedTaskId,
        [string]$ExpectedRequestId,
        [string]$ExpectedClaimId
    )

    Assert-JsonHasProperty $Json "project_id" $StepName
    Assert-JsonHasProperty $Json "node_id" $StepName
    Assert-JsonHasProperty $Json "task_id" $StepName
    Assert-JsonHasProperty $Json "request_id" $StepName
    Assert-JsonHasProperty $Json "claim_id" $StepName
    Assert-JsonHasProperty $Json "attempt_id" $StepName
    Assert-JsonHasProperty $Json "attempt_status" $StepName
    Assert-JsonHasProperty $Json "execution_mode" $StepName
    Assert-JsonHasProperty $Json "execution_result" $StepName
    Assert-JsonHasProperty $Json "safe_to_auto_execute" $StepName
    Assert-JsonHasProperty $Json "side_effects_performed" $StepName

    if ($Json.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($Json.node_id -ne $NodeId) {
        throw "$StepName returned node_id '$($Json.node_id)', expected '$NodeId'."
    }

    if ($Json.task_id -ne $ExpectedTaskId) {
        throw "$StepName returned task_id '$($Json.task_id)', expected '$ExpectedTaskId'."
    }

    if ($Json.request_id -ne $ExpectedRequestId) {
        throw "$StepName returned request_id '$($Json.request_id)', expected '$ExpectedRequestId'."
    }

    if ($Json.claim_id -ne $ExpectedClaimId) {
        throw "$StepName returned claim_id '$($Json.claim_id)', expected '$ExpectedClaimId'."
    }

    if ([string]::IsNullOrWhiteSpace([string]$Json.attempt_id)) {
        throw "$StepName did not include a non-empty attempt_id."
    }

    if ($Json.attempt_status -ne "recorded") {
        throw "$StepName returned attempt_status '$($Json.attempt_status)', expected 'recorded'."
    }

    if ($Json.execution_mode -ne "no_op") {
        throw "$StepName returned execution_mode '$($Json.execution_mode)', expected 'no_op'."
    }

    if ($Json.execution_result -ne "no_op_recorded") {
        throw "$StepName returned execution_result '$($Json.execution_result)', expected 'no_op_recorded'."
    }

    if ($Json.safe_to_auto_execute -ne $false) {
        throw "$StepName returned safe_to_auto_execute='$($Json.safe_to_auto_execute)', expected 'False'."
    }

    if ($Json.side_effects_performed -ne $false) {
        throw "$StepName returned side_effects_performed='$($Json.side_effects_performed)', expected 'False'."
    }
}

function Assert-AssistedExecutionAttemptListContract {
    param(
        $Json,
        [string]$StepName
    )

    Assert-JsonHasProperty $Json "project_id" $StepName
    Assert-JsonHasProperty $Json "node_id" $StepName
    Assert-JsonHasProperty $Json "count" $StepName
    Assert-JsonHasProperty $Json "attempts" $StepName

    if ($Json.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($Json.node_id -ne $NodeId) {
        throw "$StepName returned node_id '$($Json.node_id)', expected '$NodeId'."
    }

    if ($Json.attempts -isnot [System.Array] -and $Json.attempts -isnot [System.Collections.IEnumerable]) {
        throw "$StepName did not return 'attempts' as an array."
    }
}

function Assert-AssistedExecutionStateSummaryContract {
    param(
        $Json,
        [string]$StepName,
        [string]$ExpectedTaskId = $null,
        [switch]$RequireLocalMappingStatus
    )

    Assert-JsonHasProperty $Json "project_id" $StepName
    Assert-JsonHasProperty $Json "node_id" $StepName
    Assert-JsonHasProperty $Json "task_id" $StepName
    Assert-JsonHasProperty $Json "state" $StepName
    Assert-JsonHasProperty $Json "reason" $StepName
    Assert-JsonHasProperty $Json "safe_to_auto_execute" $StepName

    if ($Json.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($Json.node_id -ne $NodeId) {
        throw "$StepName returned node_id '$($Json.node_id)', expected '$NodeId'."
    }

    if (-not [string]::IsNullOrWhiteSpace($ExpectedTaskId) -and $Json.task_id -ne $ExpectedTaskId) {
        throw "$StepName returned task_id '$($Json.task_id)', expected '$ExpectedTaskId'."
    }

    if ([string]::IsNullOrWhiteSpace([string]$Json.task_id)) {
        throw "$StepName did not include a non-empty task_id."
    }

    if ([string]::IsNullOrWhiteSpace([string]$Json.state)) {
        throw "$StepName did not include a non-empty state."
    }

    if ([string]::IsNullOrWhiteSpace([string]$Json.reason)) {
        throw "$StepName did not include a non-empty reason."
    }

    if ($Json.safe_to_auto_execute -ne $false) {
        throw "$StepName returned safe_to_auto_execute='$($Json.safe_to_auto_execute)', expected 'False'."
    }

    if ($Json.PSObject.Properties.Name.Contains("side_effects_performed") -and
        $null -ne $Json.side_effects_performed -and
        $Json.side_effects_performed -ne $false) {
        throw "$StepName returned side_effects_performed='$($Json.side_effects_performed)', expected 'False' when present."
    }

    if ($RequireLocalMappingStatus) {
        Assert-JsonHasProperty $Json "local_mapping_status" $StepName

        $allowedLocalMappingStatuses = @(
            "already_mapped",
            "missing_mapping",
            "ambiguous",
            "not_applicable",
            "warning",
            "unknown",
            "no_report"
        )

        if ([string]::IsNullOrWhiteSpace([string]$Json.local_mapping_status)) {
            throw "$StepName did not include a non-empty local_mapping_status."
        }

        if ($allowedLocalMappingStatuses -notcontains [string]$Json.local_mapping_status) {
            throw "$StepName returned local_mapping_status '$($Json.local_mapping_status)', expected one of: $($allowedLocalMappingStatuses -join ', ')."
        }
    }
}

function Assert-AssistedExecutionStateSummaryListContract {
    param(
        $Json,
        [string]$StepName
    )

    Assert-JsonHasProperty $Json "project_id" $StepName
    Assert-JsonHasProperty $Json "node_id" $StepName
    Assert-JsonHasProperty $Json "count" $StepName
    Assert-JsonHasProperty $Json "summaries" $StepName

    if ($Json.project_id -ne $ProjectId) {
        throw "$StepName returned project_id '$($Json.project_id)', expected '$ProjectId'."
    }

    if ($Json.node_id -ne $NodeId) {
        throw "$StepName returned node_id '$($Json.node_id)', expected '$NodeId'."
    }

    if ($Json.summaries -isnot [System.Array] -and $Json.summaries -isnot [System.Collections.IEnumerable]) {
        throw "$StepName did not return 'summaries' as an array."
    }
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

Write-Step "Publish action results from DATUM without execution"
Run-CommandOrFail `
    -WorkingDirectory $DatumDir `
    -Command @("cargo", "run", "--", "dispatch", "--config", $ResolvedClientConfig, "--publish-results") `
    -StepName "DATUM action result publish" | Out-Null

if (-not $SkipServerChecks) {
    Write-Step "Read action results from server"

    $nodeActionResults = Invoke-JsonEndpoint `
        -Url "$ServerUrl/api/v1/nodes/$NodeId/action-results/latest" `
        -Headers $headers `
        -StepName "Node latest action-results"
    Assert-HttpOk $nodeActionResults "Node latest action-results"
    Assert-ActionResultNodeContract $nodeActionResults.Json

    $projectActionResults = Invoke-JsonEndpoint `
        -Url "$ServerUrl/api/v1/action-results/latest" `
        -Headers $headers `
        -StepName "Project latest action-results"
    Assert-HttpOk $projectActionResults "Project latest action-results"
    Assert-ActionResultProjectContract $projectActionResults.Json

    Write-Step "Read deployment planning from server"

    $globalDeploymentPlanning = Invoke-JsonEndpoint `
        -Url "$ServerUrl/api/v1/deployment-planning/latest" `
        -Headers $headers `
        -StepName "Global deployment planning"
    Assert-HttpOk $globalDeploymentPlanning "Global deployment planning"
    Assert-PlanningContract `
        -Json $globalDeploymentPlanning.Json `
        -PlanningProperty "deployment_planning" `
        -StepName "Global deployment planning"

    $nodeDeploymentPlanning = Invoke-JsonEndpoint `
        -Url "$ServerUrl/api/v1/deployment-planning/for-node/$NodeId" `
        -Headers $headers `
        -StepName "Node deployment planning"
    Assert-HttpOk $nodeDeploymentPlanning "Node deployment planning"
    Assert-PlanningContract `
        -Json $nodeDeploymentPlanning.Json `
        -PlanningProperty "deployment_planning" `
        -StepName "Node deployment planning" `
        -RequireNodeId
}

Write-Step "Read deployment planning from DATUM"
Run-CommandOrFail `
    -WorkingDirectory $DatumDir `
    -Command @("cargo", "run", "--", "deployment-planning", "--config", $ResolvedClientConfig) `
    -StepName "DATUM deployment planning viewer" | Out-Null

if (-not $SkipServerChecks) {
    Write-Step "Read installation plan from server"

    $globalInstallationPlan = Invoke-JsonEndpoint `
        -Url "$ServerUrl/api/v1/installation-plan/latest" `
        -Headers $headers `
        -StepName "Global installation plan"
    Assert-HttpOk $globalInstallationPlan "Global installation plan"
    Assert-PlanningContract `
        -Json $globalInstallationPlan.Json `
        -PlanningProperty "installation_plan" `
        -StepName "Global installation plan"

    $nodeInstallationPlan = Invoke-JsonEndpoint `
        -Url "$ServerUrl/api/v1/installation-plan/for-node/$NodeId" `
        -Headers $headers `
        -StepName "Node installation plan"
    Assert-HttpOk $nodeInstallationPlan "Node installation plan"
    Assert-PlanningContract `
        -Json $nodeInstallationPlan.Json `
        -PlanningProperty "installation_plan" `
        -StepName "Node installation plan" `
        -RequireNodeId
}

Write-Step "Read installation plan from DATUM"
Run-CommandOrFail `
    -WorkingDirectory $DatumDir `
    -Command @("cargo", "run", "--", "installation-plan", "--config", $ResolvedClientConfig) `
    -StepName "DATUM installation plan viewer" | Out-Null

Write-Step "Materialize installation tasks from DATUM"
Run-CommandOrFail `
    -WorkingDirectory $DatumDir `
    -Command @("cargo", "run", "--", "installation-tasks", "--config", $ResolvedClientConfig, "--materialize") `
    -StepName "DATUM installation task materialize" | Out-Null

Write-Step "Read installation tasks from DATUM"
$installationTasksOutput = Run-CommandOrFail `
    -WorkingDirectory $DatumDir `
    -Command @("cargo", "run", "--", "installation-tasks", "--config", $ResolvedClientConfig) `
    -StepName "DATUM installation task viewer"

$taskUpdatePerformed = $false
$policyValidated = $false
$taskCount = 0
$approvalCandidateDetected = $false
$approvalSubmissionPerformed = $false
$serverLatestApprovalReadPerformed = $false
$serverApprovalListReadPerformed = $false
$clientLatestApprovalReadPerformed = $false
$clientApprovalListReadPerformed = $false
$serverNodeReadinessListReadPerformed = $false
$clientNodeReadinessViewerPerformed = $false
$serverSingleTaskReadinessReadPerformed = $false
$clientSingleTaskReadinessViewerPerformed = $false
$safeToAutoExecuteFalseValidated = $false
$readinessReadyObserved = $false
$readyTaskDetected = $false
$assistedExecutionRequestSubmissionPerformed = $false
$serverLatestAssistedExecutionRequestReadPerformed = $false
$serverAssistedExecutionRequestListReadPerformed = $false
$clientLatestAssistedExecutionRequestReadPerformed = $false
$clientAssistedExecutionRequestListReadPerformed = $false
$requestedAssistedExecutionRequestDetected = $false
$assistedExecutionClaimSubmissionPerformed = $false
$assistedExecutionClaimAlreadyClaimed = $false
$serverLatestAssistedExecutionClaimReadPerformed = $false
$serverAssistedExecutionClaimListReadPerformed = $false
$clientLatestAssistedExecutionClaimReadPerformed = $false
$clientAssistedExecutionClaimListReadPerformed = $false
$assistedClaimLifecycleUpdatePerformed = $false
$assistedClaimLifecycleAlreadyReleased = $false
$serverLatestAssistedExecutionClaimLifecycleReadPerformed = $false
$clientLatestAssistedExecutionClaimLifecycleReadPerformed = $false
$assistedExecutionAttemptSubmissionPerformed = $false
$serverLatestAssistedExecutionAttemptReadPerformed = $false
$serverAssistedExecutionAttemptListReadPerformed = $false
$clientLatestAssistedExecutionAttemptReadPerformed = $false
$clientAssistedExecutionAttemptListReadPerformed = $false
$serverAssistedExecutionStateListReadPerformed = $false
$clientAssistedExecutionStateListReadPerformed = $false
$serverSingleTaskAssistedExecutionStateReadPerformed = $false
$clientSingleTaskAssistedExecutionStateReadPerformed = $false
$safeToAutoExecuteFalseValidatedForStateSummaries = $false
$sideEffectsPerformedFalseValidatedForStateSummaries = $false
$serverProjectLocalMappingRecommendationsReadPerformed = $false
$serverNodeLocalMappingRecommendationsReadPerformed = $false
$clientLocalMappingRecommendationViewerPerformed = $false
$clientLocalMappingPreflightPerformed = $false
$clientSingleTaskLocalMappingPreflightPerformed = $false
$clientLocalMappingPreflightPublishPerformed = $false
$serverNodeLatestLocalMappingPreflightReportReadPerformed = $false
$serverProjectLatestLocalMappingPreflightReportListReadPerformed = $false
$clientSingleTaskLocalMappingPreflightPublishPerformed = $false
$serverAssistedExecutionStateLocalMappingContextReadPerformed = $false
$serverSingleTaskAssistedExecutionStateLocalMappingContextReadPerformed = $false
$clientAssistedExecutionStateLocalMappingContextViewerPerformed = $false
$clientSingleTaskAssistedExecutionStateLocalMappingContextViewerPerformed = $false
$serverProjectLocalMappingPatchProposalsReadPerformed = $false
$serverNodeLocalMappingPatchProposalsReadPerformed = $false
$clientLocalMappingPatchProposalViewerPerformed = $false
$clientLocalMappingPatchProposalFileGenerationPerformed = $false
$generatedLocalMappingPatchProposalFileValidated = $false
$clientConfigUnchangedAfterPatchProposalGenerationValidated = $false
$clientLocalMappingPatchRevalidationPerformed = $false
$generatedLocalMappingPatchProposalFileRevalidated = $false
$clientConfigUnchangedAfterPatchRevalidationValidated = $false
$advisoryOnlyLocalMappingPatchProposalFileValidated = $false
$appliedFalseValidated = $false
$generatedSmokeArtifactCleanupPerformed = $false
$safeToAutoApplyFalseValidated = $false
$requiresOperatorTrueValidated = $false
$safeToAutoExecuteFalseValidatedForLocalMappingPreflight = $false
$sideEffectsPerformedFalseValidatedForLocalMappingPreflight = $false
$clientLocalWasmRuntimeAddTestPerformed = $false
$clientLocalWasmRuntimeJsonTestPerformed = $false
$clientWasmRuntimeReportPublishPerformed = $false
$serverNodeLatestWasmRuntimeReportReadPerformed = $false
$serverProjectLatestWasmRuntimeReportListReadPerformed = $false
$clientLatestWasmRuntimeReportViewerPerformed = $false
$clientLatestWasmRuntimeReportsViewerPerformed = $false
$serverWasmModuleDescriptorRegistrationPerformed = $false
$serverWasmModuleRegistryListReadPerformed = $false
$serverSingleWasmModuleDescriptorReadPerformed = $false
$clientWasmModuleRegistryViewerPerformed = $false
$clientSingleWasmModuleViewerPerformed = $false
$clientWasmModuleRegistryJsonViewerPerformed = $false
$clientSingleWasmModuleJsonViewerPerformed = $false
$safeAccessFlagsFalseValidatedForWasm = $false
$wasmArtifactMetadataOnlyValidated = $false
$selectedRequestTaskId = $null
$selectedRequestId = $null
$selectedRequestEncodedTaskId = $null
$selectedRequestEncodedRequestId = $null
$selectedClaimId = $null
$selectedClaimEncodedId = $null
$selectedClaimStatus = $null
$sideEffectsPerformedFalseValidated = $false
$serverProjectPlannerDryRunReadPerformed = $false
$serverProjectPlannerDryRunSummaryReadPerformed = $false
$clientPlannerDryRunViewerPerformed = $false
$clientPlannerSummaryViewerPerformed = $false
$clientPlannerDryRunJsonViewerPerformed = $false
$clientPlannerSummaryJsonViewerPerformed = $false
$sideEffectsPerformedFalseValidatedForDgraphPlanner = $false
$localFixtureScenarioJsonChecksPerformed = $false
$fixtureMetadataValidated = $false
$fixturePublishReadBackPerformed = $false
$materializedAfterFixturePublishValidated = $false
$plannerAfterFixturePublishValidated = $false
$sideEffectsPerformedFalseValidatedForDgraphFixtures = $false
if (-not $SkipServerChecks) {
    Write-Step "Read installation tasks from server"

    $nodeInstallationTasks = Invoke-JsonEndpoint `
        -Url "$ServerUrl/api/v1/installation-tasks/for-node/$NodeId" `
        -Headers $headers `
        -StepName "Node installation tasks"
    Assert-HttpOk $nodeInstallationTasks "Node installation tasks"
    Assert-InstallationTaskListContract `
        -Json $nodeInstallationTasks.Json `
        -StepName "Node installation tasks" `
        -RequireNodeId
    $taskCount = @($nodeInstallationTasks.Json.tasks).Count

    $projectInstallationTasks = Invoke-JsonEndpoint `
        -Url "$ServerUrl/api/v1/installation-tasks" `
        -Headers $headers `
        -StepName "Project installation tasks"
    Assert-HttpOk $projectInstallationTasks "Project installation tasks"
    Assert-InstallationTaskListContract `
        -Json $projectInstallationTasks.Json `
        -StepName "Project installation tasks"

    $firstPolicyTask = $nodeInstallationTasks.Json.tasks `
        | Where-Object { $null -ne $_.installation_policy } `
        | Select-Object -First 1
    if ($null -ne $firstPolicyTask) {
        $policyValidated = Assert-InstallationPolicyContract `
            -Json $firstPolicyTask `
            -StepName "Node installation tasks"
    }

    if ($taskCount -gt 0 -and $policyValidated) {
        if ($installationTasksOutput -notmatch "policy:" -and
            $installationTasksOutput -notmatch "candidate_for_assisted_execution" -and
            $installationTasksOutput -notmatch "manual_only" -and
            $installationTasksOutput -notmatch "blocked" -and
            $installationTasksOutput -notmatch "unknown") {
            throw "DATUM installation task viewer output did not include installation policy display markers."
        }
    }

    $firstTaskId = $nodeInstallationTasks.Json.tasks `
        | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_.task_id) } `
        | Select-Object -First 1 -ExpandProperty task_id

    if ([string]::IsNullOrWhiteSpace([string]$firstTaskId)) {
        Write-Host "No installation tasks were materialized for this node; skipping single-task get/status update."
    }
    else {
        $encodedTaskId = [System.Uri]::EscapeDataString($firstTaskId)

        Write-Step "Read one installation task from server"
        $singleInstallationTask = Invoke-JsonEndpoint `
            -Url "$ServerUrl/api/v1/installation-tasks/for-node/$NodeId/$encodedTaskId" `
            -Headers $headers `
            -StepName "Single installation task"
        Assert-HttpOk $singleInstallationTask "Single installation task"
        Assert-InstallationTaskContract `
            -Json $singleInstallationTask.Json `
            -StepName "Single installation task"
        $null = Assert-InstallationPolicyContract `
            -Json $singleInstallationTask.Json `
            -StepName "Single installation task"

        Write-Step "Acknowledge installation task from DATUM"
        Run-CommandOrFail `
            -WorkingDirectory $DatumDir `
            -Command @(
                "cargo",
                "run",
                "--",
                "installation-tasks",
                "--config",
                $ResolvedClientConfig,
                "--task-id",
                $firstTaskId,
                "--status",
                "acknowledged",
                "--reason",
                "smoke test acknowledged task"
            ) `
            -StepName "DATUM installation task acknowledge" | Out-Null

        Write-Step "Confirm acknowledged installation task from server"
        $acknowledgedInstallationTask = Invoke-JsonEndpoint `
            -Url "$ServerUrl/api/v1/installation-tasks/for-node/$NodeId/$encodedTaskId" `
            -Headers $headers `
            -StepName "Acknowledged installation task"
        Assert-HttpOk $acknowledgedInstallationTask "Acknowledged installation task"
        Assert-InstallationTaskContract `
            -Json $acknowledgedInstallationTask.Json `
            -StepName "Acknowledged installation task"
        $null = Assert-InstallationPolicyContract `
            -Json $acknowledgedInstallationTask.Json `
            -StepName "Acknowledged installation task"

        if ($acknowledgedInstallationTask.Json.status -ne "acknowledged") {
            throw "Acknowledged installation task returned status '$($acknowledgedInstallationTask.Json.status)', expected 'acknowledged'."
        }

        $taskUpdatePerformed = $true
    }

    $candidateTask = $nodeInstallationTasks.Json.tasks `
        | Where-Object {
            $null -ne $_.installation_policy -and
            $_.installation_policy.decision -eq "candidate_for_assisted_execution"
        } `
        | Select-Object -First 1

    if ($null -eq $candidateTask) {
        Write-Host "No candidate_for_assisted_execution task found; skipping operator approval checks."
    }
    else {
        $approvalCandidateDetected = $true
        $candidateTaskId = [string]$candidateTask.task_id
        $encodedCandidateTaskId = [System.Uri]::EscapeDataString($candidateTaskId)

        Write-Step "Submit installation task approval from DATUM"
        $approvalSubmitOutput = Run-CommandOrFail `
            -WorkingDirectory $DatumDir `
            -Command @(
                "cargo",
                "run",
                "--",
                "installation-tasks",
                "--config",
                $ResolvedClientConfig,
                "--task-id",
                $candidateTaskId,
                "--approve",
                "--approved-by",
                "smoke-test",
                "--reason",
                "smoke test operator approval"
            ) `
            -StepName "DATUM installation task approve"

        if ($approvalSubmitOutput -notmatch "Submitted installation task approval" -or
            $approvalSubmitOutput -notmatch "approval metadata was recorded on the server" -or
            $approvalSubmitOutput -notmatch "did not execute installation") {
            throw "DATUM installation task approve output did not confirm metadata-only approval submission."
        }
        $approvalSubmissionPerformed = $true

        Write-Step "Read latest installation task approval from server"
        $latestApproval = Invoke-JsonEndpoint `
            -Url "$ServerUrl/api/v1/installation-tasks/for-node/$NodeId/$encodedCandidateTaskId/approval/latest" `
            -Headers $headers `
            -StepName "Latest installation task approval"
        Assert-HttpOk $latestApproval "Latest installation task approval"
        Assert-InstallationTaskApprovalContract `
            -Json $latestApproval.Json `
            -StepName "Latest installation task approval" `
            -ExpectedTaskId $candidateTaskId
        $serverLatestApprovalReadPerformed = $true

        Write-Step "List installation task approvals from server"
        $approvalList = Invoke-JsonEndpoint `
            -Url "$ServerUrl/api/v1/installation-task-approvals/for-node/$NodeId" `
            -Headers $headers `
            -StepName "Installation task approvals for node"
        Assert-HttpOk $approvalList "Installation task approvals for node"
        Assert-InstallationTaskApprovalListContract `
            -Json $approvalList.Json `
            -StepName "Installation task approvals for node"
        $matchingApproval = @($approvalList.Json.approvals | Where-Object {
            $_.task_id -eq $candidateTaskId -and
            $_.approved -eq $true -and
            $_.approval_mode -eq "operator_approved"
        } | Select-Object -First 1)
        if ($matchingApproval.Count -eq 0) {
            throw "Installation task approvals for node did not include the submitted operator approval for task '$candidateTaskId'."
        }
        $serverApprovalListReadPerformed = $true

        Write-Step "Read latest installation task approval from DATUM"
        $clientLatestApprovalOutput = Run-CommandOrFail `
            -WorkingDirectory $DatumDir `
            -Command @(
                "cargo",
                "run",
                "--",
                "installation-tasks",
                "--config",
                $ResolvedClientConfig,
                "--task-id",
                $candidateTaskId,
                "--approval"
            ) `
            -StepName "DATUM latest installation task approval"

        if ($clientLatestApprovalOutput -notmatch [regex]::Escape("Latest approval for task $candidateTaskId") -or
            $clientLatestApprovalOutput -notmatch "approved:\s+true" -or
            $clientLatestApprovalOutput -notmatch "mode:\s+operator_approved") {
            throw "DATUM latest installation task approval output did not include the expected approval details."
        }
        $clientLatestApprovalReadPerformed = $true

        Write-Step "List installation task approvals from DATUM"
        $clientApprovalListOutput = Run-CommandOrFail `
            -WorkingDirectory $DatumDir `
            -Command @(
                "cargo",
                "run",
                "--",
                "installation-tasks",
                "--config",
                $ResolvedClientConfig,
                "--approvals"
            ) `
            -StepName "DATUM installation task approvals"

        if ($clientApprovalListOutput -notmatch [regex]::Escape("Installation task approvals for node $NodeId")) {
            throw "DATUM installation task approvals output did not include the node approval list header."
        }

        if ($clientApprovalListOutput -match "No installation task approvals for this node\." ) {
            throw "DATUM installation task approvals output reported no approvals after approval submission."
        }

        if ($clientApprovalListOutput -notmatch [regex]::Escape("task_id: $candidateTaskId")) {
            throw "DATUM installation task approvals output did not include the submitted approval task id."
        }
        $clientApprovalListReadPerformed = $true
    }

    Write-Step "Read installation task readiness list from server"
    $nodeReadiness = Invoke-JsonEndpoint `
        -Url "$ServerUrl/api/v1/installation-task-readiness/for-node/$NodeId" `
        -Headers $headers `
        -StepName "Installation task readiness for node"
    Assert-HttpOk $nodeReadiness "Installation task readiness for node"
    Assert-InstallationTaskReadinessListContract `
        -Json $nodeReadiness.Json `
        -StepName "Installation task readiness for node"
    $serverNodeReadinessListReadPerformed = $true

    $firstReadinessEntry = @($nodeReadiness.Json.readiness | Select-Object -First 1)
    if ($firstReadinessEntry.Count -gt 0) {
        Assert-JsonHasProperty $firstReadinessEntry[0] "task_id" "Installation task readiness for node"
        Assert-JsonHasProperty $firstReadinessEntry[0] "readiness" "Installation task readiness for node"
        Assert-JsonHasProperty $firstReadinessEntry[0] "reason" "Installation task readiness for node"
        Assert-JsonHasProperty $firstReadinessEntry[0] "safe_to_auto_execute" "Installation task readiness for node"
        if ($firstReadinessEntry[0].safe_to_auto_execute -ne $false) {
            throw "Installation task readiness for node returned safe_to_auto_execute='$($firstReadinessEntry[0].safe_to_auto_execute)', expected 'False'."
        }
        $safeToAutoExecuteFalseValidated = $true
    }

    Write-Step "Read installation task readiness from DATUM"
    $clientNodeReadinessOutput = Run-CommandOrFail `
        -WorkingDirectory $DatumDir `
        -Command @(
            "cargo",
            "run",
            "--",
            "installation-tasks",
            "--config",
            $ResolvedClientConfig,
            "--readiness"
        ) `
        -StepName "DATUM installation task readiness viewer"

    if ($clientNodeReadinessOutput -notmatch [regex]::Escape("Assisted installation readiness for node $NodeId") -and
        $clientNodeReadinessOutput -notmatch "No readiness decisions for this node\.") {
        throw "DATUM installation task readiness viewer output did not include readiness details or an empty-state message."
    }
    $clientNodeReadinessViewerPerformed = $true

    if ([string]::IsNullOrWhiteSpace([string]$firstTaskId)) {
        Write-Host "No installation task available; skipping single-task readiness checks."
    }
    else {
        Write-Step "Read one installation task readiness from server"
        $singleTaskReadiness = Invoke-JsonEndpoint `
            -Url "$ServerUrl/api/v1/installation-tasks/for-node/$NodeId/$encodedTaskId/readiness" `
            -Headers $headers `
            -StepName "Single installation task readiness"
        Assert-HttpOk $singleTaskReadiness "Single installation task readiness"
        Assert-InstallationTaskReadinessContract `
            -Json $singleTaskReadiness.Json `
            -StepName "Single installation task readiness" `
            -ExpectedTaskId $firstTaskId
        $serverSingleTaskReadinessReadPerformed = $true
        $safeToAutoExecuteFalseValidated = $true

        Write-Step "Read one installation task readiness from DATUM"
        $clientSingleTaskReadinessOutput = Run-CommandOrFail `
            -WorkingDirectory $DatumDir `
            -Command @(
                "cargo",
                "run",
                "--",
                "installation-tasks",
                "--config",
                $ResolvedClientConfig,
                "--task-id",
                $firstTaskId,
                "--readiness"
            ) `
            -StepName "DATUM single installation task readiness viewer"

        if ($clientSingleTaskReadinessOutput -notmatch [regex]::Escape("Readiness for task $firstTaskId") -or
            $clientSingleTaskReadinessOutput -notmatch "readiness:") {
            throw "DATUM single installation task readiness viewer output did not include readiness details."
        }
        $clientSingleTaskReadinessViewerPerformed = $true
    }

    $allReadinessEntries = @($nodeReadiness.Json.readiness)
    if ($serverSingleTaskReadinessReadPerformed) {
        $allReadinessEntries += @($singleTaskReadiness.Json)
    }
    if (@($allReadinessEntries | Where-Object { $_.readiness -eq "ready_for_assisted_execution" }).Count -gt 0) {
        $readinessReadyObserved = $true
        Write-Host "Readiness is informational only; no installation execution was performed."
    }

    $readyTask = $allReadinessEntries `
        | Where-Object {
            $_.readiness -eq "ready_for_assisted_execution" -and
            -not [string]::IsNullOrWhiteSpace([string]$_.task_id)
        } `
        | Select-Object -First 1

    if ($null -eq $readyTask) {
        Write-Host "No ready_for_assisted_execution task found; skipping assisted execution request checks."
    }
    else {
        $readyTaskDetected = $true
        $readyTaskId = [string]$readyTask.task_id
        $encodedReadyTaskId = [System.Uri]::EscapeDataString($readyTaskId)

        if ($readyTask.safe_to_auto_execute -ne $false) {
            throw "Ready installation task readiness returned safe_to_auto_execute='$($readyTask.safe_to_auto_execute)', expected 'False'."
        }
        $safeToAutoExecuteFalseValidated = $true

        Write-Step "Submit assisted execution request from DATUM"
        $requestSubmitOutput = Run-CommandOrFail `
            -WorkingDirectory $DatumDir `
            -Command @(
                "cargo",
                "run",
                "--",
                "installation-tasks",
                "--config",
                $ResolvedClientConfig,
                "--task-id",
                $readyTaskId,
                "--request-assisted-execution",
                "--requested-by",
                "smoke-test",
                "--reason",
                "smoke test assisted execution request"
            ) `
            -StepName "DATUM assisted execution request submit"

        if ($requestSubmitOutput -notmatch "Submitted assisted execution request" -or
            $requestSubmitOutput -notmatch "request metadata was recorded on the server" -or
            $requestSubmitOutput -notmatch "did not execute installation") {
            throw "DATUM assisted execution request submit output did not confirm metadata-only submission."
        }
        $assistedExecutionRequestSubmissionPerformed = $true

        Write-Step "Read latest assisted execution request from server"
        $latestAssistedExecutionRequest = Invoke-JsonEndpoint `
            -Url "$ServerUrl/api/v1/installation-tasks/for-node/$NodeId/$encodedReadyTaskId/assisted-execution-request/latest" `
            -Headers $headers `
            -StepName "Latest assisted execution request"
        Assert-HttpOk $latestAssistedExecutionRequest "Latest assisted execution request"
        Assert-AssistedExecutionRequestContract `
            -Json $latestAssistedExecutionRequest.Json `
            -StepName "Latest assisted execution request" `
            -ExpectedTaskId $readyTaskId
        $serverLatestAssistedExecutionRequestReadPerformed = $true
        $requestedAssistedExecutionRequestDetected = $true
        $safeToAutoExecuteFalseValidated = $true
        $selectedRequestTaskId = [string]$latestAssistedExecutionRequest.Json.task_id
        $selectedRequestId = [string]$latestAssistedExecutionRequest.Json.request_id
        $selectedRequestEncodedTaskId = [System.Uri]::EscapeDataString($selectedRequestTaskId)
        $selectedRequestEncodedRequestId = [System.Uri]::EscapeDataString($selectedRequestId)

        Write-Step "List assisted execution requests from server"
        $assistedExecutionRequestList = Invoke-JsonEndpoint `
            -Url "$ServerUrl/api/v1/assisted-execution-requests/for-node/$NodeId" `
            -Headers $headers `
            -StepName "Assisted execution requests for node"
        Assert-HttpOk $assistedExecutionRequestList "Assisted execution requests for node"
        Assert-AssistedExecutionRequestListContract `
            -Json $assistedExecutionRequestList.Json `
            -StepName "Assisted execution requests for node"
        $matchingAssistedExecutionRequest = @($assistedExecutionRequestList.Json.requests | Where-Object {
            $_.task_id -eq $readyTaskId -and
            $_.request_status -eq "requested" -and
            $_.execution_mode -eq "assisted_operator_approved" -and
            $_.safe_to_auto_execute -eq $false
        } | Select-Object -First 1)
        if ($matchingAssistedExecutionRequest.Count -eq 0) {
            throw "Assisted execution requests for node did not include the submitted request for task '$readyTaskId'."
        }
        $serverAssistedExecutionRequestListReadPerformed = $true
        $requestedAssistedExecutionRequestDetected = $true
        $safeToAutoExecuteFalseValidated = $true

        Write-Step "Read latest assisted execution request from DATUM"
        $clientLatestAssistedExecutionRequestOutput = Run-CommandOrFail `
            -WorkingDirectory $DatumDir `
            -Command @(
                "cargo",
                "run",
                "--",
                "installation-tasks",
                "--config",
                $ResolvedClientConfig,
                "--task-id",
                $readyTaskId,
                "--execution-request"
            ) `
            -StepName "DATUM latest assisted execution request"

        if ($clientLatestAssistedExecutionRequestOutput -notmatch [regex]::Escape("Latest assisted execution request for task $readyTaskId") -or
            $clientLatestAssistedExecutionRequestOutput -notmatch "status:\s+requested" -or
            $clientLatestAssistedExecutionRequestOutput -notmatch "safe to auto execute:\s+false") {
            throw "DATUM latest assisted execution request output did not include the expected request details."
        }
        $clientLatestAssistedExecutionRequestReadPerformed = $true

        Write-Step "List assisted execution requests from DATUM"
        $clientAssistedExecutionRequestListOutput = Run-CommandOrFail `
            -WorkingDirectory $DatumDir `
            -Command @(
                "cargo",
                "run",
                "--",
                "installation-tasks",
                "--config",
                $ResolvedClientConfig,
                "--execution-requests"
            ) `
            -StepName "DATUM assisted execution request list"

        if ($clientAssistedExecutionRequestListOutput -notmatch [regex]::Escape("Assisted execution requests for node $NodeId")) {
            throw "DATUM assisted execution request list output did not include the node request list header."
        }

        if ($clientAssistedExecutionRequestListOutput -match "No assisted execution requests for this node\.") {
            throw "DATUM assisted execution request list output reported no requests after request submission."
        }

        if ($clientAssistedExecutionRequestListOutput -notmatch [regex]::Escape("task_id: $readyTaskId")) {
            throw "DATUM assisted execution request list output did not include the submitted request task id."
        }
        $clientAssistedExecutionRequestListReadPerformed = $true
        Write-Host "Assisted execution request is metadata only; no installation execution was performed."
    }

    if ([string]::IsNullOrWhiteSpace([string]$selectedRequestTaskId) -or
        [string]::IsNullOrWhiteSpace([string]$selectedRequestId)) {
        if (-not $serverAssistedExecutionRequestListReadPerformed) {
            Write-Step "List assisted execution requests from server"
            $assistedExecutionRequestList = Invoke-JsonEndpoint `
                -Url "$ServerUrl/api/v1/assisted-execution-requests/for-node/$NodeId" `
                -Headers $headers `
                -StepName "Assisted execution requests for node"
            Assert-HttpOk $assistedExecutionRequestList "Assisted execution requests for node"
            Assert-AssistedExecutionRequestListContract `
                -Json $assistedExecutionRequestList.Json `
                -StepName "Assisted execution requests for node"
            $serverAssistedExecutionRequestListReadPerformed = $true
        }

        $fallbackRequestedAssistedExecutionRequest = @($assistedExecutionRequestList.Json.requests | Where-Object {
            $_.request_status -eq "requested" -and
            -not [string]::IsNullOrWhiteSpace([string]$_.task_id) -and
            -not [string]::IsNullOrWhiteSpace([string]$_.request_id)
        } | Select-Object -First 1)

        if ($fallbackRequestedAssistedExecutionRequest.Count -gt 0) {
            $requestedAssistedExecutionRequestDetected = $true
            $selectedRequestTaskId = [string]$fallbackRequestedAssistedExecutionRequest[0].task_id
            $selectedRequestId = [string]$fallbackRequestedAssistedExecutionRequest[0].request_id
            $selectedRequestEncodedTaskId = [System.Uri]::EscapeDataString($selectedRequestTaskId)
            $selectedRequestEncodedRequestId = [System.Uri]::EscapeDataString($selectedRequestId)
        }
    }

    if ([string]::IsNullOrWhiteSpace([string]$selectedRequestTaskId) -or
        [string]::IsNullOrWhiteSpace([string]$selectedRequestId)) {
        Write-Host "No requested assisted execution request found; skipping assisted execution claim checks."
    }
    else {
        Write-Step "Submit assisted execution claim from DATUM"
        $claimSubmitOutput = $null
        $claimSubmitFailed = $false
        try {
            $claimSubmitOutput = Run-CommandOrFail `
                -WorkingDirectory $DatumDir `
                -Command @(
                    "cargo",
                    "run",
                    "--",
                    "installation-tasks",
                    "--config",
                    $ResolvedClientConfig,
                    "--task-id",
                    $selectedRequestTaskId,
                    "--request-id",
                    $selectedRequestId,
                    "--claim-assisted-execution",
                    "--claimed-by",
                    "smoke-test",
                    "--reason",
                    "smoke test assisted execution claim"
                ) `
                -StepName "DATUM assisted execution claim submit"
        }
        catch {
            $claimSubmitFailed = $true
            $claimSubmitOutput = $_.Exception.Message
        }

        if (-not $claimSubmitFailed) {
            if ($claimSubmitOutput -notmatch "Submitted assisted execution claim" -or
                $claimSubmitOutput -notmatch "claim metadata was recorded on the server" -or
                $claimSubmitOutput -notmatch "did not execute installation") {
                throw "DATUM assisted execution claim submit output did not confirm metadata-only submission."
            }
            $assistedExecutionClaimSubmissionPerformed = $true
        }

        Write-Step "Read latest assisted execution claim from server"
        $latestAssistedExecutionClaim = Invoke-JsonEndpoint `
            -Url "$ServerUrl/api/v1/installation-tasks/for-node/$NodeId/$selectedRequestEncodedTaskId/assisted-execution-request/$selectedRequestEncodedRequestId/claim/latest" `
            -Headers $headers `
            -StepName "Latest assisted execution claim"

        if ($claimSubmitFailed) {
            if ($claimSubmitOutput -match "already claimed" -and $latestAssistedExecutionClaim.StatusCode -eq 200) {
                Write-Host "Assisted execution request was already claimed; validating latest claim instead."
                $assistedExecutionClaimAlreadyClaimed = $true
            }
            else {
                throw $claimSubmitOutput
            }
        }

        Assert-HttpOk $latestAssistedExecutionClaim "Latest assisted execution claim"
        Assert-AssistedExecutionClaimContract `
            -Json $latestAssistedExecutionClaim.Json `
            -StepName "Latest assisted execution claim" `
            -ExpectedTaskId $selectedRequestTaskId `
            -ExpectedRequestId $selectedRequestId
        $serverLatestAssistedExecutionClaimReadPerformed = $true
        $safeToAutoExecuteFalseValidated = $true
        $selectedClaimId = [string]$latestAssistedExecutionClaim.Json.claim_id
        if (-not [string]::IsNullOrWhiteSpace([string]$selectedClaimId)) {
            $selectedClaimEncodedId = [System.Uri]::EscapeDataString($selectedClaimId)
        }

        Write-Step "List assisted execution claims from server"
        $assistedExecutionClaimList = Invoke-JsonEndpoint `
            -Url "$ServerUrl/api/v1/assisted-execution-claims/for-node/$NodeId" `
            -Headers $headers `
            -StepName "Assisted execution claims for node"
        Assert-HttpOk $assistedExecutionClaimList "Assisted execution claims for node"
        Assert-AssistedExecutionClaimListContract `
            -Json $assistedExecutionClaimList.Json `
            -StepName "Assisted execution claims for node"
        $matchingAssistedExecutionClaim = @($assistedExecutionClaimList.Json.claims | Where-Object {
            $_.task_id -eq $selectedRequestTaskId -and
            $_.request_id -eq $selectedRequestId -and
            -not [string]::IsNullOrWhiteSpace([string]$_.claim_id) -and
            $_.safe_to_auto_execute -eq $false
        } | Select-Object -First 1)
        if ($matchingAssistedExecutionClaim.Count -eq 0) {
            throw "Assisted execution claims for node did not include a latest claim for task '$selectedRequestTaskId' request '$selectedRequestId'."
        }
        $serverAssistedExecutionClaimListReadPerformed = $true
        $safeToAutoExecuteFalseValidated = $true
        if ([string]::IsNullOrWhiteSpace([string]$selectedClaimId)) {
            $selectedClaimId = [string]$matchingAssistedExecutionClaim[0].claim_id
            if (-not [string]::IsNullOrWhiteSpace([string]$selectedClaimId)) {
                $selectedClaimEncodedId = [System.Uri]::EscapeDataString($selectedClaimId)
            }
        }

        Write-Step "Read latest assisted execution claim from DATUM"
        $clientLatestAssistedExecutionClaimOutput = Run-CommandOrFail `
            -WorkingDirectory $DatumDir `
            -Command @(
                "cargo",
                "run",
                "--",
                "installation-tasks",
                "--config",
                $ResolvedClientConfig,
                "--task-id",
                $selectedRequestTaskId,
                "--request-id",
                $selectedRequestId,
                "--execution-claim"
            ) `
            -StepName "DATUM latest assisted execution claim"

        if ($clientLatestAssistedExecutionClaimOutput -notmatch "Latest assisted execution claim" -or
            $clientLatestAssistedExecutionClaimOutput -notmatch "safe to auto execute:\s+false") {
            throw "DATUM latest assisted execution claim output did not include the expected claim details."
        }
        $clientLatestAssistedExecutionClaimReadPerformed = $true

        Write-Step "List assisted execution claims from DATUM"
        $clientAssistedExecutionClaimListOutput = Run-CommandOrFail `
            -WorkingDirectory $DatumDir `
            -Command @(
                "cargo",
                "run",
                "--",
                "installation-tasks",
                "--config",
                $ResolvedClientConfig,
                "--execution-claims"
            ) `
            -StepName "DATUM assisted execution claim list"

        if ($clientAssistedExecutionClaimListOutput -notmatch [regex]::Escape("Assisted execution claims for node $NodeId") -and
            $clientAssistedExecutionClaimListOutput -notmatch "No assisted execution claims for this node\.") {
            throw "DATUM assisted execution claim list output did not include claim details or an empty-state message."
        }

        if ($clientAssistedExecutionClaimListOutput -match "No assisted execution claims for this node\.") {
            throw "DATUM assisted execution claim list output reported no claims after claim validation."
        }

        if ($clientAssistedExecutionClaimListOutput -notmatch [regex]::Escape("task_id: $selectedRequestTaskId")) {
            throw "DATUM assisted execution claim list output did not include the selected claim task id."
        }
        $clientAssistedExecutionClaimListReadPerformed = $true

        $claimStatusForLifecycle = $null
        if ($latestAssistedExecutionClaim.Json.PSObject.Properties.Name.Contains("claim_status")) {
            $claimStatusForLifecycle = [string]$latestAssistedExecutionClaim.Json.claim_status
        }
        $selectedClaimStatus = $claimStatusForLifecycle

        if ($claimStatusForLifecycle -eq "claimed") {
            Write-Step "Update assisted execution claim status from DATUM"
            $claimLifecycleUpdateOutput = Run-CommandOrFail `
                -WorkingDirectory $DatumDir `
                -Command @(
                    "cargo",
                    "run",
                    "--",
                    "installation-tasks",
                    "--config",
                    $ResolvedClientConfig,
                    "--task-id",
                    $selectedRequestTaskId,
                    "--request-id",
                    $selectedRequestId,
                    "--claim-status",
                    "released",
                    "--reason",
                    "smoke test released claim metadata"
                ) `
                -StepName "DATUM assisted execution claim lifecycle update"

            if ($claimLifecycleUpdateOutput -notmatch "Updated assisted execution claim" -or
                $claimLifecycleUpdateOutput -notmatch "claim metadata was updated on the server" -or
                $claimLifecycleUpdateOutput -notmatch "did not execute installation") {
                throw "DATUM assisted execution claim lifecycle update output did not confirm metadata-only update."
            }
            $assistedClaimLifecycleUpdatePerformed = $true

            Write-Step "Read updated latest assisted execution claim from server"
            $updatedLatestAssistedExecutionClaim = Invoke-JsonEndpoint `
                -Url "$ServerUrl/api/v1/installation-tasks/for-node/$NodeId/$selectedRequestEncodedTaskId/assisted-execution-request/$selectedRequestEncodedRequestId/claim/latest" `
                -Headers $headers `
                -StepName "Updated latest assisted execution claim"
            Assert-HttpOk $updatedLatestAssistedExecutionClaim "Updated latest assisted execution claim"
            Assert-AssistedExecutionClaimContract `
                -Json $updatedLatestAssistedExecutionClaim.Json `
                -StepName "Updated latest assisted execution claim" `
                -ExpectedTaskId $selectedRequestTaskId `
                -ExpectedRequestId $selectedRequestId

            if ($updatedLatestAssistedExecutionClaim.Json.claim_status -ne "released") {
                throw "Updated latest assisted execution claim returned claim_status '$($updatedLatestAssistedExecutionClaim.Json.claim_status)', expected 'released'."
            }

            if ($updatedLatestAssistedExecutionClaim.Json.safe_to_auto_execute -ne $false) {
                throw "Updated latest assisted execution claim returned safe_to_auto_execute='$($updatedLatestAssistedExecutionClaim.Json.safe_to_auto_execute)', expected 'False'."
            }
            $serverLatestAssistedExecutionClaimLifecycleReadPerformed = $true
            $safeToAutoExecuteFalseValidated = $true
            $selectedClaimStatus = [string]$updatedLatestAssistedExecutionClaim.Json.claim_status
            $selectedClaimId = [string]$updatedLatestAssistedExecutionClaim.Json.claim_id
            if (-not [string]::IsNullOrWhiteSpace([string]$selectedClaimId)) {
                $selectedClaimEncodedId = [System.Uri]::EscapeDataString($selectedClaimId)
            }

            Write-Step "Read updated latest assisted execution claim from DATUM"
            $clientUpdatedLatestAssistedExecutionClaimOutput = Run-CommandOrFail `
                -WorkingDirectory $DatumDir `
                -Command @(
                    "cargo",
                    "run",
                    "--",
                    "installation-tasks",
                    "--config",
                    $ResolvedClientConfig,
                    "--task-id",
                    $selectedRequestTaskId,
                    "--request-id",
                    $selectedRequestId,
                    "--execution-claim"
                ) `
                -StepName "DATUM updated latest assisted execution claim"

            if ($clientUpdatedLatestAssistedExecutionClaimOutput -notmatch "Latest assisted execution claim" -or
                $clientUpdatedLatestAssistedExecutionClaimOutput -notmatch "status:\s+released" -or
                $clientUpdatedLatestAssistedExecutionClaimOutput -notmatch "safe to auto execute:\s+false") {
                throw "DATUM updated latest assisted execution claim output did not include the expected released claim details."
            }
            $clientLatestAssistedExecutionClaimLifecycleReadPerformed = $true
            Write-Host "Assisted claim lifecycle update is metadata only; no installation execution was performed."
        }
        elseif ($claimStatusForLifecycle -eq "released") {
            Write-Host "Assisted execution claim is already released; validating latest claim instead of updating lifecycle."
            if ($latestAssistedExecutionClaim.Json.safe_to_auto_execute -ne $false) {
                throw "Latest assisted execution claim returned safe_to_auto_execute='$($latestAssistedExecutionClaim.Json.safe_to_auto_execute)', expected 'False'."
            }
            $assistedClaimLifecycleAlreadyReleased = $true
            $serverLatestAssistedExecutionClaimLifecycleReadPerformed = $true
            $clientLatestAssistedExecutionClaimLifecycleReadPerformed = $true
            $safeToAutoExecuteFalseValidated = $true
            $selectedClaimStatus = "released"
        }
        else {
            Write-Host "No claimed assisted execution claim found; skipping claim lifecycle update checks."
        }

        if ([string]::IsNullOrWhiteSpace([string]$selectedClaimId)) {
            Write-Host "No claimed assisted execution claim found; skipping assisted execution attempt checks."
        }
        elseif ($selectedClaimStatus -eq "released") {
            Write-Host "Latest claim is released; skipping assisted execution attempt checks because attempts require a claimed claim."
        }
        elseif ($selectedClaimStatus -ne "claimed") {
            Write-Host "No claimed assisted execution claim found; skipping assisted execution attempt checks."
        }
        else {
            Write-Step "Submit no-op assisted execution attempt from DATUM"
            $attemptSubmitOutput = Run-CommandOrFail `
                -WorkingDirectory $DatumDir `
                -Command @(
                    "cargo",
                    "run",
                    "--",
                    "installation-tasks",
                    "--config",
                    $ResolvedClientConfig,
                    "--task-id",
                    $selectedRequestTaskId,
                    "--request-id",
                    $selectedRequestId,
                    "--claim-id",
                    $selectedClaimId,
                    "--attempt-assisted-execution",
                    "--execution-mode",
                    "no_op",
                    "--attempted-by",
                    "smoke-test",
                    "--message",
                    "smoke test no-op assisted execution attempt"
                ) `
                -StepName "DATUM assisted execution attempt submit"

            if ($attemptSubmitOutput -notmatch "Submitted assisted execution attempt" -or
                $attemptSubmitOutput -notmatch "attempt metadata was recorded on the server" -or
                $attemptSubmitOutput -notmatch "did not execute installation") {
                throw "DATUM assisted execution attempt submit output did not confirm metadata-only submission."
            }
            $assistedExecutionAttemptSubmissionPerformed = $true

            Write-Step "Read latest assisted execution attempt from server"
            $latestAssistedExecutionAttempt = Invoke-JsonEndpoint `
                -Url "$ServerUrl/api/v1/installation-tasks/for-node/$NodeId/$selectedRequestEncodedTaskId/assisted-execution-request/$selectedRequestEncodedRequestId/claim/$selectedClaimEncodedId/attempt/latest" `
                -Headers $headers `
                -StepName "Latest assisted execution attempt"
            Assert-HttpOk $latestAssistedExecutionAttempt "Latest assisted execution attempt"
            Assert-AssistedExecutionAttemptContract `
                -Json $latestAssistedExecutionAttempt.Json `
                -StepName "Latest assisted execution attempt" `
                -ExpectedTaskId $selectedRequestTaskId `
                -ExpectedRequestId $selectedRequestId `
                -ExpectedClaimId $selectedClaimId
            $serverLatestAssistedExecutionAttemptReadPerformed = $true
            $safeToAutoExecuteFalseValidated = $true
            $sideEffectsPerformedFalseValidated = $true

            Write-Step "List assisted execution attempts from server"
            $assistedExecutionAttemptList = Invoke-JsonEndpoint `
                -Url "$ServerUrl/api/v1/assisted-execution-attempts/for-node/$NodeId" `
                -Headers $headers `
                -StepName "Assisted execution attempts for node"
            Assert-HttpOk $assistedExecutionAttemptList "Assisted execution attempts for node"
            Assert-AssistedExecutionAttemptListContract `
                -Json $assistedExecutionAttemptList.Json `
                -StepName "Assisted execution attempts for node"
            $matchingAssistedExecutionAttempt = @($assistedExecutionAttemptList.Json.attempts | Where-Object {
                $_.task_id -eq $selectedRequestTaskId -and
                $_.request_id -eq $selectedRequestId -and
                $_.claim_id -eq $selectedClaimId -and
                $_.attempt_status -eq "recorded" -and
                $_.execution_mode -eq "no_op" -and
                $_.execution_result -eq "no_op_recorded" -and
                $_.safe_to_auto_execute -eq $false -and
                $_.side_effects_performed -eq $false
            } | Select-Object -First 1)
            if ($matchingAssistedExecutionAttempt.Count -eq 0) {
                throw "Assisted execution attempts for node did not include the expected latest attempt for task '$selectedRequestTaskId' request '$selectedRequestId' claim '$selectedClaimId'."
            }
            $serverAssistedExecutionAttemptListReadPerformed = $true
            $safeToAutoExecuteFalseValidated = $true
            $sideEffectsPerformedFalseValidated = $true

            Write-Step "Read latest assisted execution attempt from DATUM"
            $clientLatestAssistedExecutionAttemptOutput = Run-CommandOrFail `
                -WorkingDirectory $DatumDir `
                -Command @(
                    "cargo",
                    "run",
                    "--",
                    "installation-tasks",
                    "--config",
                    $ResolvedClientConfig,
                    "--task-id",
                    $selectedRequestTaskId,
                    "--request-id",
                    $selectedRequestId,
                    "--claim-id",
                    $selectedClaimId,
                    "--execution-attempt"
                ) `
                -StepName "DATUM latest assisted execution attempt"

            if ($clientLatestAssistedExecutionAttemptOutput -notmatch "Latest assisted execution attempt" -or
                $clientLatestAssistedExecutionAttemptOutput -notmatch "safe to auto execute:\s+false" -or
                $clientLatestAssistedExecutionAttemptOutput -notmatch "side effects performed:\s+false") {
                throw "DATUM latest assisted execution attempt output did not include the expected attempt details."
            }
            $clientLatestAssistedExecutionAttemptReadPerformed = $true

            Write-Step "List assisted execution attempts from DATUM"
            $clientAssistedExecutionAttemptListOutput = Run-CommandOrFail `
                -WorkingDirectory $DatumDir `
                -Command @(
                    "cargo",
                    "run",
                    "--",
                    "installation-tasks",
                    "--config",
                    $ResolvedClientConfig,
                    "--execution-attempts"
                ) `
                -StepName "DATUM assisted execution attempt list"

            if ($clientAssistedExecutionAttemptListOutput -notmatch [regex]::Escape("Assisted execution attempts for node $NodeId") -and
                $clientAssistedExecutionAttemptListOutput -notmatch "No assisted execution attempts for this node\.") {
                throw "DATUM assisted execution attempt list output did not include attempt details or an empty-state message."
            }

            if ($clientAssistedExecutionAttemptListOutput -match "No assisted execution attempts for this node\.") {
                throw "DATUM assisted execution attempt list output reported no attempts after attempt submission."
            }

            if ($clientAssistedExecutionAttemptListOutput -notmatch [regex]::Escape("task_id: $selectedRequestTaskId")) {
                throw "DATUM assisted execution attempt list output did not include the selected attempt task id."
            }
            $clientAssistedExecutionAttemptListReadPerformed = $true
            Write-Host "Assisted execution attempt is metadata only; no installation execution was performed."
        }

        Write-Host "Assisted execution claim is metadata only; no installation execution was performed."

        Write-Step "Read assisted execution state list from server"
        $assistedExecutionStateList = Invoke-JsonEndpoint `
            -Url "$ServerUrl/api/v1/assisted-execution-state/for-node/$NodeId" `
            -Headers $headers `
            -StepName "Assisted execution state for node"
        Assert-HttpOk $assistedExecutionStateList "Assisted execution state for node"
        Assert-AssistedExecutionStateSummaryListContract `
            -Json $assistedExecutionStateList.Json `
            -StepName "Assisted execution state for node"
        $firstAssistedExecutionStateSummary = @($assistedExecutionStateList.Json.summaries | Select-Object -First 1)
        if ($firstAssistedExecutionStateSummary.Count -gt 0) {
            Assert-AssistedExecutionStateSummaryContract `
                -Json $firstAssistedExecutionStateSummary[0] `
                -StepName "Assisted execution state for node"
            $safeToAutoExecuteFalseValidated = $true
            $safeToAutoExecuteFalseValidatedForStateSummaries = $true
            if ($firstAssistedExecutionStateSummary[0].PSObject.Properties.Name.Contains("side_effects_performed") -and
                $null -ne $firstAssistedExecutionStateSummary[0].side_effects_performed) {
                $sideEffectsPerformedFalseValidated = $true
                $sideEffectsPerformedFalseValidatedForStateSummaries = $true
            }
        }
        $serverAssistedExecutionStateListReadPerformed = $true

        Write-Step "Read assisted execution state from DATUM"
        $clientAssistedExecutionStateListOutput = Run-CommandOrFail `
            -WorkingDirectory $DatumDir `
            -Command @(
                "cargo",
                "run",
                "--",
                "installation-tasks",
                "--config",
                $ResolvedClientConfig,
                "--execution-state"
            ) `
            -StepName "DATUM assisted execution state viewer"

        if ($clientAssistedExecutionStateListOutput -notmatch [regex]::Escape("Assisted execution state for node $NodeId") -and
            $clientAssistedExecutionStateListOutput -notmatch "No assisted execution state summaries for this node\.") {
            throw "DATUM assisted execution state viewer output did not include state summary details or an empty-state message."
        }
        $clientAssistedExecutionStateListReadPerformed = $true

        if ([string]::IsNullOrWhiteSpace([string]$encodedTaskId)) {
            Write-Host "No installation task available; skipping single-task assisted execution state checks."
        }
        else {
            Write-Step "Read one assisted execution state summary from server"
            $singleAssistedExecutionState = Invoke-JsonEndpoint `
                -Url "$ServerUrl/api/v1/installation-tasks/for-node/$NodeId/$encodedTaskId/assisted-execution-state" `
                -Headers $headers `
                -StepName "Single assisted execution state"
            Assert-HttpOk $singleAssistedExecutionState "Single assisted execution state"
            Assert-AssistedExecutionStateSummaryContract `
                -Json $singleAssistedExecutionState.Json `
                -StepName "Single assisted execution state" `
                -ExpectedTaskId $selectedTask.task_id
            $serverSingleTaskAssistedExecutionStateReadPerformed = $true
            $safeToAutoExecuteFalseValidated = $true
            $safeToAutoExecuteFalseValidatedForStateSummaries = $true
            if ($singleAssistedExecutionState.Json.PSObject.Properties.Name.Contains("side_effects_performed") -and
                $null -ne $singleAssistedExecutionState.Json.side_effects_performed) {
                $sideEffectsPerformedFalseValidated = $true
                $sideEffectsPerformedFalseValidatedForStateSummaries = $true
            }

            Write-Step "Read one assisted execution state summary from DATUM"
            $clientSingleAssistedExecutionStateOutput = Run-CommandOrFail `
                -WorkingDirectory $DatumDir `
                -Command @(
                    "cargo",
                    "run",
                    "--",
                    "installation-tasks",
                    "--config",
                    $ResolvedClientConfig,
                    "--task-id",
                    $selectedTask.task_id,
                    "--execution-state"
                ) `
                -StepName "DATUM single assisted execution state viewer"

            if ($clientSingleAssistedExecutionStateOutput -notmatch [regex]::Escape("Assisted execution state for task $($selectedTask.task_id)")) {
                throw "DATUM single assisted execution state viewer output did not include the expected task header."
            }

            if ($clientSingleAssistedExecutionStateOutput -notmatch "safe to auto execute:\s+false") {
                throw "DATUM single assisted execution state viewer output did not include 'safe to auto execute: false'."
            }

            if ($clientSingleAssistedExecutionStateOutput -match "side effects performed:") {
                if ($clientSingleAssistedExecutionStateOutput -notmatch "side effects performed:\s+false") {
                    throw "DATUM single assisted execution state viewer output did not include 'side effects performed: false' when side effects were shown."
                }
            }
            $clientSingleTaskAssistedExecutionStateReadPerformed = $true
        }

        Write-Step "Read project local mapping recommendations from server"
        $projectLocalMappingRecommendations = Invoke-JsonEndpoint `
            -Url "$ServerUrl/api/v1/local-mapping-recommendations" `
            -Headers $headers `
            -StepName "Project local mapping recommendations"
        Assert-HttpOk $projectLocalMappingRecommendations "Project local mapping recommendations"
        Assert-LocalMappingRecommendationListContract `
            -Json $projectLocalMappingRecommendations.Json `
            -StepName "Project local mapping recommendations"
        $firstProjectLocalMappingRecommendation = @(
            $projectLocalMappingRecommendations.Json.recommendations | Select-Object -First 1
        )
        if ($firstProjectLocalMappingRecommendation.Count -gt 0) {
            Assert-LocalMappingRecommendationSafetyContract `
                -Recommendation $firstProjectLocalMappingRecommendation[0] `
                -StepName "Project local mapping recommendations"
            $safeToAutoApplyFalseValidated = $true
        }
        $serverProjectLocalMappingRecommendationsReadPerformed = $true

        Write-Step "Read node local mapping recommendations from server"
        $nodeLocalMappingRecommendations = Invoke-JsonEndpoint `
            -Url "$ServerUrl/api/v1/local-mapping-recommendations/for-node/$NodeId" `
            -Headers $headers `
            -StepName "Node local mapping recommendations"
        Assert-HttpOk $nodeLocalMappingRecommendations "Node local mapping recommendations"
        Assert-LocalMappingRecommendationListContract `
            -Json $nodeLocalMappingRecommendations.Json `
            -StepName "Node local mapping recommendations" `
            -RequireNodeId
        $firstNodeLocalMappingRecommendation = @(
            $nodeLocalMappingRecommendations.Json.recommendations | Select-Object -First 1
        )
        if ($firstNodeLocalMappingRecommendation.Count -gt 0) {
            Assert-LocalMappingRecommendationSafetyContract `
                -Recommendation $firstNodeLocalMappingRecommendation[0] `
                -StepName "Node local mapping recommendations"
            $safeToAutoApplyFalseValidated = $true
        }
        $serverNodeLocalMappingRecommendationsReadPerformed = $true

        Write-Step "Read local mapping recommendations from DATUM"
        $clientLocalMappingRecommendationOutput = Run-CommandOrFail `
            -WorkingDirectory $DatumDir `
            -Command @(
                "cargo",
                "run",
                "--",
                "installation-tasks",
                "--config",
                $ResolvedClientConfig,
                "--local-mapping-recommendations"
            ) `
            -StepName "DATUM local mapping recommendation viewer"

        if ($clientLocalMappingRecommendationOutput -notmatch [regex]::Escape("Local mapping recommendations for node $NodeId") -and
            $clientLocalMappingRecommendationOutput -notmatch "No local mapping recommendations for this node\.") {
            throw "DATUM local mapping recommendation viewer output did not include recommendation details or an empty-state message."
        }

        if ($clientLocalMappingRecommendationOutput -notmatch "Local mapping recommendations are advisory only\. This command did not write config and did not execute installation\.") {
            throw "DATUM local mapping recommendation viewer output did not include the advisory no-write/no-execution note."
        }

        if ($clientLocalMappingRecommendationOutput -match "safe to auto apply:") {
            if ($clientLocalMappingRecommendationOutput -notmatch "safe to auto apply:\s+false") {
                throw "DATUM local mapping recommendation viewer output did not include 'safe to auto apply: false' when recommendations were shown."
            }
            if ($clientLocalMappingRecommendationOutput -notmatch "requires operator:\s+true") {
                throw "DATUM local mapping recommendation viewer output did not include 'requires operator: true' when recommendations were shown."
            }
            $safeToAutoApplyFalseValidated = $true
        }
        $clientLocalMappingRecommendationViewerPerformed = $true

        Write-Step "Run local mapping preflight from DATUM"
        $clientLocalMappingPreflightOutput = Run-CommandOrFail `
            -WorkingDirectory $DatumDir `
            -Command @(
                "cargo",
                "run",
                "--",
                "installation-tasks",
                "--config",
                $ResolvedClientConfig,
                "--local-mapping-preflight"
            ) `
            -StepName "DATUM local mapping preflight"

        if ($clientLocalMappingPreflightOutput -notmatch [regex]::Escape("Local mapping preflight for node $NodeId") -and
            $clientLocalMappingPreflightOutput -notmatch "No installation tasks require local mapping preflight\.") {
            throw "DATUM local mapping preflight output did not include preflight details or an empty-state message."
        }

        if ($clientLocalMappingPreflightOutput -notmatch "Note: this preflight is advisory only\. No local config was written and no installation was executed\.") {
            throw "DATUM local mapping preflight output did not include the advisory no-write/no-execution note."
        }

        if ($clientLocalMappingPreflightOutput -notmatch "Safe to auto execute:\s+false") {
            throw "DATUM local mapping preflight output did not include 'Safe to auto execute: false'."
        }

        if ($clientLocalMappingPreflightOutput -notmatch "Side effects performed:\s+false") {
            throw "DATUM local mapping preflight output did not include 'Side effects performed: false'."
        }
        $clientLocalMappingPreflightPerformed = $true
        $safeToAutoExecuteFalseValidated = $true
        $safeToAutoExecuteFalseValidatedForLocalMappingPreflight = $true
        $sideEffectsPerformedFalseValidated = $true
        $sideEffectsPerformedFalseValidatedForLocalMappingPreflight = $true

        if ([string]::IsNullOrWhiteSpace([string]$firstTaskId)) {
            Write-Host "No installation task available; skipping single-task local mapping preflight."
        }
        else {
            Write-Step "Run single-task local mapping preflight from DATUM"
            $clientSingleTaskLocalMappingPreflightOutput = Run-CommandOrFail `
                -WorkingDirectory $DatumDir `
                -Command @(
                    "cargo",
                    "run",
                    "--",
                    "installation-tasks",
                    "--config",
                    $ResolvedClientConfig,
                    "--task-id",
                    $firstTaskId,
                    "--local-mapping-preflight"
                ) `
                -StepName "DATUM single-task local mapping preflight"

            if ($clientSingleTaskLocalMappingPreflightOutput -notmatch [regex]::Escape("Local mapping preflight for node $NodeId")) {
                throw "DATUM single-task local mapping preflight output did not include the expected preflight header."
            }

            if ($clientSingleTaskLocalMappingPreflightOutput -notmatch [regex]::Escape("task_id: $firstTaskId")) {
                throw "DATUM single-task local mapping preflight output did not include the requested task id."
            }

            if ($clientSingleTaskLocalMappingPreflightOutput -notmatch "Note: this preflight is advisory only\. No local config was written and no installation was executed\.") {
                throw "DATUM single-task local mapping preflight output did not include the advisory no-write/no-execution note."
            }

            if ($clientSingleTaskLocalMappingPreflightOutput -notmatch "Safe to auto execute:\s+false") {
                throw "DATUM single-task local mapping preflight output did not include 'Safe to auto execute: false'."
            }

            if ($clientSingleTaskLocalMappingPreflightOutput -notmatch "Side effects performed:\s+false") {
                throw "DATUM single-task local mapping preflight output did not include 'Side effects performed: false'."
            }

            $clientSingleTaskLocalMappingPreflightPerformed = $true
            $safeToAutoExecuteFalseValidated = $true
            $safeToAutoExecuteFalseValidatedForLocalMappingPreflight = $true
            $sideEffectsPerformedFalseValidated = $true
            $sideEffectsPerformedFalseValidatedForLocalMappingPreflight = $true
        }

        Write-Step "Publish local mapping preflight from DATUM"
        $clientLocalMappingPreflightPublishOutput = Run-CommandOrFail `
            -WorkingDirectory $DatumDir `
            -Command @(
                "cargo",
                "run",
                "--",
                "installation-tasks",
                "--config",
                $ResolvedClientConfig,
                "--local-mapping-preflight",
                "--publish-local-mapping-preflight"
            ) `
            -StepName "DATUM local mapping preflight publish"

        if ($clientLocalMappingPreflightPublishOutput -notmatch "Published local mapping preflight report") {
            throw "DATUM local mapping preflight publish output did not indicate that a report was published."
        }

        if ($clientLocalMappingPreflightPublishOutput -notmatch "safe to auto execute:\s+false") {
            throw "DATUM local mapping preflight publish output did not include 'safe to auto execute: false'."
        }

        if ($clientLocalMappingPreflightPublishOutput -notmatch "side effects performed:\s+false") {
            throw "DATUM local mapping preflight publish output did not include 'side effects performed: false'."
        }

        if ($clientLocalMappingPreflightPublishOutput -notmatch "Note: this command did not write local config and did not execute installation\.") {
            throw "DATUM local mapping preflight publish output did not include the advisory no-write/no-execution publish note."
        }
        $clientLocalMappingPreflightPublishPerformed = $true
        $safeToAutoExecuteFalseValidated = $true
        $safeToAutoExecuteFalseValidatedForLocalMappingPreflight = $true
        $sideEffectsPerformedFalseValidated = $true
        $sideEffectsPerformedFalseValidatedForLocalMappingPreflight = $true

        Write-Step "Read latest local mapping preflight report for node from server"
        $nodeLatestLocalMappingPreflightReport = Invoke-JsonEndpoint `
            -Url "$ServerUrl/api/v1/nodes/$NodeId/local-mapping-preflight/latest" `
            -Headers $headers `
            -StepName "Node latest local mapping preflight report"
        Assert-HttpOk $nodeLatestLocalMappingPreflightReport "Node latest local mapping preflight report"
        Assert-LocalMappingPreflightReportContract `
            -Json $nodeLatestLocalMappingPreflightReport.Json `
            -StepName "Node latest local mapping preflight report" `
            -RequireNodeId
        $serverNodeLatestLocalMappingPreflightReportReadPerformed = $true
        $safeToAutoExecuteFalseValidated = $true
        $safeToAutoExecuteFalseValidatedForLocalMappingPreflight = $true
        $sideEffectsPerformedFalseValidated = $true
        $sideEffectsPerformedFalseValidatedForLocalMappingPreflight = $true

        Write-Step "Read latest local mapping preflight reports for project from server"
        $projectLatestLocalMappingPreflightReports = Invoke-JsonEndpoint `
            -Url "$ServerUrl/api/v1/local-mapping-preflight/latest" `
            -Headers $headers `
            -StepName "Project latest local mapping preflight reports"
        Assert-HttpOk $projectLatestLocalMappingPreflightReports "Project latest local mapping preflight reports"
        Assert-LocalMappingPreflightReportListContract `
            -Json $projectLatestLocalMappingPreflightReports.Json `
            -StepName "Project latest local mapping preflight reports"
        $firstProjectLatestLocalMappingPreflightReport = @(
            $projectLatestLocalMappingPreflightReports.Json.reports | Select-Object -First 1
        )
        if ($firstProjectLatestLocalMappingPreflightReport.Count -gt 0) {
            Assert-LocalMappingPreflightReportContract `
                -Json $firstProjectLatestLocalMappingPreflightReport[0] `
                -StepName "Project latest local mapping preflight reports"
            $safeToAutoExecuteFalseValidated = $true
            $safeToAutoExecuteFalseValidatedForLocalMappingPreflight = $true
            $sideEffectsPerformedFalseValidated = $true
            $sideEffectsPerformedFalseValidatedForLocalMappingPreflight = $true
        }
        $serverProjectLatestLocalMappingPreflightReportListReadPerformed = $true

        Write-Step "Read assisted execution state list with local mapping context from server"
        $assistedExecutionStateListWithLocalMappingContext = Invoke-JsonEndpoint `
            -Url "$ServerUrl/api/v1/assisted-execution-state/for-node/$NodeId" `
            -Headers $headers `
            -StepName "Assisted execution state for node with local mapping context"
        Assert-HttpOk $assistedExecutionStateListWithLocalMappingContext "Assisted execution state for node with local mapping context"
        Assert-AssistedExecutionStateSummaryListContract `
            -Json $assistedExecutionStateListWithLocalMappingContext.Json `
            -StepName "Assisted execution state for node with local mapping context"
        $firstAssistedExecutionStateSummaryWithLocalMappingContext = @(
            $assistedExecutionStateListWithLocalMappingContext.Json.summaries | Select-Object -First 1
        )
        if ($firstAssistedExecutionStateSummaryWithLocalMappingContext.Count -gt 0) {
            Assert-AssistedExecutionStateSummaryContract `
                -Json $firstAssistedExecutionStateSummaryWithLocalMappingContext[0] `
                -StepName "Assisted execution state for node with local mapping context" `
                -RequireLocalMappingStatus
            $safeToAutoExecuteFalseValidated = $true
            $safeToAutoExecuteFalseValidatedForStateSummaries = $true
            if ($firstAssistedExecutionStateSummaryWithLocalMappingContext[0].PSObject.Properties.Name.Contains("side_effects_performed") -and
                $null -ne $firstAssistedExecutionStateSummaryWithLocalMappingContext[0].side_effects_performed) {
                $sideEffectsPerformedFalseValidated = $true
                $sideEffectsPerformedFalseValidatedForStateSummaries = $true
            }
        }
        $serverAssistedExecutionStateLocalMappingContextReadPerformed = $true

        Write-Step "Read assisted execution state with local mapping context from DATUM"
        $clientAssistedExecutionStateLocalMappingContextOutput = Run-CommandOrFail `
            -WorkingDirectory $DatumDir `
            -Command @(
                "cargo",
                "run",
                "--",
                "installation-tasks",
                "--config",
                $ResolvedClientConfig,
                "--execution-state"
            ) `
            -StepName "DATUM assisted execution state viewer with local mapping context"

        if ($clientAssistedExecutionStateLocalMappingContextOutput -notmatch [regex]::Escape("Assisted execution state for node $NodeId") -and
            $clientAssistedExecutionStateLocalMappingContextOutput -notmatch "No assisted execution state summaries for this node\.") {
            throw "DATUM assisted execution state viewer with local mapping context did not include state summary details or an empty-state message."
        }

        if ($firstAssistedExecutionStateSummaryWithLocalMappingContext.Count -gt 0) {
            if ($clientAssistedExecutionStateLocalMappingContextOutput -notmatch "local mapping:\s+(already_mapped|missing_mapping|ambiguous|not_applicable|warning|unknown|no_report)") {
                throw "DATUM assisted execution state viewer with local mapping context did not include a local mapping status line."
            }

            if ($clientAssistedExecutionStateLocalMappingContextOutput -notmatch "safe to auto execute:\s+false") {
                throw "DATUM assisted execution state viewer with local mapping context did not include 'safe to auto execute: false'."
            }

            if ($clientAssistedExecutionStateLocalMappingContextOutput -match "side effects performed:") {
                if ($clientAssistedExecutionStateLocalMappingContextOutput -notmatch "side effects performed:\s+false") {
                    throw "DATUM assisted execution state viewer with local mapping context did not include 'side effects performed: false' when side effects were shown."
                }
            }

            if ($firstAssistedExecutionStateSummaryWithLocalMappingContext[0].PSObject.Properties.Name.Contains("local_mapping_reason") -and
                -not [string]::IsNullOrWhiteSpace([string]$firstAssistedExecutionStateSummaryWithLocalMappingContext[0].local_mapping_reason)) {
                $escapedLocalMappingReason = [regex]::Escape([string]$firstAssistedExecutionStateSummaryWithLocalMappingContext[0].local_mapping_reason)
                if ($clientAssistedExecutionStateLocalMappingContextOutput -notmatch "local mapping reason:\s+$escapedLocalMappingReason") {
                    throw "DATUM assisted execution state viewer with local mapping context did not include the expected local mapping reason."
                }
            }

            if ($firstAssistedExecutionStateSummaryWithLocalMappingContext[0].PSObject.Properties.Name.Contains("local_mapping_report_id") -and
                -not [string]::IsNullOrWhiteSpace([string]$firstAssistedExecutionStateSummaryWithLocalMappingContext[0].local_mapping_report_id)) {
                $escapedLocalMappingReportId = [regex]::Escape([string]$firstAssistedExecutionStateSummaryWithLocalMappingContext[0].local_mapping_report_id)
                if ($clientAssistedExecutionStateLocalMappingContextOutput -notmatch "local mapping report:\s+$escapedLocalMappingReportId") {
                    throw "DATUM assisted execution state viewer with local mapping context did not include the expected local mapping report id."
                }
            }

            if ($firstAssistedExecutionStateSummaryWithLocalMappingContext[0].PSObject.Properties.Name.Contains("local_mapping_reported_at_utc") -and
                -not [string]::IsNullOrWhiteSpace([string]$firstAssistedExecutionStateSummaryWithLocalMappingContext[0].local_mapping_reported_at_utc)) {
                $escapedLocalMappingReportedAt = [regex]::Escape([string]$firstAssistedExecutionStateSummaryWithLocalMappingContext[0].local_mapping_reported_at_utc)
                if ($clientAssistedExecutionStateLocalMappingContextOutput -notmatch "local mapping reported at:\s+$escapedLocalMappingReportedAt") {
                    throw "DATUM assisted execution state viewer with local mapping context did not include the expected local mapping reported-at value."
                }
            }
        }
        $clientAssistedExecutionStateLocalMappingContextViewerPerformed = $true

        if ([string]::IsNullOrWhiteSpace([string]$firstTaskId)) {
            Write-Host "No installation task available; skipping single-task local mapping state check."
        }
        else {
            Write-Step "Read one assisted execution state summary with local mapping context from server"
            $singleAssistedExecutionStateWithLocalMappingContext = Invoke-JsonEndpoint `
                -Url "$ServerUrl/api/v1/installation-tasks/for-node/$NodeId/$encodedTaskId/assisted-execution-state" `
                -Headers $headers `
                -StepName "Single assisted execution state with local mapping context"
            Assert-HttpOk $singleAssistedExecutionStateWithLocalMappingContext "Single assisted execution state with local mapping context"
            Assert-AssistedExecutionStateSummaryContract `
                -Json $singleAssistedExecutionStateWithLocalMappingContext.Json `
                -StepName "Single assisted execution state with local mapping context" `
                -ExpectedTaskId $firstTaskId `
                -RequireLocalMappingStatus
            $serverSingleTaskAssistedExecutionStateLocalMappingContextReadPerformed = $true
            $safeToAutoExecuteFalseValidated = $true
            $safeToAutoExecuteFalseValidatedForStateSummaries = $true
            if ($singleAssistedExecutionStateWithLocalMappingContext.Json.PSObject.Properties.Name.Contains("side_effects_performed") -and
                $null -ne $singleAssistedExecutionStateWithLocalMappingContext.Json.side_effects_performed) {
                $sideEffectsPerformedFalseValidated = $true
                $sideEffectsPerformedFalseValidatedForStateSummaries = $true
            }

            Write-Step "Read one assisted execution state summary with local mapping context from DATUM"
            $clientSingleAssistedExecutionStateLocalMappingContextOutput = Run-CommandOrFail `
                -WorkingDirectory $DatumDir `
                -Command @(
                    "cargo",
                    "run",
                    "--",
                    "installation-tasks",
                    "--config",
                    $ResolvedClientConfig,
                    "--task-id",
                    $firstTaskId,
                    "--execution-state"
                ) `
                -StepName "DATUM single assisted execution state viewer with local mapping context"

            if ($clientSingleAssistedExecutionStateLocalMappingContextOutput -notmatch [regex]::Escape("Assisted execution state for task $firstTaskId")) {
                throw "DATUM single assisted execution state viewer with local mapping context did not include the expected task header."
            }

            if ($clientSingleAssistedExecutionStateLocalMappingContextOutput -notmatch "local mapping:\s+(already_mapped|missing_mapping|ambiguous|not_applicable|warning|unknown|no_report)") {
                throw "DATUM single assisted execution state viewer with local mapping context did not include a local mapping status line."
            }

            if ($clientSingleAssistedExecutionStateLocalMappingContextOutput -notmatch "safe to auto execute:\s+false") {
                throw "DATUM single assisted execution state viewer with local mapping context did not include 'safe to auto execute: false'."
            }

            if ($clientSingleAssistedExecutionStateLocalMappingContextOutput -match "side effects performed:") {
                if ($clientSingleAssistedExecutionStateLocalMappingContextOutput -notmatch "side effects performed:\s+false") {
                    throw "DATUM single assisted execution state viewer with local mapping context did not include 'side effects performed: false' when side effects were shown."
                }
            }

            $clientSingleTaskAssistedExecutionStateLocalMappingContextViewerPerformed = $true
        }

        Write-Step "Read project local mapping patch proposals from server"
        $projectLocalMappingPatchProposals = Invoke-JsonEndpoint `
            -Url "$ServerUrl/api/v1/local-mapping-patch-proposals" `
            -Headers $headers `
            -StepName "Project local mapping patch proposals"
        Assert-HttpOk $projectLocalMappingPatchProposals "Project local mapping patch proposals"
        Assert-LocalMappingPatchProposalListContract `
            -Json $projectLocalMappingPatchProposals.Json `
            -StepName "Project local mapping patch proposals"
        $firstProjectLocalMappingPatchProposal = @(
            $projectLocalMappingPatchProposals.Json.proposals | Select-Object -First 1
        )
        if ($firstProjectLocalMappingPatchProposal.Count -gt 0) {
            Assert-LocalMappingPatchProposalSafetyContract `
                -Proposal $firstProjectLocalMappingPatchProposal[0] `
                -StepName "Project local mapping patch proposals"
            $safeToAutoApplyFalseValidated = $true
            $sideEffectsPerformedFalseValidated = $true
            $requiresOperatorTrueValidated = $true
        }
        $serverProjectLocalMappingPatchProposalsReadPerformed = $true

        Write-Step "Read node local mapping patch proposals from server"
        $nodeLocalMappingPatchProposals = Invoke-JsonEndpoint `
            -Url "$ServerUrl/api/v1/local-mapping-patch-proposals/for-node/$NodeId" `
            -Headers $headers `
            -StepName "Node local mapping patch proposals"
        Assert-HttpOk $nodeLocalMappingPatchProposals "Node local mapping patch proposals"
        Assert-LocalMappingPatchProposalListContract `
            -Json $nodeLocalMappingPatchProposals.Json `
            -StepName "Node local mapping patch proposals" `
            -RequireNodeId
        $firstNodeLocalMappingPatchProposal = @(
            $nodeLocalMappingPatchProposals.Json.proposals | Select-Object -First 1
        )
        if ($firstNodeLocalMappingPatchProposal.Count -gt 0) {
            Assert-LocalMappingPatchProposalSafetyContract `
                -Proposal $firstNodeLocalMappingPatchProposal[0] `
                -StepName "Node local mapping patch proposals" `
                -RequireNodeId `
                -AllowedProposalStatuses @("proposed", "blocked", "ambiguous", "not_applicable") `
                -AllowedProposalKinds @("add_container_mapping", "review_ambiguous_mapping", "no_change", "unknown")
            $safeToAutoApplyFalseValidated = $true
            $sideEffectsPerformedFalseValidated = $true
            $requiresOperatorTrueValidated = $true
        }
        $serverNodeLocalMappingPatchProposalsReadPerformed = $true

        Write-Step "Read local mapping patch proposals from DATUM"
        $clientLocalMappingPatchProposalOutput = Run-CommandOrFail `
            -WorkingDirectory $DatumDir `
            -Command @(
                "cargo",
                "run",
                "--",
                "installation-tasks",
                "--config",
                $ResolvedClientConfig,
                "--local-mapping-patch-proposals"
            ) `
            -StepName "DATUM local mapping patch proposal viewer"

        if ($clientLocalMappingPatchProposalOutput -notmatch [regex]::Escape("Local mapping patch proposals for node $NodeId") -and
            $clientLocalMappingPatchProposalOutput -notmatch "No local mapping patch proposals for this node\.") {
            throw "DATUM local mapping patch proposal viewer output did not include proposal details or an empty-state message."
        }

        if ($clientLocalMappingPatchProposalOutput -notmatch "Note: this proposal viewer is advisory only\. This command did not write config, did not apply mappings, and did not execute installation\.") {
            throw "DATUM local mapping patch proposal viewer output did not include the advisory no-write/no-execution note."
        }

        if ($clientLocalMappingPatchProposalOutput -match "safe to auto apply:") {
            if ($clientLocalMappingPatchProposalOutput -notmatch "safe to auto apply:\s+false") {
                throw "DATUM local mapping patch proposal viewer output did not include 'safe to auto apply: false' when proposals were shown."
            }

            if ($clientLocalMappingPatchProposalOutput -notmatch "side effects performed:\s+false") {
                throw "DATUM local mapping patch proposal viewer output did not include 'side effects performed: false' when proposals were shown."
            }

            if ($clientLocalMappingPatchProposalOutput -notmatch "requires operator:\s+true") {
                throw "DATUM local mapping patch proposal viewer output did not include 'requires operator: true' when proposals were shown."
            }

            $safeToAutoApplyFalseValidated = $true
            $sideEffectsPerformedFalseValidated = $true
            $requiresOperatorTrueValidated = $true
        }
        $clientLocalMappingPatchProposalViewerPerformed = $true

        Write-Step "Generate local mapping patch proposal file from DATUM"
        $smokeOutputDir = Join-Path $ScriptRoot ".smoke-output"
        $smokeOutputProposalPath = Join-Path $smokeOutputDir "local-mapping-proposal.toml"
        $smokeOutputProposalRelativePath = "scripts/.smoke-output/local-mapping-proposal.toml"
        $clientConfigFingerprintBeforePatchProposalGeneration = Get-FileFingerprint -Path $ResolvedClientConfig
        $clientLocalMappingPatchProposalFileOutput = Run-CommandOrFail `
            -WorkingDirectory $DatumDir `
            -Command @(
                "cargo",
                "run",
                "--",
                "installation-tasks",
                "--config",
                $ResolvedClientConfig,
                "--local-mapping-patch-proposals",
                "--write-local-mapping-patch",
                $smokeOutputProposalPath,
                "--overwrite-local-mapping-patch"
            ) `
            -StepName "DATUM local mapping patch proposal file generation"

        if ($clientLocalMappingPatchProposalFileOutput -notmatch "Wrote local mapping patch proposal file") {
            throw "DATUM local mapping patch proposal file generation output did not indicate that a proposal file was written."
        }

        if ($clientLocalMappingPatchProposalFileOutput -notmatch "advisory:\s+true") {
            throw "DATUM local mapping patch proposal file generation output did not include 'advisory: true'."
        }

        if ($clientLocalMappingPatchProposalFileOutput -notmatch "applied:\s+false") {
            throw "DATUM local mapping patch proposal file generation output did not include 'applied: false'."
        }

        if ($clientLocalMappingPatchProposalFileOutput -notmatch "safe to auto apply:\s+false") {
            throw "DATUM local mapping patch proposal file generation output did not include 'safe to auto apply: false'."
        }

        if (-not (Test-Path -LiteralPath $smokeOutputProposalPath)) {
            throw "DATUM local mapping patch proposal file generation did not create the expected proposal file at $smokeOutputProposalPath."
        }

        $proposalFileContent = Get-Content -LiteralPath $smokeOutputProposalPath -Raw
        if ([string]::IsNullOrWhiteSpace($proposalFileContent)) {
            throw "DATUM local mapping patch proposal file generation created an empty proposal file."
        }

        foreach ($requiredProposalFileMarker in @(
            "advisory",
            "operator review",
            "not applied automatically",
            "config.toml only after validation"
        )) {
            if ($proposalFileContent -notmatch [regex]::Escape($requiredProposalFileMarker)) {
                throw "DATUM local mapping patch proposal file did not include required advisory wording '$requiredProposalFileMarker'."
            }
        }

        if ($proposalFileContent -notmatch [regex]::Escape("dispatch.container_mappings") -and
            $proposalFileContent -notmatch [regex]::Escape("No local mapping patch proposals") -and
            $proposalFileContent -notmatch [regex]::Escape("No proposed config snippet was available")) {
            throw "DATUM local mapping patch proposal file did not include a config snippet section or a safe empty-proposal message."
        }

        $clientConfigFingerprintAfterPatchProposalGeneration = Get-FileFingerprint -Path $ResolvedClientConfig
        if ($clientConfigFingerprintBeforePatchProposalGeneration -ne $clientConfigFingerprintAfterPatchProposalGeneration) {
            throw "Client config changed during patch proposal generation, which is not allowed."
        }

        $generatedFileGitStatus = Run-CommandOrFail `
            -WorkingDirectory $PdRoot `
            -Command @("git", "status", "--short", "--", $smokeOutputProposalRelativePath) `
            -StepName "Git status for generated local mapping patch proposal file"
        if (-not [string]::IsNullOrWhiteSpace($generatedFileGitStatus)) {
            Write-Warning "Generated local mapping patch proposal file appears in git status; do not stage or commit $smokeOutputProposalRelativePath."
        }

        $clientLocalMappingPatchProposalFileGenerationPerformed = $true
        $generatedLocalMappingPatchProposalFileValidated = $true
        $clientConfigUnchangedAfterPatchProposalGenerationValidated = $true
        $advisoryOnlyLocalMappingPatchProposalFileValidated = $true

        Write-Step "Revalidate local mapping patch proposal file from DATUM"
        $clientConfigFingerprintBeforePatchRevalidation = Get-FileFingerprint -Path $ResolvedClientConfig
        $clientLocalMappingPatchRevalidationOutput = Run-CommandOrFail `
            -WorkingDirectory $DatumDir `
            -Command @(
                "cargo",
                "run",
                "--",
                "installation-tasks",
                "--config",
                $ResolvedClientConfig,
                "--revalidate-local-mapping-patch",
                $smokeOutputProposalPath
            ) `
            -StepName "DATUM local mapping patch revalidation"

        if ($clientLocalMappingPatchRevalidationOutput -notmatch "Local mapping patch revalidation") {
            throw "DATUM local mapping patch revalidation output did not include the revalidation heading."
        }

        if ($clientLocalMappingPatchRevalidationOutput -notmatch "Safe to auto apply:\s+false") {
            throw "DATUM local mapping patch revalidation output did not include 'Safe to auto apply: false'."
        }

        if ($clientLocalMappingPatchRevalidationOutput -notmatch "Side effects performed:\s+false") {
            throw "DATUM local mapping patch revalidation output did not include 'Side effects performed: false'."
        }

        if ($clientLocalMappingPatchRevalidationOutput -notmatch "Applied:\s+false") {
            throw "DATUM local mapping patch revalidation output did not include 'Applied: false'."
        }

        $clientConfigFingerprintAfterPatchRevalidation = Get-FileFingerprint -Path $ResolvedClientConfig
        if ($clientConfigFingerprintBeforePatchRevalidation -ne $clientConfigFingerprintAfterPatchRevalidation) {
            throw "Client config changed during patch proposal revalidation, which is not allowed."
        }

        Remove-Item -LiteralPath $smokeOutputProposalPath -Force
        if ((Test-Path -LiteralPath $smokeOutputDir) -and
            -not (Get-ChildItem -LiteralPath $smokeOutputDir -Force | Select-Object -First 1)) {
            Remove-Item -LiteralPath $smokeOutputDir -Force
        }

        $clientLocalMappingPatchRevalidationPerformed = $true
        $generatedLocalMappingPatchProposalFileRevalidated = $true
        $clientConfigUnchangedAfterPatchRevalidationValidated = $true
        $appliedFalseValidated = $true
        $generatedSmokeArtifactCleanupPerformed = $true
        $safeToAutoApplyFalseValidated = $true
        $sideEffectsPerformedFalseValidated = $true

        if ([string]::IsNullOrWhiteSpace([string]$firstTaskId)) {
            Write-Host "No installation task available; skipping single-task local mapping preflight publish."
        }
        else {
            Write-Step "Publish single-task local mapping preflight from DATUM"
            $clientSingleTaskLocalMappingPreflightPublishOutput = Run-CommandOrFail `
                -WorkingDirectory $DatumDir `
                -Command @(
                    "cargo",
                    "run",
                    "--",
                    "installation-tasks",
                    "--config",
                    $ResolvedClientConfig,
                    "--task-id",
                    $firstTaskId,
                    "--local-mapping-preflight",
                    "--publish-local-mapping-preflight"
                ) `
                -StepName "DATUM single-task local mapping preflight publish"

            if ($clientSingleTaskLocalMappingPreflightPublishOutput -notmatch "Published local mapping preflight report") {
                throw "DATUM single-task local mapping preflight publish output did not indicate that a report was published."
            }

            if ($clientSingleTaskLocalMappingPreflightPublishOutput -notmatch "safe to auto execute:\s+false") {
                throw "DATUM single-task local mapping preflight publish output did not include 'safe to auto execute: false'."
            }

            if ($clientSingleTaskLocalMappingPreflightPublishOutput -notmatch "side effects performed:\s+false") {
                throw "DATUM single-task local mapping preflight publish output did not include 'side effects performed: false'."
            }

            if ($clientSingleTaskLocalMappingPreflightPublishOutput -notmatch "Note: this command did not write local config and did not execute installation\.") {
                throw "DATUM single-task local mapping preflight publish output did not include the advisory no-write/no-execution publish note."
            }

            $clientSingleTaskLocalMappingPreflightPublishPerformed = $true
            $safeToAutoExecuteFalseValidated = $true
            $safeToAutoExecuteFalseValidatedForLocalMappingPreflight = $true
            $sideEffectsPerformedFalseValidated = $true
            $sideEffectsPerformedFalseValidatedForLocalMappingPreflight = $true
        }
    }
}

Write-Step "Run local WASM runtime add test from DATUM"
$clientLocalWasmRuntimeAddOutput = Run-CommandOrFail `
    -WorkingDirectory $DatumDir `
    -Command @("cargo", "run", "--quiet", "--", "wasm-test", "--add", "2", "3") `
    -StepName "DATUM local WASM runtime add test"

if ($clientLocalWasmRuntimeAddOutput -notmatch "WASM runtime v0 report") {
    throw "DATUM local WASM runtime add test output did not include the report heading."
}

if ($clientLocalWasmRuntimeAddOutput -notmatch "result:\s+5") {
    throw "DATUM local WASM runtime add test output did not include 'result: 5'."
}

foreach ($requiredLocalWasmMarker in @(
    "safe to auto execute:\s+false",
    "side effects performed:\s+false",
    "filesystem access:\s+false",
    "network access:\s+false",
    "host command access:\s+false",
    "docker access:\s+false",
    "artifact access:\s+false",
    "installation task access:\s+false",
    "assisted execution access:\s+false",
    "isolated, local-only, and not connected to installation tasks or assisted execution"
)) {
    if ($clientLocalWasmRuntimeAddOutput -notmatch $requiredLocalWasmMarker) {
        throw "DATUM local WASM runtime add test output did not include required marker '$requiredLocalWasmMarker'."
    }
}

$clientLocalWasmRuntimeAddTestPerformed = $true
$safeToAutoExecuteFalseValidated = $true
$sideEffectsPerformedFalseValidated = $true
$safeAccessFlagsFalseValidatedForWasm = $true

Write-Step "Run local WASM runtime JSON test from DATUM"
$clientLocalWasmRuntimeJsonOutput = Run-CommandOrFail `
    -WorkingDirectory $DatumDir `
    -Command @("cargo", "run", "--quiet", "--", "wasm-test", "--add", "2", "3", "--json") `
    -StepName "DATUM local WASM runtime JSON test"
$clientLocalWasmRuntimeJson = Convert-CommandOutputToJson `
    -Output $clientLocalWasmRuntimeJsonOutput `
    -StepName "DATUM local WASM runtime JSON test"
Assert-WasmRuntimeReportContract `
    -Json $clientLocalWasmRuntimeJson `
    -StepName "DATUM local WASM runtime JSON test"
$clientLocalWasmRuntimeJsonTestPerformed = $true
$safeToAutoExecuteFalseValidated = $true
$sideEffectsPerformedFalseValidated = $true
$safeAccessFlagsFalseValidatedForWasm = $true

if (-not $SkipServerChecks) {
    Write-Step "Publish WASM runtime report from DATUM"
    $clientWasmRuntimeReportPublishOutput = Run-CommandOrFail `
        -WorkingDirectory $DatumDir `
        -Command @(
            "cargo",
            "run",
            "--quiet",
            "--",
            "wasm-test",
            "--config",
            $ResolvedClientConfig,
            "--add",
            "2",
            "3",
            "--publish-report"
        ) `
        -StepName "DATUM WASM runtime report publish"

    if ($clientWasmRuntimeReportPublishOutput -notmatch "Published WASM runtime report") {
        throw "DATUM WASM runtime report publish output did not indicate that a report was published."
    }

    foreach ($requiredPublishedWasmMarker in @(
        "status:\s+executed",
        "result:\s+5",
        "safe to auto execute:\s+false",
        "side effects performed:\s+false",
        "published an isolated WASM runtime test report only",
        "did not execute installation",
        "not connected to assisted execution"
    )) {
        if ($clientWasmRuntimeReportPublishOutput -notmatch $requiredPublishedWasmMarker) {
            throw "DATUM WASM runtime report publish output did not include required marker '$requiredPublishedWasmMarker'."
        }
    }

    $clientWasmRuntimeReportPublishPerformed = $true
    $safeToAutoExecuteFalseValidated = $true
    $sideEffectsPerformedFalseValidated = $true
    $safeAccessFlagsFalseValidatedForWasm = $true

    Write-Step "Read latest WASM runtime report for node from server"
    $nodeLatestWasmRuntimeReport = Invoke-JsonEndpoint `
        -Url "$ServerUrl/api/v1/nodes/$NodeId/wasm-runtime-reports/latest" `
        -Headers $headers `
        -StepName "Node latest WASM runtime report"
    Assert-HttpOk $nodeLatestWasmRuntimeReport "Node latest WASM runtime report"
    Assert-WasmRuntimePublishedReportContract `
        -Json $nodeLatestWasmRuntimeReport.Json `
        -StepName "Node latest WASM runtime report"
    $serverNodeLatestWasmRuntimeReportReadPerformed = $true
    $safeToAutoExecuteFalseValidated = $true
    $sideEffectsPerformedFalseValidated = $true
    $safeAccessFlagsFalseValidatedForWasm = $true

    Write-Step "Read latest WASM runtime reports for project from server"
    $projectLatestWasmRuntimeReports = Invoke-JsonEndpoint `
        -Url "$ServerUrl/api/v1/wasm-runtime-reports/latest" `
        -Headers $headers `
        -StepName "Project latest WASM runtime reports"
    Assert-HttpOk $projectLatestWasmRuntimeReports "Project latest WASM runtime reports"
    Assert-WasmRuntimeReportListContract `
        -Json $projectLatestWasmRuntimeReports.Json `
        -StepName "Project latest WASM runtime reports"
    $firstProjectLatestWasmRuntimeReport = @(
        $projectLatestWasmRuntimeReports.Json.reports | Select-Object -First 1
    )
    if ($firstProjectLatestWasmRuntimeReport.Count -gt 0) {
        Assert-WasmRuntimeReportContract `
            -Json $firstProjectLatestWasmRuntimeReport[0] `
            -StepName "Project latest WASM runtime reports"
        $safeToAutoExecuteFalseValidated = $true
        $sideEffectsPerformedFalseValidated = $true
        $safeAccessFlagsFalseValidatedForWasm = $true
    }
    $serverProjectLatestWasmRuntimeReportListReadPerformed = $true

    Write-Step "Read latest WASM runtime report from DATUM"
    $clientLatestWasmRuntimeReportOutput = Run-CommandOrFail `
        -WorkingDirectory $DatumDir `
        -Command @(
            "cargo",
            "run",
            "--quiet",
            "--",
            "wasm-test",
            "--config",
            $ResolvedClientConfig,
            "--latest-report"
        ) `
        -StepName "DATUM latest WASM runtime report viewer"

    if ($clientLatestWasmRuntimeReportOutput -notmatch "Latest WASM runtime report") {
        throw "DATUM latest WASM runtime report viewer output did not include the expected heading."
    }

    foreach ($requiredLatestWasmMarker in @(
        "status:\s+executed",
        "result:\s+5",
        "safe to auto execute:\s+false",
        "side effects performed:\s+false",
        "filesystem access:\s+false",
        "network access:\s+false",
        "host command access:\s+false",
        "docker access:\s+false",
        "artifact access:\s+false",
        "installation task access:\s+false",
        "assisted execution access:\s+false"
    )) {
        if ($clientLatestWasmRuntimeReportOutput -notmatch $requiredLatestWasmMarker) {
            throw "DATUM latest WASM runtime report viewer output did not include required marker '$requiredLatestWasmMarker'."
        }
    }

    $clientLatestWasmRuntimeReportViewerPerformed = $true
    $safeToAutoExecuteFalseValidated = $true
    $sideEffectsPerformedFalseValidated = $true
    $safeAccessFlagsFalseValidatedForWasm = $true

    Write-Step "Read latest WASM runtime reports from DATUM"
    $clientLatestWasmRuntimeReportsOutput = Run-CommandOrFail `
        -WorkingDirectory $DatumDir `
        -Command @(
            "cargo",
            "run",
            "--quiet",
            "--",
            "wasm-test",
            "--config",
            $ResolvedClientConfig,
            "--latest-reports"
        ) `
        -StepName "DATUM latest WASM runtime reports viewer"

    if ($clientLatestWasmRuntimeReportsOutput -notmatch "Latest WASM runtime reports") {
        throw "DATUM latest WASM runtime reports viewer output did not include the expected heading."
    }

    if ($clientLatestWasmRuntimeReportsOutput -match "safe to auto execute:") {
        foreach ($requiredLatestWasmListMarker in @(
            "safe to auto execute:\s+false",
            "side effects performed:\s+false",
            "filesystem access:\s+false",
            "network access:\s+false",
            "host command access:\s+false",
            "docker access:\s+false",
            "artifact access:\s+false",
            "installation task access:\s+false",
            "assisted execution access:\s+false"
        )) {
            if ($clientLatestWasmRuntimeReportsOutput -notmatch $requiredLatestWasmListMarker) {
                throw "DATUM latest WASM runtime reports viewer output did not include required marker '$requiredLatestWasmListMarker'."
            }
        }

        $safeToAutoExecuteFalseValidated = $true
        $sideEffectsPerformedFalseValidated = $true
        $safeAccessFlagsFalseValidatedForWasm = $true
    }

    $clientLatestWasmRuntimeReportsViewerPerformed = $true

    Write-Step "Register or validate metadata-only WASM module descriptor on server"
    $embeddedAddModuleDescriptorBody = @{
        module_id = "embedded-add"
        module_name = "embedded:add"
        module_version = "v0"
        module_kind = "embedded_test"
        status = "registered"
        allowed_functions = @(
            @{
                function_name = "add"
                allowed_execution_modes = @("embedded_test")
                max_fuel = 1000000
                timeout_ms = 1000
            }
        )
        artifact_ref = @{
            artifact_id = "embedded-add-metadata-only"
            artifact_uri = "metadata-only://embedded-add"
            sha256 = $null
            signature_ref = $null
            provenance = "embedded test descriptor only"
        }
        safety = @{
            filesystem_access = $false
            network_access = $false
            host_command_access = $false
            docker_access = $false
            artifact_access = $false
            installation_task_access = $false
            assisted_execution_access = $false
            side_effects_allowed = $false
            safe_to_auto_execute = $false
        }
        metadata = @{
            smoke_test = $true
            metadata_only = $true
        }
    }
    $wasmModuleRegistrationResponse = Invoke-JsonPostEndpoint `
        -Url "$ServerUrl/api/v1/wasm-modules" `
        -Headers $headers `
        -Body $embeddedAddModuleDescriptorBody `
        -StepName "WASM module descriptor registration"

    if ($wasmModuleRegistrationResponse.StatusCode -ge 200 -and $wasmModuleRegistrationResponse.StatusCode -lt 300) {
        Assert-WasmModuleDescriptorContract `
            -Json $wasmModuleRegistrationResponse.Json `
            -StepName "WASM module descriptor registration"
        Assert-WasmModuleArtifactMetadataOnly `
            -Json $wasmModuleRegistrationResponse.Json `
            -StepName "WASM module descriptor registration"
        $serverWasmModuleDescriptorRegistrationPerformed = $true
        $safeAccessFlagsFalseValidatedForWasm = $true
        $wasmArtifactMetadataOnlyValidated = $true
    }
    elseif ($wasmModuleRegistrationResponse.StatusCode -in @(409, 422)) {
        $registrationBodyExcerpt = ($wasmModuleRegistrationResponse.Body | Out-String).Trim()
        Write-Warning "WASM module descriptor registration returned status $($wasmModuleRegistrationResponse.StatusCode); falling back to GET /api/v1/wasm-modules/embedded-add. $registrationBodyExcerpt"

        $existingEmbeddedAddModule = Invoke-JsonEndpoint `
            -Url "$ServerUrl/api/v1/wasm-modules/embedded-add" `
            -Headers $headers `
            -StepName "Fallback WASM module descriptor"
        Assert-HttpOk $existingEmbeddedAddModule "Fallback WASM module descriptor"
        Assert-WasmModuleDescriptorContract `
            -Json $existingEmbeddedAddModule.Json `
            -StepName "Fallback WASM module descriptor"
        Assert-WasmModuleArtifactMetadataOnly `
            -Json $existingEmbeddedAddModule.Json `
            -StepName "Fallback WASM module descriptor"
        $serverWasmModuleDescriptorRegistrationPerformed = $true
        $serverSingleWasmModuleDescriptorReadPerformed = $true
        $safeAccessFlagsFalseValidatedForWasm = $true
        $wasmArtifactMetadataOnlyValidated = $true
    }
    else {
        $registrationExcerpt = ($wasmModuleRegistrationResponse.Body | Out-String).Trim()
        throw "WASM module descriptor registration expected a compatible success status or duplicate-compatible fallback, got HTTP $($wasmModuleRegistrationResponse.StatusCode).`n$registrationExcerpt"
    }

    Write-Step "Read WASM module registry list from server"
    $projectWasmModuleRegistry = Invoke-JsonEndpoint `
        -Url "$ServerUrl/api/v1/wasm-modules" `
        -Headers $headers `
        -StepName "Project WASM module registry"
    Assert-HttpOk $projectWasmModuleRegistry "Project WASM module registry"
    Assert-WasmModuleDescriptorListContract `
        -Json $projectWasmModuleRegistry.Json `
        -StepName "Project WASM module registry"
    $embeddedAddModuleListEntry = Find-WasmModuleDescriptorById `
        -Modules $projectWasmModuleRegistry.Json.modules `
        -ModuleId "embedded-add"
    if ($embeddedAddModuleListEntry.Count -eq 0) {
        throw "Project WASM module registry did not include module_id 'embedded-add' after registration or fallback validation."
    }
    Assert-WasmModuleDescriptorContract `
        -Json $embeddedAddModuleListEntry[0] `
        -StepName "Project WASM module registry"
    Assert-WasmModuleArtifactMetadataOnly `
        -Json $embeddedAddModuleListEntry[0] `
        -StepName "Project WASM module registry"
    $serverWasmModuleRegistryListReadPerformed = $true
    $safeAccessFlagsFalseValidatedForWasm = $true
    $wasmArtifactMetadataOnlyValidated = $true

    Write-Step "Read single WASM module descriptor from server"
    $singleWasmModuleDescriptor = Invoke-JsonEndpoint `
        -Url "$ServerUrl/api/v1/wasm-modules/embedded-add" `
        -Headers $headers `
        -StepName "Single WASM module descriptor"
    Assert-HttpOk $singleWasmModuleDescriptor "Single WASM module descriptor"
    Assert-WasmModuleDescriptorContract `
        -Json $singleWasmModuleDescriptor.Json `
        -StepName "Single WASM module descriptor"
    Assert-WasmModuleArtifactMetadataOnly `
        -Json $singleWasmModuleDescriptor.Json `
        -StepName "Single WASM module descriptor"
    $serverSingleWasmModuleDescriptorReadPerformed = $true
    $safeAccessFlagsFalseValidatedForWasm = $true
    $wasmArtifactMetadataOnlyValidated = $true

    Write-Step "Read WASM module registry from DATUM"
    $clientWasmModuleRegistryOutput = Run-CommandOrFail `
        -WorkingDirectory $DatumDir `
        -Command @(
            "cargo",
            "run",
            "--quiet",
            "--",
            "wasm-test",
            "--config",
            $ResolvedClientConfig,
            "--modules"
        ) `
        -StepName "DATUM WASM module registry viewer"

    foreach ($requiredModuleRegistryMarker in @(
        "WASM module registry",
        "embedded-add",
        "name:\s+embedded:add",
        "kind:\s+embedded_test",
        "functions:\s+add",
        "safe to auto execute:\s+false",
        "side effects allowed:\s+false",
        "filesystem access:\s+false",
        "network access:\s+false",
        "host command access:\s+false",
        "docker access:\s+false",
        "artifact access:\s+false",
        "installation task access:\s+false",
        "assisted execution access:\s+false",
        "metadata only",
        "did not download artifacts or execute WASM modules"
    )) {
        if ($clientWasmModuleRegistryOutput -notmatch $requiredModuleRegistryMarker) {
            throw "DATUM WASM module registry viewer output did not include required marker '$requiredModuleRegistryMarker'."
        }
    }

    $clientWasmModuleRegistryViewerPerformed = $true
    $safeAccessFlagsFalseValidatedForWasm = $true
    $wasmArtifactMetadataOnlyValidated = $true

    Write-Step "Read single WASM module descriptor from DATUM"
    $clientSingleWasmModuleOutput = Run-CommandOrFail `
        -WorkingDirectory $DatumDir `
        -Command @(
            "cargo",
            "run",
            "--quiet",
            "--",
            "wasm-test",
            "--config",
            $ResolvedClientConfig,
            "--module",
            "embedded-add"
        ) `
        -StepName "DATUM single WASM module viewer"

    foreach ($requiredSingleModuleMarker in @(
        "WASM module descriptor:\s+embedded-add",
        "name:\s+embedded:add",
        "kind:\s+embedded_test",
        "status:\s+registered",
        "artifact uri:\s+metadata only",
        "Functions:",
        "- add",
        "modes:\s+embedded_test",
        "max fuel:\s+1000000",
        "timeout ms:\s+1000",
        "safe to auto execute:\s+false",
        "side effects allowed:\s+false",
        "filesystem access:\s+false",
        "network access:\s+false",
        "host command access:\s+false",
        "docker access:\s+false",
        "artifact access:\s+false",
        "installation task access:\s+false",
        "assisted execution access:\s+false",
        "artifact references are metadata only",
        "did not fetch artifacts or execute modules"
    )) {
        if ($clientSingleWasmModuleOutput -notmatch $requiredSingleModuleMarker) {
            throw "DATUM single WASM module viewer output did not include required marker '$requiredSingleModuleMarker'."
        }
    }

    $clientSingleWasmModuleViewerPerformed = $true
    $safeAccessFlagsFalseValidatedForWasm = $true
    $wasmArtifactMetadataOnlyValidated = $true

    Write-Step "Read WASM module registry JSON from DATUM"
    $clientWasmModuleRegistryJsonOutput = Run-CommandOrFail `
        -WorkingDirectory $DatumDir `
        -Command @(
            "cargo",
            "run",
            "--quiet",
            "--",
            "wasm-test",
            "--config",
            $ResolvedClientConfig,
            "--modules",
            "--json"
        ) `
        -StepName "DATUM WASM module registry JSON viewer"
    $clientWasmModuleRegistryJson = Convert-CommandOutputToJson `
        -Output $clientWasmModuleRegistryJsonOutput `
        -StepName "DATUM WASM module registry JSON viewer"
    Assert-WasmModuleDescriptorListContract `
        -Json $clientWasmModuleRegistryJson `
        -StepName "DATUM WASM module registry JSON viewer"
    $clientEmbeddedAddModuleJsonListEntry = Find-WasmModuleDescriptorById `
        -Modules $clientWasmModuleRegistryJson.modules `
        -ModuleId "embedded-add"
    if ($clientEmbeddedAddModuleJsonListEntry.Count -eq 0) {
        throw "DATUM WASM module registry JSON viewer did not include module_id 'embedded-add'."
    }
    Assert-WasmModuleDescriptorContract `
        -Json $clientEmbeddedAddModuleJsonListEntry[0] `
        -StepName "DATUM WASM module registry JSON viewer"
    Assert-WasmModuleArtifactMetadataOnly `
        -Json $clientEmbeddedAddModuleJsonListEntry[0] `
        -StepName "DATUM WASM module registry JSON viewer"
    $clientWasmModuleRegistryJsonViewerPerformed = $true
    $safeAccessFlagsFalseValidatedForWasm = $true
    $wasmArtifactMetadataOnlyValidated = $true

    Write-Step "Read single WASM module descriptor JSON from DATUM"
    $clientSingleWasmModuleJsonOutput = Run-CommandOrFail `
        -WorkingDirectory $DatumDir `
        -Command @(
            "cargo",
            "run",
            "--quiet",
            "--",
            "wasm-test",
            "--config",
            $ResolvedClientConfig,
            "--module",
            "embedded-add",
            "--json"
        ) `
        -StepName "DATUM single WASM module JSON viewer"
    $clientSingleWasmModuleJson = Convert-CommandOutputToJson `
        -Output $clientSingleWasmModuleJsonOutput `
        -StepName "DATUM single WASM module JSON viewer"
    Assert-WasmModuleDescriptorContract `
        -Json $clientSingleWasmModuleJson `
        -StepName "DATUM single WASM module JSON viewer"
    Assert-WasmModuleArtifactMetadataOnly `
        -Json $clientSingleWasmModuleJson `
        -StepName "DATUM single WASM module JSON viewer"
    $clientSingleWasmModuleJsonViewerPerformed = $true
    $safeAccessFlagsFalseValidatedForWasm = $true
    $wasmArtifactMetadataOnlyValidated = $true
}

Write-Step "Run local d-graph snapshot summary from DATUM"
$clientLocalDgraphSnapshotOutput = Run-CommandOrFail `
    -WorkingDirectory $DatumDir `
    -Command @("cargo", "run", "--quiet", "--", "dgraph-snapshot", "--config", $ResolvedClientConfig) `
    -StepName "DATUM local d-graph snapshot summary"

foreach ($requiredLocalDgraphMarker in @(
    "Local d-graph snapshot",
    "node:\s+$([regex]::Escape($NodeId))",
    "runtimes:\s+\d+",
    "modules:\s+\d+",
    "flows:\s+\d+",
    "health entries:\s+\d+",
    "capabilities:\s+\d+",
    "safe to auto execute:\s+false",
    "side effects performed:\s+false",
    "did not inspect Docker/container state, execute WASM, deploy modules, or modify config"
)) {
    if ($clientLocalDgraphSnapshotOutput -notmatch $requiredLocalDgraphMarker) {
        throw "DATUM local d-graph snapshot summary output did not include required marker '$requiredLocalDgraphMarker'."
    }
}

$clientLocalDgraphSnapshotSummaryPerformed = $true
$safeToAutoExecuteFalseValidated = $true
$sideEffectsPerformedFalseValidated = $true
$safeToAutoExecuteFalseValidatedForDgraphSnapshots = $true
$sideEffectsPerformedFalseValidatedForDgraphSnapshots = $true

Write-Step "Run local d-graph snapshot JSON from DATUM"
$clientLocalDgraphSnapshotJsonOutput = Run-CommandOrFail `
    -WorkingDirectory $DatumDir `
    -Command @("cargo", "run", "--quiet", "--", "dgraph-snapshot", "--config", $ResolvedClientConfig, "--json") `
    -StepName "DATUM local d-graph snapshot JSON"
$clientLocalDgraphSnapshotJson = Convert-CommandOutputToJson `
    -Output $clientLocalDgraphSnapshotJsonOutput `
    -StepName "DATUM local d-graph snapshot JSON"
Assert-LocalDGraphSnapshotContract `
    -Json $clientLocalDgraphSnapshotJson `
    -StepName "DATUM local d-graph snapshot JSON" `
    -RequireNodeId
$clientLocalDgraphSnapshotJsonPerformed = $true
$safeToAutoExecuteFalseValidated = $true
$sideEffectsPerformedFalseValidated = $true
$safeToAutoExecuteFalseValidatedForDgraphSnapshots = $true
$sideEffectsPerformedFalseValidatedForDgraphSnapshots = $true

if (-not $SkipServerChecks) {
    Write-Step "Publish local d-graph snapshot from DATUM"
    $clientPublishDgraphSnapshotOutput = Run-CommandOrFail `
        -WorkingDirectory $DatumDir `
        -Command @(
            "cargo",
            "run",
            "--quiet",
            "--",
            "dgraph-snapshot",
            "--config",
            $ResolvedClientConfig,
            "--publish"
        ) `
        -StepName "DATUM local d-graph snapshot publish"

    foreach ($requiredPublishedDgraphMarker in @(
        "Published local d-graph snapshot",
        "safe to auto execute:\s+false",
        "side effects performed:\s+false",
        "published local d-graph snapshot metadata only",
        "did not run planner logic, deploy modules, or reconfigure flows"
    )) {
        if ($clientPublishDgraphSnapshotOutput -notmatch $requiredPublishedDgraphMarker) {
            throw "DATUM local d-graph snapshot publish output did not include required marker '$requiredPublishedDgraphMarker'."
        }
    }

    $clientLocalDgraphSnapshotPublishPerformed = $true
    $safeToAutoExecuteFalseValidated = $true
    $sideEffectsPerformedFalseValidated = $true
    $safeToAutoExecuteFalseValidatedForDgraphSnapshots = $true
    $sideEffectsPerformedFalseValidatedForDgraphSnapshots = $true

    Write-Step "Read latest d-graph snapshot for node from server"
    $nodeLatestDgraphSnapshot = Invoke-JsonEndpoint `
        -Url "$ServerUrl/api/v1/nodes/$NodeId/dgraph-snapshot/latest" `
        -Headers $headers `
        -StepName "Node latest d-graph snapshot"
    Assert-HttpOk $nodeLatestDgraphSnapshot "Node latest d-graph snapshot"
    Assert-LocalDGraphSnapshotContract `
        -Json $nodeLatestDgraphSnapshot.Json `
        -StepName "Node latest d-graph snapshot" `
        -RequireProjectId `
        -RequireNodeId `
        -RequireSnapshotMetadata
    $serverNodeLatestDgraphSnapshotReadPerformed = $true
    $safeToAutoExecuteFalseValidated = $true
    $sideEffectsPerformedFalseValidated = $true
    $safeToAutoExecuteFalseValidatedForDgraphSnapshots = $true
    $sideEffectsPerformedFalseValidatedForDgraphSnapshots = $true

    Write-Step "Read latest d-graph snapshots for project from server"
    $projectLatestDgraphSnapshots = Invoke-JsonEndpoint `
        -Url "$ServerUrl/api/v1/dgraph-snapshots/latest" `
        -Headers $headers `
        -StepName "Project latest d-graph snapshots"
    Assert-HttpOk $projectLatestDgraphSnapshots "Project latest d-graph snapshots"
    Assert-LocalDGraphSnapshotListContract `
        -Json $projectLatestDgraphSnapshots.Json `
        -StepName "Project latest d-graph snapshots"
    $firstProjectLatestDgraphSnapshot = @(
        $projectLatestDgraphSnapshots.Json.snapshots | Select-Object -First 1
    )
    if ($firstProjectLatestDgraphSnapshot.Count -gt 0) {
        Assert-LocalDGraphSnapshotContract `
            -Json $firstProjectLatestDgraphSnapshot[0] `
            -StepName "Project latest d-graph snapshots"
        $safeToAutoExecuteFalseValidated = $true
        $sideEffectsPerformedFalseValidated = $true
        $safeToAutoExecuteFalseValidatedForDgraphSnapshots = $true
        $sideEffectsPerformedFalseValidatedForDgraphSnapshots = $true
    }
    $serverProjectLatestDgraphSnapshotListReadPerformed = $true

    Write-Step "Read latest d-graph snapshot from DATUM"
    $clientLatestDgraphSnapshotOutput = Run-CommandOrFail `
        -WorkingDirectory $DatumDir `
        -Command @(
            "cargo",
            "run",
            "--quiet",
            "--",
            "dgraph-snapshot",
            "--config",
            $ResolvedClientConfig,
            "--latest"
        ) `
        -StepName "DATUM latest d-graph snapshot viewer"

    foreach ($requiredLatestDgraphMarker in @(
        "Latest local d-graph snapshot",
        "safe to auto execute:\s+false",
        "side effects performed:\s+false",
        "did not run planner logic, deploy modules, or reconfigure flows"
    )) {
        if ($clientLatestDgraphSnapshotOutput -notmatch $requiredLatestDgraphMarker) {
            throw "DATUM latest d-graph snapshot viewer output did not include required marker '$requiredLatestDgraphMarker'."
        }
    }

    $clientLatestDgraphSnapshotViewerPerformed = $true
    $safeToAutoExecuteFalseValidated = $true
    $sideEffectsPerformedFalseValidated = $true
    $safeToAutoExecuteFalseValidatedForDgraphSnapshots = $true
    $sideEffectsPerformedFalseValidatedForDgraphSnapshots = $true

    Write-Step "Read latest d-graph snapshots from DATUM"
    $clientLatestAllDgraphSnapshotsOutput = Run-CommandOrFail `
        -WorkingDirectory $DatumDir `
        -Command @(
            "cargo",
            "run",
            "--quiet",
            "--",
            "dgraph-snapshot",
            "--config",
            $ResolvedClientConfig,
            "--latest-all"
        ) `
        -StepName "DATUM latest-all d-graph snapshot viewer"

    foreach ($requiredLatestAllDgraphMarker in @(
        "Latest local d-graph snapshots",
        "count:\s+\d+",
        "metadata only",
        "did not run planner logic, deploy modules, or reconfigure flows"
    )) {
        if ($clientLatestAllDgraphSnapshotsOutput -notmatch $requiredLatestAllDgraphMarker) {
            throw "DATUM latest-all d-graph snapshot viewer output did not include required marker '$requiredLatestAllDgraphMarker'."
        }
    }

    if ($clientLatestAllDgraphSnapshotsOutput -match "safe to auto execute:") {
        foreach ($requiredLatestAllSafetyMarker in @(
            "safe to auto execute:\s+false",
            "side effects performed:\s+false"
        )) {
            if ($clientLatestAllDgraphSnapshotsOutput -notmatch $requiredLatestAllSafetyMarker) {
                throw "DATUM latest-all d-graph snapshot viewer output did not include required marker '$requiredLatestAllSafetyMarker'."
            }
        }

        $safeToAutoExecuteFalseValidated = $true
        $sideEffectsPerformedFalseValidated = $true
        $safeToAutoExecuteFalseValidatedForDgraphSnapshots = $true
        $sideEffectsPerformedFalseValidatedForDgraphSnapshots = $true
    }

    $clientLatestAllDgraphSnapshotViewerPerformed = $true

    Write-Step "Read project d-graph materialized from server"
    $projectMaterializedDgraph = Invoke-JsonEndpoint `
        -Url "$ServerUrl/api/v1/dgraph/materialized" `
        -Headers $headers `
        -StepName "Project d-graph materialized"
    Assert-HttpOk $projectMaterializedDgraph "Project d-graph materialized"
    Assert-ProjectDGraphMaterializationContract `
        -Json $projectMaterializedDgraph.Json `
        -StepName "Project d-graph materialized"
    $serverProjectMaterializedDgraphReadPerformed = $true
    $sideEffectsPerformedFalseValidated = $true
    $sideEffectsPerformedFalseValidatedForDgraphMaterialization = $true

    Write-Step "Read project d-graph materialized summary from server"
    $projectMaterializedDgraphSummary = Invoke-JsonEndpoint `
        -Url "$ServerUrl/api/v1/dgraph/materialized/summary" `
        -Headers $headers `
        -StepName "Project d-graph materialized summary"
    Assert-HttpOk $projectMaterializedDgraphSummary "Project d-graph materialized summary"
    Assert-ProjectDGraphMaterializationSummaryContract `
        -Json $projectMaterializedDgraphSummary.Json `
        -StepName "Project d-graph materialized summary"
    $serverProjectMaterializedDgraphSummaryReadPerformed = $true
    $sideEffectsPerformedFalseValidated = $true
    $sideEffectsPerformedFalseValidatedForDgraphMaterialization = $true

    Write-Step "Read project d-graph materialized from DATUM"
    $clientMaterializedDgraphOutput = Run-CommandOrFail `
        -WorkingDirectory $DatumDir `
        -Command @(
            "cargo",
            "run",
            "--quiet",
            "--",
            "dgraph-snapshot",
            "--config",
            $ResolvedClientConfig,
            "--materialized"
        ) `
        -StepName "DATUM materialized d-graph viewer"

    foreach ($requiredMaterializedDgraphMarker in @(
        "Project d-graph materialized",
        "Project:\s+$([regex]::Escape($ProjectId))",
        "Source snapshots:\s+\d+",
        "Side effects performed:\s+false",
        "did not run planner, deploy modules, or reconfigure flows"
    )) {
        if ($clientMaterializedDgraphOutput -notmatch $requiredMaterializedDgraphMarker) {
            throw "DATUM materialized d-graph viewer output did not include required marker '$requiredMaterializedDgraphMarker'."
        }
    }

    $clientMaterializedDgraphViewerPerformed = $true
    $sideEffectsPerformedFalseValidated = $true
    $sideEffectsPerformedFalseValidatedForDgraphMaterialization = $true

    Write-Step "Read project d-graph materialized summary from DATUM"
    $clientMaterializedDgraphSummaryOutput = Run-CommandOrFail `
        -WorkingDirectory $DatumDir `
        -Command @(
            "cargo",
            "run",
            "--quiet",
            "--",
            "dgraph-snapshot",
            "--config",
            $ResolvedClientConfig,
            "--materialized-summary"
        ) `
        -StepName "DATUM materialized d-graph summary viewer"

    foreach ($requiredMaterializedDgraphSummaryMarker in @(
        "Project d-graph summary",
        "Project:\s+$([regex]::Escape($ProjectId))",
        "Source snapshots:\s+\d+",
        "Side effects performed:\s+false",
        "did not run planner, deploy modules, or reconfigure flows"
    )) {
        if ($clientMaterializedDgraphSummaryOutput -notmatch $requiredMaterializedDgraphSummaryMarker) {
            throw "DATUM materialized d-graph summary viewer output did not include required marker '$requiredMaterializedDgraphSummaryMarker'."
        }
    }

    $clientMaterializedDgraphSummaryViewerPerformed = $true
    $sideEffectsPerformedFalseValidated = $true
    $sideEffectsPerformedFalseValidatedForDgraphMaterialization = $true

    Write-Step "Read project d-graph materialized JSON from DATUM"
    $clientMaterializedDgraphJsonOutput = Run-CommandOrFail `
        -WorkingDirectory $DatumDir `
        -Command @(
            "cargo",
            "run",
            "--quiet",
            "--",
            "dgraph-snapshot",
            "--config",
            $ResolvedClientConfig,
            "--materialized",
            "--json"
        ) `
        -StepName "DATUM materialized d-graph JSON viewer"
    $clientMaterializedDgraphJson = Convert-CommandOutputToJson `
        -Output $clientMaterializedDgraphJsonOutput `
        -StepName "DATUM materialized d-graph JSON viewer"
    Assert-ProjectDGraphMaterializationContract `
        -Json $clientMaterializedDgraphJson `
        -StepName "DATUM materialized d-graph JSON viewer"
    $clientMaterializedDgraphJsonViewerPerformed = $true
    $sideEffectsPerformedFalseValidated = $true
    $sideEffectsPerformedFalseValidatedForDgraphMaterialization = $true

    Write-Step "Read project d-graph materialized summary JSON from DATUM"
    $clientMaterializedDgraphSummaryJsonOutput = Run-CommandOrFail `
        -WorkingDirectory $DatumDir `
        -Command @(
            "cargo",
            "run",
            "--quiet",
            "--",
            "dgraph-snapshot",
            "--config",
            $ResolvedClientConfig,
            "--materialized-summary",
            "--json"
        ) `
        -StepName "DATUM materialized d-graph summary JSON viewer"
    $clientMaterializedDgraphSummaryJson = Convert-CommandOutputToJson `
        -Output $clientMaterializedDgraphSummaryJsonOutput `
        -StepName "DATUM materialized d-graph summary JSON viewer"
    Assert-ProjectDGraphMaterializationSummaryContract `
        -Json $clientMaterializedDgraphSummaryJson `
        -StepName "DATUM materialized d-graph summary JSON viewer"
    $clientMaterializedDgraphSummaryJsonViewerPerformed = $true
    $sideEffectsPerformedFalseValidated = $true
    $sideEffectsPerformedFalseValidatedForDgraphMaterialization = $true

    Write-Step "Read project d-graph planner dry-run from server"
    $projectPlannerDryRun = Invoke-JsonEndpoint `
        -Url "$ServerUrl/api/v1/dgraph/planner/dry-run" `
        -Headers $headers `
        -StepName "Project d-graph planner dry-run"
    Assert-HttpOk $projectPlannerDryRun "Project d-graph planner dry-run"
    Assert-ProjectDGraphPlannerDryRunContract `
        -Json $projectPlannerDryRun.Json `
        -StepName "Project d-graph planner dry-run"
    $serverProjectPlannerDryRunReadPerformed = $true
    $sideEffectsPerformedFalseValidated = $true
    $sideEffectsPerformedFalseValidatedForDgraphPlanner = $true

    Write-Step "Read project d-graph planner dry-run summary from server"
    $projectPlannerDryRunSummary = Invoke-JsonEndpoint `
        -Url "$ServerUrl/api/v1/dgraph/planner/dry-run/summary" `
        -Headers $headers `
        -StepName "Project d-graph planner dry-run summary"
    Assert-HttpOk $projectPlannerDryRunSummary "Project d-graph planner dry-run summary"
    Assert-ProjectDGraphPlannerDryRunSummaryContract `
        -Json $projectPlannerDryRunSummary.Json `
        -StepName "Project d-graph planner dry-run summary"
    $serverProjectPlannerDryRunSummaryReadPerformed = $true
    $sideEffectsPerformedFalseValidated = $true
    $sideEffectsPerformedFalseValidatedForDgraphPlanner = $true

    Write-Step "Read project d-graph planner dry-run from DATUM"
    $clientPlannerDryRunOutput = Run-CommandOrFail `
        -WorkingDirectory $DatumDir `
        -Command @(
            "cargo",
            "run",
            "--quiet",
            "--",
            "dgraph-snapshot",
            "--config",
            $ResolvedClientConfig,
            "--planner-dry-run"
        ) `
        -StepName "DATUM planner dry-run viewer"

    foreach ($requiredPlannerDryRunMarker in @(
        "D-graph planner dry-run",
        "Project:\s+$([regex]::Escape($ProjectId))",
        "Source snapshots:\s+\d+",
        "Status:\s+(planned|partially_blocked|blocked|empty|unknown)",
        "Side effects performed:\s+false",
        "advisory only",
        "did not execute"
    )) {
        if ($clientPlannerDryRunOutput -notmatch $requiredPlannerDryRunMarker) {
            throw "DATUM planner dry-run viewer output did not include required marker '$requiredPlannerDryRunMarker'."
        }
    }

    $clientPlannerDryRunViewerPerformed = $true
    $sideEffectsPerformedFalseValidated = $true
    $sideEffectsPerformedFalseValidatedForDgraphPlanner = $true

    Write-Step "Read project d-graph planner dry-run summary from DATUM"
    $clientPlannerSummaryOutput = Run-CommandOrFail `
        -WorkingDirectory $DatumDir `
        -Command @(
            "cargo",
            "run",
            "--quiet",
            "--",
            "dgraph-snapshot",
            "--config",
            $ResolvedClientConfig,
            "--planner-summary"
        ) `
        -StepName "DATUM planner summary viewer"

    foreach ($requiredPlannerSummaryMarker in @(
        "D-graph planner dry-run summary",
        "Project:\s+$([regex]::Escape($ProjectId))",
        "Source snapshots:\s+\d+",
        "Status:\s+(planned|partially_blocked|blocked|empty|unknown)",
        "Side effects performed:\s+false",
        "did not deploy modules, execute installation, call Docker, or reconfigure flows"
    )) {
        if ($clientPlannerSummaryOutput -notmatch $requiredPlannerSummaryMarker) {
            throw "DATUM planner summary viewer output did not include required marker '$requiredPlannerSummaryMarker'."
        }
    }

    $clientPlannerSummaryViewerPerformed = $true
    $sideEffectsPerformedFalseValidated = $true
    $sideEffectsPerformedFalseValidatedForDgraphPlanner = $true

    Write-Step "Read project d-graph planner dry-run JSON from DATUM"
    $clientPlannerDryRunJsonOutput = Run-CommandOrFail `
        -WorkingDirectory $DatumDir `
        -Command @(
            "cargo",
            "run",
            "--quiet",
            "--",
            "dgraph-snapshot",
            "--config",
            $ResolvedClientConfig,
            "--planner-dry-run",
            "--json"
        ) `
        -StepName "DATUM planner dry-run JSON viewer"
    $clientPlannerDryRunJson = Convert-CommandOutputToJson `
        -Output $clientPlannerDryRunJsonOutput `
        -StepName "DATUM planner dry-run JSON viewer"
    Assert-ProjectDGraphPlannerDryRunContract `
        -Json $clientPlannerDryRunJson `
        -StepName "DATUM planner dry-run JSON viewer"
    $clientPlannerDryRunJsonViewerPerformed = $true
    $sideEffectsPerformedFalseValidated = $true
    $sideEffectsPerformedFalseValidatedForDgraphPlanner = $true

    Write-Step "Read project d-graph planner dry-run summary JSON from DATUM"
    $clientPlannerSummaryJsonOutput = Run-CommandOrFail `
        -WorkingDirectory $DatumDir `
        -Command @(
            "cargo",
            "run",
            "--quiet",
            "--",
            "dgraph-snapshot",
            "--config",
            $ResolvedClientConfig,
            "--planner-summary",
            "--json"
        ) `
        -StepName "DATUM planner summary JSON viewer"
    $clientPlannerSummaryJson = Convert-CommandOutputToJson `
        -Output $clientPlannerSummaryJsonOutput `
        -StepName "DATUM planner summary JSON viewer"
    Assert-ProjectDGraphPlannerDryRunSummaryContract `
        -Json $clientPlannerSummaryJson `
        -StepName "DATUM planner summary JSON viewer"
    $clientPlannerSummaryJsonViewerPerformed = $true
    $sideEffectsPerformedFalseValidated = $true
    $sideEffectsPerformedFalseValidatedForDgraphPlanner = $true
}

Write-Step "Validate local d-graph fixture scenario JSON from DATUM"
$fixtureScenarios = @(
    "baseline",
    "degraded-node",
    "unavailable-node",
    "missing-module",
    "degraded-flow",
    "no-candidate"
)

foreach ($fixtureScenario in $fixtureScenarios) {
    $fixtureJsonOutput = Run-CommandOrFail `
        -WorkingDirectory $DatumDir `
        -Command @(
            "cargo",
            "run",
            "--quiet",
            "--",
            "dgraph-snapshot",
            "--config",
            $ResolvedClientConfig,
            "--fixture",
            $fixtureScenario,
            "--json"
        ) `
        -StepName "DATUM local d-graph fixture JSON ($fixtureScenario)"
    $fixtureJson = Convert-CommandOutputToJson `
        -Output $fixtureJsonOutput `
        -StepName "DATUM local d-graph fixture JSON ($fixtureScenario)"
    Assert-LocalDGraphFixtureSnapshotContract `
        -Json $fixtureJson `
        -StepName "DATUM local d-graph fixture JSON ($fixtureScenario)" `
        -Scenario $fixtureScenario
}

$localFixtureScenarioJsonChecksPerformed = $true
$fixtureMetadataValidated = $true
$safeToAutoExecuteFalseValidated = $true
$sideEffectsPerformedFalseValidated = $true
$safeToAutoExecuteFalseValidatedForDgraphSnapshots = $true
$sideEffectsPerformedFalseValidatedForDgraphSnapshots = $true
$sideEffectsPerformedFalseValidatedForDgraphFixtures = $true

if (-not $SkipServerChecks) {
    Write-Step "Publish selected d-graph fixture scenarios from DATUM"
    foreach ($fixtureScenario in @("baseline", "unavailable-node")) {
        $fixturePublishOutput = Run-CommandOrFail `
            -WorkingDirectory $DatumDir `
            -Command @(
                "cargo",
                "run",
                "--quiet",
                "--",
                "dgraph-snapshot",
                "--config",
                $ResolvedClientConfig,
                "--fixture",
                $fixtureScenario,
                "--publish"
            ) `
            -StepName "DATUM local d-graph fixture publish ($fixtureScenario)"

        foreach ($requiredFixturePublishMarker in @(
            "Published local d-graph fixture snapshot",
            "scenario:\s+$([regex]::Escape($fixtureScenario))",
            "safe to auto execute:\s+false",
            "side effects performed:\s+false",
            "recorded fixture metadata only",
            "No planner decision was executed"
        )) {
            if ($fixturePublishOutput -notmatch $requiredFixturePublishMarker) {
                throw "DATUM local d-graph fixture publish ($fixtureScenario) output did not include required marker '$requiredFixturePublishMarker'."
            }
        }
    }

    $fixturePublishReadBackPerformed = $true
    $safeToAutoExecuteFalseValidated = $true
    $sideEffectsPerformedFalseValidated = $true
    $safeToAutoExecuteFalseValidatedForDgraphSnapshots = $true
    $sideEffectsPerformedFalseValidatedForDgraphSnapshots = $true
    $sideEffectsPerformedFalseValidatedForDgraphFixtures = $true

    Write-Step "Read latest published d-graph fixture snapshot from server"
    $fixtureLatestDgraphSnapshot = Invoke-JsonEndpoint `
        -Url "$ServerUrl/api/v1/nodes/$NodeId/dgraph-snapshot/latest" `
        -Headers $headers `
        -StepName "Node latest d-graph fixture snapshot"
    Assert-HttpOk $fixtureLatestDgraphSnapshot "Node latest d-graph fixture snapshot"
    Assert-LocalDGraphSnapshotContract `
        -Json $fixtureLatestDgraphSnapshot.Json `
        -StepName "Node latest d-graph fixture snapshot" `
        -RequireProjectId `
        -RequireNodeId `
        -RequireSnapshotMetadata
    if ($fixtureLatestDgraphSnapshot.Json.PSObject.Properties.Name -contains "metadata" -and $null -ne $fixtureLatestDgraphSnapshot.Json.metadata) {
        if ($fixtureLatestDgraphSnapshot.Json.metadata.PSObject.Properties.Name -contains "fixture") {
            if ($fixtureLatestDgraphSnapshot.Json.metadata.fixture -ne $true) {
                throw "Node latest d-graph fixture snapshot returned metadata.fixture='$($fixtureLatestDgraphSnapshot.Json.metadata.fixture)', expected 'True' when present."
            }
        }

        if ($fixtureLatestDgraphSnapshot.Json.metadata.PSObject.Properties.Name -contains "fixture_scenario") {
            if ($fixtureLatestDgraphSnapshot.Json.metadata.fixture_scenario -ne "unavailable-node") {
                throw "Node latest d-graph fixture snapshot returned metadata.fixture_scenario='$($fixtureLatestDgraphSnapshot.Json.metadata.fixture_scenario)', expected 'unavailable-node' when present."
            }
        }
    }

    $sideEffectsPerformedFalseValidated = $true
    $safeToAutoExecuteFalseValidated = $true
    $safeToAutoExecuteFalseValidatedForDgraphSnapshots = $true
    $sideEffectsPerformedFalseValidatedForDgraphSnapshots = $true
    $sideEffectsPerformedFalseValidatedForDgraphFixtures = $true

    Write-Step "Read project d-graph materialized after fixture publish from server"
    $projectMaterializedAfterFixture = Invoke-JsonEndpoint `
        -Url "$ServerUrl/api/v1/dgraph/materialized" `
        -Headers $headers `
        -StepName "Project d-graph materialized after fixture publish"
    Assert-HttpOk $projectMaterializedAfterFixture "Project d-graph materialized after fixture publish"
    Assert-ProjectDGraphMaterializationContract `
        -Json $projectMaterializedAfterFixture.Json `
        -StepName "Project d-graph materialized after fixture publish"
    $materializedAfterFixturePublishValidated = $true
    $sideEffectsPerformedFalseValidated = $true
    $sideEffectsPerformedFalseValidatedForDgraphMaterialization = $true
    $sideEffectsPerformedFalseValidatedForDgraphFixtures = $true

    Write-Step "Read project d-graph planner dry-run after fixture publish from server"
    $projectPlannerAfterFixture = Invoke-JsonEndpoint `
        -Url "$ServerUrl/api/v1/dgraph/planner/dry-run" `
        -Headers $headers `
        -StepName "Project d-graph planner dry-run after fixture publish"
    Assert-HttpOk $projectPlannerAfterFixture "Project d-graph planner dry-run after fixture publish"
    Assert-ProjectDGraphPlannerDryRunContract `
        -Json $projectPlannerAfterFixture.Json `
        -StepName "Project d-graph planner dry-run after fixture publish"
    $plannerAfterFixturePublishValidated = $true
    $sideEffectsPerformedFalseValidated = $true
    $sideEffectsPerformedFalseValidatedForDgraphPlanner = $true
    $sideEffectsPerformedFalseValidatedForDgraphFixtures = $true

    Write-Step "Read project d-graph planner dry-run from DATUM after fixture publish"
    $clientPlannerDryRunAfterFixtureOutput = Run-CommandOrFail `
        -WorkingDirectory $DatumDir `
        -Command @(
            "cargo",
            "run",
            "--quiet",
            "--",
            "dgraph-snapshot",
            "--config",
            $ResolvedClientConfig,
            "--planner-dry-run"
        ) `
        -StepName "DATUM planner dry-run viewer after fixture publish"

    foreach ($requiredPlannerAfterFixtureMarker in @(
        "D-graph planner dry-run",
        "Side effects performed:\s+false",
        "advisory only",
        "did not execute"
    )) {
        if ($clientPlannerDryRunAfterFixtureOutput -notmatch $requiredPlannerAfterFixtureMarker) {
            throw "DATUM planner dry-run viewer after fixture publish output did not include required marker '$requiredPlannerAfterFixtureMarker'."
        }
    }

    $plannerAfterFixturePublishValidated = $true
    $sideEffectsPerformedFalseValidated = $true
    $sideEffectsPerformedFalseValidatedForDgraphPlanner = $true
    $sideEffectsPerformedFalseValidatedForDgraphFixtures = $true
}

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
Write-Host "- action result publish"
Write-Host "- node action result read-back"
Write-Host "- project action result list"
Write-Host "- deployment planning global endpoint"
Write-Host "- deployment planning node-scoped endpoint"
Write-Host "- client deployment planning viewer"
Write-Host "- installation plan global endpoint"
Write-Host "- installation plan node-scoped endpoint"
Write-Host "- client installation plan viewer"
Write-Host "- client installation task materialization"
Write-Host "- server node installation task list"
Write-Host "- server project installation task list"
Write-Host "- installation policy metadata check"
Write-Host "- client installation policy display check"
if ($taskUpdatePerformed) {
    Write-Host "- installation task single-task get"
    Write-Host "- installation task acknowledgement"
}
else {
    Write-Host "- installation task single-task get (skipped: no tasks materialized)"
    Write-Host "- installation task acknowledgement (skipped: no tasks materialized)"
}
if ($approvalCandidateDetected) {
    Write-Host "- operator approval candidate detection"
}
else {
    Write-Host "- operator approval candidate detection (skipped: no candidate_for_assisted_execution task found)"
}
if ($approvalSubmissionPerformed) {
    Write-Host "- client approval submission"
}
else {
    Write-Host "- client approval submission (skipped: no candidate_for_assisted_execution task found)"
}
if ($serverLatestApprovalReadPerformed) {
    Write-Host "- server latest approval read-back"
}
else {
    Write-Host "- server latest approval read-back (skipped: no candidate_for_assisted_execution task found)"
}
if ($serverApprovalListReadPerformed) {
    Write-Host "- server approval list for node"
}
else {
    Write-Host "- server approval list for node (skipped: no candidate_for_assisted_execution task found)"
}
if ($clientLatestApprovalReadPerformed) {
    Write-Host "- client latest approval viewer"
}
else {
    Write-Host "- client latest approval viewer (skipped: no candidate_for_assisted_execution task found)"
}
if ($clientApprovalListReadPerformed) {
    Write-Host "- client approval list viewer"
}
else {
    Write-Host "- client approval list viewer (skipped: no candidate_for_assisted_execution task found)"
}
if ($serverNodeReadinessListReadPerformed) {
    Write-Host "- server node readiness list"
}
else {
    Write-Host "- server node readiness list (skipped: server checks disabled)"
}
if ($clientNodeReadinessViewerPerformed) {
    Write-Host "- client readiness viewer"
}
else {
    Write-Host "- client readiness viewer (skipped: server checks disabled)"
}
if ($serverSingleTaskReadinessReadPerformed) {
    Write-Host "- server single-task readiness"
}
else {
    Write-Host "- server single-task readiness (skipped: no tasks materialized)"
}
if ($clientSingleTaskReadinessViewerPerformed) {
    Write-Host "- client single-task readiness viewer"
}
else {
    Write-Host "- client single-task readiness viewer (skipped: no tasks materialized)"
}
if ($safeToAutoExecuteFalseValidated) {
    Write-Host "- safe_to_auto_execute false validation"
}
else {
    Write-Host "- safe_to_auto_execute false validation (skipped: no readiness entries returned)"
}
if ($readyTaskDetected) {
    Write-Host "- ready task detection"
}
else {
    Write-Host "- ready task detection (skipped: no ready_for_assisted_execution task found)"
}
if ($assistedExecutionRequestSubmissionPerformed) {
    Write-Host "- client assisted execution request submission"
}
else {
    Write-Host "- client assisted execution request submission (skipped: no ready_for_assisted_execution task found)"
}
if ($serverLatestAssistedExecutionRequestReadPerformed) {
    Write-Host "- server latest assisted execution request read-back"
}
else {
    Write-Host "- server latest assisted execution request read-back (skipped: no ready_for_assisted_execution task found)"
}
if ($serverAssistedExecutionRequestListReadPerformed) {
    Write-Host "- server assisted execution request list"
}
else {
    Write-Host "- server assisted execution request list (skipped: no ready_for_assisted_execution task found)"
}
if ($clientLatestAssistedExecutionRequestReadPerformed) {
    Write-Host "- client latest assisted execution request viewer"
}
else {
    Write-Host "- client latest assisted execution request viewer (skipped: no ready_for_assisted_execution task found)"
}
if ($clientAssistedExecutionRequestListReadPerformed) {
    Write-Host "- client assisted execution request list"
}
else {
    Write-Host "- client assisted execution request list (skipped: no ready_for_assisted_execution task found)"
}
if ($requestedAssistedExecutionRequestDetected) {
    Write-Host "- requested assisted execution request detection"
}
else {
    Write-Host "- requested assisted execution request detection (skipped: no requested assisted execution request found)"
}
if ($assistedExecutionClaimSubmissionPerformed) {
    Write-Host "- client assisted execution claim submission"
}
elseif ($assistedExecutionClaimAlreadyClaimed) {
    Write-Host "- client assisted execution claim submission (already claimed; validated latest claim instead)"
}
else {
    Write-Host "- client assisted execution claim submission (skipped: no requested assisted execution request found)"
}
if ($serverLatestAssistedExecutionClaimReadPerformed) {
    Write-Host "- server latest assisted execution claim read-back"
}
else {
    Write-Host "- server latest assisted execution claim read-back (skipped: no requested assisted execution request found)"
}
if ($serverAssistedExecutionClaimListReadPerformed) {
    Write-Host "- server assisted execution claim list"
}
else {
    Write-Host "- server assisted execution claim list (skipped: no requested assisted execution request found)"
}
if ($clientLatestAssistedExecutionClaimReadPerformed) {
    Write-Host "- client latest assisted execution claim viewer"
}
else {
    Write-Host "- client latest assisted execution claim viewer (skipped: no requested assisted execution request found)"
}
if ($clientAssistedExecutionClaimListReadPerformed) {
    Write-Host "- client assisted execution claim list"
}
else {
    Write-Host "- client assisted execution claim list (skipped: no requested assisted execution request found)"
}
if ($assistedClaimLifecycleUpdatePerformed) {
    Write-Host "- client assisted claim lifecycle update"
}
elseif ($assistedClaimLifecycleAlreadyReleased) {
    Write-Host "- client assisted claim lifecycle update (already released; validated latest claim instead)"
}
else {
    Write-Host "- client assisted claim lifecycle update (skipped: no claimed assisted execution claim found)"
}
if ($serverLatestAssistedExecutionClaimLifecycleReadPerformed) {
    Write-Host "- server latest assisted claim status read-back"
}
else {
    Write-Host "- server latest assisted claim status read-back (skipped: no claimed assisted execution claim found)"
}
if ($clientLatestAssistedExecutionClaimLifecycleReadPerformed) {
    Write-Host "- client latest assisted claim status viewer"
}
else {
    Write-Host "- client latest assisted claim status viewer (skipped: no claimed assisted execution claim found)"
}
if ($assistedExecutionAttemptSubmissionPerformed) {
    Write-Host "- client assisted execution attempt submission"
}
elseif ($selectedClaimStatus -eq "released") {
    Write-Host "- client assisted execution attempt submission (skipped: latest claim is released)"
}
else {
    Write-Host "- client assisted execution attempt submission (skipped: no claimed assisted execution claim found)"
}
if ($serverLatestAssistedExecutionAttemptReadPerformed) {
    Write-Host "- server latest assisted execution attempt read-back"
}
elseif ($selectedClaimStatus -eq "released") {
    Write-Host "- server latest assisted execution attempt read-back (skipped: latest claim is released)"
}
else {
    Write-Host "- server latest assisted execution attempt read-back (skipped: no claimed assisted execution claim found)"
}
if ($serverAssistedExecutionAttemptListReadPerformed) {
    Write-Host "- server assisted execution attempt list"
}
elseif ($selectedClaimStatus -eq "released") {
    Write-Host "- server assisted execution attempt list (skipped: latest claim is released)"
}
else {
    Write-Host "- server assisted execution attempt list (skipped: no claimed assisted execution claim found)"
}
if ($clientLatestAssistedExecutionAttemptReadPerformed) {
    Write-Host "- client latest assisted execution attempt viewer"
}
elseif ($selectedClaimStatus -eq "released") {
    Write-Host "- client latest assisted execution attempt viewer (skipped: latest claim is released)"
}
else {
    Write-Host "- client latest assisted execution attempt viewer (skipped: no claimed assisted execution claim found)"
}
if ($clientAssistedExecutionAttemptListReadPerformed) {
    Write-Host "- client assisted execution attempt list"
}
elseif ($selectedClaimStatus -eq "released") {
    Write-Host "- client assisted execution attempt list (skipped: latest claim is released)"
}
else {
    Write-Host "- client assisted execution attempt list (skipped: no claimed assisted execution claim found)"
}
if ($serverAssistedExecutionStateListReadPerformed) {
    Write-Host "- server assisted execution state list"
}
else {
    Write-Host "- server assisted execution state list (skipped: server checks disabled)"
}
if ($clientAssistedExecutionStateListReadPerformed) {
    Write-Host "- client assisted execution state viewer"
}
else {
    Write-Host "- client assisted execution state viewer (skipped: server checks disabled)"
}
if ($serverSingleTaskAssistedExecutionStateReadPerformed) {
    Write-Host "- optional server single-task assisted execution state"
}
else {
    Write-Host "- optional server single-task assisted execution state (skipped: no tasks materialized)"
}
if ($clientSingleTaskAssistedExecutionStateReadPerformed) {
    Write-Host "- optional client single-task assisted execution state viewer"
}
else {
    Write-Host "- optional client single-task assisted execution state viewer (skipped: no tasks materialized)"
}
if ($serverProjectLocalMappingRecommendationsReadPerformed) {
    Write-Host "- server project local mapping recommendations"
}
else {
    Write-Host "- server project local mapping recommendations (skipped: server checks disabled)"
}
if ($serverNodeLocalMappingRecommendationsReadPerformed) {
    Write-Host "- server node local mapping recommendations"
}
else {
    Write-Host "- server node local mapping recommendations (skipped: server checks disabled)"
}
if ($clientLocalMappingRecommendationViewerPerformed) {
    Write-Host "- DATUM local mapping recommendation viewer"
}
else {
    Write-Host "- DATUM local mapping recommendation viewer (skipped: server checks disabled)"
}
if ($serverProjectLocalMappingPatchProposalsReadPerformed) {
    Write-Host "- server project local mapping patch proposals"
}
else {
    Write-Host "- server project local mapping patch proposals (skipped: server checks disabled)"
}
if ($serverNodeLocalMappingPatchProposalsReadPerformed) {
    Write-Host "- server node local mapping patch proposals"
}
else {
    Write-Host "- server node local mapping patch proposals (skipped: server checks disabled)"
}
if ($clientLocalMappingPatchProposalViewerPerformed) {
    Write-Host "- DATUM local mapping patch proposal viewer"
}
else {
    Write-Host "- DATUM local mapping patch proposal viewer (skipped: server checks disabled)"
}
if ($clientLocalMappingPatchProposalFileGenerationPerformed) {
    Write-Host "- DATUM local mapping patch proposal file generation"
}
else {
    Write-Host "- DATUM local mapping patch proposal file generation (skipped: server checks disabled)"
}
if ($generatedLocalMappingPatchProposalFileValidated) {
    Write-Host "- generated proposal file existence/content validation"
}
else {
    Write-Host "- generated proposal file existence/content validation (skipped: server checks disabled)"
}
if ($clientLocalMappingPatchRevalidationPerformed) {
    Write-Host "- DATUM local mapping patch revalidation"
}
else {
    Write-Host "- DATUM local mapping patch revalidation (skipped: server checks disabled)"
}
if ($generatedLocalMappingPatchProposalFileRevalidated) {
    Write-Host "- generated proposal file revalidated"
}
else {
    Write-Host "- generated proposal file revalidated (skipped: server checks disabled)"
}
if ($clientConfigUnchangedAfterPatchProposalGenerationValidated) {
    Write-Host "- active config unchanged after proposal file generation"
}
else {
    Write-Host "- active config unchanged after proposal file generation (skipped: server checks disabled)"
}
if ($clientConfigUnchangedAfterPatchRevalidationValidated) {
    Write-Host "- active config unchanged after revalidation"
}
else {
    Write-Host "- active config unchanged after revalidation (skipped: server checks disabled)"
}
if ($advisoryOnlyLocalMappingPatchProposalFileValidated) {
    Write-Host "- advisory-only/no-auto-apply validation"
}
else {
    Write-Host "- advisory-only/no-auto-apply validation (skipped: server checks disabled)"
}
if ($appliedFalseValidated) {
    Write-Host "- applied false validation"
}
else {
    Write-Host "- applied false validation (skipped: patch revalidation not performed)"
}
if ($generatedSmokeArtifactCleanupPerformed) {
    Write-Host "- generated smoke artifact cleanup"
}
else {
    Write-Host "- generated smoke artifact cleanup (skipped: server checks disabled)"
}
if ($clientLocalMappingPreflightPerformed) {
    Write-Host "- DATUM local mapping preflight"
}
else {
    Write-Host "- DATUM local mapping preflight (skipped: server checks disabled)"
}
if ($clientSingleTaskLocalMappingPreflightPerformed) {
    Write-Host "- optional DATUM single-task local mapping preflight"
}
else {
    Write-Host "- optional DATUM single-task local mapping preflight (skipped: no tasks materialized)"
}
if ($clientLocalMappingPreflightPublishPerformed) {
    Write-Host "- DATUM local mapping preflight publish"
}
else {
    Write-Host "- DATUM local mapping preflight publish (skipped: server checks disabled)"
}
if ($serverNodeLatestLocalMappingPreflightReportReadPerformed) {
    Write-Host "- server node latest local mapping preflight report read-back"
}
else {
    Write-Host "- server node latest local mapping preflight report read-back (skipped: server checks disabled)"
}
if ($serverProjectLatestLocalMappingPreflightReportListReadPerformed) {
    Write-Host "- server project latest local mapping preflight report list"
}
else {
    Write-Host "- server project latest local mapping preflight report list (skipped: server checks disabled)"
}
if ($clientSingleTaskLocalMappingPreflightPublishPerformed) {
    Write-Host "- optional DATUM single-task local mapping preflight publish"
}
else {
    Write-Host "- optional DATUM single-task local mapping preflight publish (skipped: no tasks materialized)"
}
if ($clientLocalWasmRuntimeAddTestPerformed) {
    Write-Host "- DATUM local WASM runtime add test"
}
else {
    Write-Host "- DATUM local WASM runtime add test (skipped)"
}
if ($clientLocalWasmRuntimeJsonTestPerformed) {
    Write-Host "- DATUM local WASM JSON report test"
}
else {
    Write-Host "- DATUM local WASM JSON report test (skipped)"
}
if ($clientWasmRuntimeReportPublishPerformed) {
    Write-Host "- DATUM WASM runtime report publish"
}
else {
    Write-Host "- DATUM WASM runtime report publish (skipped: server checks disabled)"
}
if ($serverNodeLatestWasmRuntimeReportReadPerformed) {
    Write-Host "- server node latest WASM runtime report read-back"
}
else {
    Write-Host "- server node latest WASM runtime report read-back (skipped: server checks disabled)"
}
if ($serverProjectLatestWasmRuntimeReportListReadPerformed) {
    Write-Host "- server project latest WASM runtime report list"
}
else {
    Write-Host "- server project latest WASM runtime report list (skipped: server checks disabled)"
}
if ($clientLatestWasmRuntimeReportViewerPerformed) {
    Write-Host "- DATUM latest WASM runtime report viewer"
}
else {
    Write-Host "- DATUM latest WASM runtime report viewer (skipped: server checks disabled)"
}
if ($clientLatestWasmRuntimeReportsViewerPerformed) {
    Write-Host "- DATUM project latest WASM runtime reports viewer"
}
else {
    Write-Host "- DATUM project latest WASM runtime reports viewer (skipped: server checks disabled)"
}
if ($serverWasmModuleDescriptorRegistrationPerformed) {
    Write-Host "- server WASM module descriptor registration/check"
}
else {
    Write-Host "- server WASM module descriptor registration/check (skipped: server checks disabled)"
}
if ($serverWasmModuleRegistryListReadPerformed) {
    Write-Host "- server WASM module registry list"
}
else {
    Write-Host "- server WASM module registry list (skipped: server checks disabled)"
}
if ($serverSingleWasmModuleDescriptorReadPerformed) {
    Write-Host "- server single WASM module descriptor"
}
else {
    Write-Host "- server single WASM module descriptor (skipped: server checks disabled)"
}
if ($clientWasmModuleRegistryViewerPerformed) {
    Write-Host "- DATUM WASM module registry viewer"
}
else {
    Write-Host "- DATUM WASM module registry viewer (skipped: server checks disabled)"
}
if ($clientSingleWasmModuleViewerPerformed) {
    Write-Host "- DATUM single WASM module viewer"
}
else {
    Write-Host "- DATUM single WASM module viewer (skipped: server checks disabled)"
}
if ($clientLocalDgraphSnapshotSummaryPerformed) {
    Write-Host "- DATUM local d-graph snapshot summary"
}
else {
    Write-Host "- DATUM local d-graph snapshot summary (skipped)"
}
if ($clientLocalDgraphSnapshotJsonPerformed) {
    Write-Host "- DATUM local d-graph snapshot JSON"
}
else {
    Write-Host "- DATUM local d-graph snapshot JSON (skipped)"
}
if ($clientLocalDgraphSnapshotPublishPerformed) {
    Write-Host "- DATUM local d-graph snapshot publish"
}
else {
    Write-Host "- DATUM local d-graph snapshot publish (skipped: server checks disabled)"
}
if ($serverNodeLatestDgraphSnapshotReadPerformed) {
    Write-Host "- server node latest d-graph snapshot read-back"
}
else {
    Write-Host "- server node latest d-graph snapshot read-back (skipped: server checks disabled)"
}
if ($serverProjectLatestDgraphSnapshotListReadPerformed) {
    Write-Host "- server project latest d-graph snapshot list"
}
else {
    Write-Host "- server project latest d-graph snapshot list (skipped: server checks disabled)"
}
if ($serverProjectMaterializedDgraphReadPerformed) {
    Write-Host "- server project d-graph materialized endpoint"
}
else {
    Write-Host "- server project d-graph materialized endpoint (skipped: server checks disabled)"
}
if ($serverProjectMaterializedDgraphSummaryReadPerformed) {
    Write-Host "- server project d-graph materialized summary endpoint"
}
else {
    Write-Host "- server project d-graph materialized summary endpoint (skipped: server checks disabled)"
}
if ($serverProjectPlannerDryRunReadPerformed) {
    Write-Host "- server d-graph planner dry-run endpoint"
}
else {
    Write-Host "- server d-graph planner dry-run endpoint (skipped: server checks disabled)"
}
if ($serverProjectPlannerDryRunSummaryReadPerformed) {
    Write-Host "- server d-graph planner dry-run summary endpoint"
}
else {
    Write-Host "- server d-graph planner dry-run summary endpoint (skipped: server checks disabled)"
}
if ($clientLatestDgraphSnapshotViewerPerformed) {
    Write-Host "- DATUM latest d-graph snapshot viewer"
}
else {
    Write-Host "- DATUM latest d-graph snapshot viewer (skipped: server checks disabled)"
}
if ($clientLatestAllDgraphSnapshotViewerPerformed) {
    Write-Host "- DATUM latest-all d-graph snapshot viewer"
}
else {
    Write-Host "- DATUM latest-all d-graph snapshot viewer (skipped: server checks disabled)"
}
if ($clientMaterializedDgraphViewerPerformed) {
    Write-Host "- DATUM materialized d-graph viewer"
}
else {
    Write-Host "- DATUM materialized d-graph viewer (skipped: server checks disabled)"
}
if ($clientMaterializedDgraphSummaryViewerPerformed) {
    Write-Host "- DATUM materialized d-graph summary viewer"
}
else {
    Write-Host "- DATUM materialized d-graph summary viewer (skipped: server checks disabled)"
}
if ($clientMaterializedDgraphJsonViewerPerformed) {
    Write-Host "- DATUM materialized d-graph JSON viewer"
}
else {
    Write-Host "- DATUM materialized d-graph JSON viewer (skipped: server checks disabled)"
}
if ($clientMaterializedDgraphSummaryJsonViewerPerformed) {
    Write-Host "- DATUM materialized d-graph summary JSON viewer"
}
else {
    Write-Host "- DATUM materialized d-graph summary JSON viewer (skipped: server checks disabled)"
}
if ($clientPlannerDryRunViewerPerformed) {
    Write-Host "- DATUM planner dry-run viewer"
}
else {
    Write-Host "- DATUM planner dry-run viewer (skipped: server checks disabled)"
}
if ($clientPlannerSummaryViewerPerformed) {
    Write-Host "- DATUM planner summary viewer"
}
else {
    Write-Host "- DATUM planner summary viewer (skipped: server checks disabled)"
}
if ($clientPlannerDryRunJsonViewerPerformed) {
    Write-Host "- DATUM planner dry-run JSON viewer"
}
else {
    Write-Host "- DATUM planner dry-run JSON viewer (skipped: server checks disabled)"
}
if ($clientPlannerSummaryJsonViewerPerformed) {
    Write-Host "- DATUM planner summary JSON viewer"
}
else {
    Write-Host "- DATUM planner summary JSON viewer (skipped: server checks disabled)"
}
if ($localFixtureScenarioJsonChecksPerformed) {
    Write-Host "- local fixture scenario JSON checks"
}
else {
    Write-Host "- local fixture scenario JSON checks (skipped)"
}
if ($fixtureMetadataValidated) {
    Write-Host "- fixture metadata validation"
}
else {
    Write-Host "- fixture metadata validation (skipped)"
}
if ($fixturePublishReadBackPerformed) {
    Write-Host "- selected fixture publish/read-back"
}
else {
    Write-Host "- selected fixture publish/read-back (skipped: server checks disabled)"
}
if ($materializedAfterFixturePublishValidated) {
    Write-Host "- materialization after fixture publish"
}
else {
    Write-Host "- materialization after fixture publish (skipped: server checks disabled)"
}
if ($plannerAfterFixturePublishValidated) {
    Write-Host "- planner dry-run after fixture publish"
}
else {
    Write-Host "- planner dry-run after fixture publish (skipped: server checks disabled)"
}
if ($serverAssistedExecutionStateLocalMappingContextReadPerformed) {
    Write-Host "- server assisted execution state local mapping context check"
}
else {
    Write-Host "- server assisted execution state local mapping context check (skipped: server checks disabled)"
}
if ($serverSingleTaskAssistedExecutionStateLocalMappingContextReadPerformed) {
    Write-Host "- optional server single-task local mapping context check"
}
else {
    Write-Host "- optional server single-task local mapping context check (skipped: no tasks materialized)"
}
if ($clientAssistedExecutionStateLocalMappingContextViewerPerformed) {
    Write-Host "- DATUM execution-state local mapping context viewer"
}
else {
    Write-Host "- DATUM execution-state local mapping context viewer (skipped: server checks disabled)"
}
if ($clientSingleTaskAssistedExecutionStateLocalMappingContextViewerPerformed) {
    Write-Host "- optional DATUM single-task local mapping context viewer"
}
else {
    Write-Host "- optional DATUM single-task local mapping context viewer (skipped: no tasks materialized)"
}
if ($safeToAutoApplyFalseValidated) {
    Write-Host "- safe_to_auto_apply false validation"
}
else {
    Write-Host "- safe_to_auto_apply false validation (skipped: no local mapping recommendations or patch proposals returned)"
}
if ($requiresOperatorTrueValidated) {
    Write-Host "- requires_operator true validation"
}
else {
    Write-Host "- requires_operator true validation (skipped: no local mapping recommendations or patch proposals returned)"
}
Write-Host "- advisory-only proposed config snippet guarantee"
if ($assistedExecutionAttemptSubmissionPerformed -or
    $serverLatestAssistedExecutionAttemptReadPerformed -or
    $serverAssistedExecutionAttemptListReadPerformed) {
    Write-Host "- safe_to_auto_execute false validation for attempts"
}
else {
    Write-Host "- safe_to_auto_execute false validation for attempts (skipped: no assisted execution attempts validated)"
}
if ($sideEffectsPerformedFalseValidated) {
    Write-Host "- side_effects_performed false validation"
}
else {
    Write-Host "- side_effects_performed false validation (skipped: no side_effects_performed response field was validated)"
}
if ($safeToAutoExecuteFalseValidatedForStateSummaries) {
    Write-Host "- safe_to_auto_execute false validation for state summaries"
}
else {
    Write-Host "- safe_to_auto_execute false validation for state summaries (skipped: no assisted execution state summaries returned)"
}
if ($sideEffectsPerformedFalseValidatedForStateSummaries) {
    Write-Host "- side_effects_performed false validation for state summaries"
}
else {
    Write-Host "- side_effects_performed false validation for state summaries (skipped: no state summary exposed side_effects_performed)"
}
if ($safeToAutoExecuteFalseValidatedForLocalMappingPreflight) {
    Write-Host "- safe_to_auto_execute false validation for local mapping preflight"
}
else {
    Write-Host "- safe_to_auto_execute false validation for local mapping preflight (skipped: no local mapping preflight was run)"
}
if ($sideEffectsPerformedFalseValidatedForLocalMappingPreflight) {
    Write-Host "- side_effects_performed false validation for local mapping preflight"
}
else {
    Write-Host "- side_effects_performed false validation for local mapping preflight (skipped: no local mapping preflight was run)"
}
if ($safeAccessFlagsFalseValidatedForWasm) {
    Write-Host "- all WASM safety/access flags false validation"
}
else {
    Write-Host "- all WASM safety/access flags false validation (skipped: no WASM runtime report or registry descriptor was validated)"
}
if ($wasmArtifactMetadataOnlyValidated) {
    Write-Host "- metadata-only artifact reference validation"
}
else {
    Write-Host "- metadata-only artifact reference validation (skipped: no WASM module registry descriptor was validated)"
}
if ($safeToAutoExecuteFalseValidatedForDgraphSnapshots) {
    Write-Host "- safe_to_auto_execute false validation for d-graph snapshots"
}
else {
    Write-Host "- safe_to_auto_execute false validation for d-graph snapshots (skipped: no d-graph snapshots were validated)"
}
if ($sideEffectsPerformedFalseValidatedForDgraphSnapshots) {
    Write-Host "- side_effects_performed false validation for d-graph snapshots"
}
else {
    Write-Host "- side_effects_performed false validation for d-graph snapshots (skipped: no d-graph snapshots were validated)"
}
if ($sideEffectsPerformedFalseValidatedForDgraphMaterialization) {
    Write-Host "- materialized graph side_effects_performed false validation"
}
else {
    Write-Host "- materialized graph side_effects_performed false validation (skipped: no materialized d-graph responses were validated)"
}
if ($sideEffectsPerformedFalseValidatedForDgraphPlanner) {
    Write-Host "- planner side_effects_performed false validation"
}
else {
    Write-Host "- planner side_effects_performed false validation (skipped: no planner dry-run responses were validated)"
}
if ($sideEffectsPerformedFalseValidatedForDgraphFixtures) {
    Write-Host "- fixture scenario side_effects_performed false validation"
}
else {
    Write-Host "- fixture scenario side_effects_performed false validation (skipped: no fixture scenarios were validated)"
}
Write-Host "- planner JSON viewer checks"
Write-Host "- D-graph planner decisions not executed"
Write-Host "- D-graph deployment/reconfiguration not executed"
Write-Host ""
Write-Host "No dispatch execution was performed."
Write-Host "No deployment or installation execution was performed."
Write-Host "No Docker/container/WASM-host/migration side effects were performed."
Write-Host "Operator approval, when performed, recorded metadata only."
Write-Host "Assisted readiness, when ready, was informational only."
Write-Host "Assisted execution request, when performed, recorded metadata only."
Write-Host "Assisted execution claim, when performed, recorded metadata only."
Write-Host "Assisted claim lifecycle update, when performed, recorded metadata only."
Write-Host "Assisted execution attempt, when performed, recorded no-op/dry-run metadata only."
Write-Host "Assisted execution state checks were read-only."
Write-Host "Local mapping recommendations and preflight checks were advisory only."
Write-Host "Local mapping preflight publishing, when performed, recorded advisory metadata only."
Write-Host "Local mapping context in assisted execution state was advisory only."
Write-Host "Local mapping patch proposals were advisory only."
Write-Host "Local mapping patch proposal file generation wrote only a separate operator-review file."
Write-Host "Local mapping patch revalidation was read-only and did not apply changes."
Write-Host "WASM runtime test executed only an isolated embedded module."
Write-Host "WASM runtime report publishing recorded metadata only."
Write-Host "WASM module registry checks were metadata-only."
Write-Host "WASM artifact references were not fetched or executed."
Write-Host "D-graph fixture scenarios were metadata-only."
Write-Host "D-graph fixture publishing recorded metadata only."
Write-Host "D-graph snapshot publishing recorded metadata only."
Write-Host "D-graph materialization was read-only."
Write-Host "D-graph planner dry-run was advisory/read-only."
Write-Host "D-graph planner decisions were not executed."
Write-Host "D-graph deployment/reconfiguration execution was not performed."
Write-Host "WASM was not connected to installation tasks or assisted execution."
Write-Host "No local config or mapping files were modified."
Write-Host "No proposed config snippet was applied."
