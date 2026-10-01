# Phase 119H0 — Operational Reconciliation Bootstrap Production-Path Decoupling Audit

- Status: Audit only. No production code was added or changed by this phase.
- Branch: `agent/phase119-migration`, continuing from Phase 119G, HEAD
  `ec437a9c274100110221a21f4da2a0da062babe6`.
- Scope: determine the smallest safe production path by which
  `smartsentinel-operational-reconcile` can obtain `KnownBoundNode` /
  `resolved_node_id` / `resource_id` from current authoritative
  DATUM/DServer state without an unnecessary hard dependency on legacy
  Firebase, so Phase 119H's live re-proof can exercise the real production
  binaries end to end.

## 1. Executive conclusion — **Decision B**

**A small, precisely-localized bootstrap fix is needed. Every authoritative
fact reconciliation actually consumes downstream (`resource_id`,
`resolved_node_id` in the common already-bound case, and the entire
authorization/mutation path) is already Firebase-free today.** The hard
Firebase dependency exists in exactly two call sites inside one function
(`run_bootstrap_with_resource_discovery`, `DATUM/src/agent/bootstrap.rs`),
and in both cases the current code treats a failure as **fatal** even
though, in the common case, the result either has an existing Firebase-free
fallback or is never consumed by reconciliation at all. This is not a
missing server-side authority (Decision C) — every fact needed already has
a Firebase-free server-side source; the agent's bootstrap code simply
doesn't use graceful degradation around two calls that don't need to
succeed.

## 2. Exact current bootstrap flow (freshly re-traced)

```
main() [smartsentinel-operational-reconcile.rs]
  -> run_operational_reconciliation(cfg, node_id, hostname, dserver_url, project_id, timeout_s, execute, rollback, cleanup)
  -> run_bootstrap(cfg, requested_node_id, hostname_override, dserver_url, project_id, timeout_s)
       -> collect_unbound_resource_discovery(cfg, hostname_override)   -- 100% LOCAL, no network at all
       -> run_bootstrap_with_resource_discovery(discovery, requested_node_id, dserver_url, project_id, timeout_s)
  -> run_operational_reconciliation_from_bootstrap(cfg, bootstrap, ...)
       -> requires bootstrap.bootstrap_state == BootstrapState::KnownBoundNode  (operational_reconciliation.rs:1423)
       -> uses bootstrap.control_plane.resolved_node_id                        (operational_reconciliation.rs:1431)
       -> uses bootstrap.control_plane.resource_id                             (operational_reconciliation.rs:1512, 1523, 2638)
       -> fetches operational-reconciliation-context, authorizes, reconciles/rolls-back/cleans-up
          (entirely separate code, entirely separate DServer endpoints -- see §5)
```

`collect_unbound_resource_discovery` (`DATUM/src/agent/resource_discovery.rs:97`)
performs only local hardware/platform/capability observation
(`collect_discovery`) and a pure local computation
(`derive_resource_lookup_id`, `resource_discovery.rs:117`, deriving
`resource_id` from a hardware fingerprint or OS-install id, or falling back
to a random `instance_id`) — **zero network calls, zero Firebase**.

`run_bootstrap_with_resource_discovery` (`bootstrap.rs:69`) then makes, in
order:

1. `GET /api/v1/resources/{resource_id}` (resource lookup, `bootstrap.rs:99`).
2. `query_node_context` → `GET /api/v1/nodes/{node_id}/latest`
   (`bootstrap.rs:195`), only if a `node_candidate` exists.
3. `query_optional_capabilities` → `GET /api/v1/nodes/{node_id}/capabilities`
   (`bootstrap.rs:226`) and `query_optional_registration` →
   `GET /api/v1/sentinels/{node_id}` (`bootstrap.rs:241`), only if
   `resolved_node_id` exists.

`bootstrap_state` is then `classify_bootstrap_state(node_known,
resource_record.is_some(), registry_assigned_node_id.is_some())`
(`bootstrap.rs:181`):

```rust
if node_known || resource_has_assigned_node { KnownBoundNode }
else if resource_known { KnownUnboundResource }
else { UnknownResource }
```

and `resolved_node_id` (`bootstrap.rs:146`) is `node_candidate` when
`node_known`, **else `registry_assigned_node_id`**.

## 3. Exact point(s) Firebase enters — server side, not the agent's own code

The agent's HTTP calls above are ordinary, Firebase-unaware requests to
DServer's own `/api/v1/...` surface. Tracing each handler:

