# Implementando etapa 61/100 — SmartSentinel observacional multinode

A etapa 61 adiciona o binário `smartsentinel-observe`, um entrypoint explícito para observação read-only.

Uso:

```bash
cargo run --locked   --manifest-path DATUM/Cargo.toml   --bin smartsentinel-observe --   --config DATUM/config.linux   --node-id <node-id>   --dserver-url http://192.168.10.101:8080   --project-id monitor-f49c8   --out /tmp/smartsentinel-observe.json
```

O processo:

1. carrega a configuração local;
2. coleta um ciclo usando a identidade lógica do nó;
3. grava o snapshot localmente;
4. publica o envelope no endpoint de snapshots do dserver;
5. encerra sem deploy, reconciliação ou mutação de host.

O binário é deliberadamente separado de `smartsentinel-operational-reconcile`. Nesta etapa, não exige binding, autorização de host ou serviço desejado; seu papel é tornar a observação multinode operacional. A evidência ausente continua sendo tratada pelo dserver como `missing`.
