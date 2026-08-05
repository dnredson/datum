# C49 Fail-Closed Artifact Integrity and Provenance Contract

## Status

Candidate increment `121/130`. Implementation is ready for local, full-suite, repository, and live validation. Progress remains `120/130` until that evidence is complete.

## Purpose

C49 formalizes the third execution-safety gate without loading or trusting any artifact. It exposes the absence of verified digests, signatures, and provenance through a deterministic fail-closed contract.

This increment defines a verification boundary; it does not implement artifact inventory, byte loading, hashing, signature verification, provenance resolution, artifact transfer, authorization, or execution.

## Read-Only Endpoint

```text
GET /api/v1/dgraph/artifact-integrity
X-Datum-Project: <project-id>
```

The project header only scopes and labels the response. The endpoint does not read or write Firebase, snapshots, configuration, registries, artifact bytes, local files, network resources, trust stores, signatures, provenance attestations, or executor state.

## Contract Semantics

The candidate response contains:

- `contract_version=0.1.0`;
- `verification_status=not_evaluated`;
- `artifact_inventory_loaded=false`;
- `artifact_count=0`;
- no digest algorithm;
- `artifact_digests_verified=false`;
- `signatures_verified=false`;
- `provenance_verified=false`;
- no trusted provenance source;
- `integrity_verified=false`;
- `authorization_granted=false`;
- `side_effects_performed=false`;
- metadata stating that artifact references are not verification, no bytes were read, no transfer or persistence occurred, and the C46 integrity gate remains unsatisfied.

## Separation From Artifact References

An `artifact_ref` in D-Graph metadata identifies a desired artifact but does not prove content identity, origin, signer trust, immutability, or suitability for execution. C49 does not dereference, download, hash, sign, verify, or execute those references.

A future verifier must bind a canonical artifact digest and trusted provenance evidence to the exact executable bytes. Until then, references remain non-authoritative metadata.

## C46 Relationship

C49 does not satisfy `artifact_integrity_and_provenance`. No inventory, digest, signature, trust root, or provenance attestation is evaluated. C46 therefore remains:

- `decision=denied`;
- `authorization_granted=false`;
- `artifact_integrity_and_provenance.satisfied=false`;
- all eight operations unauthorized;
- evaluation-only and side-effect-free.

Even future verified integrity and provenance would satisfy only one gate and would not independently authorize an operation.

## Safety Boundary

C49 remains metadata-only, read-only, evaluation-only, and dry-run. It does not:

- resolve or fetch artifact references;
- read local artifact bytes;
- compute or compare hashes;
- verify signatures, certificates, trust roots, or attestations;
- transfer, cache, persist, install, load, or execute artifacts;
- inspect or mutate Docker, WASM, files, mappings, or configuration;
- execute placement, recovery, migration, reconfiguration, deployment, installation, dispatch, or WASM;
- invoke health checks or collection cycles;
- add AI or LLM orchestration.

## Pending Validation

Before this increment can advance progress to `121/130`, validate:

- `cargo fmt --check`;
- all focused C49 artifact-integrity tests;
- all focused C46 execution-safety tests;
- the complete dserver test suite;
- `git diff --check origin/main...HEAD`;
- a clean `git status --short`;
- live artifact-integrity and execution-safety assertions.

No later increment is started by this candidate.
