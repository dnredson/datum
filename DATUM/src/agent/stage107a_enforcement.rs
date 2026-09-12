//! Stage 107A enforcement primitives (ADR-0011, ADR-0012).
//!
//! Pure, unit-tested decision logic factored out of
//! `operational_reconciliation.rs` so the safety-critical checks -- lease
//! validity at each governed mutation boundary (M1-M8), the pinning-mode
//! floor, and node-local policy no-substitution -- can be tested without a
//! Docker daemon (this crate's existing test suite already follows that
//! pattern: pure decision functions are unit-tested, `Command::new("docker")`
//! call sites are exercised only in a live environment).
//!
//! Stable block-reason codes (Section I / evidence): every rejection path in
//! this module returns a `String` prefixed with one of the constants below,
//! so a caller (or a human reading evidence) can match on the code without
//! parsing prose.

use chrono::{DateTime, Duration, Utc};
use sha2::{Digest, Sha256};

use crate::agent::operational_reconciliation::{
    OperationalPortMapping, OperationalReconciliationAuthorization,
};
use crate::config::{DispatchContainerMapping, DispatchContainerPortMapping};

// ---------------------------------------------------------------------- //
// Stable block-reason codes (Section I).
// ---------------------------------------------------------------------- //
pub const BLOCKED_AUTHORIZATION_NOT_LEASED: &str = "blocked_authorization_not_leased";
pub const BLOCKED_LEASE_EXPIRED: &str = "blocked_lease_expired";
pub const BLOCKED_GENERATION_MISMATCH: &str = "blocked_generation_mismatch";
pub const BLOCKED_PROJECTION_MISMATCH: &str = "blocked_projection_mismatch";
pub const BLOCKED_CONFIG_CONTENT_MISMATCH: &str = "blocked_config_content_mismatch";
pub const BLOCKED_LOCAL_POLICY_SUBSTITUTION: &str = "blocked_local_policy_substitution";
pub const BLOCKED_PINNING_MODE_INSUFFICIENT: &str = "blocked_pinning_mode_insufficient";
pub const BLOCKED_MISSING_CLOCK_SKEW_CONFIG: &str = "blocked_missing_clock_skew_config";
pub const BLOCKED_REQUIRED_INCOMPLETE: &str = "blocked_required_incomplete";
pub const BLOCKED_WRONG_RESOURCE_IDENTITY: &str = "blocked_wrong_resource_identity";
/// Stage 107B-B (ADR-0014): a trustworthy pin (`resolved_image_identity`)
/// exists, but runtime verification against the actual local Docker state
/// could not be completed with confidence -- no matching/unambiguous local
/// daemon platform, the Docker daemon could not be queried for identity
/// purposes (distinct from `docker_unavailable`, which covers the daemon
/// being entirely unreachable for ordinary container observation), or a
/// pull reported success but the image still could not be found afterward.
/// This is "we don't know," never "we know, and it's wrong" -- see
/// `BLOCKED_IMAGE_IDENTITY_MISMATCH` for that. In `required` mode this always
/// blocks; in `legacy_partial` it may continue (existing N-2 semantics
/// permitting), but must record `verified=false` evidence -- never silent.
pub const BLOCKED_IMAGE_IDENTITY_UNVERIFIED: &str = "blocked_image_identity_unverified";
/// Stage 107B-B (ADR-0014): a trustworthy pin exists AND runtime evidence was
/// successfully gathered, but it actively CONTRADICTS the authorized identity
/// (wrong manifest, wrong config/image ID, wrong platform, or a
/// running/stopped container bound to different image content than the one
/// verified). This is a known mismatch, not partial hardening -- it blocks in
/// BOTH `required` and `legacy_partial`. `legacy_partial` never means
/// "execute the wrong content anyway."
pub const BLOCKED_IMAGE_IDENTITY_MISMATCH: &str = "blocked_image_identity_mismatch";

pub const EXECUTION_PINNING_MODE_LEGACY_PARTIAL: &str = "legacy_partial";
pub const EXECUTION_PINNING_MODE_REQUIRED: &str = "required";

fn normalize_mode(mode: &str) -> Result<&'static str, String> {
    match mode.trim() {
        EXECUTION_PINNING_MODE_REQUIRED => Ok(EXECUTION_PINNING_MODE_REQUIRED),
        EXECUTION_PINNING_MODE_LEGACY_PARTIAL => Ok(EXECUTION_PINNING_MODE_LEGACY_PARTIAL),
        // A pre-Stage-107A authorization carries an empty
        // execution_pinning_mode. Fail closed: treat it as the weakest mode
        // for the "does it satisfy the agent's minimum" check below, never
        // silently promote it.
        "" => Ok(EXECUTION_PINNING_MODE_LEGACY_PARTIAL),
        other => Err(format!("unsupported execution_pinning_mode '{other}'")),
    }
}

fn is_required(mode: &str) -> bool {
    mode == EXECUTION_PINNING_MODE_REQUIRED
}

/// The agent's `minimum_execution_pinning_mode` may only make enforcement
/// STRICTER than the project's bound `execution_pinning_mode`, never weaker
/// (spec decision 5 / N-2). If the agent's local minimum is `required` and
/// the authorization is not, this fails closed -- the agent refuses to
/// relax down to whatever the authorization declared. Otherwise the
/// effective mode is exactly the authorization's bound mode (which may
/// itself be `required` even when the agent's local minimum is
/// `legacy_partial` -- the agent always honours the stricter of the two).
pub fn effective_pinning_mode(
    authorization_mode: &str,
    agent_minimum: &str,
) -> Result<&'static str, String> {
    let bound = normalize_mode(authorization_mode)?;
    let minimum = normalize_mode(agent_minimum)?;
    if is_required(minimum) && !is_required(bound) {
        return Err(format!(
            "{BLOCKED_PINNING_MODE_INSUFFICIENT}: agent minimum_execution_pinning_mode='{minimum}' but authorization execution_pinning_mode='{bound}'"
        ));
    }
    Ok(bound)
}

/// `required` mode must not silently default `authorization_clock_skew_seconds`
/// to zero (ADR-0012 design-review finding 2): an unconfigured skew is
/// indistinguishable from assuming perfect clock synchronization, which the
/// architecture must never claim. `legacy_partial` may fall back to 0
/// (weaker guarantee, recorded in evidence via `f2_status`).
pub fn resolve_clock_skew_seconds(
    effective_mode: &str,
    configured_skew_seconds: Option<u64>,
) -> Result<u64, String> {
    match configured_skew_seconds {
        Some(seconds) => Ok(seconds),
        None if is_required(effective_mode) => Err(format!(
            "{BLOCKED_MISSING_CLOCK_SKEW_CONFIG}: required mode needs an explicit authorization_clock_skew_seconds; there is no implicit zero"
        )),
        None => Ok(0),
    }
}

/// Bounded-lease state for one reconciliation cycle (ADR-0012 Option A),
/// evaluated fresh before **each** governed mutation boundary (M1-M8) rather
/// than once per cycle (design-review finding 3).
#[derive(Debug, Clone)]
pub struct LeaseGuard {
    has_finite_lease: bool,
    expires_at: Option<DateTime<Utc>>,
    skew: Duration,
}

