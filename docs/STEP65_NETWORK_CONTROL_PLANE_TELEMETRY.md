# Implementando etapas 65/100 — telemetria de rede e control plane

A etapa 65 adiciona evidência tipada de rede ao contrato do SmartSentinel.

## O que foi implementado

O campo namespaced `attrs.telemetry.network` contém:

- interfaces observadas, estado up/down, endereços e contadores quando disponíveis;
- probes de conectividade já produzidos pelo coletor de ping;
- alcance e RTT TCP até o endpoint do dserver usado para publicação.

A medição do control plane é um teste TCP de leitura/conectividade. Ela não executa requisições mutáveis e não altera o host.

```json
{
  "network": {
    "interfaces": [],
    "control_plane": {
      "target": "192.168.10.101:8080",
      "reachable": true,
      "rtt_ms": 1.8
    },
    "probes": []
  }
}
```

A interface pode ter campos ausentes quando o sistema operacional não disponibilizar determinada métrica. O contrato não inventa endereços, contadores ou latências.

O dserver preserva essa evidência dentro do snapshot bruto já aceito; nenhum endpoint antigo foi alterado.
