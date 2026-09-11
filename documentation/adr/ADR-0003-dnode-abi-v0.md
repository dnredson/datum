# ADR-0003 — D-Node ABI `datum-dnode/0`

- Status: Accepted (Stage 107 baseline)
- Decision refs: D-1
- Implementation: Stage 109 (invoker) and Stage 113 (`datum-dnode` runtime). Not implemented in Stage 107.

## Context

D-Code must run inside a D-Node as a service that processes messages. The closed
Controlled WASM Deployment phase only ever invoked a single exported scalar
function once, with all host imports rejected
(`DATUM/src/wasm_runtime/generic_runner.rs`, `deployment.rs`). That substrate has
no I/O, no long-lived instance and no notion of a D-Call. A first ABI is needed
that keeps the "no host imports" guarantee while allowing message handling.

## Decision

**ABI version 0 is a core WebAssembly module with no imports.** The D-Node performs
all communication on the D-Serv's behalf (the reference model's *indirect
communication via the D-Node API*). Exports:

| Export | Signature | Meaning |
|---|---|---|
| `memory` | memory | Linear memory |
| `datum_abi_version` | `() -> i32` | Must return `0` |
| `datum_alloc` | `(len: i32) -> i32` | Guest allocates an input buffer |
| `datum_dealloc` | `(ptr: i32, len: i32)` | Frees buffers |
| `datum_handle` | `(ptr: i32, len: i32) -> i64` | Handles one input envelope; returns `(out_ptr << 32) \| out_len`, `0` = no output |

Envelopes are JSON, UTF-8:

- input: `{"abi":"datum-dnode/0","port":"<port>","message_id":"…","correlation_id":"…","received_at_utc":"…","payload_b64":"…"}`
- output: `{"status":"ok"|"rejected","reason":null|"…","emits":[{"port":"<port>","payload_b64":"…"}]}`

Per-invocation host enforcement: fuel budget; epoch-deadline timeout;
`StoreLimits` memory cap; input/output size caps; emit-count cap; emits only to
ports declared in the slice; the D-Node owns the `correlation_id` and never trusts
one echoed by the guest.

A WIT / component-model ABI may become a later ABI **version**; the name/version
token in the D-Serv artifact lets versions coexist.

## Consequences

- The existing import-rejection, SHA-256 and fuel/timeout machinery is reused for
  D-Code (`wasm_runtime/deployment.rs`, `integrity.rs`, `mod.rs`).
- D-Code artifacts (`datum.dserv-artifact/1`) declare `abi = "datum-dnode/0"` and
  `allowed_imports = []`.
- Fixtures reference the ABI string only; there is no runtime in Stage 107.