impl LeaseGuard {
    /// Build from the authorization actually attached to the fetched
    /// context, and the resolved clock-skew seconds
    /// (`resolve_clock_skew_seconds`). A process-scope authorization, or one
    /// with no `expires_at_utc`, never has a finite lease (ADR-0012
    /// design-review finding 1) regardless of `allow_host_mutation` or
    /// identity match.
    pub fn from_authorization(
        authorization: &OperationalReconciliationAuthorization,
        skew_seconds: u64,
    ) -> Result<Self, String> {
        let expires_at = authorization
            .expires_at_utc
            .as_deref()
            .map(|text| {
                DateTime::parse_from_rfc3339(text)
                    .map(|parsed| parsed.with_timezone(&Utc))
                    .map_err(|error| format!("invalid authorization.expires_at_utc '{text}': {error}"))
            })
            .transpose()?;
        let has_finite_lease =
            authorization.persistence_scope == "durable" && expires_at.is_some();
        Ok(Self {
            has_finite_lease,
            expires_at,
            skew: bounded_skew_duration(skew_seconds)?,
        })
    }

    /// Evaluate immediately before beginning governed mutation boundary `op`
    /// (one of `"M1"`..`"M8"`). Production call sites use this: it samples
    /// the clock ITSELF, fresh, at the instant of the call (review finding
    /// P0-2) -- it is NOT given a cycle-wide timestamp threaded in from
    /// earlier in the reconciliation cycle, so a lease that expires between
    /// two mutation boundaries in the same governed intent is caught at the
    /// second boundary even if the first boundary's check already passed.
    pub fn check_before_mutation(&self, op: &str) -> Result<(), String> {
        self.check_before_mutation_at(op, Utc::now())
    }

    /// Same as `check_before_mutation`, but with an injected `now` for
    /// deterministic testing. Expired ⇔ `now + skew >= expires_at`
    /// (design-review finding 2: skew only ever shortens the window when
    /// honestly configured, by making expiry appear earlier to the agent).
    pub fn check_before_mutation_at(&self, op: &str, now: DateTime<Utc>) -> Result<(), String> {
        if !self.has_finite_lease {
            return Err(format!(
                "{BLOCKED_AUTHORIZATION_NOT_LEASED}: mutation boundary {op} requires a finite (durable-scope) lease; a process-scope authorization can never mutate a host"
            ));
        }
        let expires_at = self.expires_at.expect("has_finite_lease implies expires_at is Some");
        if now + self.skew >= expires_at {
            return Err(format!(
                "{BLOCKED_LEASE_EXPIRED}: mutation boundary {op} lease expired at {expires_at} (now={now}, skew={}s)",
                self.skew.num_seconds()
            ));
        }
        Ok(())
    }

    pub fn has_finite_lease(&self) -> bool {
        self.has_finite_lease
    }
}

/// Upper bound for a configured `authorization_clock_skew_seconds`: the same
/// bound as `MAX_DURABLE_AUTHORIZATION_SECONDS` (dserver) -- a skew larger
/// than the maximum possible lease duration is never a sane configuration,
/// and bounding it here means the `u64 -> i64` conversion below can never
/// wrap (review finding P1-4). A configured skew above this bound fails
/// closed rather than silently wrapping negative (which could EXTEND a lease
/// instead of shortening it).
pub const MAX_CLOCK_SKEW_SECONDS: u64 = 86_400;

fn bounded_skew_duration(skew_seconds: u64) -> Result<Duration, String> {
    if skew_seconds > MAX_CLOCK_SKEW_SECONDS {
        return Err(format!(
            "authorization_clock_skew_seconds {skew_seconds} exceeds the maximum {MAX_CLOCK_SKEW_SECONDS}; refusing rather than risking a u64->i64 wrap that could extend a lease"
        ));
    }
    // Safe: skew_seconds <= 86_400 <= i64::MAX, so this conversion cannot
    // wrap or truncate.
    Ok(Duration::seconds(skew_seconds as i64))
}

/// Per-service generation/digest gate (ADR-0011 §2): the authorization's
/// bound `{artifact_id, execution_generation, execution_projection_digest}`
/// for `service_id` must exactly match what was delivered, AND the agent's
/// own recomputed digest (`execution_projection::recompute_digest`) must
/// match the delivered `execution_projection_digest` -- catching both a
/// stale/mismatched authorization and a tampered-in-transit assignment.
pub fn check_generation_and_projection(
    authorization: &OperationalReconciliationAuthorization,
    service_id: &str,
    artifact_id: &str,
    delivered_generation: u64,
    delivered_digest: &str,
    recomputed_digest: &str,
) -> Result<(), String> {
    if !crate::agent::jcs::is_canonical_digest_format(delivered_digest) || recomputed_digest != delivered_digest {
        return Err(format!(
            "{BLOCKED_PROJECTION_MISMATCH}: recomputed digest for service '{service_id}' does not match the delivered execution_projection_digest"
        ));
    }
    let bound = authorization
        .artifact_bindings
        .iter()
        .find(|binding| binding.service_id == service_id);
    match bound {
        Some(binding)
            if binding.artifact_id == artifact_id
                && binding.execution_generation == delivered_generation
                && binding.execution_projection_digest == delivered_digest => Ok(()),
        _ => Err(format!(
            "{BLOCKED_GENERATION_MISMATCH}: authorization has no matching artifact_bindings entry for service '{service_id}' at generation {delivered_generation}"
        )),
    }
}

/// SHA-256 (hex) of raw bytes -- used for configuration mount content
/// verification (Section G), distinct from the JCS-canonical-JSON digest in
/// `jcs.rs` / `execution_projection.rs`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut s = String::with_capacity(64);
    for byte in digest {
        s.push_str(&format!("{byte:02x}"));
    }
    s
}

/// Verify the actual resolved bytes of a mounted configuration file against
/// its declared digest (Section G). `None` (undeclared) is not itself a
/// failure here -- it is reported as an unpinned external input in evidence,
/// and `required` mode's overall pinning-completeness check (not this
/// function) decides whether an unpinned mount is acceptable.
pub fn verify_configuration_mount_content(
    declared_sha256: Option<&str>,
    resolved_bytes: &[u8],
) -> Result<(), String> {
    let Some(expected) = declared_sha256 else {
        return Ok(());
    };
    let actual = sha256_hex(resolved_bytes);
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!(
            "{BLOCKED_CONFIG_CONTENT_MISMATCH}: expected {expected}, hashed actual bytes to {actual}"
        ))
    }
}

