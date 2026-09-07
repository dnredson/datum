# Implementando etapa 62/100 — SmartSentinel observador contínuo

A etapa 62 transforma o observador pontual da etapa 61 em um processo contínuo de observação e publicação.

## O que foi implementado

- flag `--continuous` no binário `smartsentinel-observe`;
- intervalo de coleta configurável por `--cycle-s`;
- heartbeat de processo configurável por `--heartbeat-s`;
- encerramento por Ctrl+C;
- encerramento por arquivo `.smartsentinel-stop`;
- encerramento finito por `--max-cycles`, útil para testes;
- publicação direta no endpoint `/api/v1/snapshots`;
- gravação atômica do último snapshot;
- bloqueio de segunda instância para o mesmo `node_id`;
- ações e mutações desabilitadas no modo observacional.

O modo contínuo reutiliza o loop de serviço existente, mas permite informar dserver e projeto pela linha de comando. A configuração local não precisa ativar `outputs.http`.

## Teste finito

Exemplo com três ciclos:

```bash
cargo run --quiet --locked \
  --manifest-path DATUM/Cargo.toml \
  --bin smartsentinel-observe \
  -- \
  --continuous \
  --config DATUM/config.linux \
  --dserver-url "http://192.168.10.101:8080" \
  --project-id "monitor-f49c8" \
  --node-id fog-01 \
  --cycle-s 5 \
  --heartbeat-s 1 \
  --max-cycles 3 \
  --out /tmp/c62-fog-01.json
```

A saída deve conter:

```text
mode=smartsentinel event=start ... mutations=disabled
mode=smartsentinel event=liveness seq=... node_id=fog-01 status=alive ... side_effects=false
mode=smartsentinel event=collection seq=1 status=ok ... http_json=ok ... actions=0 ... side_effects=false
mode=smartsentinel event=collection seq=2 status=ok ... http_json=ok ... actions=0 ... side_effects=false
mode=smartsentinel event=collection seq=3 status=ok ... http_json=ok ... actions=0 ... side_effects=false
mode=smartsentinel event=shutdown reason=max_cycles completed_collections=3 ... side_effects=false
```

Os UUIDs, tempos e quantidades de entidades variam conforme a coleta real.

## Execução contínua

Para manter o agente em execução, remova `--max-cycles`:

```bash
cargo run --quiet --locked \
  --manifest-path DATUM/Cargo.toml \
  --bin smartsentinel-observe \
  -- \
  --continuous \
  --config DATUM/config.linux \
  --dserver-url "http://192.168.10.101:8080" \
  --project-id "monitor-f49c8" \
  --node-id fog-01 \
  --cycle-s 5 \
  --heartbeat-s 1 \
  --out /tmp/c62-fog-01.json
```

O agente pode ser encerrado com Ctrl+C. O encerramento esperado é:

```text
mode=smartsentinel event=shutdown reason=ctrl_c ... side_effects=false
```

Para validar o encerramento por arquivo, crie `.smartsentinel-stop` no diretório do processo:

```bash
touch .smartsentinel-stop
```

Nesse caso, o processo deve terminar com:

```text
mode=smartsentinel event=shutdown reason=stop_file ... side_effects=false
```

A etapa não cria deploy, não inicia containers e não executa reconciliação.
