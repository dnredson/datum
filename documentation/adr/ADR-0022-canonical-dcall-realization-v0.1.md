# ADR-0022 — Canonical D-Call realization v0.1

- Status: **Accepted architecture; implemented, v7 endpoint transport-authority closure, awaiting independent review.**
- Decision refs: ADR-0002, ADR-0003, ADR-0005, ADR-0009, ADR-0017, ADR-0018, ADR-0019, ADR-0020, ADR-0021.
- Conceptual roadmap phase: 112 — real D-Call.

## Context and preserved decision

Phase 111 supplies canonical executable D-Serv endpoints. Phase 112 connects
them through the existing canonical DGraph call edge. `CanonicalDCallV1`
remains `{call_id, source_service_id, target_service_id}`. Operational routing
and transport endpoints never become canonical placement inputs.

`derive_dcall_route` resolves both sides against the currently active accepted
DMap, immutable DGraph snapshot, DNode ABI declarations, and Phase-111 artifact
registry. Source output participation must exist; the destination input port
is the unique input sharing its payload-schema identifier. Payload bytes remain
opaque. Destination node/service/port/URL cannot be supplied as routing authority
in the grant request. Fan-out is sorted by call_id, one edge per attempt.

The separate operational endpoint registry maps node/transport to an HTTP URL
and revision/digest. It validates node registration, URL scheme/userinfo, and
persisted integrity. It does not contaminate canonical DNode/DContinuum state.

## Governed source provenance and authority continuity

Production `dispatch_validated_emit` accepts only `GovernedDCodeInvocationResult`
plus an emit index. That opaque type has no public constructor, deserializer,
or generic-result conversion. Its production producer resolves installation
identity and the trusted sibling worker and executes in the real isolated
worker. It accepts no arbitrary executor, worker path, or authoritative node
override. The non-default `dcall-test-support` producer permits synthetic
executors for tests and is excluded from default builds. Compiler probes check
the production boundary directly; comments must not describe synthetic test
results as production provenance.

The dispatcher derives `SourceExecutionBindingV1` exclusively from the retained
source result's evidence: source service/node, exact DGraph/DMap ids, revisions,
digests, artifact id, module digest, and descriptor digest. Real grant issuance
compares every field with freshly derived current authority BEFORE insertion,
under `DDEPLOY -> DNODE -> DCODE -> DCALL` transaction locks. Mismatch is HTTP
422 `dcall_source_execution_stale`. Source evidence is referenced by its
deterministic bare-hex SHA-256. DServer stores/echoes that opaque reference; the
dispatcher rejects an echoed substitution before redemption or execution.

This proves continuity for the governed trusted installation. The HTTP binding
is caller-declared; DServer has no source record from which to attest remote
execution cryptographically. It does not defend against a hostile actor already
able to replace installed binaries or identity files.

## Grants, execution, and evidence

Grants remain server-issued, 30-second, single-use, and digest/size-bound. Only
SHA-256(token) is persisted. Redemption checks token, expiry, destination,
payload identity, both sides' current DGraph/DMap/artifact authority, and the
remote endpoint identity. The redeemed transition is persisted before execution;
no server lock crosses remote network I/O. Restart does not reset redemption.

Local dispatch and remote reception share the private destination execution
helper. The public governed receive entrypoint resolves local identity/worker,
recomputes payload identity, and redeems before executing. It exposes no arbitrary
worker/node override or grant-only execution bypass. The listener first rejects
base64/digest/size tampering before spending a redemption. The shared helper
fetches fresh Phase-111 authorization/artifact and invokes an isolated worker.

Source and destination raw-payload message limits are conservative preflight
bounds before grant insertion. Exact serialized ADR-0003 envelope limits remain
Phase 111's runtime responsibility. A boundary-accepted raw-payload grant is not
a guarantee the final envelope fits.

Every delivery gets a fresh host message_id. Grant, delivery evidence, and
destination invocation use that same identity. The source's host correlation_id
is unchanged through fan-out and destination execution. Returned destination
evidence is checked against grant identities, graph/map references, artifact id
and both digests, input port/digest/size, message and correlation id, and schema.
Source/destination evidence digest references use deterministic struct JSON
serialization. Persistent evidence contains no raw payload or envelope, and
diagnostics are fixed codes with digest-only references to uncontrolled text.

## Persistence integrity