/// Section H (review finding P1-1): the authorization's own `resource_id`
/// (bound at issuance from the `ResourceRoleBindingRecord` active for this
/// node) must match the agent's LOCALLY bootstrap-resolved `resource_id`.
/// The agent does not rely solely on DServer having already filtered the
/// authorization by binding when the context was served -- it verifies this
/// itself, independently, before any mutation. `None` (a pre-Stage-107A
/// authorization, which never carried `resource_id` at all) fails closed,
/// same as an outright mismatch.
///
/// PRECISE VERIFICATION BOUNDARY (review finding P1-6, v4): this function
/// checks `resource_id` ONLY. The authorization also carries `binding_id`
/// (`OperationalReconciliationAuthorization::binding_id`), which the agent
/// deserializes but does NOT independently verify here or anywhere else --
/// there is no local, trusted source the agent can compare it against while
/// possibly offline from DServer. `binding_id` verification against the
/// CURRENTLY active Resource Registry binding happens only on the DServer
/// side, at the moment a context is SERVED
/// (`get_operational_reconciliation_context`). Consequently, an
/// already-delivered Stage 107A snapshot (ADR-0012 Option A) may retain
/// bounded `binding_id` staleness for the remainder of its lease -- if the
/// resource's active binding changes after an authorization was issued but
/// before it expires, the agent has no independent way to detect that and
/// will keep treating the authorization as valid for `resource_id` (which
/// does not change under a re-binding) until the lease itself expires. A
/// future trusted local binding-attestation mechanism could close this; none
/// exists today. Do not describe this function, in code or documentation, as
/// verifying `binding_id` -- it verifies `resource_id`, a related but
/// distinct guarantee.
pub fn check_resource_identity(
    authorization_resource_id: Option<&str>,
    local_resource_id: &str,
) -> Result<(), String> {
    match authorization_resource_id {
        Some(id) if id == local_resource_id => Ok(()),
        Some(id) => Err(format!(
            "{BLOCKED_WRONG_RESOURCE_IDENTITY}: authorization resource_id '{id}' does not match local resource_id '{local_resource_id}'"
        )),
        None => Err(format!(
            "{BLOCKED_WRONG_RESOURCE_IDENTITY}: authorization carries no resource_id (pre-Stage-107A authorization); cannot verify resource-binding identity"
        )),
    }
}

/// Section F/G (ADR-0011 N-2), agent-side defense in depth (review finding
/// P0-3): mirrors
/// `dserver::core::artifact_reconciliation_context::required_completeness_gaps`
/// exactly, evaluated independently against the DELIVERED assignment. DServer
/// already refuses to ISSUE a `required` authorization over an incomplete
/// snapshot; this is the second, independent check the agent performs before
/// ever mutating, in case that server-side gate was bypassed, misconfigured,
/// or the assignment was tampered with in transit (the digest check catches
/// tampering; this catches "digest matches, but the content it matches was
/// never required-complete to begin with").
/// v5 review P1-5: `sha256:` followed by exactly 64 lowercase hex chars.
/// Mirrors `dserver::core::oci_registry_resolver::is_oci_digest_format`
/// exactly (separate, hand-written -- no shared crate).
/// Stage 107B-B (ADR-0014): made `pub(crate)` so
/// `image_identity_verification` can validate a `manifest_digest`/`config_digest`
/// (already-OCI-digest-shaped fields) before comparing them against Docker's
/// own reported values, without duplicating this validator.
pub(crate) fn is_oci_digest_format(s: &str) -> bool {
    match s.strip_prefix("sha256:") {
        Some(hex) => hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        None => false,
    }
}

/// v5 review P1-5: agent-side defense-in-depth structural validation of a
/// `resolved_image_identity` BEFORE declaring the image portion
/// required-complete -- not merely `Option::is_some()`. Unlike the
/// server side (`oci_registry_resolver::validate_resolved_image_identity`),
/// this has no reference parser available (DATUM never resolves images
/// itself), so it cannot cross-check `resolved.repository` against the
/// declared `image` string; it checks internal structural consistency
/// only: platform set non-empty and free of duplicate
/// `(os, architecture, variant)` entries, all digests are valid OCI
/// digests, and every platform's `execution_reference` is exactly
/// `"{repository}@{manifest_digest}"`.
/// v6 review P1-3: extracts a `@sha256:<64hex>` digest selector from the
/// tail of a declared image reference, if present -- a lightweight,
/// parser-free mirror of `oci_registry_resolver::ReferenceSelector::Digest`
/// on the dserver side. DATUM has no full OCI reference parser (it never
/// resolves images itself), so this only recognizes the exact
/// `@sha256:<64hex>` suffix shape, which is sufficient to fail closed on an
/// internally-inconsistent authorized assignment without reimplementing the
/// server's full reference grammar.
fn declared_digest_selector(image: &str) -> Option<String> {
    let (_, candidate) = image.rsplit_once('@')?;
    if is_oci_digest_format(candidate) {
        Some(candidate.to_string())
    } else {
        None
    }
}

/// Stage 107B-B (ADR-0014): made `pub(crate)` so
/// `operational_reconciliation`'s runtime-image-identity orchestration can
/// gate on the exact same structural-validity check `required_completeness_gaps`
/// already uses, rather than re-deriving an equivalent check.
pub(crate) fn structurally_valid_resolved_image_identity(
    image: &str,
    resolved: &crate::agent::operational_reconciliation::OperationalResolvedImageIdentity,
) -> bool {
    if let Some(index_digest) = &resolved.index_digest {
        if !is_oci_digest_format(index_digest) {
            return false;
        }
    }
    if resolved.platforms.is_empty() {
        return false;
    }
    // v7 review P2-3: mirrors the same hardening applied to dserver's
    // `oci_registry_resolver::validate_resolved_image_identity` -- reject an
    // empty os/architecture, an explicitly-empty variant, and a
    // non-canonically-ordered platform set, rather than relying solely on
    // the server side to have produced a well-formed value.
    let keys: Vec<(String, String, Option<String>)> = resolved
        .platforms
        .iter()
        .map(|platform| (platform.os.clone(), platform.architecture.clone(), platform.variant.clone()))
        .collect();
    let mut sorted_keys = keys.clone();
    sorted_keys.sort();
    if keys != sorted_keys {
        return false;
    }
    let mut seen = std::collections::BTreeSet::new();
    for platform in &resolved.platforms {
        if platform.os.trim().is_empty() || platform.architecture.trim().is_empty() {
            return false;
        }
        if let Some(variant) = &platform.variant {
            if variant.trim().is_empty() {
                return false;
            }
        }
        let key = (platform.os.clone(), platform.architecture.clone(), platform.variant.clone());
        if !seen.insert(key) {
            return false;
        }
        if !is_oci_digest_format(&platform.manifest_digest) || !is_oci_digest_format(&platform.config_digest) {
            return false;
        }
        let expected_reference = format!("{}@{}", resolved.repository, platform.manifest_digest);
        if platform.execution_reference != expected_reference {
            return false;
        }
    }
    // v6 review P1-3: mirror of dserver's digest-selector consistency check.
    if let Some(declared) = declared_digest_selector(image) {
        match &resolved.index_digest {
            Some(index_digest) => {
                if &declared != index_digest {
                    return false;
                }
            }
            None => {
                if resolved.platforms.len() != 1 || resolved.platforms[0].manifest_digest != declared {
                    return false;
                }
            }
        }
    }
    true
}

