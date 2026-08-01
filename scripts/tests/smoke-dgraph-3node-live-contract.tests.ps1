$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$scriptPath = Join-Path $repoRoot "scripts\smoke-dgraph-3node-live.ps1"
$source = Get-Content -LiteralPath $scriptPath -Raw

function Assert-Contains([string]$Text, [string]$Pattern) {
    if ($Text -notmatch $Pattern) { throw "Expected script contract matching: $Pattern" }
}

foreach ($parameter in @("CloudConfig", "Fog01Config", "Fog02Config")) {
    Assert-Contains $source ("\[string\]\$" + $parameter)
}
foreach ($removed in @("CloudSshTarget", "CloudDatumDir", "Invoke-CloudDatum", "ConvertTo-PosixSingleQuoted", "& ssh", "remote cargo")) {
    if ($source -match [regex]::Escape($removed)) { throw "I32 must not require remote cloud execution: found '$removed'." }
}
foreach ($required in @(
    'Publish-Fixture \$Fog01Id', 'Publish-Fixture \$Fog02Id', 'Publish-Fixture \$CloudId',
    "degraded-flow", "--observed-health", "Mandatory fog-02 baseline restoration",
    'Assert-Baseline "Restored baseline"', "checks_executed", "relationship_count", "unhealthy_targets",
    'PSVersionTable\.PSVersion\.Major -lt 7', "PowerShell 7 or later is required", "Invoke-LocalDatumJson",
    'cargo @Arguments 2> \$stderrPath', 'Stdout = \$stdout', 'Stderr = \$stderr', 'stdout:', 'stderr:'
)) { Assert-Contains $source $required }

Assert-Contains $source '\$ResolvedCloudConfig = Resolve-RepoPath \$CloudConfig'
Assert-Contains $source 'Cargo.toml'
Assert-Contains $source 'snapshot-fog-02.json'
Assert-Contains $source 'Publish-Fixture \$CloudId \$ResolvedCloudConfig "baseline" "Publish cloud-01 baseline"'

$observedHealthBlock = [regex]::Match($source, 'function Assert-ObservedHealth\(\) \{(?<body>[\s\S]*?)^\}', [System.Text.RegularExpressions.RegexOptions]::Multiline)
if (-not $observedHealthBlock.Success) { throw "Expected Assert-ObservedHealth function." }
$observedHealthSource = $observedHealthBlock.Groups['body'].Value
Assert-Contains $observedHealthSource 'Invoke-LocalDatumJson \$args "Observed-health local read"'
Assert-Contains $observedHealthSource '\$local\.Stdout \| ConvertFrom-Json'
Assert-Contains $observedHealthSource '\$local\.Stderr\.Trim\(\)'

$tokens = $null; $parseErrors = $null
[void][System.Management.Automation.Language.Parser]::ParseFile($scriptPath, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count -ne 0) { throw "PowerShell parser errors: $($parseErrors.Message -join '; ')" }
Write-Host "I32 offline smoke-script contract tests passed."
