//! D-Code invocation execution evidence (ADR-0021, 111.12). Digests/sizes
//! only -- never the full arbitrary input/output payload -- and never a
//! claim of mathematical determinism merely because the instance was fresh
//! (a D-Code module CAN still read a genuinely nondeterministic host
//! resource within its sandbox, e.g. nothing here proves the guest itself
//! is a pure function).
//!
//! Independent review v2 (R4) extension: `input_port`/`message_id`/
//! `correlation_id`/`output_status`/`output_reason`/`emits` record the
//! ADR-0003 `datum-dnode/0` envelope this invocation actually used --
//! `message_id`/`correlation_id` are always HOST-generated (`dcode::envelope`)
//! and never influenced by the guest; `emits` carries only digests/sizes per
//! validated emit, never the emitted payload bytes themselves.
//!
//! Independent review v3 (F3) extension: R2 made the canonical artifact
//! DESCRIPTOR digest security/load-bearing (distinct from the module content
//! digest), but evidence previously recorded only the module digest and only
//! DMap/DGraph id+revision -- not their canonical digests -- so a
//! post-execution evidence record alone could not uniquely identify the
//! exact limits/ports/safety/constraints/DMap-snapshot/DGraph-snapshot that
//! authorization actually bound. `dmap_digest`, `dgraph_digest`, and
//! `dserv_artifact_descriptor_digest` close that gap, and `module_size_bytes`
//! records the actual loaded module length the R3 size check verified.
//! All four are populated DIRECTLY from the authorization/artifact actually
//! used by `invoke_dcode`, never reconstructed afterward.
//!
//! Independent review v3 (F3A) extension: after R4, the guest receives the
//! serialized ADR-0003 JSON envelope, NOT the raw caller payload -- but
//! `input_sha256`/`input_size` digest only the raw payload, which is not the
//! identity of the actual bytes passed to `datum_handle`. `input_envelope_sha256`/
//! `input_envelope_size`/`received_at_utc` record the exact envelope bytes
//! and host-generated timestamp `build_input_envelope` actually produced,
//! kept deliberately distinct from the raw-payload fields (both are
//! meaningful, for different questions: "what did the caller send" vs. "what
//! did the guest actually see"). Neither the full payload nor the full
//! envelope is ever persisted here -- digests/sizes only. This remains
//! execution evidence, never a claim of cryptographic non-repudiation.
//!
//! Independent review v4 (G3) correction: this file's own header claimed
//! "digests/sizes only -- never the full arbitrary payload," but that was
//! not actually true. `output_reason` used to store the GUEST-controlled
//! `reason` string verbatim -- a malicious or merely buggy guest could put
//! arbitrary sensitive content (including the entire raw input payload) into
//! `reason`, and it was serialized into evidence as plaintext. A second leak
//! channel existed too: envelope-validation failures (`InvalidStatus`,
//! `UndeclaredOutputPort`) converted their guest-controlled offending value
//! directly into `error_message` via `Display`. Both are fixed here:
//! `output_reason` is replaced by `output_reason_present`/`_sha256`/`_size`
//! (never the string itself), and `error_message` for guest-output-
//! validation failures is now a STATIC, safe classification string, with the
//! guest-controlled offending value (if any) available only as
//! `error_offending_value_sha256`/`_size` -- see `evidence-content-leakage-audit.txt`
//! and `dcode::invoke`'s adversarial exfiltration tests
//! (`evidence_never_leaks_a_marker_via_*`).

use serde::{Deserialize, Serialize};