pub fn required_completeness_gaps(assignment: &crate::agent::operational_reconciliation::OperationalArtifactAssignment) -> Vec<String> {
    use crate::agent::operational_reconciliation::{IMAGE_SOURCE_KIND_LEGACY_UNCLASSIFIED, IMAGE_SOURCE_KIND_LOCAL_BUILD};
    let mut gaps = Vec::new();
    // Stage 107B-A (ADR-0013), mirroring
    // dserver::core::artifact_reconciliation_context::required_completeness_gaps
    // exactly: a local_build image is permanently unpinned here -- no
    // docker-image-inspect-based local trust is invented (Amendment 2). A
    // registry-sourced image is unpinned until explicitly resolved AND
    // structurally validated; once resolved_image_identity is present and
    // valid it is required-complete.
    if assignment.image_source_kind == IMAGE_SOURCE_KIND_LOCAL_BUILD {
        if !assignment.image.trim().is_empty() {
            gaps.push(format!("unpinned_image_local_build:{}", assignment.image));
        }
    } else if assignment.image_source_kind == IMAGE_SOURCE_KIND_LEGACY_UNCLASSIFIED {
        // v5 review P0-5: never required-eligible until an operator
        // explicitly classifies it as registry or local_build.
        if !assignment.image.trim().is_empty() {
            gaps.push(format!("unpinned_image_legacy_unclassified:{}", assignment.image));
        }
    } else {
        match &assignment.resolved_image_identity {
            None => {
                if !assignment.image.trim().is_empty() {
                    gaps.push(format!("unpinned_image_reference:{}", assignment.image));
                }
            }
            Some(resolved) => {
                // v5 review P1-5: validate, don't merely check Option::is_some().
                if !structurally_valid_resolved_image_identity(&assignment.image, resolved) {
                    gaps.push(format!("unpinned_image_reference:{}", assignment.image));
                }
            }
        }
    }
    // P1-3 (TOCTOU / "actual bytes used" gap), Option B, mirroring
    // dserver::core::artifact_reconciliation_context::required_completeness_gaps
    // exactly: an ordinary mutable bind-mounted configuration file is NEVER
    // required-complete, regardless of `declared_content_sha256`. The agent
    // still hashes and blocks on a mismatch
    // (`verify_configuration_mount_content` / `verify_declared_configuration_mount_content`
    // in operational_reconciliation.rs) -- that stays valuable,
    // best-effort, `legacy_partial`-appropriate protection; it is just not
    // sufficient to call the mount "required-complete".
    for mount in &assignment.configuration_mounts {
        gaps.push(format!("unpinned_configuration_mount:{}", mount.source));
    }
    // P0-3 (v4 review), mirroring
    // dserver::core::artifact_reconciliation_context::required_completeness_gaps
    // exactly: a `secret_bindings` entry's `version` string does NOT prove
    // the resolved secret's actual identity. Under today's infrastructure
    // the value always comes from a local environment variable on this node
    // (`resolve_secret_bindings` in operational_reconciliation.rs), which
    // `version` does not pin, verify, or even see. `version` remains a
    // meaningful declared safety/documentation contract and stays in the
    // projection, but every `secret_ref` is unconditionally unpinned --
    // exactly like image and configuration-mount content -- until a real
    // verified secret-identity mechanism exists (Stage 107B or later).
    for secret_ref in &assignment.secret_refs {
        gaps.push(format!("unpinned_secret_reference:{secret_ref}"));
    }
    gaps
}

/// Effective port mappings and whether they were substituted from node-local
/// policy (F2c). `required` mode never substitutes: if no control-plane
/// ports are declared and the local mapping declares any, that is a blocked
/// substitution attempt rather than a silent fallback.
pub fn resolve_ports(
    assignment_ports: &[OperationalPortMapping],
    mapping_ports: &[DispatchContainerPortMapping],
    effective_mode: &str,
) -> Result<(Vec<OperationalPortMapping>, bool), String> {
    if !assignment_ports.is_empty() {
        return Ok((assignment_ports.to_vec(), false));
    }
    if mapping_ports.is_empty() {
        return Ok((Vec::new(), false));
    }
    if is_required(effective_mode) {
        return Err(format!(
            "{BLOCKED_LOCAL_POLICY_SUBSTITUTION}: node-local ports may not substitute for undeclared control-plane ports in required mode"
        ));
    }
    let substituted = mapping_ports
        .iter()
        .map(|port| OperationalPortMapping {
            container_port: port.container_port,
            host_port: Some(port.host_port),
            protocol: port.protocol.clone(),
        })
        .collect();
    Ok((substituted, true))
}

/// Effective network aliases and whether they were substituted (F2c).
pub fn resolve_network_aliases(
    assignment_aliases: &[String],
    mapping: &DispatchContainerMapping,
    effective_mode: &str,
) -> Result<(Vec<String>, bool), String> {
    if !assignment_aliases.is_empty() {
        return Ok((assignment_aliases.to_vec(), false));
    }
    let Some(local_alias) = mapping
        .service_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok((Vec::new(), false));
    };
    if is_required(effective_mode) {
        return Err(format!(
            "{BLOCKED_LOCAL_POLICY_SUBSTITUTION}: node-local network alias '{local_alias}' may not substitute for an undeclared control-plane network_aliases entry in required mode"
        ));
    }
    Ok((vec![local_alias.to_string()], true))
}

