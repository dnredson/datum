# ADR-0010 — Cross-node readiness from fresh D-Monitor evidence only

- Status: Accepted (Stage 107 baseline)
- Decision refs: Round 2 "readiness"
- Implementation: Stages 111–115. Not implemented in Stage 107.
- **Phase 114D2 update**: this ADR's deferred gate is now implemented at the
  server derivation layer. See "Phase 114D2: implementation" below.

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

## Phase 114D2: implementation

The Phase-113 deferred gate (`readiness_authority = "dmonitor"` /
`readiness_state = "pending_fresh_evidence"`, still the STATIC authority-only
projection's own honest output -- unchanged) is now closed at the server
derivation layer by
`dserver::core::dforward_dependency_readiness::evaluate_dforward_dependency_readiness`,
consuming Phase 114D1's already-derived per-placement governance:

- **Provider resolution is topological, never positional.** Canonical
  `CanonicalDForwardV1::validate` proves hop-endpoint interface
  complementarity only (`(a.provides(I) && b.requires(I)) ||
  (a.requires(I) && b.provides(I))`); it never fixes which side of a hop is
  "the provider". D2 identifies the consumer's own side of a hop by exact
  `diot_id`, then classifies the OTHER side as a candidate provider only if
  it is a declared DIoT whose `provides[]` genuinely contains the required
  interface -- `hop.from`/`hop.to` position is never treated as authority.
- **Ambiguity fails closed, never guessed.** Canonical validation proves
  nothing about provider multiplicity (zero, one, or many DIoTs may
  legitimately provide the same interface to one consumer across one or
  more chains) NOR about placement multiplicity (`DIoTPlacementV2`
  uniqueness is enforced only on `placement_id`, never on `diot_id` --
  audited directly in `canonical_dmap_v2.rs`). Since no accepted contract
  establishes a selection rule for either case, more than one distinct
  logical provider, or more than one placement of the resolved provider,
  is `Blocked`/`provider_ambiguous`/`provider_placement_ambiguous` --
  never first-wins, nearest-node, healthiest-wins, or any other invented
  selection policy (that would be new placement/routing authority this
  phase does not have).
- **The gate is the EXACT provider PLACEMENT's readiness, never whole-
  application readiness.** `any_element` is `Satisfied` purely on the
  strength of `DMonitorDiotPlacementReadinessV1.readiness == Ready` for the
  one placement D2 resolved -- an unrelated D-Serv placement being
  `NotReady` never blocks it. Proven both at the unit level
  (`unrelated_dserv_notready_does_not_block_a_ready_provider`) and by a real
  end-to-end integration proof against a live DServer process.
- **Same-node providers are not special-cased.** `AnyElement` is the scope
  name, not `OtherElement`; nothing in the accepted contracts forbids the
  resolved provider from sharing the consumer's own node, so D2 does not
  invent a same-node rejection.
- See "DMonitor-governed any_element provider readiness (Phase 114D2)" in
  ADR-0024 for the full derivation/API/testing detail.
