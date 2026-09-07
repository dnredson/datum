# Etapas 68 + 69 — matriz de conectividade e dashboard live

As etapas 68 e 69 formam uma entrega única de observabilidade operacional:

- etapa 68: o SmartSentinel pode medir alvos TCP entre nós configurados e publicar a matriz tipada em `network.peers`;
- etapa 69: o dserver expõe essas medições no D-Graph observado e serve um dashboard mínimo de atualização automática.

## Etapa 68 — conectividade entre pares

A configuração opcional usa:

```toml
[[collectors.connection_ping.peer_targets]]
node_id = "cloud-01"
target = "192.168.10.108"
port = 22
timeout_s = 2
```

O próprio nó é ignorado automaticamente. Cada observação contém:

- nó de destino;
- alvo testado;
- alcançabilidade;
- RTT;
- falhas consecutivas da observação;
- último sucesso;
- erro, quando houver.

O probe é TCP, read-only e não executa comandos no peer. O fixture de laboratório usa a porta SSH como alvo leve de reachability; quando um listener próprio do SmartSentinel for implantado, o alvo poderá ser trocado sem alteração do contrato.

A telemetria continua sendo publicada para:

```text
POST /api/v1/sentinels/{node_id}/telemetry
```

## Etapa 69 — D-Graph observado e dashboard

O endpoint existente `GET /api/v1/observed-dgraph` agora inclui:

- `measured_peer_edges`;
- origem e destino do peer;
- alcançabilidade;
- RTT;
- falhas;
- frescor da evidência.

O dashboard mínimo está disponível em:

```text
GET /dashboard/live?project=<project-id>
```

Seu estado JSON correspondente está em:

```text
GET /api/v1/dashboard/live?project=<project-id>
X-Datum-Project: <project-id>
```

A interface:

- atualiza a cada dois segundos;
- mostra estado dos nós;
- mostra idade e sequência da telemetria;
- mostra CPU, memória e serviços desejados;
- mostra conectividade medida entre peers;
- não executa mutações;
- preserva `side_effects_performed=false`.

## Limites

As etapas não fazem failover, alteração de rota, deploy ou reconciliação. A matriz é evidência para as futuras etapas de estado de falha, planejamento what-if e aplicação controlada.

## Validação

Nos nós, execute o SmartSentinel com a configuração atualizada:

```bash
cargo run --quiet --locked \
  --manifest-path DATUM/Cargo.toml \
  --bin smartsentinel-observe \
  -- \
  --config DATUM/config.linux \
  --dserver-url "$DSERVER_URL" \
  --project-id "$PROJECT_ID" \
  --node-id fog-01 \
  --telemetry-out /tmp/step68-fog-01-telemetry.json \
  --out /tmp/step68-fog-01-snapshot.json
```

No cloud, substitua o node id e os nomes dos arquivos.

A telemetria deve conter:

```text
network.peers
side_effects_performed=false
```

No desktop:

```powershell
curl.exe -fsS `
  -X GET `
  "$DSERVER_URL/api/v1/observed-dgraph" `
  -H "X-Datum-Project: $PROJECT_ID" |
jq '{
  revision,
  path_status,
  nodes: [.nodes[] | {
    node_id,
    telemetry_freshness: .observed.telemetry_freshness
  }],
  measured_peer_edges,
  side_effects_performed
}'
```

O dashboard pode ser aberto no navegador em `http://192.168.10.101:8080/dashboard/live?project=monitor-f49c8`.


## Etapas 70 + 71 — visão topológica observada

As etapas 70 e 71 acrescentam uma visão gráfica da topologia observada, mantendo a distinção entre configuração, evidência e estado de freshness.

### Etapa 70 — contrato de visão topológica

O endpoint dedicado é:

```text
GET /api/v1/graph-overview?project=<project-id>
X-Datum-Project: <project-id>
```

O contrato `datum_observed_topology_graph` contém:

- `nodes`: nós esperados, estado observado, freshness, idade, sequência e quantidade de serviços desejados;
- `edges`: arestas de conectividade medidas pelos SmartSentinels;
- `edge_status`: `up`, `stale`, `degraded`, `down` ou `unknown`;
- `summary`: contagem de nós e classificação das arestas;
- revisão do D-Graph e `path_status`.

A classificação de arestas é:

| Estado | Semântica |
|---|---|
| `up` | peer alcançável com telemetria fresh |
| `stale` | peer alcançável, mas evidência antiga |
| `down` | peer não alcançável com telemetria fresh |
| `degraded` | peer não alcançável e evidência antiga |
| `unknown` | evidência insuficiente ou inválida |

### Etapa 71 — visualização gráfica

O dashboard `/dashboard/live` agora exibe:

- grafo SVG sem dependências externas;
- nós coloridos pelo estado observado;
- arestas direcionadas coloridas pelo `edge_status`;
- RTT exibido na aresta quando disponível;
- legenda semântica;
- resumo de nós e arestas;
- tabela detalhada de conectividade;
- atualização automática a cada dois segundos.

O grafo é somente leitura. As etapas não fazem deploy, failover, alteração de rota, reconciliação ou execução de ações. Todas as respostas preservam `side_effects_performed=false`.
