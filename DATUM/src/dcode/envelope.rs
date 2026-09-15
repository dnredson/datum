//! ADR-0003's `datum-dnode/0` guest-visible message envelope (independent
//! review v2, R4). This was previously unimplemented -- v1 passed a caller's
//! raw payload bytes directly to `datum_handle` and returned the guest's raw
//! output bytes directly to the caller, with no envelope at all. ADR-0003
//! does not define ONLY function exports; it also defines this envelope and
//! the host-enforcement obligations that come with it (max emit count,
//! output ports declared on the artifact, host-owned `correlation_id`).
//!
//! D-Node (this slice, phase 111) owns envelope CONSTRUCTION and VALIDATION.
//! D-Call (phase 112) will later TRANSPORT/ROUTE already-valid envelopes
//! between D-Servs -- it does not need to exist for this slice to correctly
//! construct and validate one envelope locally.
//!
//! `message_id`/`correlation_id` are ALWAYS host-generated
//! (`build_input_envelope`) and never derived from, or influenced by,
//! anything the guest returns -- the output envelope schema
//! (`DCodeOutputEnvelopeV1`, `#[serde(deny_unknown_fields)]`) has no slot for
//! either, so a guest attempting to inject its own `message_id`/
//! `correlation_id` into its output produces a structurally invalid output
//! envelope (rejected as `InvalidJson`), not an accepted override.

use base64::Engine as _;
use serde::{Deserialize, Serialize};

pub const DNODE_ENVELOPE_ABI: &str = "datum-dnode/0";

#[derive(Debug, Clone, Serialize)]
pub struct DCodeInputEnvelopeV1 {
    pub abi: String,
    pub port: String,
    pub message_id: String,
    pub correlation_id: String,
    pub received_at_utc: String,
    pub payload_b64: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DCodeOutputEnvelopeV1 {
    pub status: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub emits: Vec<DCodeEmitV1>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DCodeEmitV1 {
    pub port: String,
    pub payload_b64: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvelopeError {
    InvalidUtf8,
    InvalidJson(String),
    InvalidStatus(String),
    MalformedBase64(String),
    TooManyEmits { max: u32, actual: usize },
    UndeclaredInputPort(String),
    UndeclaredOutputPort(String),
}

impl std::fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnvelopeError::InvalidUtf8 => write!(f, "output envelope bytes are not valid UTF-8"),
            EnvelopeError::InvalidJson(message) => write!(f, "output envelope is not valid JSON: {message}"),
            EnvelopeError::InvalidStatus(status) => {
                write!(f, "output envelope status '{status}' is not 'ok' or 'rejected'")
            }
            EnvelopeError::MalformedBase64(message) => write!(f, "output envelope emit payload_b64 is malformed: {message}"),
            EnvelopeError::TooManyEmits { max, actual } => {
                write!(f, "output envelope declares {actual} emits, exceeding max_emits_per_invocation {max}")
            }
            EnvelopeError::UndeclaredInputPort(port) => {
                write!(f, "input port '{port}' is not declared in artifact.ports.inputs")
            }
            EnvelopeError::UndeclaredOutputPort(port) => {
                write!(f, "output envelope emits on port '{port}', which is not declared in artifact.ports.outputs")
            }
        }
    }
}

/// Independent review v4 (G3B): a STATIC, safe classification message for an
/// `EnvelopeError`, plus the specific guest-controlled offending value's raw
/// bytes (if any) -- for the CALLER to digest/size into evidence, never to
/// persist verbatim. Some variants (`InvalidStatus`, `UndeclaredOutputPort`)
/// previously had their `Display` text -- which embeds the guest-controlled
/// value directly -- copied straight into execution evidence's
/// `error_message`; this closes that leak while keeping `error_code`
/// (already a separate, deterministic, non-content-bearing classification)
/// unaffected.
pub fn safe_error_message(error: &EnvelopeError) -> (String, Option<Vec<u8>>) {
    match error {
        EnvelopeError::InvalidUtf8 => ("output envelope bytes are not valid UTF-8".to_string(), None),
        EnvelopeError::InvalidJson(_) => ("output envelope is not valid JSON".to_string(), None),
        EnvelopeError::InvalidStatus(status) => (
            "output envelope status is not 'ok' or 'rejected'".to_string(),
            Some(status.clone().into_bytes()),
        ),
        EnvelopeError::MalformedBase64(_) => ("output envelope emit payload_b64 is malformed".to_string(), None),
        EnvelopeError::TooManyEmits { max, actual } => (
            format!("output envelope declares {actual} emits, exceeding max_emits_per_invocation {max}"),
            None,
        ),
        EnvelopeError::UndeclaredInputPort(port) => {
            // Not guest-controlled -- this is the CALLER's own chosen input
            // port, validated before any guest code ever runs. Safe to keep
            // dynamic.
            (format!("input port '{port}' is not declared in artifact.ports.inputs"), None)
        }
        EnvelopeError::UndeclaredOutputPort(port) => (
            "output envelope emits on a port not declared in artifact.ports.outputs".to_string(),
            Some(port.clone().into_bytes()),
        ),
    }
}

