$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$scriptPath = Join-Path $repoRoot "scripts\smoke-dgraph-observed-health-matrix.ps1"
$source = Get-Content -LiteralPath $scriptPath -Raw

function Assert-Contains([string]$Text, [string]$Pattern) {
    if ($Text -notmatch $Pattern) { throw "Expected script contract matching: $Pattern" }
}

foreach ($parameter in @(
    "CloudConfig", "Fog01Config", "Fog02Config", "ObservedSnapshot",
    "ObservedModuleId", "OutputDir", "KeepEvidence"
)) {
    Assert-Contains $source ("\$" + $parameter)
}

foreach ($scenario in @("healthy", "degraded", "stale", "absent", "unrecognized", "future-skewed")) {
    Assert-Contains $source ('Name\s*=\s*"' + [regex]::Escape($scenario) + '"')
}

foreach ($required in @(
    'New-ScenarioSnapshot \$scenario \$scenarioSnapshotPath',
    'New-ScenarioConfig \$scenarioSnapshotPath \$scenarioConfigPath',
    'Assert-ScenarioHealth \$local \$scenario',
    'Assert-PublishedScenario \$scenario \$scenarioConfigPath',
    'finally \{\s*Restore-Fog02Baseline',
    'Assert-Baseline "Initial baseline"',
    'I33 final emergency restoration',
    'Get-FileHash -LiteralPath \$ResolvedFog02Config',
    'Get-FileHash -LiteralPath \$ResolvedObservedSnapshot',
    'source config and persisted snapshot hashes unchanged',
    'metadata\.checks_executed',
    'ReasonPattern',
    'health\.reason',
    'safe_to_auto_execute',
    'side_effects_performed',
    '--url", \$DGraphApiUrl',
    '--project-id", \$ProjectId',
    'PSVersionTable\.PSVersion\.Major -lt 7',
    'PowerShell 7 or later is required'
)) { Assert-Contains $source $required }

$snapshotWriter = [regex]::Match(
    $source,
    'function New-ScenarioSnapshot\([\s\S]*?\) \{(?<body>[\s\S]*?)^\}',
    [System.Text.RegularExpressions.RegexOptions]::Multiline
)
if (-not $snapshotWriter.Success) { throw "Expected New-ScenarioSnapshot function." }
Assert-Contains $snapshotWriter.Groups['body'].Value 'Get-Content -LiteralPath \$ResolvedObservedSnapshot'
Assert-Contains $snapshotWriter.Groups['body'].Value 'Set-Content -LiteralPath \$ScenarioSnapshotPath'
Assert-Contains $snapshotWriter.Groups['body'].Value 'Add-Member -NotePropertyName "status"'
if ($snapshotWriter.Groups['body'].Value -match 'Set-Content -LiteralPath \$ResolvedObservedSnapshot') {
    throw "I33 must never write the source persisted snapshot."
}

$configWriter = [regex]::Match(
    $source,
    'function New-ScenarioConfig\([\s\S]*?\) \{(?<body>[\s\S]*?)^\}',
    [System.Text.RegularExpressions.RegexOptions]::Multiline
)
if (-not $configWriter.Success) { throw "Expected New-ScenarioConfig function." }
Assert-Contains $configWriter.Groups['body'].Value 'Get-Content -LiteralPath \$ResolvedFog02Config'
Assert-Contains $configWriter.Groups['body'].Value 'Set-Content -LiteralPath \$ScenarioConfigPath'
if ($configWriter.Groups['body'].Value -match 'Set-Content -LiteralPath \$ResolvedFog02Config') {
    throw "I33 must never write the source fog-02 config."
}

$implementation = $source
foreach ($forbidden in @(
    "datum once", "Command::Once", "dispatch --execute", "planner decision execution",
    "docker run", "docker pull", "run_embedded_wasm", "Invoke-CloudDatum", "& ssh"
)) {
    if ($implementation -match [regex]::Escape($forbidden)) {
        # The leading safety comment/final summary may name forbidden actions. Only
        # reject command-like occurrences outside explanatory prose.
        $commandPattern = '(?im)^\s*[^#\"]*' + [regex]::Escape($forbidden)
        if ($implementation -match $commandPattern) {
            throw "I33 must not invoke forbidden operation: $forbidden"
        }
    }
}

$tokens = $null
$parseErrors = $null
[void][System.Management.Automation.Language.Parser]::ParseFile(
    $scriptPath,
    [ref]$tokens,
    [ref]$parseErrors
)
if ($parseErrors.Count -ne 0) {
    throw "PowerShell parser errors: $($parseErrors.Message -join '; ')"
}

Write-Host "I33 observed-health matrix offline contract tests passed."
