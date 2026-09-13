# Architecture Decision Records

This directory records the decisions that were accepted and **frozen as the Stage 107
baseline** by the joint architecture review (rounds 1–3, 2026-09-10). The full
rationale, the spec-to-implementation gap analysis and the VS1 vertical-slice design
are in `docs/STAGE107_DATUM_SPEC_GAP_REPORT_AND_VERTICAL_SLICE_PLAN.md`. These ADRs
are the short, citable form of that baseline.

Stage 107 is a **specification / contracts / vocabulary** stage. No ADR here changes
DServer or SmartSentinel runtime behaviour, no Rust type is renamed, and no API
changes. Implementation belongs to later stages (108 onward), and the F2 hardening
work belongs to Stages 107A and 107B.

## Status legend

| Status | Meaning |
|---|---|
| Accepted | Decided and frozen as the Stage 107 baseline. Changing it needs a new explicit review decision. |
| Deferred | Recorded as an open question that must be decided before a named later stage. No option is chosen. A Deferred ADR may later become **Accepted** in the stage that resolves it, once the selected decision and its test obligations are recorded. Any constraints the Deferred ADR imposed can be **option-dependent** after resolution — see ADR-0012, where a "no immediate revocation claim" rule survives selection of Option A and survives selection of Option B until Option B's implementation/test evidence is recorded. |

## Index

| ADR | Title | Status | Decision refs |
|---|---|---|---|
| [ADR-0001](ADR-0001-canonical-datum-vocabulary.md) | Canonical DATUM vocabulary and legacy-structure labels | Accepted | D-4, working decisions 1–12 |
| [ADR-0002](ADR-0002-dmap-single-placement-carrier.md) | D-Map is the single authoritative placement carrier | Accepted | F6, D-Map acceptance |
| [ADR-0003](ADR-0003-dnode-abi-v0.md) | D-Node ABI `datum-dnode/0` | Accepted | D-1 |
| [ADR-0004](ADR-0004-dnode-runtime-agnostic-host.md) | Runtime-agnostic D-Node; `DServExecutionContractV1` host invariance | Accepted | D-2, correction 1 |
| [ADR-0005](ADR-0005-dcode-and-management-wasm.md) | D-Code vs Management WASM vs container/runtime artifacts | Accepted | D-8 |
| [ADR-0006](ADR-0006-canonical-digest-jcs-sha256.md) | Canonical digest: RFC 8785 JCS + SHA-256 | Accepted | D-3 |
| [ADR-0007](ADR-0007-datum-model-crate.md) | New `datum-model` crate; no legacy Rust renames | Accepted | D-6 |
| [ADR-0008](ADR-0008-fresh-instance-per-invocation.md) | Fresh D-Serv WASM instance per invocation (ABI v0) | Accepted | D-9 |
| [ADR-0009](ADR-0009-application-identity.md) | `application_id` distinct from `project_id` | Accepted | Identity |
| [ADR-0010](ADR-0010-cross-node-readiness-fresh-evidence.md) | Cross-node readiness from fresh D-Monitor evidence only | Accepted | Readiness |
| [ADR-0011](ADR-0011-f2-execution-content-authorization.md) | F2: execution-content-bound authorization (Stages 107A/107B) | Accepted (specified, not implemented) | D-7, correction 2, D-7a, N-1, N-2, N-3 |
| [ADR-0012](ADR-0012-toctou-authorization-validity.md) | Authorization validity during execution (TOCTOU) | **Deferred** | TD-1 |
| [ADR-0013](ADR-0013-oci-catalog-image-identity-resolution.md) | OCI catalog image identity resolution (Stage 107B-A) | Accepted and implemented | ADR-0006, ADR-0011, ADR-0012 |
| [ADR-0014](ADR-0014-runtime-image-identity-verification.md) | Runtime image identity verification (Stage 107B-B) | Accepted and implemented | ADR-0011, ADR-0012, ADR-0013 |
| [ADR-0015](ADR-0015-runtime-kind-dispatch-seam.md) | Runtime-kind dispatch seam (Stage 108) | Accepted and implemented | ADR-0002, ADR-0006, ADR-0011 |
| [ADR-0016](ADR-0016-native-process-runtime-v0.1.md) | Native process runtime v0.1 (Stage 109) | Accepted and implemented | ADR-0015, ADR-0006, ADR-0011, ADR-0012 |

## Relationship to closed phases

The Controlled WASM Deployment phase (C59–I44) is closed
(`docs/CONTROLLED_WASM_DEPLOYMENT_CHECKPOINT.md`). Nothing here reopens it or
rewrites its claims. Where these ADRs mention structures from that phase (for
example the composite `DGraph`), they label them by their real role and leave the
code and its evidence untouched.
