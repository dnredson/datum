param(
    [Parameter(Mandatory = $true)]
    [string]$ProjectId,

    [string]$RunRegion,
    [string]$FunctionName = "datum-firestore-passive-relay",
    [string]$TriggerName = "datum-firestore-latest-passive",
    [string]$TopicName = "datum-passive-observation-notifications",
    [string]$ServiceAccountName = "datum-passive-relay"
)

$ErrorActionPreference = "Stop"

function Invoke-GCloud {
    param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Args)
    & gcloud @Args
    if ($LASTEXITCODE -ne 0) {
        throw "gcloud failed: gcloud $($Args -join ' ')"
    }
}

Invoke-GCloud config set project $ProjectId

Invoke-GCloud services enable `
    run.googleapis.com `
    eventarc.googleapis.com `
    pubsub.googleapis.com `
    firestore.googleapis.com `
    cloudbuild.googleapis.com `
    artifactregistry.googleapis.com

$FirestoreLocation = (& gcloud firestore databases describe `
    --database="(default)" `
    --project=$ProjectId `
    --format="value(locationId)").Trim()
if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($FirestoreLocation)) {
    throw "Could not determine Firestore location for project '$ProjectId'."
}

if ([string]::IsNullOrWhiteSpace($RunRegion)) {
    if ($FirestoreLocation -match '^[a-z]+(?:-[a-z]+)+[0-9]$') {
        $RunRegion = $FirestoreLocation
    } else {
        throw "Firestore uses multi-region '$FirestoreLocation'. Re-run with -RunRegion set to a supported Cloud Run region near that database."
    }
}

$ServiceAccountEmail = "$ServiceAccountName@$ProjectId.iam.gserviceaccount.com"

& gcloud iam service-accounts describe $ServiceAccountEmail --project=$ProjectId *> $null
if ($LASTEXITCODE -ne 0) {
    Invoke-GCloud iam service-accounts create $ServiceAccountName `
        --project=$ProjectId `
        --display-name="DATUM passive observation relay"
}

& gcloud pubsub topics describe $TopicName --project=$ProjectId *> $null
if ($LASTEXITCODE -ne 0) {
    Invoke-GCloud pubsub topics create $TopicName --project=$ProjectId
}

foreach ($Role in @(
    "roles/pubsub.publisher",
    "roles/eventarc.eventReceiver",
    "roles/run.invoker"
)) {
    Invoke-GCloud projects add-iam-policy-binding $ProjectId `
        --member="serviceAccount:$ServiceAccountEmail" `
        --role=$Role `
        --condition=None
}

$SourceDir = Split-Path -Parent $MyInvocation.MyCommand.Path
Push-Location $SourceDir
try {
    Invoke-GCloud run deploy $FunctionName `
        --project=$ProjectId `
        --region=$RunRegion `
        --source=. `
        --function=relay_firestore_latest `
        --base-image=python312 `
        --service-account=$ServiceAccountEmail `
        --set-env-vars="DATUM_PASSIVE_NOTIFICATION_TOPIC=$TopicName" `
        --no-allow-unauthenticated
}
finally {
    Pop-Location
}

& gcloud eventarc triggers describe $TriggerName `
    --project=$ProjectId `
    --location=$FirestoreLocation *> $null

if ($LASTEXITCODE -ne 0) {
    Invoke-GCloud eventarc triggers create $TriggerName `
        --project=$ProjectId `
        --location=$FirestoreLocation `
        --destination-run-service=$FunctionName `
        --destination-run-region=$RunRegion `
        --event-filters="type=google.cloud.firestore.document.v1.written" `
        --event-filters="database=(default)" `
        --event-data-content-type=application/protobuf `
        --event-filters-path-pattern="document=datum_snapshots_latest/{document}" `
        --service-account=$ServiceAccountEmail
}
else {
    Write-Host "Eventarc trigger '$TriggerName' already exists; leaving it unchanged."
}

Write-Host "STEP67 relay deployment configured"
Write-Host "project=$ProjectId"
Write-Host "firestore_location=$FirestoreLocation"
Write-Host "run_region=$RunRegion"
Write-Host "topic=$TopicName"
Write-Host "service_account=$ServiceAccountEmail"
