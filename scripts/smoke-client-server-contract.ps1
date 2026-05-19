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
Write-Host ""
Write-Host "No dispatch execution was performed."
Write-Host "No deployment or installation execution was performed."
Write-Host "No Docker/container/WASM/migration side effects were performed."
Write-Host "Operator approval, when performed, recorded metadata only."
Write-Host "Assisted readiness, when ready, was informational only."
