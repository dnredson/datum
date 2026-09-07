# C70 — primeira GNN experimental

A C70 adiciona o primeiro experimento reproduzível sobre o contrato `datum_ai_gnn_input` da C69.

Endpoint:

```
POST /api/v1/analysis-context/active?format=gnn-dry-run
X-Datum-Project: <project_id>
```

O experimento usa uma camada determinística de message passing, com agregação média entre o sinal local e o sinal dos vizinhos. Não há treinamento, pesos persistidos, execução de host ou alteração do D-Graph.

São avaliados três cenários controlados:

- `fresh`: evidência fresca e utilizável;
- `stale`: evidência stale;
- `missing`: evidência ausente.

O resultado inclui a predição por cenário, score por nó, threshold, descrição do modelo e dois baselines: sempre inelegível e regra de freshness. A saída é somente uma proposta experimental; `side_effects_performed` permanece `false`.

A C70 não afirma desempenho científico da GNN. Ela fecha a primeira prova de integração entre contrato, representação de grafo, message passing determinístico, cenários controlados e baselines. O próximo passo poderá substituir o classificador experimental por pesos aprendidos, mantendo o mesmo contrato e a mesma fronteira read-only.
