# ADR-0023 — Canonical DForward/DIoT and DMap v2 control-plane foundation

- Status: Implemented Phase 113A foundation; runtime transport deferred.
- Scope: canonical logical middleware composition, DIoT realization placement,
  DMap/2 validation, and deterministic derived projection inputs.

`datum.dforward/1` is the application/platform-semantic description of an
IoT-middleware composition. It contains logical DIoT identities, artifact
requirements, logical interfaces, scoped requirements, external logical
endpoints, ordered logical hops, and support-job semantics. It contains no
resolved address, host kind, container identity, PID, health, telemetry, or
runtime readiness.

The former Stage-107 sketch is relabelled
`datum.x-fixture.vs1-dforward/1`. It remains useful historical specification
material and is checked independently. It cannot claim the production schema.

The compiled `CanonicalDForwardV1` is mirrored independently in DServer and
SmartSentinel, as required by ADR-0007. Its digest is RFC 8785 JCS plus
SHA-256, matching ADR-0006 and the other canonical contracts. Validation is
fail-closed for identity, uniqueness, interface compatibility, endpoint and
hop resolution, and requirement scope. Cycles in chains remain legal;
communication topology is not deployment ordering.

`datum.dmap/2` is a new wire contract. `datum.dmap/1` remains unchanged and
fully supported. DMap/2 adds exact DForward references, DIoT placements with
immutable artifact bindings, DCall realization identities, and external logical
port bindings. It remains the one authoritative carrier for both DServ and
DIoT placement. It never carries resolved broker addresses, host kinds,
RuntimeKind, process state, health, telemetry, or DMonitor evidence.

The DDeploy/2 state type defines a version-aware active-authority view and its
acceptance transition preserves logical DMap identity, increments revision
across schema versions, and never down-converts a DMap/2 digest. These
invariants are implemented in the new state module and persisted additively
inside the existing `DDeployStateV1` / `DOMAIN_DDEPLOY_STATE` blob. The
common accessor rejects simultaneous v1/v2 active authorities. Phase-112
authority consumers and the HTTP surface remain v1-only; wiring v2 into those
callers is a follow-on closure item. Cross-version integrity evaluates both
accepted histories and rejects a missing, stale, or dual active authority.

DIoT artifact requirements are logical. The v2 record carries binding fields
(the pre-existing, canonical-`datum.dmap/2` `DIoTArtifactBindingV2`, still
caller-declared and format-checked only -- unchanged by this closure pass).
Closure pass 3 adds a SEPARATE, purely operational authority binding
(`DIoTArtifactAuthorityBindingV2`, restricted to `artifact_id` +
`execution_generation` + `execution_projection_digest`) that IS now wired:
`resolve_diot_artifact_authority` resolves every DIoT's artifact requirement
against a stable `ServiceArtifactRegistry` snapshot at proposal-creation time,
the result is embedded directly in `DDeployProposalV2` (making it load-bearing
in `proposal_digest`), and acceptance re-resolves against the CURRENT registry
and requires exact equality, rejecting `ddeploy_v2_artifact_authority_stale`
before any mutation if the artifact changed. This proves the requirement
resolves to one specific registry-computed identity at each point in time; it
does NOT prove exact configuration-content pinning or secret-version
identity, since the registry itself does not yet provide those as verifiable
authority. Mutable version or tag strings are not authority. Existing
configuration-content and secret-version limitations remain open.

Phase-112 DCall routing, authorization, grant issuance, redemption, replay
protection, endpoint authority, and stale checks remain authoritative. DMap/2
only supplies an additional validated realization; it cannot select a
different service or node. No MQTT adapter, ingress listener, egress publisher,
retry protocol, recursive dispatch, or exactly-once claim is part of this ADR.

`same_element` requirements can be structurally validated and projected in
Phase 113A. `any_element` requirements may be represented but are not ready
from static configuration. Any execution path requiring them remains pending
until fresh DMonitor evidence exists, per ADR-0010.

The operational realization projection helper validates supplied DMap/2 and
exact DForward references before deriving a node-local view. It may repeat placement, artifact bindings,
logical dependencies, realization identities, and external logical bindings;
it may not originate placement or claim runtime health. Real middleware
transport is Phase 113B.
