# Etapa 66 — ingestão de telemetria e histórico limitado

A etapa 66 acrescenta uma API específica para a telemetria tipada do SmartSentinel:

- POST /api/v1/sentinels/{node_id}/telemetry
- GET /api/v1/sentinels/{node_id}/telemetry
- GET /api/v1/sentinels/telemetry
- GET /api/v1/sentinels/{node_id}/telemetry/history?limit=50

O dserver mantém a observação mais recente e um histórico limitado a 256 observações por projeto e nó. A persistência dos snapshots completos continua sendo feita pelo caminho existente /api/v1/snapshots no Firebase; o histórico em memória é a janela operacional rápida para o agente e para o D-Graph observado.

A ingestão valida:

- versão do schema;
- identidade do nó e do projeto;
- sequência monotônica por instância do Sentinel;
- timestamp RFC3339 e tolerância de relógio futuro;
- presença do host;
- faixa de CPU;
- safe_to_auto_execute=false;
- side_effects_performed=false;
- limite de 256 KiB por payload.

Reenvio idêntico da mesma sequência é idempotente. Uma nova instância pode iniciar uma nova sequência, mas uma mesma instância não pode retroceder ou reutilizar uma sequência com payload diferente.

O smartsentinel-observe publica agora dois artefatos read-only: o snapshot completo e a telemetria tipada. Nenhum dos dois caminhos executa mutações.

## Validação live

Nos dois nós, execute a coleta:

~~~bash
cd ~/datum-i38
export PROJECT_ID="monitor-f49c8"
export DSERVER_URL="http://192.168.10.101:8080"

cargo run --quiet --locked --manifest-path DATUM/Cargo.toml --bin smartsentinel-observe -- --config DATUM/config.linux --dserver-url "$DSERVER_URL" --project-id "$PROJECT_ID" --node-id fog-01 --telemetry-out /tmp/step66-fog-01-telemetry.json --out /tmp/step66-fog-01-snapshot.json
~~~

No cloud-01, troque somente --node-id fog-01 por --node-id cloud-01 e os nomes dos arquivos.

Saída esperada em cada nó:

~~~text
status=ok
snapshot_published=true
telemetry_published=true
side_effects_performed=false
~~~

No desktop:

~~~powershell
$PROJECT_ID = "monitor-f49c8"
$DSERVER_URL = "http://192.168.10.101:8080"

curl.exe -fsS -X GET "$DSERVER_URL/api/v1/sentinels/telemetry" -H "X-Datum-Project: $PROJECT_ID" | jq '{
  project_id,
  count,
  history_capacity,
  observations: [
    .observations[] | {
      node_id,
      sequence,
      telemetry_freshness,
      telemetry_age_seconds,
      side_effects_performed
    }
  ],
  side_effects_performed
}'
~~~

Esperado: count=2, history_capacity=256, os nós fog-01 e cloud-01 com telemetry_freshness="fresh" e side_effects_performed=false.

Valide também a janela histórica:

~~~powershell
curl.exe -fsS -X GET "$DSERVER_URL/api/v1/sentinels/fog-01/telemetry/history?limit=10" -H "X-Datum-Project: $PROJECT_ID" | jq '{
  node_id,
  count,
  limit,
  history_capacity,
  observations: [.observations[] | {sequence, observed_at_utc}],
  side_effects_performed
}'
~~~

count deve ser maior ou igual a 1, sem ultrapassar 10.
