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
authority consumers (D-Code authorization, D-Call route derivation, grant
issuance) and the HTTP surface consume this common accessor directly and are
identical whether the active authority is DMap/1 or DMap/2; this wiring is
complete, not a follow-on item. D-Call route derivation additionally carries
an exact, typed D-Call realization authority (`DCallRealizationAuthorityV1`)
whenever the active authority is DMap/2, sourced exclusively from the
matching `DCallRealizationV2` and the governing DForward's own identity --
fabricating one when absent is a defect, not an acceptable fallback, since a
validated DMap/2 always carries exactly one realization per DGraph call.
Cross-version integrity evaluates both accepted histories and rejects a
missing, stale, or dual active authority.

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

Phase 113B1 adds the DIoT runtime-binding registry (`diot_runtime_binding`,
`DOMAIN_DIOT_RUNTIME_BINDING_REGISTRY`): a server-owned OPERATIONAL record of
which broker URI a governed DIoT placement's realization resolves to,
mirroring the Phase-112 D-Call endpoint registry's structural pattern
(mutable, monotonic revision, JCS+SHA-256 descriptor digest, full structural
re-validation on reload) rather than its HTTP-specific shape. Every identity
field except the broker URI is derived exclusively from the active DMap/2
placement authority; a fail-closed staleness check
(`verify_runtime_binding_still_current`) proves a previously-registered
binding's DMap/DForward/artifact-authority identity is still current before
any consumer may treat it as authoritative, exactly mirroring Phase 112's
`check_source_execution_binding` philosophy. This registry performs NO MQTT
network I/O -- no client, connection, publish, subscribe, or credential
handling exists yet anywhere in DServer.

Phase 113B2 implements the locked design decision the previous paragraph
describes: `DCallDeliveryGrantV1` gains one optional, server-derived field
(`v2_dcall_transport_authority: Option<DCallRealizationTransportAuthorityV1>`)
rather than a second, parallel authorization type -- `None` for every DMap/1
grant, `Some(..)` binding BOTH the exact accepted D-Call realization
(kind/placement/interface/channel/DForward reference) AND the exact governed
DIoT runtime-binding identity (`binding_id`/`binding_revision`/
`descriptor_digest`) current at issuance. `post_request_grant` replaces its
former blanket `dcall_realization_transport_not_implemented` rejection with
governed resolution: it resolves the runtime binding for the realization's
placement, requires it exist (`dcall_diot_runtime_binding_missing`) and
validate against current authority via the Phase-113B1 validator
(`diot_runtime_binding_stale`), then constructs the grant's transport
authority itself -- a destination HTTP endpoint is never consulted, and
`route_kind` (local/remote) never gates this decision, so a same-node
`via_diot_pubsub` call still receives typed authority rather than silently
degrading to direct local delivery. `issue_grant`'s delivery authority is now
a closed three-variant type (no HTTP endpoint, an HTTP endpoint, or DIoT
transport authority) so the impossible combinations -- DMap/1 with DIoT
authority, or DMap/2 `via_diot_pubsub` with HTTP endpoint authority as
delivery authority -- are unrepresentable rather than merely rejected at
runtime. Redemption re-derives and re-proves this ENTIRE authority chain
(current route and realization, current runtime binding and its identity)
before a DMap/2 grant may become `Redeemed`; any staleness leaves it
`Pending`, never silently consumed. On the DATUM side, `dcall::delivery`
gains a mandatory, temporary fail-closed seam: once a grant carrying this
typed authority is obtained, delivery halts immediately with a dedicated
deterministic evidence code (`dcall_diot_transport_not_implemented`) --
before any local D-Code invocation, any HTTP request, or any grant
redemption -- since realization authorization succeeding is not the same as
a runtime transport existing to act on it. Still no MQTT adapter, ingress
listener, egress publisher, retry protocol, or exactly-once claim exists
anywhere in either crate; actual MQTT publish/subscribe transport remains
the next slice (Phase 113B3).
