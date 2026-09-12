# ADR-0013 — OCI catalog image identity resolution (Stage 107B-A)

- Status: **Accepted and implemented (Stage 107B-A), v3.**
- Decision refs: ADR-0006 (canonical digest), ADR-0011 (F2 execution-content
  authorization), ADR-0012 (TOCTOU, Option A).
- Scope: **only** registry-backed image identity resolution/pinning, run
  explicitly before authorization. Agent-side post-pull verification
  (Stage 107B-B), configuration-content materialization (Stage 107B-C), and
  full required-mode E2E across the whole gap set (Stage 107B-D) are separate,
  later slices — not decided or implemented by this ADR.
- Revision history: v1 implemented the base design; v2 fixed 6 independent-
  review P0 findings (server-owned write path, CAS pinning, fully-qualified
  repository, verified platform via config blob, `legacy_unclassified`
  migration default, `source_kind` in the projection) and 5 P1 findings; v3
  fixed a further 2 P0 findings (all 22 catalog artifacts must explicitly
  classify `image_source_kind`; the pin endpoint is not actually
  authorization-gated) and 6 more P1/P2 findings. This document reflects v3
  behavior throughout — see "Superseded v1/v2 statements" at the end for what
  changed and why.

## Context

Stage 107A's `ArtifactExecutionProjectionV1.image_identity.declared_digest`
existed only as an interface Stage 107B would complete; `required_completeness_gaps`
unconditionally reported every image as unpinned. A design audit of the
`agent/stage107b-catalog-pinning` tree found that **zero of the 22 real
catalog artifacts could satisfy `required`, including the one already
digest-qualified image** (`chirpstack-gateway-bridge`), because the gap check
inspected only string non-emptiness, never actual pin validity.

## Decision

### 1. Digest-domain separation (extends ADR-0006)

An OCI content digest (over raw registry manifest/config bytes) and a
canonical DATUM contract digest (over RFC 8785 JCS-canonicalized JSON) are
different digest domains, even though both happen to share the
`sha256:<64 lowercase hex>` string shape. `oci_registry_resolver::is_oci_digest_format`
is a **separate** validator from `jcs::is_canonical_digest_format` — never
reused across the two domains. Raw exact-byte content hashes
(`ConfigurationMountV1.declared_content_sha256`) remain their own third,
unprefixed domain, unaffected by this ADR.

### 2. Schema must version (per-artifact axis) — and every real artifact must
    explicitly classify `image_source_kind`

`SERVICE_ARTIFACT_SCHEMA_VERSION` bumps `"0.1.0"` → `"0.2.0"`. This is
distinct from `SERVICE_ARTIFACT_REGISTRY_STATE_SCHEMA_VERSION` (the envelope
version, already `"0.2.0"` since Stage 107A and unaffected here, since no
envelope-level field changed).

Adding `ContainerDeploymentSpec.resolved_image_identity` (and, in v2,
`image_source_kind`; in v2's follow-up, `ImageIdentityV1.source_kind` in the
projection) as new sibling keys changes the canonical JCS bytes — and
therefore `execution_projection_digest` — of every existing artifact,
regardless of a field's own null-when-absent reservation: object-key
presence perturbs JCS serialization independent of value. This is a genuine,
one-time, schema-caused content change under D-7a, not a cosmetic one, so
`validate_artifact`'s strict per-artifact `schema_version` equality check is
what makes a pre-Stage-107B-A binary refuse a migrated artifact rather than
silently re-serializing it without the new fields — the same
downgrade-safety mechanism ADR-0011 §3 already established at the envelope
level.

`migrate_legacy_artifact_schema_versions` runs as a second, independent
migration stage in `ServiceArtifactRegistry::load_from_path`, triggered
whenever any artifact's own `schema_version` is legacy — regardless of
envelope status. It recomputes the projection digest and bumps
`execution_generation` by exactly one via the existing `checked_next_generation`
high-water mechanism, validates the candidate, and persists immediately.

**`image_source_kind`'s serde default is `"legacy_unclassified"`, not
`"registry"`** (v2 correction — see §3). This means the migration path never
silently infers trust for old data missing the field. It does **not**,
however, retroactively fix real catalog data that is missing the field going
forward: **every one of the 22 real, checked-in `artifacts/catalog/*.json`
files must explicitly set `image_source_kind`** (`"registry"` × 19,
`"local_build"` × 3). A v2 bundle incorrectly implied this catalog change had
already been made when only the 3 `local_build` files had it; the 19
registry-backed files were still silently deserializing as
`legacy_unclassified` (and therefore permanently unpinnable) until the v3
fix. `catalog_inventory_classifies_every_real_artifact_explicitly`
(`dserver/src/core/service_artifact_registry.rs`) reads the real, checked-in
files directly from disk and asserts this exactly — do not rely on a Rust
struct literal standing in for the real files.

