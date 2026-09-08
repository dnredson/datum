# Etapa 72 — máquina de estados de falha e recuperação

A etapa 72 transforma freshness e presença de telemetria em um estado observacional estabilizado por nó. Ela não executa failover, reroteamento, deploy ou reconciliação.

## Estados

```text
healthy -> suspect -> unavailable
unavailable -> recovering -> healthy
```

Transições:

- a primeira evidência stale/missing leva `healthy` a `suspect`;
- três evidências de falha distintas levam `suspect` a `unavailable`;
- a primeira evidência fresh após indisponibilidade leva a `recovering`;
- três evidências fresh consecutivas levam `recovering` a `healthy`;
- perda de evidência durante `recovering` retorna a `unavailable`.

## Evidência distinta

O estado avança somente quando chega uma nova observação fresh ou quando uma evidência não-fresh (`stale` ou `missing`) entra em um novo intervalo de avaliação de cinco segundos. Consultas repetidas ao dashboard dentro do mesmo intervalo não contam como novas falhas.

Cada nó expõe:

- estado atual;
- razão da última avaliação;
- falhas consecutivas;
- sucessos consecutivos;
- instante da mudança de estado;
- última avaliação;
- histórico limitado de transições.

## Contrato

O D-Graph observado inclui em cada nó:

```json
{
  "failure_state": {
    "state": "suspect",
    "reason": "stale_or_missing_telemetry",
    "consecutive_failures": 1,
    "consecutive_successes": 0,
    "state_changed_at_utc": "...",
    "last_evaluated_at_utc": "...",
    "transitions": []
  }
}
```

A configuração da máquina também é publicada em `failure_state_machine`, permitindo que consumidores conheçam os limiares utilizados.

## Segurança

A etapa é somente observacional:

```text
side_effects_performed = false
```

O planejador de caminhos alternativos pertence à etapa 73 e só deve consumir estados estabilizados desta etapa.
