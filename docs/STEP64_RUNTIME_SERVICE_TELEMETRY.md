# Implementando etapas 64/100 — evidência tipada de runtime e serviços

A etapa 64 amplia o contrato de telemetria sem alterar o envelope histórico do snapshot.

## O que foi implementado

O campo namespaced `attrs.telemetry.runtime` agrega somente entidades observadas no ciclo atual:

- containers observados pelo Docker;
- processos observados no host;
- serviços configurados com estado observado;
- estado de binding e checks quando disponíveis;
- quantidade de containers somente quando há evidência suficiente.

A ausência de um container ou processo não é convertida automaticamente em “parado”. Quando o runtime não fornece evidência, o campo permanece ausente ou o estado permanece `unknown`.

A coleta continua somente leitura. Nenhum container é iniciado, parado ou alterado.

## Forma resumida

```json
{
  "runtime": {
    "container_runtime_available": true,
    "container_count": 1,
    "containers": [],
    "processes": [],
    "services": []
  }
}
```

Os dados são derivados do mesmo snapshot que o dserver recebe, evitando divergência entre observabilidade e contexto da IA.