### 3. LocalBuild and LegacyUnclassified are never automatically trusted

`ContainerDeploymentSpec.image_source_kind ∈ {"registry", "local_build",
"legacy_unclassified"}` (v2 adds the third value).

**`local_build`**: three real catalog images have no upstream registry at
all (`datum/sma:dev`, `local/datum-lora-device-simulator:stage78`,
`local/datum-lora-packet-forwarder:stage78`). `pin_image_identity` refuses to
resolve a `local_build` artifact (`ImagePinningError::LocalBuildNotResolvable`)
— a local Docker daemon's `image ID`/`RepoDigests` is observed local state,
not an authoritative catalog pin, since DServer did not govern or attest
whatever build produced it. `required_completeness_gaps` reports a distinct
`unpinned_image_local_build:` gap for these, permanently, in this slice.

**`legacy_unclassified`** (v2): the safe default for ANY artifact missing the
field entirely — a genuinely old (pre-Stage-107B-A) record, or a careless
new-schema write. Never inferred from the image name in either direction (a
registry-shaped name and a local-build-shaped name migrate identically).
`pin_image_identity` refuses it (`ImagePinningError::LegacyUnclassifiedNotResolvable`);
`required_completeness_gaps` reports a distinct
`unpinned_image_legacy_unclassified:` gap, permanently, until an operator
explicitly reclassifies the artifact via an ordinary PUT.

A future governed-build-attestation path for `local_build` (build →
immutable output identity → attestation/register → catalog pin →
authorization) is a plausible later design, explicitly **not** invented here.

### 4. Multi-platform binding rule, canonical ordering, and the trust boundary
    for resolution itself

`ResolvedImageIdentityV1` distinguishes: the original mutable reference; the
resolved, **fully-qualified** repository (v2 correction — see below); an
optional `index_digest` (present only when the reference resolved through an
OCI index / Docker manifest list); and, per requested platform, a mandatory
`manifest_digest` (the platform-specific manifest actually selected,
independently VERIFIED against the image config blob — v2 correction, see
below), `config_digest`, and `execution_reference =
"{repository}@{manifest_digest}"`.

**Binding always happens on the platform-specific `manifest_digest`, never on
`index_digest` alone.** An index digest changing does not imply a given
node's platform-specific content changed, and vice versa.
`oci_registry_resolver::resolve_image_identity` enforces this: it requires
EXACTLY ONE matching descriptor for each requested platform (zero →
`PlatformNotFoundInIndex`; more than one → `AmbiguousPlatformMatchInIndex`,
v2 correction — a naive `.find()` previously accepted the first of several
matches silently) against the index's declared platforms, and verifies the
matched descriptor's own `mediaType` is actually a manifest before fetching
it as one. A non-index (single) manifest requires exactly one requested
platform, since it carries no per-platform disambiguation of its own — and,
since a bare manifest proves nothing about platform on its own, the
CALLER-provided platform is never trusted without independently fetching and
verifying the image CONFIG BLOB's own declared `os`/`architecture`/`variant`
(v2 correction: previously the requested platform was copied through
unverified). `variant` is optional in OCI image configs: on the index path,
an absent config-blob `variant` is accepted once the index descriptor has
already confirmed it exactly; on the single-manifest path, where no index
exists at all, the config blob is the ONLY authority and an absent variant
when one was requested is a fail-closed mismatch (v3 addition).

**`ResolvedImageIdentityV1.repository` and every `execution_reference` are
fully-qualified** (`docker.io/library/eclipse-mosquitto`,
`ghcr.io/absmach/atom`) — v2 correction. `ParsedImageReference` separates the
actual API host (`registry_host`, e.g. `registry-1.docker.io`) from the
canonical reference-form registry (`canonical_registry`, e.g. `docker.io`)
from the API repository path (`repository_path`); `fully_qualified_repository()`
= `"{canonical_registry}/{repository_path}"` is what every persisted/projected
identity is built from. An unqualified `absmach/atom@sha256:...` is a
materially different, ambiguous identity from `ghcr.io/absmach/atom@sha256:...`.

**`platforms` is canonicalized by sorting on `(os, architecture, variant)`**
before persistence (v3 correction): it is semantically a SET of concrete
platform bindings, not an ordered sequence, so requesting the same platforms
in a different order must not change the JCS bytes / `execution_generation`.

