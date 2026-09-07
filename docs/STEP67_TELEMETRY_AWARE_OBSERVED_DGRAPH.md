# Etapa 67 — D-Graph observado orientado por telemetria

A etapa 67 junta a telemetria mais recente ao D-Graph operacional aceito sem alterar a revisão de controle.

Endpoint:

- GET /api/v1/observed-dgraph
- Header: X-Datum-Project

O retorno separa explicitamente:

- desired_services: estado desejado/configurado do D-Graph;
- observed: estado medido pelo SmartSentinel;
- configured_relations: relações declaradas no grafo;
- observed_relations: disponibilidade derivada da frescura dos nós;
- measured_connectivity_edges: medições reais do caminho até o plano de controle;
- path_status: resumo operacional.

Telemetria ausente não é inventada. Um nó sem observação recente aparece como missing, e uma relação com evidência incompleta aparece como degraded ou unavailable. A revisão, o dgraph_id e os serviços desejados continuam sendo os da única revisão operacional aceita.

## Validação live

No desktop:

~~~powershell
$PROJECT_ID = "monitor-f49c8"
$DSERVER_URL = "http://192.168.10.101:8080"

curl.exe -fsS -X GET "$DSERVER_URL/api/v1/observed-dgraph" -H "X-Datum-Project: $PROJECT_ID" | jq '{
  schema_version,
  graph_kind,
  project_id,
  dgraph_id,
  revision,
  path_status,
  nodes: [
    .nodes[] | {
      node_id,
      desired_service_count: (.desired_services | length),
      observed_status: .observed.status,
      telemetry_freshness: .observed.telemetry_freshness,
      telemetry_age_seconds: .observed.telemetry_age_seconds,
      sequence: .observed.sequence
    }
  ],
  observed_relations,
  measured_connectivity_edges: [
    .measured_connectivity_edges[] | {
      from,
      to,
      reachable,
      rtt_ms,
      telemetry_freshness
    }
  ],
  side_effects_performed
}'
~~~

Com os dois SmartSentinels publicando, esperamos:

~~~text
graph_kind = telemetry_aware_observed_dgraph
revision = 5
nodes = 2
fog-01.observed_status = fresh
cloud-01.observed_status = fresh
path_status = all_expected_nodes_fresh
side_effects_performed = false
~~~

Esse endpoint não substitui os contratos históricos analysis, ai, dgraph ou gnn. Ele fornece a visão observada que será consumida pelas próximas etapas de falha, recuperação e preparação de dados.