pub const DCODE_EVIDENCE_SCHEMA: &str = "datum.dcode-invocation-evidence/1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DCodeInvocationOutcome {
    Success,
    ValidationFailed,
    ResourceLimit,
    Trap,
    StaleAuthority,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DCodeInvocationEvidenceV1 {
    pub schema: String,
    pub authorization_id: String,
    pub project_id: String,
    pub application_id: String,
    pub service_id: String,
    pub node_id: String,
    pub dmap_id: String,
    pub dmap_revision: u64,
    /// Canonical digest of the accepted D-Map this invocation was authorized
    /// against (F3) -- populated directly from `authorization.dmap_ref.digest`.
    pub dmap_digest: String,
    pub dgraph_id: String,
    pub dgraph_revision: u64,
    /// Canonical digest of the D-Graph snapshot this invocation was
    /// authorized against (F3) -- from `authorization.dgraph_ref.digest`.
    pub dgraph_digest: String,
    pub dserv_artifact_id: String,
    /// D-Code MODULE content digest only -- see `dserv_artifact_descriptor_digest`
    /// below for why this is deliberately distinct.
    pub dserv_artifact_digest: String,
    /// Canonical JCS digest of the WHOLE `datum.dserv-artifact/1` descriptor
    /// (F3) -- from `authorization.dserv_artifact_descriptor_digest`. This is
    /// what actually identifies the exact limits/ports/safety/constraints
    /// bound at authorization time; the module digest alone does not.
    pub dserv_artifact_descriptor_digest: String,
    /// Actual loaded module byte length (F3) -- the value the R3 `size_bytes`
    /// check verified against `artifact.dcode.size_bytes`. `None` if the
    /// invocation failed before the module was ever loaded.
    pub module_size_bytes: Option<u64>,
    pub dnode_abi: String,
    pub fresh_instance_per_invocation: bool,
    /// ADR-0003 envelope input port (R4A).
    pub input_port: String,
    /// Host-generated (`dcode::envelope::build_input_envelope`); never
    /// influenced by the guest.
    pub message_id: String,
    /// Host-generated; never influenced by the guest.
    pub correlation_id: String,
    /// Digest/size of the RAW caller-supplied payload, BEFORE ADR-0003
    /// envelope wrapping (F3A) -- distinct from `input_envelope_sha256`
    /// below, which is what the guest actually received.
    pub input_sha256: String,
    pub input_size: usize,
    /// Digest/size of the EXACT serialized ADR-0003 input envelope bytes
    /// actually passed to `datum_handle` (F3A) -- distinct from
    /// `input_sha256` above (the raw payload before envelope wrapping).
    pub input_envelope_sha256: String,
    pub input_envelope_size: usize,
    /// The host-generated timestamp embedded in that same input envelope
    /// (F3A) -- from `build_input_envelope`, never guest-influenced.
    pub received_at_utc: String,
    /// Digest/size of the RAW bytes returned by `datum_handle` (the
    /// serialized output envelope JSON), before envelope parsing/validation.
    pub output_sha256: Option<String>,
    pub output_size: Option<usize>,
    /// The guest's own business-level status ("ok"/"rejected") from a
    /// successfully PARSED AND VALIDATED output envelope. `None` if the
    /// invocation never reached a parseable output (e.g. it failed earlier).
    pub output_status: Option<String>,
    /// G3A: whether the guest's output envelope included a `reason` field --
    /// never the reason text itself, which is guest-controlled and may
    /// contain arbitrary (potentially sensitive) content.
    pub output_reason_present: bool,
    /// Digest/size of the guest-supplied `reason` string's UTF-8 bytes, if
    /// present. `None` when `output_reason_present` is `false`.
    pub output_reason_sha256: Option<String>,
    pub output_reason_size: Option<usize>,
    /// Validated emits from a successfully parsed output envelope -- digests/
    /// sizes only, never routed anywhere (D-Call, phase 112, will later
    /// deliver them).
    #[serde(default)]
    pub emits: Vec<DCodeEmitEvidenceV1>,
    pub limits_applied: DCodeAppliedLimits,
    pub outcome: DCodeInvocationOutcome,
    pub error_code: Option<String>,
    /// G3B: for a guest-output-validation failure, this is now a STATIC,
    /// safe classification message -- it never embeds an arbitrary
    /// guest-controlled value (the offending `status`/output `port`/etc.)
    /// verbatim. See `error_offending_value_sha256`/`_size` for a
    /// content-free way to identify the offending value without persisting
    /// it. Internal/host-side diagnostic messages that cannot contain
    /// caller/guest payload (e.g. "failed to spawn worker process") may
    /// still appear here freely.
    pub error_message: Option<String>,
    /// G3B: digest/size of the specific guest-controlled value that caused a
    /// guest-output-validation failure (e.g. the invalid `status` string, or
    /// the undeclared output `port` name), when applicable. Never the value
    /// itself.
    #[serde(default)]
    pub error_offending_value_sha256: Option<String>,
    #[serde(default)]
    pub error_offending_value_size: Option<usize>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DCodeEmitEvidenceV1 {
    pub port: String,
    pub payload_sha256: String,
    pub payload_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DCodeAppliedLimits {
    pub fuel_per_invocation: u64,
    pub timeout_ms: u64,
    pub max_memory_pages: u32,
    pub max_message_bytes: u32,
}

/// Independent review fix v5 (section 9): the ONE deterministic digest
/// algorithm used to identify a `DCodeInvocationEvidenceV1` record for D-Call
/// binding purposes -- used identically for BOTH the source invocation that
/// produced a dispatched emit and the destination invocation D-Call actually
/// executed, so `dcall::delivery` never maintains two subtly different
/// evidence-digest algorithms. This is plain `serde_json` serialization
/// (struct field order is fixed by the type definition, so this is already
/// deterministic for structurally identical records) -- identical to the
/// algorithm `dcall::evidence::build_success_evidence` already used for the
/// destination side before this fix, now shared rather than duplicated.
/// Returns the bare 64-lowercase-hex representation (this crate's existing
/// `sha256_hex` convention), never the full evidence content itself.
pub fn dcode_evidence_sha256(evidence: &DCodeInvocationEvidenceV1) -> String {
    crate::dcode::identity::sha256_hex(&serde_json::to_vec(evidence).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_evidence() -> DCodeInvocationEvidenceV1 {
        DCodeInvocationEvidenceV1 {
            schema: DCODE_EVIDENCE_SCHEMA.to_string(),
            authorization_id: "auth-1".to_string(),
            project_id: "proj:lab".to_string(),
            application_id: "app:a".to_string(),
            service_id: "source".to_string(),
            node_id: "fog-01".to_string(),
            dmap_id: "dmap:1".to_string(),
            dmap_revision: 1,
            dmap_digest: format!("sha256:{}", "1".repeat(64)),
            dgraph_id: "dgraph:1".to_string(),
            dgraph_revision: 1,
            dgraph_digest: format!("sha256:{}", "2".repeat(64)),
            dserv_artifact_id: "dserv:pipeline:source".to_string(),
            dserv_artifact_digest: "3".repeat(64),
            dserv_artifact_descriptor_digest: format!("sha256:{}", "4".repeat(64)),
            module_size_bytes: Some(64),
            dnode_abi: "datum-dnode/0".to_string(),
            fresh_instance_per_invocation: true,
            input_port: "in".to_string(),
            message_id: "m1".to_string(),
            correlation_id: "c1".to_string(),
            input_sha256: "5".repeat(64),
            input_size: 7,
            input_envelope_sha256: "6".repeat(64),
            input_envelope_size: 100,
            received_at_utc: "2026-01-01T00:00:00Z".to_string(),
            output_sha256: None,
            output_size: None,
            output_status: Some("ok".to_string()),
            output_reason_present: false,
            output_reason_sha256: None,
            output_reason_size: None,
            emits: Vec::new(),
            limits_applied: DCodeAppliedLimits { fuel_per_invocation: 1_000_000, timeout_ms: 100, max_memory_pages: 4, max_message_bytes: 65536 },
            outcome: DCodeInvocationOutcome::Success,
            error_code: None,
            error_message: None,
            error_offending_value_sha256: None,
            error_offending_value_size: None,
            duration_ms: 1,
        }
    }

    /// Section 9: the shared source/destination evidence-digest helper must
    /// be deterministic for structurally identical records...
    #[test]
    fn dcode_evidence_sha256_is_deterministic_for_identical_records() {
        let a = sample_evidence();
        let b = sample_evidence();
        assert_eq!(dcode_evidence_sha256(&a), dcode_evidence_sha256(&b));
    }

    /// ...and sensitive to any real difference (here, `node_id`), so it
    /// genuinely identifies a SPECIFIC evidence record rather than merely a
    /// coarse fingerprint of a few fields.
    #[test]
    fn dcode_evidence_sha256_changes_when_the_evidence_itself_changes() {
        let a = sample_evidence();
        let mut b = sample_evidence();
        b.node_id = "cloud-01".to_string();
        assert_ne!(dcode_evidence_sha256(&a), dcode_evidence_sha256(&b));
    }

    #[test]
    fn dcode_evidence_sha256_is_a_bare_64_char_lowercase_hex_string() {
        let digest = dcode_evidence_sha256(&sample_evidence());
        assert_eq!(digest.len(), 64);
        assert!(digest.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
    }
}
