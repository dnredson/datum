# C65 — DATUM Artifact Acquisition and Wasmtime Preflight

- checkpoint: `C65/150`
- target_progress_after_validation: `137/150`
- predecessor: `C64/150`
- phase: Controlled WASM Deployment
- status: implementation_ready_for_validation

## Objective

Consume an explicit C64 node-scoped deployment view inside DATUM, acquire the assigned `.wasm` artifacts, verify their SHA-256 content identity, and compile them with Wasmtime as a non-executing preflight.

C65 deliberately does not instantiate or invoke a WebAssembly module. WebAssembly instantiation may execute a module start function, so the first explicit instantiation/invocation remains a C66 live-validation concern.

## DATUM-side node-scoped contract

`DATUM/src/wasm_runtime/deployment.rs` introduces a DATUM consumer representation compatible with the C64 JSON shape:

- `DatumNodeScopedDeployment`;
- `DatumNodeDeploymentScope::{Full, NodeSlice, Empty}`;
- `DatumDeploymentAssignment`;
- `DatumDeploymentArtifact`;
- `DatumDeploymentRuntimeLimits`;
- `DatumDeploymentExecutionBoundary`.

Unknown C64 fields remain safely ignored by serde, allowing the dserver representation to carry audit metadata that DATUM does not need for artifact preparation.

Before any artifact access, DATUM validates:

- the node-scoped payload is addressed to the local DATUM node;
- every non-empty assignment targets that same local node;
- assignment IDs are unique;
- `content_id == sha256:<digest>`;
- fuel/timeout metadata are positive when present;
- empty views carry no assignments;
- non-empty views carry at least one assignment;
- `explicit_invocation_required=true`;
- `automatic_execution_allowed=false`;
- `governed_execution_authorized=false`.

## Acquisition policy

C65 introduces `ArtifactAcquisitionPolicy`.

Default policy:

```text
allow_local_file=true
allow_https=true
allow_insecure_http=false
max_artifact_bytes=8 MiB
request_timeout_ms=10000
redirects=disabled
```

Supported sources:

- `file://...` — explicit local artifact read;
- `https://...` — bounded network acquisition into memory;
- `http://...` — blocked by default, available only through explicit policy opt-in for trusted lab/LAN validation.

Other URI schemes are rejected.

Remote artifacts must identify a `.wasm` resource. Network responses are size-bounded and redirects are disabled so the configured artifact URI cannot silently move the acquisition to another endpoint.

## In-memory delivery semantics

Network artifacts are not written to disk by C65. The acquired bytes remain in `AcquiredWasmArtifact` memory.

Acquisition evidence distinguishes host delivery activity from guest execution:

```text
artifact_delivery_performed
host_filesystem_read_performed
host_network_fetch_performed
persistent_write_performed=false
host_mutation_performed=false
```

For `file://`, `artifact_delivery_performed=false` and `host_filesystem_read_performed=true`.

For HTTPS or explicitly enabled lab HTTP, `artifact_delivery_performed=true` and `host_network_fetch_performed=true`, while persistent write and host mutation remain false.

These host-side acquisition facts must not be confused with WebAssembly guest side effects.

## Integrity and Wasmtime preflight

After acquisition, C65 computes SHA-256 directly over the exact acquired bytes and requires it to match the assignment digest and content identity.

Only after integrity succeeds does C65 build a Wasmtime module.

Preflight then requires:

- valid WebAssembly compilation;
- zero guest host imports;
- the explicitly assigned export exists;
- the assigned export is a function.

The resulting evidence records:

```text
integrity_verified=true
wasmtime_compiled=true
wasmtime_instantiated=false
guest_execution_performed=false
wasm_guest_side_effects=false
governed_execution_performed=false
```

The assignment timeout remains metadata in C65; no claim is made that runtime timeout enforcement exists yet.

## Why C65 compiles but does not instantiate

`Instance::new` is intentionally deferred. A valid WebAssembly module may contain a start function that runs as part of instantiation. Compiling and inspecting the module provides a strong readiness boundary without introducing implicit guest execution.

C66 can therefore make instantiation an explicit operator-visible live action and retain a clean audit boundary between artifact preparation and guest execution.

## Tests added

C65 adds 12 DATUM unit tests covering:

1. local file acquisition, integrity verification, and compile-only preflight;
2. wrong DATUM recipient rejected before artifact access;
3. cross-node assignment leakage rejected;
4. explicit empty scope treated as a valid no-op;
5. SHA-256 mismatch rejected before Wasmtime compilation;
6. guest host imports rejected during preflight;
7. missing assigned export rejected;
8. insecure HTTP blocked by default;
9. explicitly enabled loopback/LAN HTTP fetched into memory without persistent write;
10. artifact size limit enforced;
11. automatic execution boundary cannot be enabled;
12. C64 dserver JSON shape deserializes with extra audit fields.

The validated pre-C65 DATUM baseline is `389 passed; 0 failed`. With 12 new tests, the expected C65 total is `401 passed; 0 failed`.

## Validation required before checkpoint advancement

Run on the C65 branch:

```bash
cargo fmt --manifest-path DATUM/Cargo.toml -- --check
cargo test --manifest-path DATUM/Cargo.toml
cargo build --manifest-path DATUM/Cargo.toml
```

Do not advance to `137/150` and do not merge until these checks pass.

## Next intended increment

C66 performs the first live single-node path using a real node-scoped assignment and acquired verified artifact. Instantiation and invocation must remain explicit, bounded, reportable, and separate from canonical governed `wasm_execution`.
