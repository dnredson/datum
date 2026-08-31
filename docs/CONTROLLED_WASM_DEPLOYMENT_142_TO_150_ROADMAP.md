# Controlled WASM Deployment — Roadmap 142 to 150

- current validated checkpoint: `I35/141`
- target: `150/150`
- principle: small independently testable increments, preserving fail-closed governed execution
- lab baseline: `.101` dserver/local Sentinel, `.103` fog Sentinel, `.108` cloud Sentinel

## Strategy

The remaining nine checkpoints should close infrastructure gaps exposed by C67/I34/I35 rather than add unrelated functionality. Each increment must have a narrow contract, local regression, and a concrete live or fixture-based PASS gate.

The sequence intentionally supports three research tracks around DATUM/SmartSentinel:

```text
D-Graph representation / observed state
Deployment across the computing continuum
AI-assisted analysis / what-if over graph state (future research consumer)
```

The third line is a compatible research track previously discussed around D-Graph/GNN what-if analysis; it is not required for the core 150/150 closure.

## I36/142 — Persistent controlled deployment state

Goal: remove the most visible laboratory limitation in I34/I35: dserver losing source D-Graph, active manifest, and live reports on restart.

Scope:

- define a small persisted controlled-deployment state envelope;
- persist source D-Graph, derived manifest, and accepted live reports;
- restore state on dserver startup;
- use atomic replace/write discipline;
- preserve all existing validation before accepting restored state;
- fail closed on malformed/corrupt persisted state.

PASS:

```text
publish D-Graph
execute/report at least one assignment
restart dserver
GET manifest/source/reports still return the same validated state
corrupt persisted state -> startup does not activate invalid deployment
```

Research relevance: deployment-continuum foundation; reproducible experiments.

## I37/143 — Sentinel registration and lease state

Goal: let dserver know which logical Sentinels are currently present without trusting arbitrary deployment requests as proof of presence.

Scope:

- Sentinel registration endpoint/model;
- `agent.node_id`, instance identifier, stage/capability summary;
- heartbeat/lease timestamp;
- active/stale status;
- no cryptographic attestation claim yet;
- no automatic execution.

PASS:

```text
fog/local/cloud register independently
three active Sentinel registrations visible centrally
expired heartbeat becomes stale
node identity mismatch is rejected
```

Research relevance: D-Graph node state and deployment-continuum availability.

## I38/144 — Sentinel deployment inbox discovery

Goal: move from an operator naming a deployment URL on every invocation toward the Sentinel discovering whether dserver currently has work for its bound identity.

Scope:

- Sentinel command for controlled inbox/status discovery;
- fetch current node slice using bound `agent.node_id`;
- expose zero/one/multiple pending assignments without executing them;
- no execution without explicit operator transition;
- preserve HTTP/TLS policy.

PASS:

```text
no active deployment -> explicit empty/no-work result
one assignment -> discovered for correct node
cross-node work never appears
no guest execution occurs during discovery
```

Research relevance: deployment triggering and node-targeted distribution.

## I39/145 — Explicit multi-assignment scheduling

Goal: remove I35's intentionally narrow exactly-one-assignment constraint while keeping execution explicit.

Scope:

- deterministic assignment ordering;
- explicit batch invocation contract;
- per-assignment report and aggregate cycle result;
- stop/continue policy defined and tested;
- no automatic governed execution.

PASS:

```text
node slice with 2+ assignments is accepted only by batch mode
stable deterministic order
one report per attempted assignment
cross-node assignment remains rejected
```

Research relevance: deployment scheduling in heterogeneous continuum nodes.

## I40/146 — Dependency-aware deployment stages

Goal: make deployment order derive from D-Graph dependency/flow semantics instead of only a flat assignment list.

Scope:

- derive dependency edges between deployable assignments;
- topological stage planning;
- reject cycles/unsatisfied dependencies fail-closed;
- materialize planned stage metadata into deployment/observed state;
- execution remains explicit.

PASS:

```text
acyclic dependency fixture -> deterministic stages
cycle -> rejected
missing dependency -> rejected
three-node fixture yields expected dependency-aware plan
```