/// Effective target environment-variable name for `secret_ref`, and whether
/// it was substituted from node-local `DispatchSecretBinding` (F2c).
/// `required` mode requires a control-plane-declared `secret_bindings` entry
/// for every `secret_ref`; a node-local-only mapping is a blocked
/// substitution attempt.
pub fn resolve_secret_target_environment_variable(
    secret_ref: &str,
    declared_target: Option<&str>,
    local_target: Option<&str>,
    effective_mode: &str,
) -> Result<(String, bool), String> {
    if let Some(target) = declared_target {
        return Ok((target.to_string(), false));
    }
    let Some(local_target) = local_target else {
        return Err(format!(
            "secret reference '{secret_ref}' has no control-plane secret_bindings entry and no node-local environment_variable binding"
        ));
    };
    if is_required(effective_mode) {
        return Err(format!(
            "{BLOCKED_LOCAL_POLICY_SUBSTITUTION}: node-local target environment variable '{local_target}' for secret '{secret_ref}' may not substitute for an undeclared control-plane secret_bindings entry in required mode"
        ));
    }
    Ok((local_target.to_string(), true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::operational_reconciliation::ArtifactBindingV1;
    use chrono::TimeZone;

    fn auth(persistence_scope: &str, expires_at_utc: Option<&str>) -> OperationalReconciliationAuthorization {
        OperationalReconciliationAuthorization {
            authorization_id: "opauth:test".to_string(),
            project_id: "p".to_string(),
            dgraph_id: "d".to_string(),
            dgraph_revision: Some(1),
            node_id: "n".to_string(),
            binding_id: Some("binding:test".to_string()),
            resource_id: Some("resource:test".to_string()),
            authorized_by: "operator".to_string(),
            authorized_at_utc: "2026-01-01T00:00:00Z".to_string(),
            allow_host_mutation: true,
            operation: "reconcile".to_string(),
            expires_at_utc: expires_at_utc.map(str::to_string),
            persistence_scope: persistence_scope.to_string(),
            artifact_bindings: vec![ArtifactBindingV1 {
                service_id: "mqtt".to_string(),
                artifact_id: "mqtt".to_string(),
                execution_generation: 3,
                execution_projection_digest: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            }],
            execution_pinning_mode: "required".to_string(),
        }
    }

    // ---- pinning mode -------------------------------------------------- //
    #[test]
    fn effective_mode_required_local_min_required_ok() {
        assert_eq!(effective_pinning_mode("required", "required").unwrap(), "required");
    }
    #[test]
    fn effective_mode_legacy_local_min_required_blocks() {
        let err = effective_pinning_mode("legacy_partial", "required").unwrap_err();
        assert!(err.starts_with(BLOCKED_PINNING_MODE_INSUFFICIENT));
    }
    #[test]
    fn effective_mode_required_local_min_legacy_stays_required() {
        // Agent may accept a stricter authorization even with a looser local minimum.
        assert_eq!(effective_pinning_mode("required", "legacy_partial").unwrap(), "required");
    }
    #[test]
    fn effective_mode_legacy_local_min_legacy_ok() {
        assert_eq!(effective_pinning_mode("legacy_partial", "legacy_partial").unwrap(), "legacy_partial");
    }
    #[test]
    fn empty_authorization_mode_treated_as_legacy_partial_and_blocked_by_required_minimum() {
        let err = effective_pinning_mode("", "required").unwrap_err();
        assert!(err.starts_with(BLOCKED_PINNING_MODE_INSUFFICIENT));
    }

    // ---- clock skew ------------------------------------------------------ //
    #[test]
    fn required_mode_without_configured_skew_blocks() {
        let err = resolve_clock_skew_seconds(EXECUTION_PINNING_MODE_REQUIRED, None).unwrap_err();
        assert!(err.starts_with(BLOCKED_MISSING_CLOCK_SKEW_CONFIG));
    }
    #[test]
    fn required_mode_with_configured_skew_ok() {
        assert_eq!(resolve_clock_skew_seconds(EXECUTION_PINNING_MODE_REQUIRED, Some(2)).unwrap(), 2);
    }
    #[test]
    fn legacy_partial_without_configured_skew_defaults_to_zero() {
        assert_eq!(resolve_clock_skew_seconds(EXECUTION_PINNING_MODE_LEGACY_PARTIAL, None).unwrap(), 0);
    }

    // ---- lease guard ------------------------------------------------------ //
    fn t(minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, minute, 0).unwrap()
    }

    #[test]
    fn process_scope_never_has_finite_lease() {
        let guard = LeaseGuard::from_authorization(&auth("process", None), 0).unwrap();
        assert!(!guard.has_finite_lease());
        let err = guard.check_before_mutation_at("M1", t(0)).unwrap_err();
        assert!(err.starts_with(BLOCKED_AUTHORIZATION_NOT_LEASED));
    }

    #[test]
    fn durable_with_expiry_is_valid_before_expiry() {
        let guard = LeaseGuard::from_authorization(
            &auth("durable", Some("2026-01-01T00:10:00Z")),
            0,
        )
        .unwrap();
        assert!(guard.has_finite_lease());
        assert!(guard.check_before_mutation_at("M1", t(5)).is_ok());
    }

    #[test]
    fn durable_lease_expired_blocks_next_boundary() {
        let guard = LeaseGuard::from_authorization(
            &auth("durable", Some("2026-01-01T00:10:00Z")),
            0,
        )
        .unwrap();
        // O-4: Mx already begun (checked valid at t(9)) runs to completion --
        // that is a property of the CALL SITE (it does not re-check mid-op),
        // not of this function. The NEXT boundary, checked at/after expiry,
        // must block.
        assert!(guard.check_before_mutation_at("M3", t(9)).is_ok());
        let err = guard.check_before_mutation_at("M1", t(10)).unwrap_err();
        assert!(err.starts_with(BLOCKED_LEASE_EXPIRED));
    }

    #[test]
    fn clock_skew_makes_expiry_appear_earlier_never_later() {
        // Lease expires at :10. With skew=120s (2 minutes), the agent must
        // treat it as expired starting at :08, not :10 and not :12 -- i.e.
        // strictly earlier than the unadjusted expiry, never later.
        let guard = LeaseGuard::from_authorization(
            &auth("durable", Some("2026-01-01T00:10:00Z")),
            120,
        )
        .unwrap();
        assert!(guard.check_before_mutation_at("M1", t(7)).is_ok());
        assert!(guard.check_before_mutation_at("M1", t(8)).is_err());
        assert!(guard.check_before_mutation_at("M1", t(9)).is_err());
    }

    // P0-2 regression: `check_before_mutation` (the production, no-`now`-arg
    // entry point) must sample the real clock itself, fresh, at the instant
    // of each call -- not reuse a timestamp threaded in from earlier in the
    // cycle. Proven without flakiness by using expiry timestamps far enough
    // in the past/future that `Utc::now()` (whenever this test actually
    // runs) is unambiguously on one side.
    #[test]
    fn production_check_before_mutation_samples_the_real_clock_expired_case() {
        let guard = LeaseGuard::from_authorization(
            &auth("durable", Some("2000-01-01T00:00:00Z")),
            0,
        )
        .unwrap();
        let err = guard.check_before_mutation("M1").unwrap_err();
        assert!(err.starts_with(BLOCKED_LEASE_EXPIRED));
    }

    #[test]
    fn production_check_before_mutation_samples_the_real_clock_valid_case() {
        let guard = LeaseGuard::from_authorization(
            &auth("durable", Some("2100-01-01T00:00:00Z")),
            0,
        )
        .unwrap();
        assert!(guard.check_before_mutation("M1").is_ok());
    }

    // P0-2 regression, the exact sequence from the review: M3 passes at a
    // time before expiry; the SAME governed intent's next boundary (M1, the
    // recursive re-entry after a recreate) is checked again independently
    // and must observe a freshly-sampled clock, not the value captured for
    // M3. `check_before_mutation_at` is the deterministic stand-in for "the
    // clock advanced" between two sequential production calls.
    #[test]
    fn sequential_boundaries_in_one_governed_intent_each_get_a_fresh_clock_sample() {
        let guard = LeaseGuard::from_authorization(
            &auth("durable", Some("2026-01-01T00:00:05Z")),
            0,
        )
        .unwrap();
        let m3_time = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 2).unwrap();
        let m1_time = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 7).unwrap(); // after expiry
        assert!(guard.check_before_mutation_at("M3", m3_time).is_ok());
        let err = guard.check_before_mutation_at("M1", m1_time).unwrap_err();
        assert!(err.starts_with(BLOCKED_LEASE_EXPIRED));
    }

    // ---- clock skew bound (P1-4) ------------------------------------------ //
    #[test]
    fn normal_skew_is_accepted() {
        assert!(LeaseGuard::from_authorization(&auth("durable", Some("2026-01-01T00:10:00Z")), 30).is_ok());
    }

    #[test]
    fn skew_at_the_max_bound_is_accepted() {
        assert!(LeaseGuard::from_authorization(
            &auth("durable", Some("2026-01-01T00:10:00Z")),
            MAX_CLOCK_SKEW_SECONDS
        )
        .is_ok());
    }

    #[test]
    fn u64_max_skew_is_rejected_not_wrapped() {
        let err = LeaseGuard::from_authorization(&auth("durable", Some("2026-01-01T00:10:00Z")), u64::MAX)
            .unwrap_err();
        assert!(err.contains("exceeds the maximum"));
    }

    #[test]
    fn skew_just_above_the_bound_is_rejected() {
        assert!(LeaseGuard::from_authorization(
            &auth("durable", Some("2026-01-01T00:10:00Z")),
            MAX_CLOCK_SKEW_SECONDS + 1
        )
        .is_err());
    }

    // ---- generation / projection ------------------------------------------ //
    #[test]
    fn matching_generation_and_digest_passes() {
        assert!(check_generation_and_projection(&auth("durable", None), "mqtt", "mqtt", 3, "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").is_ok());
    }
    #[test]
    fn recomputed_digest_mismatch_blocks() {
        let err = check_generation_and_projection(&auth("durable", None), "mqtt", "mqtt", 3, "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "different").unwrap_err();
        assert!(err.starts_with(BLOCKED_PROJECTION_MISMATCH));
    }
    #[test]
    fn generation_mismatch_blocks() {
        let err = check_generation_and_projection(&auth("durable", None), "mqtt", "mqtt", 4, "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap_err();
        assert!(err.starts_with(BLOCKED_GENERATION_MISMATCH));
    }
    #[test]
    fn unknown_service_blocks() {
        let err = check_generation_and_projection(&auth("durable", None), "other", "other", 1, "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap_err();
        assert!(err.starts_with(BLOCKED_GENERATION_MISMATCH));
    }

    // ---- resource identity (P1-1) -------------------------------------- //
    #[test]
    fn matching_resource_id_passes() {
        assert!(check_resource_identity(Some("resource:1"), "resource:1").is_ok());
    }
    #[test]
    fn mismatched_resource_id_blocks() {
        let err = check_resource_identity(Some("resource:1"), "resource:2").unwrap_err();
        assert!(err.starts_with(BLOCKED_WRONG_RESOURCE_IDENTITY));
    }
    #[test]
    fn missing_resource_id_blocks() {
        let err = check_resource_identity(None, "resource:2").unwrap_err();
        assert!(err.starts_with(BLOCKED_WRONG_RESOURCE_IDENTITY));
    }

    // ---- required-mode completeness, agent side (P0-3) ------------------ //
    fn complete_assignment() -> crate::agent::operational_reconciliation::OperationalArtifactAssignment {
        use crate::agent::operational_reconciliation::{
            ArtifactWasmDecisionPolicy, OperationalConfigurationMount, OperationalLifecycle,
            OperationalSecretBinding,
        };
        crate::agent::operational_reconciliation::OperationalArtifactAssignment {
            artifact_id: "mqtt".to_string(),
            service_id: "mqtt".to_string(),
            artifact_service_id: "mqtt".to_string(),
            version: "1".to_string(),
            artifact_kind: "container".to_string(),
            runtime_kind: "container".to_string(),
            container_name: "datum-mqtt".to_string(),
            image: "eclipse-mosquitto:2".to_string(),
            native_process: None,
            image_source_kind: crate::agent::operational_reconciliation::IMAGE_SOURCE_KIND_REGISTRY.to_string(),
            resolved_image_identity: None,
            target_node: "fog-01".to_string(),
            lifecycle: OperationalLifecycle::default(),
            network: None,
            network_aliases: Vec::new(),
            ports: Vec::new(),
            volumes: Vec::new(),
            configuration_mounts: vec![OperationalConfigurationMount {
                source: "mosquitto.conf".to_string(),
                target: "/etc/mosquitto.conf".to_string(),
                read_only: true,
                declared_content_sha256: Some("a".repeat(64)),
            }],
            command: Vec::new(),
            dependencies: Vec::new(),
            wasm_decision: ArtifactWasmDecisionPolicy {
                explicit_invocation_required: true,
                module_id: "m".to_string(),
                function_name: "f".to_string(),
                host_imports_allowed: false,
                side_effects_allowed: false,
                safe_to_auto_execute: false,
            },
            bindings: std::collections::BTreeMap::new(),
            secret_refs: vec!["token".to_string()],
            secret_bindings: vec![OperationalSecretBinding {
                secret_ref: "token".to_string(),
                target_environment_variable: "TOKEN".to_string(),
                version: Some("v1".to_string()),
            }],
            execution_generation: 1,
            execution_projection_digest: String::new(),
        }
    }

    #[test]
    fn agent_side_bare_image_is_always_a_gap() {
        let gaps = required_completeness_gaps(&complete_assignment());
        assert!(gaps.iter().any(|gap| gap.starts_with("unpinned_image_reference:")), "{gaps:?}");
    }

    #[test]
    fn agent_side_resolved_registry_image_emits_no_image_gap() {
        use crate::agent::operational_reconciliation::{OperationalResolvedImageIdentity, OperationalResolvedPlatformManifest};
        let mut assignment = complete_assignment();
        assignment.resolved_image_identity = Some(OperationalResolvedImageIdentity {
            repository: "docker.io/library/eclipse-mosquitto".to_string(),
            index_digest: None,
            platforms: vec![OperationalResolvedPlatformManifest {
                os: "linux".to_string(),
                architecture: "amd64".to_string(),
                variant: None,
                manifest_digest: format!("sha256:{}", "1".repeat(64)),
                config_digest: format!("sha256:{}", "2".repeat(64)),
                execution_reference: format!("docker.io/library/eclipse-mosquitto@sha256:{}", "1".repeat(64)),
            }],
            resolved_at_utc: "2026-01-01T00:00:00Z".to_string(),
            resolver_tool_version: "test/0.1.0".to_string(),
            resolved_by: "operator:test".to_string(),
            registry_host: "registry-1.docker.io".to_string(),
        });
        let gaps = required_completeness_gaps(&assignment);
        assert!(
            !gaps.iter().any(|gap| gap.starts_with("unpinned_image_reference:") || gap.starts_with("unpinned_image_local_build:")),
            "{gaps:?}"
        );
    }

    #[test]
    fn agent_side_local_build_image_is_always_gated_with_a_distinct_message() {
        use crate::agent::operational_reconciliation::IMAGE_SOURCE_KIND_LOCAL_BUILD;
        let mut assignment = complete_assignment();
        assignment.image_source_kind = IMAGE_SOURCE_KIND_LOCAL_BUILD.to_string();
        let gaps = required_completeness_gaps(&assignment);
        assert!(gaps.iter().any(|gap| gap.starts_with("unpinned_image_local_build:")), "{gaps:?}");
        assert!(!gaps.iter().any(|gap| gap.starts_with("unpinned_image_reference:")), "{gaps:?}");
    }

    // ---- v5 review P0-5: legacy_unclassified is always gated ----------- //
    #[test]
    fn agent_side_legacy_unclassified_image_is_always_gated_with_a_distinct_message() {
        use crate::agent::operational_reconciliation::IMAGE_SOURCE_KIND_LEGACY_UNCLASSIFIED;
        let mut assignment = complete_assignment();
        assignment.image_source_kind = IMAGE_SOURCE_KIND_LEGACY_UNCLASSIFIED.to_string();
        let gaps = required_completeness_gaps(&assignment);
        assert!(gaps.iter().any(|gap| gap.starts_with("unpinned_image_legacy_unclassified:")), "{gaps:?}");
        assert!(!gaps.iter().any(|gap| gap.starts_with("unpinned_image_reference:")), "{gaps:?}");
    }

    // ---- v5 review P1-5: structural validation, not Option::is_some() -- //
    #[test]
    fn agent_side_structurally_inconsistent_resolved_identity_is_still_a_gap() {
        use crate::agent::operational_reconciliation::{OperationalResolvedImageIdentity, OperationalResolvedPlatformManifest};
        let mut assignment = complete_assignment();
        assignment.resolved_image_identity = Some(OperationalResolvedImageIdentity {
            repository: "docker.io/library/eclipse-mosquitto".to_string(),
            index_digest: None,
            platforms: vec![OperationalResolvedPlatformManifest {
                os: "linux".to_string(),
                architecture: "amd64".to_string(),
                variant: None,
                manifest_digest: format!("sha256:{}", "1".repeat(64)),
                config_digest: format!("sha256:{}", "2".repeat(64)),
                // Deliberately inconsistent: does not match
                // "{repository}@{manifest_digest}".
                execution_reference: "docker.io/library/eclipse-mosquitto@sha256:not-the-manifest-digest".to_string(),
            }],
            resolved_at_utc: "2026-01-01T00:00:00Z".to_string(),
            resolver_tool_version: "test/0.1.0".to_string(),
            resolved_by: "operator:test".to_string(),
            registry_host: "registry-1.docker.io".to_string(),
        });
        let gaps = required_completeness_gaps(&assignment);
        assert!(
            gaps.iter().any(|gap| gap.starts_with("unpinned_image_reference:")),
            "a structurally-inconsistent resolved identity must still be a gap: {gaps:?}"
        );
    }

    // ---- v6 review P1-3: digest-selector consistency mirror ------------ //
    #[test]
    fn agent_side_digest_selector_matching_the_single_manifest_digest_is_accepted() {
        use crate::agent::operational_reconciliation::{OperationalResolvedImageIdentity, OperationalResolvedPlatformManifest};
        let manifest_digest = format!("sha256:{}", "1".repeat(64));
        let mut assignment = complete_assignment();
        assignment.image = format!("eclipse-mosquitto@{manifest_digest}");
        assignment.resolved_image_identity = Some(OperationalResolvedImageIdentity {
            repository: "docker.io/library/eclipse-mosquitto".to_string(),
            index_digest: None,
            platforms: vec![OperationalResolvedPlatformManifest {
                os: "linux".to_string(),
                architecture: "amd64".to_string(),
                variant: None,
                manifest_digest: manifest_digest.clone(),
                config_digest: format!("sha256:{}", "2".repeat(64)),
                execution_reference: format!("docker.io/library/eclipse-mosquitto@{manifest_digest}"),
            }],
            resolved_at_utc: "2026-01-01T00:00:00Z".to_string(),
            resolver_tool_version: "test/0.1.0".to_string(),
            resolved_by: "operator:test".to_string(),
            registry_host: "registry-1.docker.io".to_string(),
        });
        let gaps = required_completeness_gaps(&assignment);
        assert!(!gaps.iter().any(|gap| gap.starts_with("unpinned_image_reference:")), "{gaps:?}");
    }

    #[test]
    fn agent_side_digest_selector_not_matching_the_single_manifest_digest_is_a_gap() {
        use crate::agent::operational_reconciliation::{OperationalResolvedImageIdentity, OperationalResolvedPlatformManifest};
        let mut assignment = complete_assignment();
        // The declared image's digest selector ('9') disagrees with the
        // resolved platform's own manifest_digest ('1').
        assignment.image = format!("eclipse-mosquitto@sha256:{}", "9".repeat(64));
        assignment.resolved_image_identity = Some(OperationalResolvedImageIdentity {
            repository: "docker.io/library/eclipse-mosquitto".to_string(),
            index_digest: None,
            platforms: vec![OperationalResolvedPlatformManifest {
                os: "linux".to_string(),
                architecture: "amd64".to_string(),
                variant: None,
                manifest_digest: format!("sha256:{}", "1".repeat(64)),
                config_digest: format!("sha256:{}", "2".repeat(64)),
                execution_reference: format!("docker.io/library/eclipse-mosquitto@sha256:{}", "1".repeat(64)),
            }],
            resolved_at_utc: "2026-01-01T00:00:00Z".to_string(),
            resolver_tool_version: "test/0.1.0".to_string(),
            resolved_by: "operator:test".to_string(),
            registry_host: "registry-1.docker.io".to_string(),
        });
        let gaps = required_completeness_gaps(&assignment);
        assert!(gaps.iter().any(|gap| gap.starts_with("unpinned_image_reference:")), "{gaps:?}");
    }

    // ---- v7 review P2-3: hardened structural validator mirror ---------- //
    #[test]
    fn agent_side_empty_platform_os_is_still_a_gap() {
        use crate::agent::operational_reconciliation::{OperationalResolvedImageIdentity, OperationalResolvedPlatformManifest};
        let mut assignment = complete_assignment();
        assignment.resolved_image_identity = Some(OperationalResolvedImageIdentity {
            repository: "docker.io/library/eclipse-mosquitto".to_string(),
            index_digest: None,
            platforms: vec![OperationalResolvedPlatformManifest {
                os: "".to_string(),
                architecture: "amd64".to_string(),
                variant: None,
                manifest_digest: format!("sha256:{}", "1".repeat(64)),
                config_digest: format!("sha256:{}", "2".repeat(64)),
                execution_reference: format!("docker.io/library/eclipse-mosquitto@sha256:{}", "1".repeat(64)),
            }],
            resolved_at_utc: "2026-01-01T00:00:00Z".to_string(),
            resolver_tool_version: "test/0.1.0".to_string(),
            resolved_by: "operator:test".to_string(),
            registry_host: "registry-1.docker.io".to_string(),
        });
        let gaps = required_completeness_gaps(&assignment);
        assert!(gaps.iter().any(|gap| gap.starts_with("unpinned_image_reference:")), "{gaps:?}");
    }

    #[test]
    fn agent_side_explicitly_empty_variant_is_still_a_gap() {
        use crate::agent::operational_reconciliation::{OperationalResolvedImageIdentity, OperationalResolvedPlatformManifest};
        let mut assignment = complete_assignment();
        assignment.resolved_image_identity = Some(OperationalResolvedImageIdentity {
            repository: "docker.io/library/eclipse-mosquitto".to_string(),
            index_digest: None,
            platforms: vec![OperationalResolvedPlatformManifest {
                os: "linux".to_string(),
                architecture: "arm64".to_string(),
                variant: Some("".to_string()),
                manifest_digest: format!("sha256:{}", "1".repeat(64)),
                config_digest: format!("sha256:{}", "2".repeat(64)),
                execution_reference: format!("docker.io/library/eclipse-mosquitto@sha256:{}", "1".repeat(64)),
            }],
            resolved_at_utc: "2026-01-01T00:00:00Z".to_string(),
            resolver_tool_version: "test/0.1.0".to_string(),
            resolved_by: "operator:test".to_string(),
            registry_host: "registry-1.docker.io".to_string(),
        });
        let gaps = required_completeness_gaps(&assignment);
        assert!(gaps.iter().any(|gap| gap.starts_with("unpinned_image_reference:")), "{gaps:?}");
    }

    #[test]
    fn agent_side_non_canonically_ordered_platforms_is_still_a_gap() {
        use crate::agent::operational_reconciliation::{OperationalResolvedImageIdentity, OperationalResolvedPlatformManifest};
        let repository = "docker.io/library/eclipse-mosquitto".to_string();
        let mut assignment = complete_assignment();
        assignment.resolved_image_identity = Some(OperationalResolvedImageIdentity {
            repository: repository.clone(),
            index_digest: None,
            platforms: vec![
                OperationalResolvedPlatformManifest {
                    os: "linux".to_string(),
                    architecture: "arm64".to_string(),
                    variant: None,
                    manifest_digest: format!("sha256:{}", "1".repeat(64)),
                    config_digest: format!("sha256:{}", "2".repeat(64)),
                    execution_reference: format!("{repository}@sha256:{}", "1".repeat(64)),
                },
                OperationalResolvedPlatformManifest {
                    os: "linux".to_string(),
                    architecture: "amd64".to_string(),
                    variant: None,
                    manifest_digest: format!("sha256:{}", "3".repeat(64)),
                    config_digest: format!("sha256:{}", "4".repeat(64)),
                    execution_reference: format!("{repository}@sha256:{}", "3".repeat(64)),
                },
            ],
            resolved_at_utc: "2026-01-01T00:00:00Z".to_string(),
            resolver_tool_version: "test/0.1.0".to_string(),
            resolved_by: "operator:test".to_string(),
            registry_host: "registry-1.docker.io".to_string(),
        });
        let gaps = required_completeness_gaps(&assignment);
        assert!(gaps.iter().any(|gap| gap.starts_with("unpinned_image_reference:")), "{gaps:?}");
    }

    #[test]
    fn agent_side_versioned_secret_is_still_a_gap_p0_3() {
        // P1-3: a bind-mounted config is never required-complete, even with
        // a declared+verified digest (TOCTOU, Option B). P0-3 (v4 review):
        // a versioned secret binding is likewise never required-complete --
        // `version` does not prove the resolved (locally-env-var-sourced)
        // secret's actual identity, so it cannot close the gap either.
        let gaps = required_completeness_gaps(&complete_assignment());
        assert!(gaps.iter().any(|gap| gap.starts_with("unpinned_configuration_mount:")), "{gaps:?}");
        assert!(gaps.iter().any(|gap| gap == "unpinned_secret_reference:token"), "{gaps:?}");
    }

    #[test]
    fn agent_side_config_mount_is_a_gap_whether_or_not_a_digest_is_declared() {
        let mut undeclared = complete_assignment();
        undeclared.configuration_mounts[0].declared_content_sha256 = None;
        let gaps_undeclared = required_completeness_gaps(&undeclared);
        assert!(gaps_undeclared.iter().any(|gap| gap == "unpinned_configuration_mount:mosquitto.conf"), "{gaps_undeclared:?}");

        let declared = complete_assignment();
        assert!(declared.configuration_mounts[0].declared_content_sha256.is_some());
        let gaps_declared = required_completeness_gaps(&declared);
        assert!(gaps_declared.iter().any(|gap| gap == "unpinned_configuration_mount:mosquitto.conf"), "{gaps_declared:?}");
    }

    #[test]
    fn agent_side_unversioned_secret_is_a_gap() {
        let mut assignment = complete_assignment();
        assignment.secret_bindings[0].version = None;
        let gaps = required_completeness_gaps(&assignment);
        assert!(gaps.iter().any(|gap| gap == "unpinned_secret_reference:token"), "{gaps:?}");
    }

    // ---- config content ----------------------------------------------- //
    #[test]
    fn undeclared_config_digest_is_not_a_failure() {
        assert!(verify_configuration_mount_content(None, b"anything").is_ok());
    }
    #[test]
    fn matching_config_digest_passes() {
        let expected = sha256_hex(b"hello");
        assert!(verify_configuration_mount_content(Some(&expected), b"hello").is_ok());
    }
    #[test]
    fn tampered_config_bytes_blocks() {
        let expected = sha256_hex(b"hello");
        let err = verify_configuration_mount_content(Some(&expected), b"goodbye").unwrap_err();
        assert!(err.starts_with(BLOCKED_CONFIG_CONTENT_MISMATCH));
    }

    // ---- local policy no-substitution ----------------------------------- //
    fn local_ports() -> Vec<DispatchContainerPortMapping> {
        vec![DispatchContainerPortMapping { host_port: 1883, container_port: 1883, protocol: "tcp".to_string() }]
    }

    #[test]
    fn declared_ports_are_used_even_if_local_mapping_disagrees() {
        let declared = vec![OperationalPortMapping { container_port: 1883, host_port: Some(9999), protocol: "tcp".to_string() }];
        let (effective, substituted) = resolve_ports(&declared, &local_ports(), EXECUTION_PINNING_MODE_REQUIRED).unwrap();
        assert!(!substituted);
        assert_eq!(effective[0].host_port, Some(9999));
    }

    #[test]
    fn undeclared_ports_in_required_mode_with_local_mapping_blocks() {
        let err = resolve_ports(&[], &local_ports(), EXECUTION_PINNING_MODE_REQUIRED).unwrap_err();
        assert!(err.starts_with(BLOCKED_LOCAL_POLICY_SUBSTITUTION));
    }

    #[test]
    fn undeclared_ports_in_legacy_partial_mode_substitutes_and_is_recorded() {
        let (effective, substituted) = resolve_ports(&[], &local_ports(), EXECUTION_PINNING_MODE_LEGACY_PARTIAL).unwrap();
        assert!(substituted);
        assert_eq!(effective[0].container_port, 1883);
    }

    #[test]
    fn undeclared_ports_with_no_local_mapping_is_not_a_substitution() {
        let (effective, substituted) = resolve_ports(&[], &[], EXECUTION_PINNING_MODE_REQUIRED).unwrap();
        assert!(!substituted);
        assert!(effective.is_empty());
    }

    fn local_mapping(service_name: &str) -> DispatchContainerMapping {
        DispatchContainerMapping {
            module_id: None,
            service_name: Some(service_name.to_string()),
            container_name: "c".to_string(),
            artifact: Some("img".to_string()),
            ports: Vec::new(),
        }
    }

    #[test]
    fn declared_aliases_used_even_with_local_mapping() {
        let (effective, substituted) = resolve_network_aliases(&["mqtt".to_string()], &local_mapping("other"), EXECUTION_PINNING_MODE_REQUIRED).unwrap();
        assert!(!substituted);
        assert_eq!(effective, vec!["mqtt".to_string()]);
    }

    #[test]
    fn undeclared_alias_in_required_mode_blocks() {
        let err = resolve_network_aliases(&[], &local_mapping("mqtt"), EXECUTION_PINNING_MODE_REQUIRED).unwrap_err();
        assert!(err.starts_with(BLOCKED_LOCAL_POLICY_SUBSTITUTION));
    }

    #[test]
    fn undeclared_alias_in_legacy_partial_mode_substitutes() {
        let (effective, substituted) = resolve_network_aliases(&[], &local_mapping("mqtt"), EXECUTION_PINNING_MODE_LEGACY_PARTIAL).unwrap();
        assert!(substituted);
        assert_eq!(effective, vec!["mqtt".to_string()]);
    }

    #[test]
    fn declared_secret_target_used_even_with_local_binding() {
        let (target, substituted) = resolve_secret_target_environment_variable("ref", Some("DECLARED"), Some("LOCAL"), EXECUTION_PINNING_MODE_REQUIRED).unwrap();
        assert!(!substituted);
        assert_eq!(target, "DECLARED");
    }

    #[test]
    fn undeclared_secret_target_in_required_mode_blocks() {
        let err = resolve_secret_target_environment_variable("ref", None, Some("LOCAL"), EXECUTION_PINNING_MODE_REQUIRED).unwrap_err();
        assert!(err.starts_with(BLOCKED_LOCAL_POLICY_SUBSTITUTION));
    }

    #[test]
    fn undeclared_secret_target_in_legacy_partial_mode_substitutes() {
        let (target, substituted) = resolve_secret_target_environment_variable("ref", None, Some("LOCAL"), EXECUTION_PINNING_MODE_LEGACY_PARTIAL).unwrap();
        assert!(substituted);
        assert_eq!(target, "LOCAL");
    }

    #[test]
    fn undeclared_secret_target_with_no_local_binding_fails_regardless_of_mode() {
        assert!(resolve_secret_target_environment_variable("ref", None, None, EXECUTION_PINNING_MODE_LEGACY_PARTIAL).is_err());
    }
}
