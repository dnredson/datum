# Etapa 73 — contexto histórico para what-if de IA

A etapa 73 prepara o contexto temporal para que um componente externo de IA
projete caminhos alternativos. O dserver não executa a inferência, não treina
um modelo e não aplica reroteamento.

## Endpoint

`GET /api/v1/alternative-paths?project=<project_id>`

O escopo pode ser reduzido a um par:

`GET /api/v1/alternative-paths?project=<project_id>&from=fog-01&to=cloud-01`

Também é possível informar `max_hops`, que representa uma restrição para a
projeção futura. O valor padrão é 4 e o limite máximo é 8.

## Papel do dserver

O endpoint combina:

- o D-Graph observado atual;
- as séries históricas mantidas pelo SmartSentinel/dserver;
- timestamps, ciclos, sequências e frescura de cada observação;
- features numéricas extraídas da telemetria tipada;
- métricas de conectividade entre pares.

A série bruta permanece no campo `history.node_series[].observations[].payload`.
O campo `feature_series` oferece uma representação tabular reproduzível para
um encoder ou modelo externo.

## Papel da IA externa

A resposta declara explicitamente:

- `prediction_performed=false`;
- `training_performed=false`;
- `projection_available=false`;
- `projections=[]`;
- `inference.mode=external_ai_pending`.

Assim, a etapa prepara dados para uma IA/GNN externa sem fabricar uma previsão
determinística baseada apenas no snapshot corrente.

## Contrato

O contrato `datum_alternative_path_what_if_context` publica:

- `observed_dgraph`: estado atual completo;
- `history`: série por nó, limitada à capacidade configurada;
- `feature_schema` e `feature_series`: dados prontos para consumo temporal;
- `requested_scope`: origem, destino e limite de saltos solicitados;
- `baseline`: contagens do grafo observado;
- `inference`: estado da preparação da inferência;
- `what_if`: confirma que nenhuma alteração foi aplicada.

## Segurança

```json
{
  "what_if": {
    "mode": "historical_projection_input",
    "changes_applied": false,
    "execution_allowed": false,
    "side_effects_performed": false
  },
  "side_effects_performed": false
}
```
