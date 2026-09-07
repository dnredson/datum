# C69 — contrato de dados AI/GNN

O dserver expõe o contrato pelo mesmo endpoint ativo do C68:

```
POST /api/v1/analysis-context/active?format=gnn
X-Datum-Project: <project_id>
```

O contrato possui schema `0.1.0`, tipo `datum_ai_gnn_input` e preserva uma única revisão aceita do D-Graph. Ele contém:

- topologia explícita, com nós tipados como `operational_node` e arestas com relação e atributos;
- features numéricas e categóricas de nós e arestas;
- máscaras explícitas por feature;
- payload bruto de cada snapshot, ciclo, timestamp, status e serviços desejados;
- elegibilidade para inferência;
- metadados de normalização e labels;
- proveniência e semântica de execução somente por proposta.

## Features

Features numéricas de nó:

`desired_service_count`, `telemetry_age_seconds`, `telemetry_fresh`, `usable_for_inference`, `snapshot_entity_count`, `snapshot_edge_count`, `snapshot_event_count` e `snapshot_action_count`.

Features categóricas de nó:

`telemetry_freshness` e `snapshot_status`.

Features numéricas de aresta:

`latency_ms` e `weight`, quando presentes nos atributos da relação operacional.

O dserver não normaliza valores. O contrato informa `normalization.method = "none"`; estatísticas de treino e transformação pertencem ao consumidor do modelo. Valores ausentes são `null` e têm `*_mask = false`; não há imputação silenciosa.

Labels não são inventados pelo dserver. O campo `labels` permanece vazio e `label_mask = false` até que uma fonte autorizada de labels seja definida.

## Elegibilidade

Snapshots stale, ausentes ou inválidos permanecem no contrato como evidência histórica, mas não habilitam inferência. A elegibilidade só é verdadeira quando todos os nós esperados têm evidência fresca e utilizável.

`side_effects_performed` é sempre `false`. O contrato não contém autorização, plano de execução nem chamada de host; sua semântica é `proposal_only_no_direct_execution`.

Os formatos existentes `analysis`, `ai` e `dgraph` continuam disponíveis sem alteração.