Reload validates state version, non-empty grant id and matching map key, token
hash, timestamps/status relationships, identifiers, and SHA-256 shapes. Local
means equal source/destination nodes with no endpoint fields. Remote means
different nodes with all endpoint fields present, non-empty endpoint id,
revision >= 1, and valid descriptor digest. Graph/map references require
non-empty ids, revisions >= 1, and canonical digests. Both endpoints require
non-empty service/node/port/artifact ids, bare module digests and canonical
descriptor digests; source evidence references require bare SHA-256. Corrupted
state fails closed with no repair. Fresh replay/reload regression uses the
current grant schema.

## v6 proof and review artifacts

`DATUM/tests/dcall_two_node_integration.rs` runs seven ignored real-process
proofs using DEFAULT production APIs. It provisions the actual Cargo-built
sibling worker and isolated DServer executable/config/storage. It covers remote
and local execution, tamper rejection, actual DGraph and DMap acceptance after
source execution, and both message-limit preflights/boundaries. Independent
binding mismatches use real HTTP issuance and read-only SQLite current/history
checks. An observer records no destination authorization attempt after stale
rejection. The real remote listener evidence is independently digest-compared;
the local suite separately observes full evidence for a fresh real redeemed
local delivery through the same helper and evidence builder.

Current commands/counts are in `MANUAL_VALIDATION_COMMANDS.md`. This pass's
audits and exact successful/failing attempt logs live in `review/phase112-v7/`;
the final ZIP includes MANIFEST.txt and test-results.txt generated from them.
Earlier v1/v2 fixes are preserved in the current architecture: opaque emit
provenance, shared message identity, destination evidence binding, two-sided
redemption recheck, raw-payload preflight, bounded remote response, and persisted
state checks. v6 completes proofs and closes public helper/echo substitution
gaps without replacing the route/grant architecture.

## Limits and exclusions

Bare HTTP assumes the existing trusted/lab network and provides no confidentiality.
Distributed TOCTOU remains between preflight/redemption and guest execution.
Lost responses are ambiguous; there are no automatic retries. Redemption is
at most once, not an exactly-once distributed execution guarantee. One delivery
traverses one edge; destination emits are not recursively dispatched. No
DForward, DIoT, DEngine, GNN/RL/LLM, DScript, DCompile, module distribution,
placement generation, or automatic proposal acceptance is added.

## v7 transport authority and response closure

Grant issuance binds endpoint E (node, transport, endpoint id, revision and
JCS descriptor digest). After the production endpoint fetch, the source checks
node == grant destination, transport == HTTP, id/revision/digest == the grant.
It recomputes `sha256:` plus lowercase SHA-256 of existing DATUM JCS bytes for
exactly `{node_id, transport, endpoint_id, revision, base_url}` and requires
recomputed == fetched == grant digest. Only then is the bearer/payload POST
constructed and sent. Failure is `dcall_endpoint_authority_mismatch`, with a
fixed diagnostic hashed into sanitized evidence and no redemption attempt.

The live-observer regression exercises production `deliver_remote` and the
actual endpoint-fetch client: a legitimate revision-2 descriptor at a changed
URL receives ZERO requests from an old revision-1 grant. All control-plane
requests are recorded (exactly the GET, no redeem). Independent node, transport,
id, revision, claimed digest and URL substitutions also fail before delivery.
The unchanged real seven-test suite supplies matching-endpoint delivery.

DServer still checks CURRENT endpoint == E during redemption. This catches a
registry change AFTER source validation and BEFORE redemption and prevents
execution. It does not make the two checks atomic: authority can change in the
small gap before network arrival, and a previously validated address can then
receive bytes. Bare HTTP also provides no confidentiality against an on-path
attacker. This is not distributed atomicity or cryptographic endpoint attestation.

Registry URLs reject query/fragment components (including empty components)
with `dcall_endpoint_url_query_rejected` and
`dcall_endpoint_url_fragment_rejected`; path prefixes remain supported.
Malformed HTTP-2xx JSON, non-object/missing/non-string/unknown outcome, or invalid
success evidence fails `dcall_remote_response_malformed`. Structured `failed`
remains destination-execution failure. The existing 1-MiB body bound remains.

P2 decision: a small typed grant-request rejection preserves ONLY the three
whitelisted stale-source/source-limit/destination-limit codes in existing
error_code evidence. Arbitrary server code/message/body is never persisted as
text; the client retains only status and a body SHA-256 diagnostic. Unknown
codes retain `dcall_grant_request_failed`. No evidence schema expansion.