**Digest-selector consistency** (v3 addition, originally required by the v1
review but missed): when the declared image reference itself carries a
digest selector (`repo@sha256:...`),
`oci_registry_resolver::validate_resolved_image_identity` requires that
digest to match the resolved ROOT content — the `index_digest` if the root
resolved as an index, or the single resolved platform's `manifest_digest` if
it resolved as a bare manifest. The agent independently mirrors a
lighter-weight version of this check (`stage107a_enforcement::declared_digest_selector`),
recognizing only the exact `@sha256:<64hex>` suffix shape, since DATUM has no
full OCI reference parser.

**Trust boundary for resolution (v3 correction — see §6).** Resolution
happens via an explicit action
(`POST /api/v1/service-artifacts/:artifact_id/image-identity/resolve`) —
**never implicitly during M1/reconciliation**. `pin_image_identity` follows
resolve → validate → CAS-apply (v2/v3): it captures the artifact's
generation/digest/image/image_source_kind BEFORE the (slow, unlocked)
network resolution, independently re-validates the FULLY resolved identity
via `validate_resolved_image_identity` (repository agreement, digest
validity, platform-set consistency, digest-selector consistency), and only
then calls `ServiceArtifactRegistry::apply_resolved_image_identity` — a
`pub(crate)`-only, internal method, never `ServiceArtifactRegistry::update`
— which re-reads the CURRENT artifact inside one write-lock critical section
and compares it against the captured expected state, refusing to persist
(`Conflict("artifact_changed_during_image_resolution")`) if anything changed
while resolution was in flight, and defensively re-validates the candidate
one more time immediately before mutating. A failed resolution, a failed
validation, or a CAS conflict all leave the registry completely unchanged.

Administrative/provenance fields on `ResolvedImageIdentityV1`
(`resolved_at_utc`, `resolver_tool_version`, `resolved_by`, `registry_host`)
are deliberately **not** mirrored into the projection
(`ImageIdentityV1.resolved`) — **nor is `index_digest`** (v3 correction, see
§5). The projection carries only `repository` and the canonicalized,
verified `platforms[]`. Re-resolving identical registry content at a
different wall-clock time, or an unrelated change elsewhere in the same
index, therefore does not perturb `execution_generation`.

Both Docker Hub and `ghcr.io` require the anonymous OCI Distribution v2
bearer-token challenge flow even for public images; `oci_registry_resolver`
implements it. The transport (`OciHttpTransport`) is deliberately
synchronous, mirroring `DATUM/src/dispatch/container_executor.rs`'s
`ContainerRuntime`/`FakeRuntime` trait+fake pattern. `ReqwestOciHttpTransport`
uses `reqwest::blocking::Client` (the existing `reqwest` dependency's
`"blocking"` Cargo feature, no new crate) with finite connect/request
timeouts, and reads every response body through a BOUNDED reader
(`read_bounded_body`, v3 correction — see §6) that never accumulates more
than `MAX_RESPONSE_BODY_BYTES + 1` bytes regardless of any (possibly absent
or dishonest) `Content-Length` header. No test in `oci_registry_resolver`/
`oci_image_pinning` contacts a live registry — all use a queued,
deterministic `FakeOciHttpTransport`.

### 5. `required_completeness_gaps` — image portion only, and VALIDATES, not
    merely checks presence

Replaces the unconditional `unpinned_image_reference:` emission with:
`local_build` source → `unpinned_image_local_build:{image}`;
`legacy_unclassified` source (v2) → `unpinned_image_legacy_unclassified:{image}`;
`registry` source with no `resolved_image_identity` → unchanged
`unpinned_image_reference:{image}` message; `registry` source with a
`resolved_image_identity` that **independently validates**
(`oci_registry_resolver::validate_resolved_image_identity` on DServer;
`stage107a_enforcement::structurally_valid_resolved_image_identity` on the
agent, a narrower mirror without a reference parser) → no image gap. A
`resolved_image_identity` that fails this validation is treated as still
unpinned (v2 correction — the v1 version only checked `Option::is_some()`).
Configuration-mount and secret-ref gap logic are byte-for-byte unchanged —
those remain Stage 107B-C/deferred territory respectively.

### 6. This is NOT an authorized administrative endpoint — the allow-list is
    the real safety boundary (v3, corrects v1/v2)

Earlier revisions of this ADR and the code's own doc comments incorrectly
described `POST /image-identity/resolve` as a "separately authorized
administrative action." **That is not true.** DServer has no
application-layer authentication/authorization middleware on this route, or
on the ordinary `service-artifacts` POST/PUT routes, and binds `0.0.0.0` by
default. `X-Datum-Project` is request scoping, not authorization. Do not
repeat the "authorized" claim anywhere in this codebase until a real
authentication/authorization layer exists.

