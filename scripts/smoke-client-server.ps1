param(
    [string]$ProjectId = "smart-city-campinas",
    [string]$NodeId = "fog-01",
    [string]$ServerDir = "C:\Users\dnred\OneDrive\projetos\PD\dserver",
    [string]$ClientDir = "C:\Users\dnred\OneDrive\projetos\PD\DATUM",
    [string]$BaseUrl = "http://localhost:8080"
)

$ErrorActionPreference = "Stop"

function Invoke-DatumCurl {
    param(
        [string]$Path
    )

    $url = "$BaseUrl$Path"
    Write-Host ">>> GET $url"
    curl.exe -s -H "X-Datum-Project: $ProjectId" $url
    Write-Host ""
}

$job = Start-Job -ScriptBlock {
    param($Dir)
    Set-Location $Dir
    cargo run
} -ArgumentList $ServerDir

try {
    Start-Sleep -Seconds 8

    Write-Host ">>> HEALTH"
    curl.exe -s -H "X-Datum-Project: $ProjectId" "$BaseUrl/health"
    Write-Host ""

    Write-Host ">>> DATUM ONCE"
    Push-Location $ClientDir
    cargo run -- once --config config.toml
    Pop-Location

    Invoke-DatumCurl "/api/v1/graph/latest"
    Invoke-DatumCurl "/api/v1/analysis/latest"
    Invoke-DatumCurl "/api/v1/action-plan/latest"
    Invoke-DatumCurl "/api/v1/dispatch/latest"
    Invoke-DatumCurl "/api/v1/dispatch/for-node/$NodeId"

    Write-Host ">>> DATUM DISPATCH"
    Push-Location $ClientDir
    cargo run -- dispatch --config config.toml

    Write-Host ">>> DATUM DISPATCH --EXECUTE"
    cargo run -- dispatch --config config.toml --execute
    Pop-Location
}
finally {
    Stop-Job -Job $job -ErrorAction SilentlyContinue | Out-Null
    Receive-Job -Job $job -Keep -ErrorAction SilentlyContinue | Out-Null
    Remove-Job -Job $job -Force -ErrorAction SilentlyContinue | Out-Null
}