#[derive(Debug)]
pub struct ValidatedEmit {
    pub port: String,
    pub payload: Vec<u8>,
}

#[derive(Debug)]
pub struct ValidatedOutput {
    pub status: String,
    pub reason: Option<String>,
    pub emits: Vec<ValidatedEmit>,
}

/// Builds the ADR-0003 input envelope. `message_id`/`correlation_id` are
/// freshly host-generated on every call -- the guest never supplies or
/// influences either. Returns the serialized JSON bytes to hand to
/// `datum_handle`, plus the generated identifiers/timestamp for evidence.
pub fn build_input_envelope(port: &str, payload: &[u8]) -> (Vec<u8>, String, String, String) {
    let message_id = format!("dcode-msg:{}", uuid::Uuid::new_v4());
    let correlation_id = format!("dcode-corr:{}", uuid::Uuid::new_v4());
    let received_at_utc = chrono::Utc::now().to_rfc3339();
    let envelope = DCodeInputEnvelopeV1 {
        abi: DNODE_ENVELOPE_ABI.to_string(),
        port: port.to_string(),
        message_id: message_id.clone(),
        correlation_id: correlation_id.clone(),
        received_at_utc: received_at_utc.clone(),
        payload_b64: base64::engine::general_purpose::STANDARD.encode(payload),
    };
    let bytes = serde_json::to_vec(&envelope).expect("DCodeInputEnvelopeV1 serialization cannot fail");
    (bytes, message_id, correlation_id, received_at_utc)
}

/// R4C: the input port must be one the artifact actually declares as an
/// input.
pub fn validate_input_port(declared_input_ports: &[String], port: &str) -> Result<(), EnvelopeError> {
    if declared_input_ports.iter().any(|declared| declared == port) {
        Ok(())
    } else {
        Err(EnvelopeError::UndeclaredInputPort(port.to_string()))
    }
}

