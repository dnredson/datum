[CmdletBinding()]
param(
    [string]$BaseUrl = "http://192.168.10.101:8080",
    [string]$ProjectId = "i44-live-project",
    [string]$ManifestId = "manifest:i44-phase-final-three-node",
    [int]$ExpectedReportCount = 3,
    [int]$ExpectedActiveSentinelCount = 3,
    [int]$ExpectedStaleSentinelCount = 0,
    [int]$ExpectedResourceObservationCount = 3,
    [int]$ExpectedFreshResourceCount = 3,
    [int]$ExpectedStaleResourceCount = 0,
    [int]$ExpectedLifecycleAttemptCount = 3,
    [ValidateSet("ready_for_explicit_execution", "blocked")]
    [string]$ExpectedReadinessStatus = "ready_for_explicit_execution",
    [int]$ExpectedReadyAssignmentCount = 0,
    [int]$ExpectedWaitingAssignmentCount = 0,
    [int]$ExpectedCompletedAssignmentCount = 3,
    [int]$ExpectedBlockedAssignmentCount = 0,
    [int]$ExpectedBlockingReasonCount = 0
)

$ErrorActionPreference = "Stop"
$BaseUrl = $BaseUrl.TrimEnd('/')

function Assert-Equal {
    param(
        [string]$Name,
        $Actual,
        $Expected
    )
    if ($Actual -ne $Expected) {
        throw "$Name mismatch: expected '$Expected', received '$Actual'"
    }
}

function Assert-False {
    param(
        [string]$Name,
        $Actual
    )
    if ($Actual -ne $false) {
        throw "$Name must remain false; received '$Actual'"
    }
}

$Source = Invoke-RestMethod "$BaseUrl/api/v1/controlled-wasm/dgraph"
$Manifest = Invoke-RestMethod "$BaseUrl/api/v1/controlled-wasm/deployment-manifest"
$Reports = Invoke-RestMethod "$BaseUrl/api/v1/controlled-wasm/live-reports"
$Sentinels = Invoke-RestMethod "$BaseUrl/api/v1/sentinels"
$Attempts = Invoke-RestMethod "$BaseUrl/api/v1/controlled-wasm/deployment-attempts"
$Observed = Invoke-RestMethod "$BaseUrl/api/v1/controlled-wasm/observed-dgraph"

Assert-Equal "source.project_id" $Source.project_id $ProjectId
Assert-Equal "manifest.project_id" $Manifest.project_id $ProjectId
Assert-Equal "manifest.manifest_id" $Manifest.manifest_id $ManifestId
Assert-Equal "observed.project_id" $Observed.project_id $ProjectId
Assert-Equal "observed.manifest_id" $Observed.manifest_id $ManifestId

Assert-Equal "manifest.assignment_count" @($Manifest.assignments).Count 3
Assert-Equal "reports.count" $Reports.count $ExpectedReportCount
Assert-Equal "observed.report_count" $Observed.report_count $ExpectedReportCount
Assert-Equal "sentinels.active_count" $Sentinels.active_count $ExpectedActiveSentinelCount
Assert-Equal "sentinels.stale_count" $Sentinels.stale_count $ExpectedStaleSentinelCount
Assert-Equal "attempts.count" $Attempts.count $ExpectedLifecycleAttemptCount
Assert-Equal "observed.validation_errors" @($Observed.validation_errors).Count 0

if ($Manifest.execution_boundary.explicit_invocation_required -ne $true) {
    throw "manifest.execution_boundary.explicit_invocation_required must remain true"
}
Assert-False "manifest.execution_boundary.automatic_execution_allowed" $Manifest.execution_boundary.automatic_execution_allowed
Assert-False "manifest.execution_boundary.governed_execution_authorized" $Manifest.execution_boundary.governed_execution_authorized

$Join = $Observed.metadata.resource_join
if ($null -eq $Join) {
    throw "observed metadata.resource_join is missing"
}
Assert-Equal "resource_join.observation_count" $Join.observation_count $ExpectedResourceObservationCount
Assert-Equal "resource_join.fresh_count" $Join.fresh_count $ExpectedFreshResourceCount
Assert-Equal "resource_join.stale_count" $Join.stale_count $ExpectedStaleResourceCount
Assert-Equal "resource_join.missing_node_ids" @($Join.missing_node_ids).Count 0
Assert-Equal "resource_join.unmatched_node_ids" @($Join.unmatched_node_ids).Count 0
Assert-False "resource_join.automatic_placement_performed" $Join.automatic_placement_performed
Assert-False "resource_join.deployment_side_effects_performed" $Join.deployment_side_effects_performed
Assert-False "resource_join.governed_execution_performed" $Join.governed_execution_performed

