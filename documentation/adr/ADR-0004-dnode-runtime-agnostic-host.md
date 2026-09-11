# ADR-0004 — Runtime-agnostic D-Node; `DServExecutionContractV1` host invariance

- Status: Accepted (Stage 107 baseline)
- Decision refs: D-2; Round 2 correction 1
- Implementation: Stages 113–114. Not implemented in Stage 107.

## Context

VS1 packages the `datum-dnode` runtime as a governed container, because that
reuses the entire validated container path and keeps the D-Node process free of
Docker authority and DServer credentials. The host choice must not leak into:

- D-Code;
- D-Serv application semantics;
- the canonical application D-Graph;
- `DServExecutionContractV1`.

**D-Map and the derived operational slice MAY explicitly represent the D-Node
hosting / lifecycle choice**: `dnode_runtime_placements[].host` is part of
`datum.dmap/1`, and the derived `dnode_runtime` workload is part of the
operational slice. That is by design, so a native host-process D-Node (for
constrained mist elements) can be added later by changing only those host-facing
parts, without redesigning the host-invariant contracts above.

Round 2 clarified the invariance requirement: the host-invariant property is the
**D-Serv execution contract**, not the whole D-Map or the whole slice. It is
**wrong** to demand that a whole operational slice be byte-identical between a
container-hosted and a native-hosted D-Node, because host lifecycle
representation legitimately differs.

## Decision

1. **The D-Serv execution contract is host-independent.** For each D-Serv instance
   the slice carries a `datum.dserv-execution-contract/1` object
   (`DServExecutionContractV1`) containing exactly:
   - `application_id` and the D-Graph identity (id, revision, digest);
   - `dserv_id`, `instance_id`, `element_id`, `dnode_id`;
   - the D-Code artifact identity (`artifact_id`, version, SHA-256) and the ABI;
   - every port: direction, payload schema, channel, D-Call id or external marker,
     and the D-IoT realization **identity** (placement id + interface) — never a
     network address;
   - limits, state model, instantiation mode.

   Two D-Maps that differ only in `dnode_runtime_placements[].host` must produce
   **identical `DServExecutionContractV1` JCS digests** for every instance.

2. **Host lifecycle representation may differ.** The `dnode_runtime` workload (host
   kind, hosting artifact, supervision, resolved endpoint addresses, its own
   `depends_on`) and everything derived from it (slice digest, slice revision,
   authorization) are allowed to differ between host kinds.

3. **No host-kind field** appears in `datum.dserv-artifact/1`, `datum.dgraph/1`,
   `datum.dmap/1` `dserv_placements`, or `DServExecutionContractV1`. A fixture or
   contract that contains one is invalid.

4. The executor abstraction is a `DNodeHost` lifecycle trait
   (`ensure_runtime` / `runtime_status` / `stop_runtime`) with `ContainerDNodeHost`
   for VS1 and `NativeProcessDNodeHost` later; module delivery (`DNodeDelivery`)
   and status reading (`DNodeStatusClient`) are host-independent. Local policy key
   `allowed_dnode_host_kinds` (narrowing only, see ADR-0011 / N-1).

## Consequences

- A future native D-Node is a `DNodeHost` implementation plus a host-prerequisite
  artifact (`RuntimeInstallArtifact` lineage); no control-plane contract changes.
- Host-invariance is testable now with fixtures:
  `operational-slice.fog-01.container.json` and
  `operational-slice.fog-01.native.json` carry byte-identical `execution_contract`
  blocks and differing `dnode_runtime` workloads. The consistency checker asserts
  both facts. `negative/execution-contract.broker-address.json` shows a rejected
  host-dependent value.