/// R4B/R4C: parses and fail-closed-validates the guest's raw output bytes as
/// an ADR-0003 output envelope. Never routes emits anywhere -- returns
/// validated, decoded emit payloads for the CALLER (here, `invoke_dcode`) to
/// digest into evidence.
pub fn parse_and_validate_output(
    raw: &[u8],
    declared_output_ports: &[String],
    max_emits: u32,
) -> Result<ValidatedOutput, EnvelopeError> {
    let text = std::str::from_utf8(raw).map_err(|_| EnvelopeError::InvalidUtf8)?;
    let envelope: DCodeOutputEnvelopeV1 =
        serde_json::from_str(text).map_err(|error| EnvelopeError::InvalidJson(error.to_string()))?;

    if envelope.status != "ok" && envelope.status != "rejected" {
        return Err(EnvelopeError::InvalidStatus(envelope.status));
    }
    if envelope.emits.len() > max_emits as usize {
        return Err(EnvelopeError::TooManyEmits { max: max_emits, actual: envelope.emits.len() });
    }

    let mut emits = Vec::with_capacity(envelope.emits.len());
    for emit in envelope.emits {
        if !declared_output_ports.iter().any(|declared| declared == &emit.port) {
            return Err(EnvelopeError::UndeclaredOutputPort(emit.port));
        }
        let payload = base64::engine::general_purpose::STANDARD
            .decode(&emit.payload_b64)
            .map_err(|error| EnvelopeError::MalformedBase64(error.to_string()))?;
        emits.push(ValidatedEmit { port: emit.port, payload });
    }

    Ok(ValidatedOutput { status: envelope.status, reason: envelope.reason, emits })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_envelope_carries_the_given_port_and_payload() {
        let (bytes, message_id, correlation_id, received_at_utc) = build_input_envelope("uplink", b"hello");
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["abi"], DNODE_ENVELOPE_ABI);
        assert_eq!(value["port"], "uplink");
        assert_eq!(value["message_id"], message_id);
        assert_eq!(value["correlation_id"], correlation_id);
        assert_eq!(value["received_at_utc"], received_at_utc);
        assert_eq!(
            value["payload_b64"],
            base64::engine::general_purpose::STANDARD.encode(b"hello")
        );
    }

    #[test]
    fn two_envelopes_never_share_message_or_correlation_id() {
        let (_, m1, c1, _) = build_input_envelope("p", b"a");
        let (_, m2, c2, _) = build_input_envelope("p", b"a");
        assert_ne!(m1, m2);
        assert_ne!(c1, c2);
    }

    #[test]
    fn declared_input_port_is_accepted() {
        assert!(validate_input_port(&["uplink".to_string()], "uplink").is_ok());
    }

    #[test]
    fn undeclared_input_port_is_rejected() {
        assert!(validate_input_port(&["uplink".to_string()], "other").is_err());
    }

    fn ok_envelope(emits: &str) -> Vec<u8> {
        format!(r#"{{"status":"ok","reason":null,"emits":[{emits}]}}"#).into_bytes()
    }

    #[test]
    fn valid_output_envelope_with_no_emits_parses() {
        let output = parse_and_validate_output(&ok_envelope(""), &["reading".to_string()], 8).unwrap();
        assert_eq!(output.status, "ok");
        assert!(output.emits.is_empty());
    }

    #[test]
    fn valid_output_envelope_with_one_emit_parses_and_decodes() {
        let payload_b64 = base64::engine::general_purpose::STANDARD.encode(b"emitted-bytes");
        let raw = format!(r#"{{"status":"ok","emits":[{{"port":"reading","payload_b64":"{payload_b64}"}}]}}"#);
        let output = parse_and_validate_output(raw.as_bytes(), &["reading".to_string()], 8).unwrap();
        assert_eq!(output.emits.len(), 1);
        assert_eq!(output.emits[0].port, "reading");
        assert_eq!(output.emits[0].payload, b"emitted-bytes");
    }

    #[test]
    fn rejected_status_is_a_valid_well_formed_output() {
        let raw = br#"{"status":"rejected","reason":"business rule declined"}"#;
        let output = parse_and_validate_output(raw, &[], 8).unwrap();
        assert_eq!(output.status, "rejected");
        assert_eq!(output.reason.as_deref(), Some("business rule declined"));
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        let raw = [0xff, 0xfe, 0xfd];
        assert_eq!(parse_and_validate_output(&raw, &[], 8).unwrap_err(), EnvelopeError::InvalidUtf8);
    }

    #[test]
    fn malformed_json_is_rejected() {
        let raw = b"not json at all";
        assert!(matches!(parse_and_validate_output(raw, &[], 8).unwrap_err(), EnvelopeError::InvalidJson(_)));
    }

    #[test]
    fn invalid_status_is_rejected() {
        let raw = br#"{"status":"maybe"}"#;
        assert!(matches!(parse_and_validate_output(raw, &[], 8).unwrap_err(), EnvelopeError::InvalidStatus(_)));
    }

    #[test]
    fn too_many_emits_is_rejected() {
        let raw = ok_envelope(
            r#"{"port":"a","payload_b64":""},{"port":"a","payload_b64":""},{"port":"a","payload_b64":""}"#,
        );
        let error = parse_and_validate_output(&raw, &["a".to_string()], 2).unwrap_err();
        assert_eq!(error, EnvelopeError::TooManyEmits { max: 2, actual: 3 });
    }

    #[test]
    fn undeclared_emit_port_is_rejected() {
        let raw = format!(
            r#"{{"status":"ok","emits":[{{"port":"undeclared","payload_b64":"{}"}}]}}"#,
            base64::engine::general_purpose::STANDARD.encode(b"x")
        );
        let error = parse_and_validate_output(raw.as_bytes(), &["reading".to_string()], 8).unwrap_err();
        assert_eq!(error, EnvelopeError::UndeclaredOutputPort("undeclared".to_string()));
    }

    #[test]
    fn malformed_base64_emit_payload_is_rejected() {
        let raw = br#"{"status":"ok","emits":[{"port":"reading","payload_b64":"not-valid-base64!!"}]}"#;
        assert!(matches!(
            parse_and_validate_output(raw, &["reading".to_string()], 8).unwrap_err(),
            EnvelopeError::MalformedBase64(_)
        ));
    }

    /// G3B: `safe_error_message` must never embed the guest-controlled
    /// offending value verbatim in its static message string -- only in the
    /// separately-returned raw bytes, for the caller to digest.
    #[test]
    fn safe_error_message_never_embeds_guest_value_in_the_static_text() {
        let marker = "SENSITIVE_MARKER_should_never_appear_in_static_text";
        let (message, offending) = safe_error_message(&EnvelopeError::InvalidStatus(marker.to_string()));
        assert!(!message.contains(marker));
        assert_eq!(offending, Some(marker.as_bytes().to_vec()));

        let (message, offending) = safe_error_message(&EnvelopeError::UndeclaredOutputPort(marker.to_string()));
        assert!(!message.contains(marker));
        assert_eq!(offending, Some(marker.as_bytes().to_vec()));
    }

    /// R4C: the output envelope schema (`deny_unknown_fields`) has no slot
    /// for `message_id`/`correlation_id` at all -- a guest attempting to
    /// claim its own identity produces a structurally invalid envelope, not
    /// an accepted override. This is the structural proof that "the host
    /// owns message/correlation identity."
    #[test]
    fn guest_cannot_inject_its_own_message_or_correlation_id_into_output() {
        let raw = br#"{"status":"ok","message_id":"guest-forged-id","emits":[]}"#;
        assert!(matches!(parse_and_validate_output(raw, &[], 8).unwrap_err(), EnvelopeError::InvalidJson(_)));
    }
}
