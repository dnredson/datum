# ADR-0007 — New `datum-model` crate; no legacy Rust renames

- Status: Accepted (Stage 107 baseline)
- Decision refs: D-6
- Implementation: Stage 108. Not implemented in Stage 107.

## Context

The canonical v2 contracts (`datum.dgraph/1`, `datum.dcontinuum/1`,
`datum.dforward/1`, `datum.dmap/1`, `datum.operational-slice/2`,
`datum.dserv-artifact/1`, `datum.dserv-execution-contract/1`) need typed Rust
representations, validators and a JCS+SHA-256 digest. The temptation is to grow or
rename the existing structs (`DGraph`, `OperationalDGraph`, …). Those structs are
exercised by validated contracts, fixtures, scripts and closed-phase evidence, and
renaming them before a projection path exists would break reproducibility without
adding capability.

## Decision

1. Stage 108 creates a **new `datum-model` crate** (path dependency) that holds the
   v2 contract types, their validators (including invariants 10–12 of the Stage 107
   report: host invariance, topology ≠ ordering, local policy narrows only) and the
   JCS+SHA-256 digest. **Stage 108** also generates JSON Schemas from those types.
   Stage 107 produces no JSON Schema files: it provides hand-authored *contract
   fixtures* for the six canonical v2 contract shapes plus a stdlib checker.
2. **No existing Rust type is migrated or renamed in this phase.** The legacy
   structures keep their names and behaviour and are relabelled in documentation
   only (ADR-0001).
3. Validator *ideas* from the closed phase (unique-id checks, missing-reference
   checks) may be reused as code in `datum-model`; the closed-phase structs are not.
4. A later migration stage may rename Rust types, but only after the v2 → v1
   compatibility projection has been shown to be byte-equivalent for the existing
   `monitor-f49c8` project.

## Consequences

- Stage 107 hand-authors fixtures and a stdlib checker; Stage 108 turns those
  fixtures into typed tests and generates schemas that must validate them.
- The two code paths (legacy v1 container path, canonical v2 path) coexist behind a
  feature flag from Stage 114; neither breaks the other.
