# Etapa 73 — planejador de caminhos alternativos em modo what-if

A etapa 73 acrescenta um planejador determinístico que enumera caminhos simples
limitados no D-Graph observado. O resultado é uma proposta para consumidores
operacionais e de IA; ele não aplica reroteamento, deploy, reconciliação ou
qualquer outra mutação.

## Endpoint

`GET /api/v1/alternative-paths?project=<project_id>`

O escopo pode ser reduzido a um par:

`GET /api/v1/alternative-paths?project=<project_id>&from=fog-01&to=cloud-01`

Também é possível limitar o número de saltos com `max_hops`. O valor padrão é
4 e o limite máximo é 8.

## Critérios

O planejador consome:

- os nós do D-Graph observado;
- as arestas de conectividade entre pares medidas pelo SmartSentinel;
- a frescura da evidência de cada nó e aresta;
- o estado estabilizado de falha/recuperação da etapa 72.

Um caminho é `eligible=true` somente quando todos os nós estão `healthy`,
com telemetria fresh, e todas as arestas estão `up`. Caminhos stale,
degradados, desconhecidos ou indisponíveis podem aparecer como hipóteses, mas
são explicitamente inelegíveis.

## Contrato

O contrato `datum_alternative_path_what_if` publica:

- `candidates`: caminhos encontrados, com nós, arestas, saltos, status e razão;
- `rejected_pairs`: pares sem caminho observado dentro do limite;
- `summary`: contagens de pares, candidatos e elegibilidade;
- `what_if`: confirma que nenhuma alteração foi aplicada;
- `side_effects_performed=false`.

O planejador é bounded e determinístico: no máximo oito saltos e três caminhos
por par, ordenados por elegibilidade, estado e comprimento.

## Segurança

A etapa permanece somente observacional:

```json
{
  "what_if": {
    "mode": "proposal_only",
    "changes_applied": false,
    "execution_allowed": false,
    "side_effects_performed": false
  },
  "side_effects_performed": false
}
```
