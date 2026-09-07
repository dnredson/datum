# Implementando etapa 63/100 — contrato tipado de telemetria de host

A etapa 63 formaliza a telemetria coletada pelo SmartSentinel em um contrato versionado, sem quebrar o envelope histórico de snapshots.

## Contrato

Cada host observado recebe o campo namespaced `attrs.telemetry` com:

- `schema_version`;
- projeto, nó, instância e sequência;
- timestamp de observação;
- hostname, sistema operacional, kernel e uptime;
- CPU: núcleos lógicos, uso e load average quando disponível;
- memória: total, usada e disponível em bytes;
- swap: total e usada em bytes;
- filesystems: ponto de montagem e capacidade em bytes;
- contagem de processos;
- `safe_to_auto_execute=false`;
- `side_effects_performed=false`.

Métricas ausentes permanecem ausentes no JSON. O SmartSentinel não substitui uma medição ausente por zero.

## Validação

O contrato rejeita:

- versão de schema desconhecida;
- identidade vazia;
- sequência zero;
- hostname vazio;
- CPU fora do intervalo 0–100;
- memória, swap ou filesystem com valores usados/disponíveis maiores que o total;
- telemetria marcada como segura para autoexecução;
- telemetria marcada com efeitos colaterais.

O campo namespaced é adicionado ao host dentro do snapshot já existente. Dessa forma, consumidores antigos continuam lendo `meta`, entidades, arestas, eventos e ações sem alteração estrutural.

## Coleta real

No Linux, o coletor usa:

- `/proc/stat` para CPU;
- `/proc/meminfo` para memória e swap;
- `/proc/uptime` para uptime;
- `/proc/loadavg` para carga;
- `/proc` para contagem de processos;
- `df -kP /` para o filesystem raiz;
- `uname -r` para o kernel.

No Windows, o coletor usa consultas CIM/PowerShell para CPU, memória, swap, uptime, processos e volumes locais.

## Teste local do contrato

O observador pontual pode gravar o contrato tipado separadamente:

```bash
cargo run --quiet --locked \
  --manifest-path DATUM/Cargo.toml \
  --bin smartsentinel-observe \
  -- \
  --config DATUM/config.linux \
  --dserver-url "http://192.168.10.101:8080" \
  --project-id "monitor-f49c8" \
  --node-id fog-01 \
  --telemetry-out /tmp/c63-fog-01-telemetry.json \
  --out /tmp/c63-fog-01-snapshot.json
```

Inspeção:

```bash
jq '{
  schema_version,
  project_id,
  node_id,
  sentinel_instance_id,
  sequence,
  observed_at_utc,
  host,
  safe_to_auto_execute,
  side_effects_performed
}' /tmp/c63-fog-01-telemetry.json
```

O mesmo contrato é anexado ao snapshot publicado no dserver em:

```text
node_snapshots[].snapshot_envelope.snapshot.entities[]
  | select(.kind == "host")
  | .attrs.telemetry
```

No modo contínuo, use `--telemetry-out` para manter o último contrato tipado no arquivo indicado.
