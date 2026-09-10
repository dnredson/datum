# Stage 89 — primeiro serviço real Magistrala

Esta etapa valida o primeiro serviço real da plataforma distribuída: somente
`magistrala-atom-db` no `cloud-01`.

## Objetivo

Validar, ponta a ponta, que:

- o artifact é registrado no dserver;
- o D-Graph atribui o serviço ao nó correto;
- o SmartSentinel executa o WASM de decisão;
- a autorização de mutação é vinculada ao projeto, D-Graph, revisão, nó e binding;
- a senha é resolvida somente no nó local;
- o container PostgreSQL é criado e iniciado;
- uma execução sem a variável local falha sem mutação.

O segredo nunca deve ser enviado ao dserver, commitado ou impresso.

## Preparação no dserver

Registre/atualize o fixture:

```powershell
$PROJECT_ID = "monitor-f49c8"
$DSERVER_URL = "http://192.168.10.101:8080"
$ARTIFACT_PATH = Join-Path (Get-Location) "artifacts/catalog/magistrala-atom-db.json"

curl.exe -fsS \
  -X PUT \
  "$DSERVER_URL/api/v1/service-artifacts/magistrala-atom-db" \
  -H "Content-Type: application/json" \
  -H "X-Datum-Project: $PROJECT_ID" \
  --data-binary "@$ARTIFACT_PATH"
```

O artifact deve mostrar:

- `artifact_id=magistrala-atom-db`;
- imagem `postgres:16-alpine`;
- `secret_refs=[magistrala_atom_db_password]`;
- bindings `POSTGRES_DB=magistrala` e `POSTGRES_USER=datum`.

## D-Graph isolado

Use uma revisão nova contendo `mqtt` no fog e apenas
`magistrala-atom-db` no cloud. Preserve a revisão anterior para restauração.
A revisão usada neste teste deve ser a informada pelo dserver após o PUT.

Consulte o contexto:

```powershell
curl.exe -fsS \
  -X GET \
  "$DSERVER_URL/api/v1/nodes/cloud-01/operational-reconciliation-context" \
  -H "X-Datum-Project: $PROJECT_ID" |
jq '{
  node_id,
  dgraph_revision,
  desired_services: [.desired_services[].service_id],
  artifacts: [.artifacts[] | {
    artifact_id,
    service_id,
    bindings,
    secret_refs
  }],
  side_effects_performed
}'
```

## Caso negativo: segredo ausente

No cloud, garanta que `POSTGRES_PASSWORD` não exista e execute o
reconciliador com autorização válida. O resultado esperado é:

- `secret_resolution_status=blocked_missing_secret`;
- `execution_status=blocked_missing_secret`;
- `host_mutation_performed=false`;
- `side_effects_performed=false`;
- nenhum container `datum-magistrala-atom-db` criado.

## Caso positivo: deploy local governado

No cloud, defina uma senha descartável apenas no ambiente do processo:

```bash
export POSTGRES_PASSWORD='valor-descartavel-do-teste'
```

Não inclua esse valor em arquivos, commits ou comandos enviados ao dserver.
Autorize a reconciliação para a revisão atual e rode:

```bash
cargo run --quiet --locked \
  --manifest-path DATUM/Cargo.toml \
  --bin smartsentinel-operational-reconcile \
  -- \
  --config DATUM/config.cloud.linux \
  --dserver-url "$DSERVER_URL" \
  --project-id "$PROJECT_ID" \
  --node-id cloud-01 \
  --execute \
  --out /tmp/c89-cloud-01-atom-db.json
```

Valide sem imprimir a senha:

```bash
jq '{
  execute_requested,
  control_plane_authorized,
  services: [.services[] | {
    service_id,
    artifact_id,
    secret_resolution_status,
    execution_status,
    observed_after_running,
    host_mutation_performed
  }],
  side_effects_performed
}' /tmp/c89-cloud-01-atom-db.json

docker inspect datum-magistrala-atom-db \
  --format 'running={{.State.Running}} status={{.State.Status}}'

docker exec datum-magistrala-atom-db pg_isready -U datum -d magistrala
```

## Rollback

Use a autorização de rollback com `operation=rollback`, a mesma revisão,
binding e `allow_host_mutation=true`; execute o reconciliador com
`--execute --rollback`. Valide que o container foi removido e que a porta
5432 não ficou aberta.

## Limite desta etapa

O metadata do artifact declara o volume lógico
`magistrala-atom-db-volume`, mas o mapping local atual ainda não monta volumes.
Logo, esta etapa valida segredo, bindings, criação, saúde e rollback do
container; persistência real será validada quando o contrato de volumes for
implementado.
