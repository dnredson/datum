$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$scriptPath = Join-Path $repoRoot "scripts\smoke-dgraph-3node-live.ps1"
$source = Get-Content -LiteralPath $scriptPath -Raw

function Assert-Contains([string]$Text, [string]$Pattern) {
    if ($Text -notmatch $Pattern) { throw "Expected script contract matching: $Pattern" }
}

foreach ($parameter in @("CloudSshTarget", "CloudDatumDir", "CloudConfig", "Fog01Config", "Fog02Config")) {
    Assert-Contains $source ("\[string\]\$" + $parameter)
}
foreach ($required in @(
    'Publish-Fixture \$Fog01Id', 'Publish-Fixture \$Fog02Id', 'Publish-Fixture \$CloudId',
    "degraded-flow", "--observed-health", "Mandatory fog-02 baseline restoration",
    'Assert-Baseline "Restored baseline"', "checks_executed", "relationship_count", "unhealthy_targets"
)) { Assert-Contains $source $required }

$tokens = $null; $parseErrors = $null
[void][System.Management.Automation.Language.Parser]::ParseFile($scriptPath, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count -ne 0) { throw "PowerShell parser errors: $($parseErrors.Message -join '; ')" }
Write-Host "I32 offline smoke-script contract tests passed."
