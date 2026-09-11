# ADR-0012 — Authorization validity during execution (TOCTOU)

- Status: Accepted
- TD-1 resolution: Option A
- Decision refs: TD-1; ADR-0011 (D-7a generation model)
- Decided in Stage 107A design, before Stage 107A implementation began. Stage 107
  recorded the question and did not choose a solution; this ADR now records the
  choice.

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

## Options considered (exactly one chosen in 107A design)

### Option A — snapshot / lease semantics  *(chosen)*

An authorization grants the exact immutable execution snapshot for a bounded
lease, even if the mutable artifact pointer later moves to a newer generation.

### Option B — immediate revocation semantics  *(not chosen)*

An artifact change must prevent not-yet-executed mutations, enforced either by an
online execution check just before each mutation start (with a stated residual
interval) or by an execution lease / serialization per node that rejects, queues
or delays artifact mutations touching a leased generation. Option B reintroduces
cross-domain coordination against the per-domain locking model (finding F14) and
makes edge execution depend on control-plane connectivity at mutation time.

Option B was rejected for Stage 107A: it couples every host mutation to live
DServer reachability, reintroduces the two-domain coordination that the D-7a
generation model was designed to remove, and its "immediate" guarantee still
carries a residual interval in variant (i). Option A gives a governance property
the protocol can actually enforce end to end from the edge.

## Selected resolution (Stage 107A): Option A — immutable authorized snapshot with bounded lease

### Architecture claim

The enforced and documented claim is:

> An authorization binds one **authorized immutable execution snapshot** and is
> valid for a **bounded lease**. A later change to the mutable artifact pointer
> takes effect for execution **no later than lease expiry** (**bounded
> staleness**). New authorizations after the change bind the current generation under
> the D-7a generation model; fetching an existing valid lease returns its snapshot.

The protocol does **not** provide, guarantee, enforce, deliver or ensure
immediate revocation of already-decided mutations, and no DATUM document,
contract fixture, evidence record or test may describe v1 (or v2) authorization
that way. "Bounded staleness up to lease expiry" is the only permitted
characterization of what a mid-execution artifact change does.

### Implemented semantics

1. **One immutable snapshot per authorization.** An authorization grants
   permission to execute exactly one immutable execution snapshot. The snapshot
   binds, and the agent validates before every governed host mutation or
   rollback: `project_id`; the application / app-deployment context
   (`dgraph_id`); the D-Graph `revision`; the node id and the per-service
   resource `binding_id`; for each authorized desired service the
   `{artifact_id, execution_generation, execution_projection_digest}` triple
   (ADR-0011 §2); the `execution_pinning_mode` bound at issuance; and every other
   execution-relevant value carried in the snapshot. All of these are fixed at
   issuance and never re-resolved during the lease.

2. **A later artifact PUT does not retroactively invalidate an issued lease.**
   Once a valid authorization / lease has been issued for snapshot `S` at
   generation `g`, a subsequent content-changing `PUT` that moves the artifact to
   `g + 1` does **not** reach back and abort an already-issued, still-valid lease
   for `S`. A *new authorization* must bind the current generation `g + 1`;
   a context fetch carrying the existing valid lease still returns snapshot `S`. This is bounded staleness, not immediate revocation.

3. **Governed host mutation requires a finite lease (corrected — review finding
   1).** Option A's bounded-staleness guarantee depends on the agent being able
   to determine, on its own, that a lease has expired even while DServer is
   unreachable. `persistence_scope = process`
   (`AUTHORIZATION_SCOPE_PROCESS`) never carries `expires_at_utc`
   (`build_reconciliation_authorization`, `dserver/src/core/operational_dgraph.rs`,
   forbids `valid_for_seconds` on that scope and always sets `expires_at_utc =
   None`); its only bound is "the issuing DServer process is still the same
   process", which an executing agent has no way to observe once it has fetched
   a snapshot. A process-scope authorization therefore **does not satisfy the
   Option-A bounded-lease property** and **must not be accepted to gate a
   governed host mutation (any of M1–M8, point 9)**. Only an authorization
   carrying a finite `expires_at_utc` — today, exactly `persistence_scope =
   durable` — may authorize a host mutation. A process-scope authorization may
   remain valid for non-mutating purposes (status inspection, dry-run reporting,
   `proposal_only` contexts) where the bounded-staleness safety property does
   not apply. **Compatibility consequence:** any existing flow that issues a
   process-scope authorization (the request default, per
   `default_authorization_scope()`) and expects it to gate a host mutation is
   **deliberately broken** by Stage 107A Section H and must switch to
   `persistence_scope = durable` with an explicit `valid_for_seconds`. Section H
   must reject a host-mutation attempt authorized only by a non-finite-expiry
   record with a distinct block reason (`blocked_authorization_not_leased`)
   rather than silently upgrading or accepting it — the safer fail-closed
   choice.