$Readiness = $Observed.metadata.deployment_readiness
if ($null -eq $Readiness) {
    throw "observed metadata.deployment_readiness is missing"
}
Assert-Equal "readiness.status" $Readiness.status $ExpectedReadinessStatus
Assert-Equal "readiness.assignment_count" $Readiness.assignment_count 3
Assert-Equal "readiness.target_node_count" $Readiness.target_node_count 3
Assert-Equal "readiness.ready_assignment_count" $Readiness.ready_assignment_count $ExpectedReadyAssignmentCount
Assert-Equal "readiness.waiting_assignment_count" $Readiness.waiting_assignment_count $ExpectedWaitingAssignmentCount
Assert-Equal "readiness.completed_assignment_count" $Readiness.completed_assignment_count $ExpectedCompletedAssignmentCount
Assert-Equal "readiness.blocked_assignment_count" $Readiness.blocked_assignment_count $ExpectedBlockedAssignmentCount
Assert-Equal "readiness.blocking_reasons" @($Readiness.blocking_reasons).Count $ExpectedBlockingReasonCount
Assert-False "readiness.automatic_execution_allowed" $Readiness.automatic_execution_allowed
Assert-False "readiness.governed_execution_performed" $Readiness.governed_execution_performed
Assert-False "readiness.readiness_side_effects_performed" $Readiness.readiness_side_effects_performed

$LifecycleCount = 0
if ($null -ne $Observed.metadata -and $null -ne $Observed.metadata.deployment_lifecycle_attempt_count) {
    $LifecycleCount = [int]$Observed.metadata.deployment_lifecycle_attempt_count
}
Assert-Equal "observed.deployment_lifecycle_attempt_count" $LifecycleCount $ExpectedLifecycleAttemptCount

$ExpectedStages = @{
    "a-fog-scale" = 0
    "m-local-aggregate" = 1
    "z-cloud-delta" = 2
}
$ExpectedNodes = @{
    "a-fog-scale" = "fog-01"
    "m-local-aggregate" = "local-01"
    "z-cloud-delta" = "cloud-01"
}

foreach ($Assignment in $Manifest.assignments) {
    $ExpectedStage = $ExpectedStages[$Assignment.assignment_id]
    $ExpectedNode = $ExpectedNodes[$Assignment.assignment_id]
    if ($null -eq $ExpectedStage -or $null -eq $ExpectedNode) {
        throw "unexpected assignment '$($Assignment.assignment_id)'"
    }
    Assert-Equal "assignment.$($Assignment.assignment_id).target_node" $Assignment.target_node $ExpectedNode
    Assert-Equal "assignment.$($Assignment.assignment_id).stage" $Assignment.metadata.deployment_stage $ExpectedStage
}

$ExpectedPlacements = @{
    "fog-scale-module" = "fog-01"
    "local-aggregate-module" = "local-01"
    "cloud-delta-module" = "cloud-01"
}
foreach ($Placement in $Observed.graph.placements) {
    if ($ExpectedPlacements.ContainsKey($Placement.module_id)) {
        Assert-Equal "placement.$($Placement.module_id)" $Placement.selected_node_id $ExpectedPlacements[$Placement.module_id]
    }
}

if ($ExpectedReportCount -eq 3) {
    $ReportAssignments = @($Reports.reports | ForEach-Object { $_.assignment_id } | Sort-Object -Unique)
    foreach ($Required in @("a-fog-scale", "m-local-aggregate", "z-cloud-delta")) {
        if ($ReportAssignments -notcontains $Required) {
            throw "required executed report for '$Required' is missing"
        }
    }
}

Write-Host "I44 controlled-state validation PASS"
Write-Host "project_id=$ProjectId"
Write-Host "manifest_id=$ManifestId"
Write-Host "report_count=$($Reports.count)"
Write-Host "active_sentinels=$($Sentinels.active_count)"
Write-Host "resource_observations=$($Join.observation_count) fresh=$($Join.fresh_count) stale=$($Join.stale_count)"
Write-Host "lifecycle_attempts=$($Attempts.count)"
Write-Host "readiness=$($Readiness.status) completed=$($Readiness.completed_assignment_count)"
Write-Host "validation_errors=0"
Write-Host "automatic_execution_allowed=false"
Write-Host "governed_execution_performed=false"
