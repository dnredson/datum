# ADR-0005 — D-Code vs Management WASM vs container/runtime artifacts

- Status: Accepted (Stage 107 baseline)
- Decision refs: D-8; working decision 5, 11

## Context

"WASM" is used for unrelated things in the repository, and the DATUM paper adds a
third meaning. Conflating them makes the governance story and the research claims
imprecise:

- The closed Controlled WASM Deployment phase delivered and ran small **management**
  modules; the operational reconciler embedded in the agent is a two-input decision
  `desired_running AND NOT observed_running`
  (`DATUM/src/agent/operational_reconciliation.rs`). Its `wasm_decision` fields on
  a `ServiceArtifact` are only checked for non-emptiness and safe flags; the
  executed module is always the embedded constant.
- The reference model's **D-Code** is the executable of an application service
  (D-Serv). Nothing in the repository executes application code as a service.

## Decision

Three categories, kept strictly separate:

| Category | What it is | Contract | Runs where |
|---|---|---|---|
| **Application WASM / D-Code** | The executable of a D-Serv, produced by D-Compile | `datum.dserv-artifact/1`: immutable, content-addressed, ABI `datum-dnode/0`, typed ports, limits, provenance. No container fields, no host kind, no lifecycle commands. | Inside a D-Node; a fresh instance per invocation (ADR-0008). |
| **Management WASM** | Local control / reconciliation logic; today the embedded `desired AND NOT observed` reconciler and the artifact `wasm_decision` policy | Legacy v1 only: `ServiceArtifactWasmDecision`, the embedded module. | The v1 executor only. **Absent from the canonical v2 path. Never called D-Code.** |
| **Container / runtime artifacts** | D-IoT middleware, D-IoT support jobs, the D-Node container host, host prerequisites | `ServiceArtifact` + `artifact_class`; `RuntimeInstallArtifact` | Governed container lifecycle / host installation. |

- **D-Code is the executable code of a D-Serv.** **WASM is the prototype target**
  (ABI `datum-dnode/0`); other targets are not precluded long-term.
- **Management WASM is not D-Code** and must not be required for the first
  canonical DATUM vertical slice. It is revisited only if there is a specific
  management-programmability research question.
- The existing WASM module registries
  (`DATUM/src/wasm_runtime/registry.rs`, `dserver/src/core/wasm_module_registry.rs`)
  describe management modules; they are superseded by the D-Serv artifact registry
  and are not overloaded with D-Code.

## Consequences

- The `datum.dserv-artifact/1` contract is a separate registry and domain from
  `ServiceArtifact`, so container-oriented validation and D-Code validation cannot
  entangle.
- Fixtures: `dserv-artifact.decode.json`, `dserv-artifact.alert.json` (D-Code);
  `diot-artifact.mosquitto.json` (container/runtime artifact with `artifact_class`);
  `negative/dserv-artifact.host-kind-field.json` (rejected).
