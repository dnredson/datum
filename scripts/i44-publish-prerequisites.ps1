[CmdletBinding()]
param(
    [string]$BaseUrl = "http://192.168.10.101:8080",
    [string]$ProjectId = "i44-live-project",
    [int]$LeaseSeconds = 300
)

$ErrorActionPreference = "Stop"
$BaseUrl = $BaseUrl.TrimEnd('/')
$Headers = @{ "X-Datum-Project" = $ProjectId }

function Publish-SentinelRegistration {
    param(
        [string]$NodeId,
        [string]$InstanceId,
        [string]$Stage
    )

    $Body = @{
        node_id = $NodeId
        instance_id = $InstanceId
        stage = $Stage
        capabilities = @("artifact_fetch", "wasmtime")
        lease_seconds = $LeaseSeconds
    } | ConvertTo-Json

    Invoke-RestMethod `
        -Method Put `
        -Uri "$BaseUrl/api/v1/sentinels/$NodeId/registration" `
        -ContentType "application/json" `
        -Body $Body | Out-Null
}

function Publish-ResourceObservation {
    param(
        [string]$NodeId,
        [int]$LogicalCores,
        [double]$CpuUsagePercent,
        [Int64]$TotalBytes,
        [Int64]$AvailableBytes,
        [int]$ProcessCount,
        [int]$ContainerCount,
        [string[]]$Capabilities
    )

    $CapabilityItems = @(
        foreach ($Capability in $Capabilities) {
            @{
                capability_id = $Capability
                capability_kind = "runtime"
                supported = $true
                source = "i44-phase-final"
            }
        }
    )

    $Body = @{
        metadata = @{
            resource_observation = @{
                observed_at_utc = [DateTime]::UtcNow.ToString("o")
                cpu = @{
                    logical_cores = $LogicalCores
                    usage_percent = $CpuUsagePercent
                }
                memory = @{
                    total_bytes = $TotalBytes
                    available_bytes = $AvailableBytes
                }
                process_count = $ProcessCount
                container_count = $ContainerCount
            }
        }
        capabilities = $CapabilityItems
        safe_to_auto_execute = $false
        side_effects_performed = $false
    } | ConvertTo-Json -Depth 10

    Invoke-RestMethod `
        -Method Post `
        -Uri "$BaseUrl/api/v1/nodes/$NodeId/dgraph-snapshot" `
        -Headers $Headers `
        -ContentType "application/json" `
        -Body $Body | Out-Null
}

Publish-SentinelRegistration -NodeId "fog-01" -InstanceId "fog-i44" -Stage "fog"
Publish-SentinelRegistration -NodeId "local-01" -InstanceId "local-i44" -Stage "local"
Publish-SentinelRegistration -NodeId "cloud-01" -InstanceId "cloud-i44" -Stage "cloud"

Publish-ResourceObservation `
    -NodeId "fog-01" `
    -LogicalCores 4 `
    -CpuUsagePercent 40.0 `
    -TotalBytes 8589934592 `
    -AvailableBytes 4294967296 `
    -ProcessCount 130 `
    -ContainerCount 3 `
    -Capabilities @("wasm", "docker")

Publish-ResourceObservation `
    -NodeId "local-01" `
    -LogicalCores 16 `
    -CpuUsagePercent 25.0 `
    -TotalBytes 34359738368 `
    -AvailableBytes 17179869184 `
    -ProcessCount 220 `
    -ContainerCount 5 `
    -Capabilities @("wasm", "docker")

Publish-ResourceObservation `
    -NodeId "cloud-01" `
    -LogicalCores 32 `
    -CpuUsagePercent 15.0 `
    -TotalBytes 68719476736 `
    -AvailableBytes 51539607552 `
    -ProcessCount 180 `
    -ContainerCount 8 `
    -Capabilities @("wasm", "gpu")

$Sentinels = Invoke-RestMethod "$BaseUrl/api/v1/sentinels"
if ($Sentinels.count -ne 3 -or $Sentinels.active_count -ne 3 -or $Sentinels.stale_count -ne 0) {
    throw "I44 prerequisite publication failed: expected three active Sentinels"
}

Write-Host "I44 prerequisites PASS"
Write-Host "project_id=$ProjectId"
Write-Host "sentinel_count=$($Sentinels.count)"
Write-Host "active_count=$($Sentinels.active_count)"
Write-Host "stale_count=$($Sentinels.stale_count)"
Write-Host "resource_observations=fog-01,local-01,cloud-01"
Write-Host "execution_performed=false"