Research relevance: strongest bridge to the D-Graph IC and deployment-continuum IC.

## I41/147 — Deployment lifecycle and retry evidence

Goal: model the controlled lifecycle around assignment execution rather than only final `executed` reports.

Scope:

- states such as discovered/prepared/executing/executed/failed/blocked;
- attempt identifier and retry lineage;
- bounded retry policy representation;
- no hidden automatic retries with side effects;
- observed D-Graph retains current and prior attempt evidence.

PASS:

```text
successful attempt lifecycle materializes correctly
failed attempt is retained
explicit retry creates linked new attempt
no duplicate execution_id accepted
```

Research relevance: resilient deployment experiments and graph provenance.

## I42/148 — Resource-aware observed D-Graph join

Goal: correlate deployment/runtime evidence with Sentinel-observed node resources in the D-Graph.

Scope:

- join node CPU/memory/process/container/capability observations with deployment state;
- timestamp/freshness metadata;
- no placement decision automation required;
- expose machine-readable materialized state for downstream algorithms.

PASS:

```text
three nodes publish observation snapshots
observed D-Graph contains deployment + node resource state
stale observations are distinguishable from fresh observations
schema/relationship validation PASS
```

Research relevance: primary foundation for the graph IC and future GNN/AI what-if work.

## I43/149 — Deployment readiness and governance evidence

Goal: consolidate all prerequisites required before a deployment could ever become governable, without enabling automatic execution.

Scope:

- readiness summary over identity, active lease, D-Graph validity, artifact integrity, target capability, dependency readiness, resource freshness, execution boundary;
- explicit blocking reasons;
- audit-friendly readiness evidence;
- `automatic_execution_allowed=false` remains unchanged.

PASS:

```text
healthy fixture -> readiness=ready_for_explicit_execution
missing/stale prerequisite -> blocked with deterministic reasons
governed automatic execution remains unavailable
```

Research relevance: policy/placement inputs for deployment-continuum research.

## I44/150 — Phase final integration and reproducibility package

Goal: close the Controlled WASM Deployment phase with one reproducible three-node campaign and documentation that can be handed to students/researchers.

Scope:

- clean-start deployment campaign;
- dserver persistence survives restart;
- three Sentinel registrations/leases;
- D-Graph-derived dependency-aware deployment;
- direct Sentinel discovery/execution/report publication;
- observed graph with resource + lifecycle evidence;
- readiness evidence;
- exact commands, fixtures, expected outputs, architecture diagrams, non-claims, and extension points;
- final checkpoint `150/150`.

PASS:

```text
all local regressions PASS
three-node live campaign PASS
restart persistence PASS
observed D-Graph validation PASS
readiness evidence PASS
no Python orchestration in Sentinel success path
no canonical automatic governed execution
final docs sufficient to reproduce the validated lab
```

## Execution discipline

To finish quickly without sacrificing traceability:

1. one branch/PR per logical increment when code risk is material;
2. stack PRs while earlier validated PRs await explicit merge authorization;
3. document exact evidence immediately after each PASS;
4. do not rerun successful live nodes unnecessarily; reset campaign state explicitly when needed;
5. keep the `.101/.103/.108` fixture stable through I44 whenever possible;
6. do not introduce Kubernetes/Ansible/AWS into the 150/150 critical path; those belong to subsequent deployment-continuum experiments;
7. do not enable canonical automatic governed execution during this phase.

## Research handoff after 150/150

The phase-final system should expose clean research surfaces rather than forcing IC students to modify safety-critical core code immediately:

```text
D-Graph track
  schemas, relationships, observed state, resource/lifecycle graph, queries, visualization, GNN-ready export

Deployment-continuum track
  placement inputs, triggering, node registration/capabilities, dependency stages, scheduling, deployment experiments

AI/what-if track
  consume snapshots/graphs to predict CPU/RAM/traffic/latency/saturation or compare candidate placements without directly authorizing execution
```

This separation lets research prototypes consume stable interfaces while the governed execution boundary remains explicit and auditable.