Given that, the actual safety boundary preventing this feature from being an
arbitrary outbound-HTTPS-request primitive for anyone who can reach DServer
is an explicit, hardcoded, default-deny allow-list, enforced BEFORE the
first outbound request:

- `oci_registry_resolver::ALLOWED_REGISTRY_HOSTS = ["registry-1.docker.io", "ghcr.io"]`
  — checked against `parsed.registry_host` in `resolve_image_identity`
  immediately after parsing the reference.
- `oci_registry_resolver::ALLOWED_TOKEN_REALM_HOSTS = ["auth.docker.io", "ghcr.io"]`
  — checked against the bearer-challenge `realm`'s host in
  `fetch_anonymous_bearer_token`, IN ADDITION TO (not instead of) the
  existing `https://`-only requirement: HTTPS alone does not make an
  arbitrary target safe, since an attacker-controlled host can terminate TLS
  perfectly well.

This is deliberately a small, hardcoded list matching the real catalog's two
registries exactly, not a general configuration subsystem. Extend it only
when a real catalog registry is added, and never treat it as a substitute
for real authentication/authorization. Target-platform requests are also
validated (`validate_target_platforms`: non-empty fields, no duplicates, a
sane maximum count) at both the API boundary and inside the resolver itself,
before any network call.

**Redirects (v4 correction).** `ALLOWED_REGISTRY_HOSTS` /
`ALLOWED_TOKEN_REALM_HOSTS` bound the FIRST-HOP host of a request only, not
every host a request's response can ultimately come from: real registries
(both Docker Hub and GHCR) routinely 307-redirect a blob GET to
provider-controlled CDN/object-storage infrastructure on a completely
different host, and `reqwest`'s default redirect policy would otherwise
follow such a redirect (or any other) to any host, over any scheme, up to
its own default hop limit — stronger than this ADR's documented guarantee
actually held. `oci_registry_resolver::evaluate_oci_redirect` is the actual,
explicit, unit-tested redirect policy, wired into `ReqwestOciHttpTransport`
via `reqwest::redirect::Policy::custom`: a redirect issued by an
already-allowed registry/token host is followed to any `https://` target
with an extractable host, up to `MAX_OCI_REDIRECTS` (5) hops. The redirect
TARGET itself is deliberately **not** re-checked against either allow-list
— it is provider-controlled (e.g. a CDN), not catalog-controlled, and
requiring allow-list membership there would break real resolution against
both of this resolver's allowed registries. What the policy does enforce,
regardless of which allowed provider issued the redirect: HTTPS-only (which
also rules out an HTTPS → HTTP downgrade by construction) and a bounded hop
count.

## Consequences

- Registry-backed catalog images can now become genuinely `required`-eligible
  (image portion only) once explicitly pinned; `local_build` and
  `legacy_unclassified` images, and images with configuration mounts or
  secret refs, remain gated by their respective, still-open gap categories.
- No registry version bump was needed for the envelope
  (`SERVICE_ARTIFACT_REGISTRY_STATE_SCHEMA_VERSION` stays `"0.2.0"`); all new
  fields are additive `Option<T>`/defaulted, so a registry with every new
  field absent still round-trips as unpinned/unclassified.
- Stage 107B is **not** complete after this slice.

## Superseded v1/v2 statements (do not rely on these)

- ~~`image_source_kind` only `registry`/`local_build`~~ — now also
  `legacy_unclassified` (§3).
- ~~`pin_image_identity` persists through ordinary `ServiceArtifactRegistry::update()`~~
  — now the internal, CAS-checked `apply_resolved_image_identity` (§4).
  `update()` is a public-API-only, non-CAS path and is no longer used by the
  pin operation at all.
- ~~`resolved_image_identity` present ⇒ no image gap~~ — now must also
  independently validate (§5).
- ~~"An explicit, separately-authorized administrative action"~~ — corrected
  to an explicit action with NO real authorization, whose actual safety
  boundary is the host allow-list (§6).
- ~~`ResolvedImageIdentityV1.repository` / `execution_reference` use the bare
  API repository path~~ — now fully-qualified (§4).
- ~~The caller-requested platform is trusted for a single (non-index)
  manifest~~ — now independently verified against the image config blob
  (§4).
- ~~`resolved.index_digest` is part of `ArtifactExecutionProjectionV1`~~ —
  now excluded; it remains in the persisted `ResolvedImageIdentityV1` as
  provenance only (§4).
- ~~`platforms` preserves the caller's requested order~~ — now canonically
  sorted before persistence (§4).
- ~~All 22 catalog artifacts explicitly declare `image_source_kind`~~ — true
  only as of v3; v1/v2 left the 19 registry-backed files relying on the
  (then newly-unsafe) serde default (§2).
