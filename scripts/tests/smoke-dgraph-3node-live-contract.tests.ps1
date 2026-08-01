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

$publicationBlock = [regex]::Match($source, 'function Invoke-LocalDatumPublication\(\[string\[\]\]\$Arguments, \[string\]\$Step\) \{(?<body>[\s\S]*?)^\}', [System.Text.RegularExpressions.RegexOptions]::Multiline)
if (-not $publicationBlock.Success) { throw "Expected Invoke-LocalDatumPublication function." }
$publicationSource = $publicationBlock.Groups['body'].Value
Assert-Contains $publicationSource '--url", \$ServerUrl'
Assert-Contains $publicationSource '--project-id", \$ProjectId'
Assert-Contains $publicationSource 'Invoke-LocalDatum \$publicationArguments \$Step'

$fixturePublishBlock = [regex]::Match($source, 'function Publish-Fixture\([\s\S]*?\) \{(?<body>[\s\S]*?)^\}', [System.Text.RegularExpressions.RegexOptions]::Multiline)
if (-not $fixturePublishBlock.Success) { throw "Expected Publish-Fixture function." }
Assert-Contains $fixturePublishBlock.Groups['body'].Value 'Invoke-LocalDatumPublication \$args \$Step'

# A local config may contain a divergent URL or project ID. The publication overrides
# below must remain authoritative for each of the three valid node configs.
foreach ($configName in @("Fog01Config", "Fog02Config", "CloudConfig")) {
    Assert-Contains $source ("\[string\]\$" + $configName)
}
Assert-Contains $source 'Invoke-LocalDatumPublication @\("run", "--quiet", "--", "dgraph-snapshot", "--config", \$ResolvedFog02Config, "--observed-health", "--publish"\) "Observed-health publish"'
if ($source -match 'Invoke-LocalDatum @\("run", "--quiet", "--", "dgraph-snapshot", "--config", \$ResolvedFog02Config, "--observed-health", "--publish"\)') {
    throw "Observed-health publication must use requested URL/project overrides."
}

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
