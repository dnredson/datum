# ADR-0010 — Cross-node readiness from fresh D-Monitor evidence only

- Status: Accepted (Stage 107 baseline)
- Decision refs: Round 2 "readiness"
- Implementation: Stages 111–115. Not implemented in Stage 107.

## Context

Dependencies are currently node-local: governed preflight blocks on a dependency
missing from the same node's context
(`dserver/src/core/governed_deployment_preflight.rs`), and the agent checks
dependencies only against local containers
(`DATUM/src/agent/operational_reconciliation.rs`). Cross-node chains
(mist bridge → fog broker → cloud adapter) exist only as fixed endpoint values in
artifacts and in the non-authoritative `scenario.json`.

## Decision

1. The D-Forward specification (`datum.dforward/1`) distinguishes
   `requires[scope = same_element]` (node-local, what preflight supports today)
   from `requires[scope = any_element]` (cross-node endpoint dependency).
2. Cross-node readiness for a `scope = any_element` requirement is established
   **only from fresh D-Monitor health evidence** of the providing D-IoT placement.
   It is never inferred from configuration, from a static endpoint value, or from
   assumed connectivity.
3. If the providing side has stale or missing evidence, the dependent workload is
   blocked or waiting — never proceeded.

## Consequences

- VS1b (`decode@fog-01`, `alert@cloud-01`) depends on this: `alert` subscribes
  through the fog broker cross-node, so it needs fresh evidence that the broker
  placement is healthy.
- Slice fixtures carry a `remote_dependencies` array; the D-Forward fixture carries
  `requires[].scope`. Negative fixture `negative/dmap.unrealized-dcall.json`
  exercises the missing-realization case.
