#!/usr/bin/env bash
set -euo pipefail

DSERVER_URL="${DSERVER_URL:-http://192.168.10.101:8080}"
PROJECT_ID="${PROJECT_ID:-monitor-f49c8}"
NODE_ID="${NODE_ID:-fog-01}"
DISCOVERY_FILE="${DISCOVERY_FILE:-DATUM/resource-discovery-fog.json}"
DGRAPH_FILE="${DGRAPH_FILE:-scripts/fixtures/o2-operational-mqtt-dgraph.json}"
AUTHORIZED_BY="${AUTHORIZED_BY:-o2-live-validation}"

require_file() {
  if [[ ! -f "$1" ]]; then
    echo "missing required file: $1" >&2
    exit 2
  fi
}

require_file "$DISCOVERY_FILE"
require_file "$DGRAPH_FILE"

RESOURCE_JSON="$(curl -fsS \
  -X POST \
  "$DSERVER_URL/api/v1/resources/discovery" \
  -H "Content-Type: application/json" \
  -H "X-Datum-Project: $PROJECT_ID" \
  --data-binary "@$DISCOVERY_FILE")"

RESOURCE_ID="$(printf '%s' "$RESOURCE_JSON" | jq -r '.resource_id')"
if [[ -z "$RESOURCE_ID" || "$RESOURCE_ID" == "null" ]]; then
  echo "resource discovery did not return resource_id" >&2
  exit 3
fi

echo "resource_id=$RESOURCE_ID"

BINDING_JSON="$(curl -fsS \
  -X POST \
  "$DSERVER_URL/api/v1/resources/$RESOURCE_ID/binding" \
  -H "Content-Type: application/json" \
  -H "X-Datum-Project: $PROJECT_ID" \
  -d "{\"node_id\":\"$NODE_ID\",\"transition_kind\":\"initial_assignment\"}")"

echo "binding_id=$(printf '%s' "$BINDING_JSON" | jq -r '.binding_id')"

DGRAPH_JSON="$(curl -fsS \
  -X PUT \
  "$DSERVER_URL/api/v1/operational-dgraph" \
  -H "Content-Type: application/json" \
  -H "X-Datum-Project: $PROJECT_ID" \
  --data-binary "@$DGRAPH_FILE")"

DGRAPH_ID="$(printf '%s' "$DGRAPH_JSON" | jq -r '.dgraph_id')"
DGRAPH_REVISION="$(printf '%s' "$DGRAPH_JSON" | jq -r '.revision')"
if [[ -z "$DGRAPH_ID" || "$DGRAPH_ID" == "null" ]]; then
  echo "operational D-Graph did not return dgraph_id" >&2
  exit 4
fi
if [[ -z "$DGRAPH_REVISION" || "$DGRAPH_REVISION" == "null" || "$DGRAPH_REVISION" == "0" ]]; then
  echo "operational D-Graph did not return a valid revision" >&2
  exit 5
fi

echo "dgraph_id=$DGRAPH_ID"
echo "dgraph_revision=$DGRAPH_REVISION"

AUTH_JSON="$(curl -fsS \
  -X POST \
  "$DSERVER_URL/api/v1/nodes/$NODE_ID/operational-reconciliation/authorize" \
  -H "Content-Type: application/json" \
  -H "X-Datum-Project: $PROJECT_ID" \
  -d "{\"dgraph_id\":\"$DGRAPH_ID\",\"dgraph_revision\":$DGRAPH_REVISION,\"authorized_by\":\"$AUTHORIZED_BY\",\"allow_host_mutation\":true}")"

echo "authorization_id=$(printf '%s' "$AUTH_JSON" | jq -r '.authorization_id')"
echo "authorization_revision=$(printf '%s' "$AUTH_JSON" | jq -r '.dgraph_revision')"

echo "rehydrate_status=ok project_id=$PROJECT_ID node_id=$NODE_ID resource_id=$RESOURCE_ID dgraph_id=$DGRAPH_ID dgraph_revision=$DGRAPH_REVISION"
