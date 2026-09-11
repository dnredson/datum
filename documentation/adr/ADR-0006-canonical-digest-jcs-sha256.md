# ADR-0006 — Canonical digest: RFC 8785 JCS + SHA-256

- Status: Accepted (Stage 107 baseline)
- Decision refs: D-3
- Implementation: Stage 108 (`datum-model`). Not implemented in Stage 107.

## Context

Several v2 contracts are content-addressed and referenced by digest: the D-Graph,
D-Continuum, D-Forward spec, D-Map, the operational slice, and the D-Serv
execution contract. The v1 F2 work (ADR-0011) needs DServer and SmartSentinel to
compute the **same** digest for the same logical content from **different Rust
structs**. A canonical serialization is required.

## Decision

1. **All canonical digests are `SHA-256` over the RFC 8785 JSON Canonicalization
   Scheme (JCS) serialization** of the contract object.
2. Encoding rules carried into every contract:
   - numeric integers in digest-critical contracts must lie in the explicit
     signed safe range `-(2^53 - 1) <= n <= 2^53 - 1`;
   - array order is significant;
   - an absent optional value is encoded as `null`, never omitted;
   - object member order is the JCS lexicographic order, so map insertion order and
     whitespace never affect the digest.
3. Digests are written as `sha256:<64 lowercase hex>`.
4. The same scheme is used for the v1 `ArtifactExecutionProjectionV1` digest
   (ADR-0011), precisely so the two crates can agree independently of their struct
   layouts.

## Consequences

- Stage 108's `datum-model` crate provides one JCS+SHA-256 function and shared
  test vectors asserted by both `dserver` and `datum` crates.
- Stage 107 fixtures cannot carry real digests, because the referenced artifacts
  do not exist yet. They use the sanctioned placeholder
  `sha256:0000000000000000000000000000000000000000000000000000000000000000`
  and the consistency checker verifies cross-references by natural keys
  (`dgraph_id` + `revision`, and so on), not by digest string. The checker also
  flags any *other* real-looking `sha256:` value in a Stage 107 fixture.