| Agent call | DServer handler | Backing store |
|---|---|---|
| `GET /api/v1/resources/:id` | `get_resource` (`dserver/src/api/resource_registry.rs:282`) | `resource_state_store()` — **local, Firebase-free** |
| `GET /api/v1/nodes/:id/latest` | `get_latest` (`dserver/src/api/nodes.rs:40`) | `state.firebase.list_nodes()` + `state.firebase.get_latest_snapshot()` — **FIREBASE** |
| `GET /api/v1/nodes/:id/capabilities` | `get_node_capabilities` (`dserver/src/api/capabilities.rs:68`) | `state.firebase.get_node_capabilities()` — **FIREBASE** |
| `GET /api/v1/sentinels/:id` | `get_sentinel_registration` (`dserver/src/api/sentinel_registration.rs:99`) | `registration_store()` — **local, Firebase-free** |

`FirebaseClient` (`dserver/src/storage/firebase.rs:22`) unconditionally
constructs a real `https://firestore.googleapis.com/...` base URL from
`app_config.firebase.project_id` at server startup (`FirebaseClient::new`,
`firebase.rs:61`) — construction itself does not fail without live
connectivity (it just builds a struct), so DServer **starts** fine offline;
the failure is purely lazy, at the moment `get_latest`/
`get_node_capabilities` are actually invoked and try to reach Firestore.
This matches, and precisely localizes, Phase 119D's observed blocker.

**Everything downstream of bootstrap — the operational-reconciliation
context fetch, both ordinary and cleanup authorization, and the entire
Stage107A mutation path — is confirmed, by direct file-wide search, to be
100% Firebase-free** (`dserver/src/api/operational_dgraph.rs` contains no
`firebase` reference at all).

## 4. Firebase dependency — answers to the six required questions

1. **Is Firebase always required?** No — only two of bootstrap's four HTTP
   calls touch it (`§3`), and both are non-load-bearing in the common case
   (`§6`).
2. **Required only under one backend?** Effectively yes: the legacy
   `datum_nodes`/`datum_snapshots_latest` Firestore collections are the
   *only* backend `get_latest`/`get_node_capabilities` know how to read —
   there is no SQLite/local fallback coded into those two handlers today.
3. **Does explicit `--node-id` avoid any portion of Firebase lookup?** **No
   — the opposite.** Supplying `--node-id` is exactly what makes
   `node_candidate` `Some`, which is exactly the precondition for
   `query_node_context` (Firebase) to be called at all (`bootstrap.rs:135`).
   Omitting `--node-id` with no resource binding yet established would
   *skip* it (`node_candidate` stays `None`), but would also then never
   reach `KnownBoundNode`.
4. **Does `--project-id` avoid any portion?** No — `project_id` is only
   used as the `X-Datum-Project` header value on every call, Firebase-backed
   or not; it doesn't change which calls are made.
5. **Is DServer already queried for the same authoritative resource
   binding later in the flow?** **Yes.** `authorize_operational_reconciliation`
   and `authorize_operational_cleanup` both call
   `active_resource_binding_for_node(&project_id, node_id)`
   (`dserver/src/api/resource_registry.rs:358`) fresh, on **every**
   authorization request — the exact same authoritative binding fact
   bootstrap tries to establish is independently re-verified, from the same
   Firebase-free store, downstream of bootstrap, every single cycle.
6. **Is there already another production endpoint that can supply
   `resource_id`/`binding_id`/resolved node identity without Firebase?**
   **Yes**, two of them: `GET /api/v1/resources/:resource_id`
   (`resource_state_store()`, supplies `assigned_node_id` +
   `binding_id`-bearing record) and `GET /api/v1/dnodes/:node_id`
   (`dserver/src/api/dnode_registry.rs:143`, `dnode_db_path()`-backed
   structural D-Node existence) — both already Firebase-free, both already
   shipped, neither currently consulted as a Firebase-avoiding path for the
   two calls in `§3`.

## 5. Authority question — who should own what, today

