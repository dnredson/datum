# C57 Canonical Execution Denial Summary Contract

## Status

Candidate contract. Validation evidence and checkpoint advancement are intentionally deferred until
the focused, complete, and live validation campaign passes.

## Purpose

C57 adds an additive, machine-readable `denial_summary` to the existing
`GET /api/v1/dgraph/execution-safety` response. It consolidates the current fail-closed denial
state so clients can evaluate its internal consistency without reconstructing counts and equality
checks themselves.

The summary is explanatory only. It neither authorizes execution nor creates an execution path.

## Derivation Sources

`denial_summary` derives exclusively from existing C54 prerequisite aggregation, C55 projected
gates and prerequisite summary, C56 generated operation denials, and the canonical C47 execution
operation taxonomy. It introduces no gate table, operation taxonomy, blocker list, reason table,
or authorization rule.

- `required_gate_count` and `satisfied_gate_count` are copied from the C55 prerequisite summary.
- `blocking_gate_count` is the length of the C55 projected unsatisfied-gate sequence.
- `operation_count` is the number of generated entries from the canonical operation taxonomy.
- `denied_operation_count` is the number of generated entries with `authorized=false`.
- `all_prerequisites_blocking` reports that all required gates are currently unsatisfied and
  represented by the projected blocking sequence.
- `all_operations_denied` reports that every generated operation is unauthorized.
- `all_operation_blockers_match_unsatisfied_gates` reports that each C56 operation's
  `blocking_gates` exactly equals the canonical unsatisfied-gate sequence, including order.

## Field Semantics

The summary also makes the established fail-closed state explicit:

- `authorization_granted=false`;
- `allow_transition_available=false`;
- `execution_performed=false`;
- `side_effects_performed=false`.

Those values describe the existing execution-safety boundary. They are not readiness, permission,
approval, or an authorization decision. Even if a future graph were internally consistent with no
unsatisfied prerequisites, C57 does not implement or infer an allow transition.

## Fail-Closed Interpretation

The enclosing C46 decision remains `denied`, all eight current operations remain unauthorized,
and `metadata.execution_allow_transition_implemented` remains false. The summary is a compact
view of this denial evidence, not an executable policy or permission grant.

## Safety Boundary

C57 is read-only, metadata-only, evaluation-only, and additive. It does not load or persist
policy; authenticate or approve an operator; read, hash, verify, load, or transfer artifacts;
load or match allowlists; negotiate capabilities; reserve or enforce resources; create
checkpoints; roll back, restore, or audit; or perform deployment, installation, dispatch,
migration, recovery, reconfiguration, placement, WASM, Docker, health-check, AI, or LLM work.
No persistence or operational side effects occur.

## Validation Plan

Validation will cover empty and ready metadata-only graphs, derived gate and operation counts,
canonical blocker order, serialization, absence of a parallel C57 gate or operation table, and
absence of execution or mutation paths. The campaign will run focused C57 through C54 tests, the
combined C46/C54/C55/C56/C57 tests, formatting, the complete dserver suite, build, diff checks,
and strict non-mutating live validation against the restored `smart-city-campinas` three-node
baseline.
