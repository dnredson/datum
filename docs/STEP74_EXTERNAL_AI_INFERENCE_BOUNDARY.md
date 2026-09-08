# Etapa 74/100 — fronteira de inferência externa

A etapa 74 define a fronteira entre o dserver e uma IA/GNN externa para projeção de caminhos alternativos.

## O que entra

O endpoint:

`GET /api/v1/alternative-paths/inference-request`

monta o contexto histórico da etapa 73 e o envolve em um contrato de requisição:

- `contract_kind=datum_ai_what_if_inference_request`;
- `task=project_alternative_paths_from_historical_telemetry`;
- revisão e identidade do D-Graph;
- topologia observada;
- séries históricas e `feature_series`;
- escopo solicitado;
- política explícita de proposta, sem ações;
- `provider_mode=external_ai_required`.

O dserver não chama um provedor, não treina a GNN e não fabrica uma projeção.

## O que volta da IA

Um consumidor externo pode enviar uma resposta para:

`POST /api/v1/alternative-paths/inference-response`

A resposta deve declarar:

- `contract_kind=datum_ai_what_if_inference_response`;
- o mesmo `request_id`;
- o mesmo projeto, D-Graph e revisão;
- projeções com caminhos formados por nós conhecidos;
- `confidence` finita entre 0 e 1;
- justificativa textual;
- execução em `proposal_only`.

A resposta é conferida contra o contexto atual. O dserver rejeita revisão divergente, request id divergente, nós desconhecidos, caminhos acima de `max_hops`, IDs repetidos, confiança inválida e qualquer campo que autorize efeitos colaterais.

Uma resposta aceita vira `datum_validated_ai_what_if_projection`. Ela é devolvida ao consumidor, mas não é persistida, implantada, reroteada ou executada. A persistência e o ciclo de treinamento serão etapas posteriores.

## Garantias

Todos os contratos da etapa mantêm:

```json
{
  "execution": {
    "mode": "proposal_only",
    "changes_applied": false,
    "execution_allowed": false,
    "side_effects_performed": false
  },
  "side_effects_performed": false
}
```

A etapa prepara a integração real com a IA sem acoplar o dserver a um provedor específico.