| Fact | Current authoritative owner (as shipped) | Firebase-free? |
|---|---|---|
| Structural node existence (`is this node_id declared at all`) | D-Node Registry (`GET /api/v1/dnodes/:node_id`, `dnode_db_path()`) | **Yes** |
| `resource_id` (this physical/virtual resource's identity) | Local hardware/OS fingerprint (`derive_resource_lookup_id`, purely local) | **Yes** |
| `binding_id` / "which node is this resource_id currently bound to" | Resource Registry (`GET /api/v1/resources/:resource_id`, `resource_state_store()`) | **Yes** |
| Current operational context / authorization | DServer operational-dgraph + Stage107A path (`dserver/src/api/operational_dgraph.rs`) | **Yes** |
| "Has this node ever reported a legacy snapshot" (`node_known` today) | Legacy Firebase `datum_nodes`/`datum_snapshots_latest` | **No** |
| Node capabilities (`expected_capabilities`, informational only) | Legacy Firebase `datum_node_capabilities` | **No** |

The audit finds **no case where legacy Firebase bootstrap should become, or
currently is, a second canonical authority for anything reconciliation
actually consumes.** `node_known`/`expected_capabilities` are the only two
Firebase-sourced facts in the whole flow, and neither is structurally
required once the Resource Registry binding exists (`§6`).

## 6. Is `resolved_node_id`/`resource_id` structurally dependent on Firebase?

**No**, in the common (already-bound) case — confirmed by direct code
reading, not inference:

- `resolved_node_id` (`bootstrap.rs:146`) already falls back to
  `registry_assigned_node_id` whenever `node_known` is `false`. It only
  *needs* `query_node_context` to succeed when the Resource Registry does
  **not yet** have an assigned node for this resource.
- `bootstrap_state`'s `KnownBoundNode` classification already accepts
  `resource_has_assigned_node` **alone**, with no dependency on
  `node_known` (`classify_bootstrap_state`, `bootstrap.rs:186`).
- `expected_capabilities`/`sentinel_registration`/`latest_snapshot`/
  `resource_record` are, confirmed by exhaustive grep across
  `operational_reconciliation.rs` and `operational_service_loop.rs`, **never
  read** by anything downstream of bootstrap except `resource_id` and
  `resolved_node_id` themselves (the only two `bootstrap.control_plane.*`
  fields consumed anywhere in the reconciliation path, plus `bootstrap_state`
  itself).

**The defect is not that these facts structurally require Firebase — it is
that the current code lets a *failure* to fetch two facts that are either
already-redundant or entirely unused be *fatal* to the whole bootstrap
call**, via unconditional `?` propagation in `run_bootstrap_with_resource_discovery`
(`bootstrap.rs:136`, `:154`) rather than graceful degradation (the same
pattern already used for a plain 404, `bootstrap.rs:207-223`, but not for
any other failure mode, including a Firebase outage surfaced as a 500 by
`get_latest`/`get_node_capabilities`).

## 7. Does explicit node-id + DServer alone suffice? (§7 of the task)

**Yes, once a Resource Registry binding exists for the resource** — which,
for Phase 119D/119H's actual RPi/fog nodes (already-configured from earlier
phases, not fresh installs), is the expected real-world state. Concretely,
today's code already performs A (implicitly, via the resource lookup) and B
(the resource lookup **is** the authoritative binding fetch) — it just also
unnecessarily attempts, and lets fail fatally, two Firebase calls whose
results are either redundant with B or unused. C (operational
reconciliation context) is confirmed entirely Firebase-free already (`§3`).

For a **completely fresh** node/resource that has never been bound in the
Resource Registry, today's code has **no** Firebase-free path to
`KnownBoundNode` — but this is not evidence of a missing DServer authority:
the structural D-Node Registry (`§4.6`) and the Resource Registry's own
binding-write endpoint (`POST /api/v1/resources/:resource_id/binding`,
`resource_registry.rs:125`, itself Firebase-free) already exist server-side
to establish this without Firebase; the agent's bootstrap simply doesn't
consult the D-Node Registry today. This is optional, deferred hardening
(`§9`), not a blocker for Phase 119H's actual scope (already-bound nodes).

## 8. Caller-trust check (task §8)

The recommended fix (`§9`) introduces **no** `--resource-id` override and
**no** new caller-supplied identity of any kind. `resource_id` remains
exactly as today: computed locally from hardware/OS fingerprinting, then
independently confirmed against the server-side Resource Registry via the
existing, unmodified `GET /api/v1/resources/:resource_id` call. Nothing
about the recommended fix lets a caller assert an identity that DServer
does not itself independently verify.

## 9. Recommended minimum fix (NOT implemented in this phase)

Two small, localized, purely error-handling changes, both confined to
`run_bootstrap_with_resource_discovery` in `DATUM/src/agent/bootstrap.rs`:

1. **`query_node_context`'s failure must not abort bootstrap** when a
   Resource Registry binding already supplies `registry_assigned_node_id`
   — degrade to `(false, latest_snapshot: None)` (informational field lost,
   nothing authoritative lost) instead of propagating the error via `?`,
   whenever the call is attempted only for enrichment rather than to
   establish `KnownBoundNode` in the first place. When no binding exists
   yet, the call remains load-bearing and its current fatal behavior is
   arguably still correct (there is genuinely no other Firebase-free
   `node_known` signal wired up today) — though see the optional
   enhancement below.