4. **A host mutation may begin only while the lease is valid, checked
   immediately before that mutation (see point 9 for the exact boundary set).**
   The agent checks lease validity — identity match **and** a finite
   `expires_at_utc` that has not passed under the bounded-clock-error model
   (point 11) — immediately before each governed mutation. If the lease is not
   valid at that instant, the agent must not begin that mutation.

5. **Lease expiry before a mutation boundary ⇒ re-authorize.** If the lease
   expires before the agent is about to begin a given host-mutating operation
   (point 9), the agent must not begin it; it stops the cycle for that service,
   reports a blocked status, and must obtain and validate a new authorized
   snapshot before attempting any further mutation for that service — including
   one that was already about to run as a direct continuation of a mutation that
   already executed (e.g. the recreate-then-create sequence in
   `execute_trusted_container_intent`).

6. **Lease expiry after a mutation boundary is crossed does not abort that
   operation.** Once the agent has begun a given host-mutating operation while
   the lease was valid, expiry of the lease during that operation does not
   abort it; the operation runs to its terminal state and is then observed.
   This is a **per-mutation-boundary** rule (point 9), not a once-per-cycle
   rule: a governed intent that performs more than one boundary in sequence
   (for example M3 then a recursive M1) re-checks the lease before each one.

7. **Rollback follows the same rule.** A rollback is a governed host mutation
   (`M8`) and obeys points 3–6 identically, against the rollback authorization's
   snapshot.

8. **No silent re-resolution.** The agent must execute exactly the immutable
   snapshot it validated. It must never silently re-read or re-resolve a mutable
   artifact pointer (control-plane record, image tag, config path) between
   validation and execution. Any value it needs at execution time comes from the
   validated snapshot.

9. **Execution-start boundary — precise definition, aligned to the audited
   mutation set (corrected — review finding 3).** The Stage 107A
   `mutation-entry-point-audit.txt` traces the complete host-mutation set for
   the v1 container path as **M1–M8**:
   `M1` `docker run` (create + start a missing container);
   `M2` `docker start` (an existing stopped container);
   `M3` `docker rm -f` (recreate for declared-attachment drift);
   `M4` `docker rm -f` (recreate a stopped container to refresh secrets);
   `M5` `docker pull` (the lifecycle `pull_image` mutation itself — the
   preceding `docker image inspect` is read-only);
   `M6` `docker rm -f` (lifecycle `remove_container`);
   `M7` `docker network create` (only when `docker network inspect` reports the
   network missing);
   `M8` `docker rm -f` (rollback).
   Each of M1–M8 is an **independent governed mutation boundary**. All other
   Docker calls in the path (`inspect`, `image inspect`, `network inspect`,
   exit-code inspection, `docker version`, config-byte hashing) are read-only
   and are never themselves a boundary. Lease validity (point 4) is evaluated
   immediately before **each** of M1–M8 individually, never once per
   reconciliation cycle. Evidence records, per service and per cycle, a
   `mutation_boundaries_crossed` sequence — one entry per Mx actually performed,
   each with `{op: "M1".."M8", crossed_at_utc, lease_valid_at_check: true}` —
   superseding the earlier single `execution_start_crossed_at_utc` field. This
   boundary set is covered by the test obligations below.

