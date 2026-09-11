# ADR-0008 — Fresh D-Serv WASM instance per invocation (ABI v0)

- Status: Accepted (Stage 107 baseline)
- Decision refs: D-9
- Implementation: Stages 109 / 113. Not implemented in Stage 107.

## Context

ABI `datum-dnode/0` handles one message per call (ADR-0003). The D-Node can either
keep a warm instance between messages or instantiate a fresh one each time. The
D-Serv state model for v0 is `stateless`, and the research plan wants an honest,
measurable cost model rather than a hidden one.

## Decision

1. The D-Node instantiates a **fresh instance per invocation** from a
   pre-compiled module (`Module` / `InstancePre`). Each call gets a clean linear
   memory and a reset fuel budget.
2. This gives strict stateless FaaS semantics for ABI v0. Stateful D-Serv and
   in-memory continuity are explicitly out of scope until a later ABI version.
3. **Instantiation overhead is measured**, not assumed away. Per-invocation
   `instantiate_ms` and `invoke_ms` samples (p50/p95) are recorded as D-Monitor
   evidence and included in the VS1b dataset, so the cost can be evaluated
   empirically and revisited.

## Consequences

- The D-Serv artifact carries an `instantiation` field
  (`fresh_instance_per_invocation`) and per-invocation `limits`
  (`fuel_per_invocation`, `timeout_ms`, `max_memory_pages`, `max_message_bytes`,
  `max_emits_per_invocation`).
- The convergence proof record (`dserv_convergence_proof`) carries an
  `instantiation` block.
- Local policy may lower the limits, never raise them (ADR-0011 / N-1).