2. **`query_optional_capabilities`'s failure must never abort bootstrap at
   all** — its result (`expected_capabilities`) is confirmed unused by
   reconciliation in every case (`§6`); treating any non-404 failure the
   same as a 404 (`Ok(None)`) is strictly safe and loses no consumed
   information.

**Optional, deferred hardening** (not required for Phase 119H, matching
this session's established pattern of deferring non-essential extensions):
consult `GET /api/v1/dnodes/:node_id` as an additional, Firebase-free
`node_known`-equivalent signal for the fresh/unbound-resource case, so a
brand-new node could reach `KnownBoundNode` without ever touching Firebase
or requiring a prior manual binding step. Not attempted or scoped further
in this audit.

**Explicitly out of scope for this fix**: no change to `desired_state`
vocabulary, `union_project_operational_graph`, D-Engine, migration/cleanup
derivation, authorization operations, or any Stage107A gate. The fix is
pure bootstrap-phase error-handling.

## 10. Compatibility

Auto-discovery mode (no `--node-id`, resource not yet bound) is
**untouched** by the recommended fix — it already, correctly, depends on
whichever backend `get_latest`/`get_node_capabilities` use to discover a
brand-new node's identity; that dependency is a property of the current
legacy discovery flow, not something this audit proposes removing. The fix
only prevents a genuinely redundant or unused Firebase round-trip from
being fatal once a Resource Registry binding already exists — it does not
touch, weaken, or bypass Firebase for any case where it is still the only
available signal.

## 11. Cleanup mode uses the identical identity path

Confirmed by direct code reading, not assumption: `reconcile`, `rollback`,
and `cleanup` are all three driven through the exact same
`run_operational_reconciliation` → `run_bootstrap` →
`run_operational_reconciliation_from_bootstrap` chain (`DATUM/src/bin/smartsentinel-operational-reconcile.rs:79`,
`DATUM/src/agent/operational_reconciliation.rs:739-800`). The `--cleanup`
flag (Phase 119G) only changes which loop runs **after** bootstrap
completes and `context` is fetched — it introduces no cleanup-specific
resource-identity shortcut of any kind, and the recommended fix in `§9`
therefore applies identically and simultaneously to all three modes.

## 12. Stage107A / downstream authority — unaffected

Re-verified against the exact facts consumed: `resolve_stage107a_cycle_gate`
(`operational_reconciliation.rs`) verifies `resource_id` against the served
authorization's own `resource_id` (`check_resource_identity`) independent
of anything bootstrap computed beyond supplying which `resource_id` to ask
about; `authorize_operational_reconciliation`/`authorize_operational_cleanup`
independently re-fetch `active_resource_binding_for_node` fresh, every
cycle (`§4.5`). The recommended fix touches only how two non-authoritative,
enrichment-only bootstrap calls fail — it changes nothing about how any
authority is established, checked, or bound anywhere downstream. Stage107A
strength is completely unaffected.

## 13. Decision

**B — small foundation fix needed.** Existing authoritative DServer facts
(local hardware fingerprint → `resource_id`; Resource Registry →
`binding`/`assigned_node_id`; D-Node Registry → structural existence;
operational-dgraph/Stage107A → current context and authorization) are
already sufficient for the common (already-bound) production case. The CLI
bootstrap path is unnecessarily, fatally coupled to two Firebase-backed
calls whose results are either redundant with an already-Firebase-free
fallback or entirely unconsumed. Per the task's explicit instruction, this
fix is **not implemented in Phase 119H0** — `§9` above is the audit's
recommendation for Phase 119H's own small foundation-fix step, immediately
before the live re-proof, not a change bundled into this audit.

## 14. Exact Phase 119H prerequisites

1. Implement the two-call graceful-degradation fix described in `§9`
   (small, single-file, `DATUM/src/agent/bootstrap.rs`), reviewed
   independently before any live infrastructure is touched.
2. Confirm, via the existing RPi (`rpi-119d`)/fog-01 (`fog-119d`) resources
   from Phase 119D, that each already has (or can trivially be given, via
   the existing `POST /api/v1/resources/:resource_id/binding` endpoint) a
   Resource Registry binding — this is what makes the fix's fast path
   reachable without depending on Firebase at all.
3. Re-run `smartsentinel-operational-reconcile --execute` and
   `smartsentinel-operational-reconcile --cleanup --execute` against the
   same isolated DServer topology Phase 119D used, this time expecting
   real Stage107A host mutation to actually proceed past bootstrap.
4. Only then power on RPi/fog for the full live A→B→A migration +
   governed source-cleanup re-proof.