10. **Lease bound reuses the existing bounded-validity mechanism, restricted to
    finite expiry (point 3).** The current
    `OperationalReconciliationAuthorization.expires_at_utc`
    (`dserver/src/core/operational_dgraph.rs`) **is** the lease bound for any
    authorization used to gate a host mutation. A durable authorization already
    carries an absolute RFC 3339 `expires_at_utc` derived from
    `valid_for_seconds` (1 .. `MAX_DURABLE_AUTHORIZATION_SECONDS` = 86 400).
    Stage 107A introduces **no new clock field and no new lease field**; it
    reuses `expires_at_utc`, requires it to be present for any mutation-gating
    authorization (point 3), and adds only the agent-side per-mutation-boundary
    check described in point 9. `valid_for_seconds` remains operator-supplied
    and configurable per authorization request.

11. **Clock source and a bounded-clock-error model (corrected — review finding
    2).** Expiry is evaluated as `expires_at_utc > now` where `now` is
    `chrono::Utc::now()` on the evaluating process (DServer at issuance and
    context assembly, the agent at its own validation point). DATUM does **not**
    assume the two clocks are perfectly synchronized, but safety under that
    assumption requires an explicit, configured, non-negative maximum trusted
    skew `Δ` (`authorization_clock_skew_seconds`) that the deployment operator
    attests bounds `|agent_clock − DServer_clock|` at all times. The agent
    treats the lease as expired once `now_agent + Δ ≥ expires_at_utc`. This is
    the safe direction: if the agent's clock is **behind** DServer's by up to
    `Δ`, a naive unadjusted comparison (`now_agent > expires_at_utc`, i.e. `Δ =
    0` with no attested bound) can let the agent believe a lease is still valid
    after it has truly expired by DServer's clock — **unsynchronized clocks can
    extend the agent's apparent window, not just shorten it, whenever `Δ` is
    not honestly configured.** The `Δ`-adjusted check closes exactly that gap:
    for any true clock error within the attested `Δ` (in either direction), the
    agent stops acting at or before the true `expires_at_utc` instant. This
    property depends entirely on `Δ` actually covering the real skew.
    **`execution_pinning_mode = required` MUST refuse to gate a host mutation
    unless `authorization_clock_skew_seconds` is explicitly present in agent
    configuration** — there is no silent default of `0`, because an unconfigured
    `Δ` is indistinguishable from "assume perfect synchronization", which is
    exactly the claim this ADR must not make. `legacy_partial` mode may fall
    back to `Δ = 0` with the weaker guarantee recorded in evidence
    (`f2_status = partial_hardening` already covers this). The permitted safety
    claim is: *"DATUM assumes a deployment-attested bounded clock error `Δ`
    between DServer and the agent, not perfect clock synchronization; the lease
    is honoured only up to `expires_at_utc − Δ` in the agent's own clock."*

### Precedence

Within one reconciliation cycle the agent evaluates, in order, and stops at the
first that fails closed: (a) authorization identity match; (b) pinning-mode
floor — the snapshot's `execution_pinning_mode` must satisfy the agent's local `minimum_execution_pinning_mode`;
(c) per-artifact generation + projection-digest match (recomputed locally);
(d) required external-input verification for `required` mode; (e) a finite
lease is present (point 3) and is re-checked for validity immediately before
**each** mutation boundary M1–M8 (point 9), not once per cycle. DServer checks the active resource binding when serving context. Project-mode
changes apply at new issuance; an existing snapshot retains its bound mode.
One snapshot per
cycle: an agent never mixes generations from two snapshots in a single cycle.

### DServer unreachable at execution time

Because the snapshot is self-contained, an agent that holds a valid,
finite-expiry, not-yet-expired lease (point 3) and a validated snapshot may
complete that cycle's remaining mutation boundaries without contacting
DServer, re-checking the lease before each one (point 9). If the lease is
absent, non-finite, or expired at a boundary check and DServer is unreachable,
the agent fails closed (`blocked_control_plane`, or
`blocked_authorization_not_leased` if the authorization was process-scope) and
performs no further mutation. Edge autonomy is bounded by the lease, which is
the intended governance property.

### Test obligations

Stage 107A must carry automated tests establishing all of the following. These
**test obligations** are part of this decision:

