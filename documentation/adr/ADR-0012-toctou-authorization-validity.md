# ADR-0012 — Authorization validity during execution (TOCTOU)

- Status: **DEFERRED / OPEN**
- Decision refs: TD-1
- Must be decided **before Stage 107A implementation begins**. Stage 107 records the
  question and does **not** choose a solution.

## Context

The generation model (ADR-0011) guarantees that an authorization is valid **against
the context an agent fetches**. It does not, on its own, say what happens in this
interval:

1. The agent fetches context `C`, valid: its bindings equal generation `g`.
2. The agent plans, then starts host mutation `M` at time `t_m`.
3. A `PUT` between the fetch and `t_m` moves the artifact to generation `g + 1`.

Without an additional synchronization mechanism the agent cannot learn of step 3.
The same question applies to rollback, multi-service batches, long mutations
(image pulls), DServer being unreachable at `t_m`, and a v2 slice digest being
superseded by a newer slice revision.

## Options (exactly one to be chosen in 107A design)

### Option A — snapshot / lease semantics

An authorization grants the exact immutable execution snapshot for a bounded
lease, even if the mutable artifact pointer later moves to a newer generation.

To be specified before adoption: what the snapshot contains and that the agent
never re-reads mutable artifact state during the lease; the lease bound (max
duration, who sets it, clock source, skew tolerance, whether it bounds mutation
*start* or *completion*); the relation to D-7a (an artifact change invalidates new
context fetches; an in-flight snapshot runs until lease expiry — **bounded
staleness**, described as such); batch behaviour (one snapshot per cycle, no
generation mixing); precedence of authorization expiry, pinning-mode upgrade,
binding change; evidence fields.

Permitted claim: *"an artifact change takes effect for execution no later than
lease expiry."* Not permitted: immediate revocation.

### Option B — immediate revocation semantics

An artifact change must prevent not-yet-executed mutations.

To be specified before adoption: the enforcement mechanism — either
(i) an **online execution check** just before each mutation start, with the
residual interval stated and bounded; or
(ii) an **execution lease / serialization** per node and bindings, where artifact
mutations that would change a leased generation are rejected, queued or delayed,
designed explicitly against the per-domain locking model (finding F14) because it
reintroduces cross-domain coordination.
Also: a precise definition of "not yet executed", including a started mutation;
behaviour when DServer is unreachable (fail-closed, no mutation, and the loss of
edge autonomy accepted explicitly); evidence fields.

Permitted claim: immediate revocation — only once the chosen mechanism is
implemented and its tests pass; any residual interval of variant (i) stated.

## Decision criteria (for 107A design)

Governance strength vs edge autonomy (option B depends on control-plane
connectivity at execution time); coordination cost (B(ii) reintroduces
registry / lease coordination; A needs lease and clock handling); consistency with
v2 (v2 authorization binds an immutable slice digest, which is naturally
snapshot-like — the decision must state whether v1 and v2 share semantics or
document the difference); operational behaviour around 107B generation changes and
live re-authorization; research defensibility (the paper's governance claims must
match the enforced semantics exactly).

## Constraints until TD-1 is decided

- No document, ADR, fixture, evidence record or test may describe v1 (or v2)
  authorization as providing **immediate revocation of already-decided mutations**.
- Stage 107 ADRs and fixtures record TD-1 as open.
- Stage 107A implementation does not begin until one option is specified with its
  tests.

## Resolving this ADR in Stage 107A

This ADR is **Deferred** only while TD-1 is unresolved. It is a real open design
question, not a permanent bar. Stage 107A resolves it by an explicit decision and
this ADR then moves to **Accepted**. The consistency checker
(`scripts/check_datum_contract_fixtures.py`, `evaluate_td1`) enforces an
**option-dependent** lifecycle across three states:

### 1. Deferred / Open (current)

- `- Status:` reads `Deferred / Open`.
- No `- TD-1 resolution:` line.
- The "immediate revocation must not be claimed" constraint above is present, and
  a "decided before Stage 107A" marker is present.
- No contract, fixture or Stage 107 document may assert immediate revocation.

### 2. Accepted — Option A (snapshot / lease)

- `- Status:` reads `Accepted`.
- `- TD-1 resolution: Option A`.
- A section recording the selected semantics and its **test obligations** (the
  words "test obligations" must appear).
- **Immediate-revocation claims remain forbidden.** Documentation may claim only
  the semantics Option A actually provides: an immutable authorized snapshot, a
  bounded lease, and bounded staleness. Any wording equivalent to "an artifact
  change immediately prevents all not-yet-executed mutations" still fails the
  checker.

### 3. Accepted — Option B (immediate revocation)

- `- Status:` reads `Accepted`.
- `- TD-1 resolution: Option B`.
- A section recording the selected semantics and its **test obligations**.
- Immediate-revocation claims are permitted **only** once the ADR also records:
  `- TD-1 Option B evidence: satisfied` (or `complete` / `completed` /
  `recorded`), meaning the required Option B implementation and test evidence is
  in place. Selecting Option B alone does **not** enable the claim; until the
  evidence marker is present the claim scan stays active.

Moving to Accepted without the resolution line and the test-obligations record is
rejected by the checker regardless of option.