- **O-1** A valid lease for snapshot `S` (generation `g`) stays valid for
  execution after a later content-changing `PUT` moves the artifact to `g + 1`,
  for the remainder of the lease; the evidence records
  `execution_semantics = snapshot_lease` and the staleness is described as
  bounded, never as revocation.
- **O-2** After that same `PUT`, a *new authorization* binds current generation
  `g + 1` under the stable registry view. An existing valid lease continues to
  serve generation `g`; an expired lease cannot revive when content returns.
- **O-3** A lease that expires **before** a given mutation boundary (M1–M8)
  blocks that operation; the agent obtains and validates a new snapshot before
  attempting it and never begins it on an expired or non-finite lease.
- **O-4** A lease that expires **after** a mutation boundary has begun does not
  abort that in-flight operation; if a second boundary would follow in the same
  governed intent (e.g. M3 then a recursive M1), it does not begin on the
  now-expired lease.
- **O-5** Rollback (M8) obeys O-3 and O-4 identically.
- **O-6** The agent never re-resolves a mutable artifact pointer between
  validation and execution (a control-plane mutation during the cycle does not
  change the values the agent executes).
- **O-7** Every governed mutation boundary matches exactly the M1–M8 set defined
  in point 9 (`docker run`, `docker start`, the two attachment/secret
  `docker rm -f` recreate paths, `docker pull`, lifecycle `remove_container`'s
  `docker rm -f`, `docker network create`, rollback's `docker rm -f`); every
  read-only Docker call (`inspect`, `image inspect`, `network inspect`,
  exit-code inspection, `docker version`) is confirmed to never itself gate as
  a boundary; `mutation_boundaries_crossed` is recorded per Mx actually
  performed.
- **O-8** A process-scope (`persistence_scope = process`) authorization is
  rejected with `blocked_authorization_not_leased` for every mutation boundary
  M1–M8, even when its identity fields match and `allow_host_mutation = true`;
  only a finite-expiry (durable-scope) authorization can gate a mutation.
- **O-9** The Stage 107 consistency checker accepts this ADR in the
  Accepted / Option A state and still rejects any assertive immediate-revocation
  claim elsewhere in the documentation or fixtures.
- **O-10** With `authorization_clock_skew_seconds` unset, `execution_pinning_mode
  = required` refuses to gate a mutation (fail closed, does not silently treat
  the skew as `0`); with it explicitly set to `Δ`, an agent clock offset of up to
  `Δ` in either direction (simulated behind and ahead of DServer's clock) never
  allows a mutation to begin after the true (DServer-clock) `expires_at_utc`
  instant.
- **O-11** `legacy_partial` mode may operate with an unset
  `authorization_clock_skew_seconds` (treated as `0`), and this is recorded in
  evidence as part of `f2_status = partial_hardening`, not silently accepted as
  equivalent to `required` mode's guarantee.

## Consequences

- ADR-0011's D-7a model and this ADR together define v1 F2 semantics:
  content-bound authorization (ADR-0011) plus snapshot / bounded-lease execution
  validity (this ADR).
- The permitted wording throughout the repository is "authorized immutable
  snapshot with bounded staleness up to lease expiry". The forbidden wording is
  any assertion that an artifact change immediately prevents, revokes or aborts
  already-decided or in-flight mutations.
- v2 authorization binds an immutable slice digest and is already snapshot-like;
  v1 and v2 therefore **share** snapshot / bounded-lease semantics, with the v1
  lease carried by `expires_at_utc` and the v2 lease by the slice authorization's
  own bounded validity.
- Live re-authorization around 107B generation changes is an explicit operator
  step: a generation bump affects new authorization issuance; existing snapshot
  leases remain bounded by their own expiry.
- **Deliberate compatibility break (point 3):** Stage 107A Section H stops
  accepting a process-scope authorization as sufficient to gate a host mutation.
  Any operator tooling or test that today issues a process-scope authorization
  and expects it to authorize a mutation must switch to durable-scope with an
  explicit `valid_for_seconds` once Section H lands; this must be called out in
  the Section H/K migration notes, not left implicit.
