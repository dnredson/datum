//! RFC 8785 JSON Canonicalization Scheme (JCS) + SHA-256, scoped to the
//! numeric domain the DATUM Stage 107A contract actually uses.
//!
//! Conformant with RFC 8785 for: object-member ordering by the UTF-16 code
//! unit sequence of the key (not raw Rust `str` scalar-value order, which
//! diverges from UTF-16 order for supplementary-plane characters); recursive
//! canonical encoding of null / bool / string / array / object with no
//! insignificant whitespace; string encoding via minimal JSON escaping
//! (delegated to `serde_json`, whose default escape set already matches
//! RFC 8259's minimal encoding used by JCS).
//!
//! This is **not** a general-purpose RFC 8785 implementation. JSON numbers
//! are restricted to signed integers in the IEEE-754 "safe integer" range
//! `-(2^53-1)..=(2^53-1)`. Any JSON number carrying a fractional part, an
//! exponent, or a magnitude outside that range is rejected as out of
//! contract, rather than attempting full ECMAScript `Number::toString`
//! formatting (a materially larger undertaking). The Stage 107A execution
//! projection never needs a non-integer or unsafe-magnitude number (ports,
//! generations and timeouts are all safe integers; everything else is a
//! string, bool, array or object), so this restriction is a deliberate,
//! tested scope boundary. Do not claim unrestricted RFC 8785 compliance for
//! this module; the conformance tests below establish exactly what it does
//! and does not cover.
//!
//! **Independence from `dserver/src/core/jcs.rs`.** This module implements
//! the same RFC 8785-scoped algorithm as DServer's copy (algorithmically
//! identical by construction -- there is exactly one correct canonical form
//! per input under this scope, so an independent implementation of the same
//! spec necessarily converges on the same source shape). What makes the two
//! sides' digests an actual cross-crate guarantee, not merely two copies of
//! the same code, is `shared_cross_crate_conformance_vectors` below: both
//! crates assert against the same externally-computed fixture file
//! (`scripts/fixtures/jcs-conformance/vectors.json`), whose `sha256` values
//! were computed by neither crate's Rust code. This module uses the `sha2`
//! crate (already a DATUM dependency); `dserver`'s copy uses a small
//! hand-rolled SHA-256 (`dserver/src/core/sha256.rs`) because adding `sha2`
//! to `dserver`'s `Cargo.toml` cannot be resolved in this environment's
//! offline Cargo cache -- see that module's doc comment.

use serde_json::{Number, Value};
use sha2::{Digest, Sha256};

/// `2^53 - 1`, the largest integer magnitude both IEEE-754 doubles and this
/// canonicalizer represent exactly.
pub const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
/// `-(2^53 - 1)`.
pub const MIN_SAFE_INTEGER: i64 = -9_007_199_254_740_991;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JcsError {
    NumberOutOfSafeIntegerRange(String),
    FractionalOrExponentialNumberRejected(String),
    StringEncodingFailed(String),
}

impl std::fmt::Display for JcsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JcsError::NumberOutOfSafeIntegerRange(n) => write!(
                f,
                "JCS: number '{n}' is outside the signed safe-integer range -(2^53-1)..=(2^53-1)"
            ),
            JcsError::FractionalOrExponentialNumberRejected(n) => write!(
                f,
                "JCS: fractional/exponential number '{n}' is out of the DATUM Stage 107A contract"
            ),
            JcsError::StringEncodingFailed(s) => write!(f, "JCS: could not encode string: {s}"),
        }
    }
}
impl std::error::Error for JcsError {}

/// RFC 8785 canonical JSON text for `value`, within the scope documented on
/// this module.
pub fn canonicalize(value: &Value) -> Result<String, JcsError> {
    let mut out = String::new();
    write_value(value, &mut out)?;
    Ok(out)
}

/// SHA-256 (lowercase hex, no prefix) of the canonical JSON text of `value`.
/// Bare digest of JCS bytes for conformance vectors. Raw files must use a byte-hash helper.
pub fn digest_sha256_hex(value: &Value) -> Result<String, JcsError> {
    let canonical = canonicalize(value)?;
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    Ok(hex_lower(&hasher.finalize()))
}

/// Canonical DATUM object digest, distinct from an exact-byte content hash.
pub fn canonical_digest_sha256_hex(value: &Value) -> Result<String, JcsError> {
    Ok(format!("sha256:{}", digest_sha256_hex(value)?))
}

pub fn is_canonical_digest_format(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn write_value(value: &Value, out: &mut String) -> Result<(), JcsError> {
    match value {
        Value::Null => {
            out.push_str("null");
            Ok(())
        }
        Value::Bool(b) => {
            out.push_str(if *b { "true" } else { "false" });
            Ok(())
        }
        Value::Number(n) => write_number(n, out),
        Value::String(s) => write_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(item, out)?;
            }
            out.push(']');
            Ok(())
        }
        Value::Object(map) => {
            out.push('{');
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| utf16_units(a).cmp(&utf16_units(b)));
            for (i, key) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(key, out)?;
                out.push(':');
                write_value(map.get(*key).expect("key came from map.keys()"), out)?;
            }
            out.push('}');
            Ok(())
        }
    }
}

/// UTF-16 code unit sequence of `s`, used for RFC 8785 object-key ordering.
/// This differs from Rust's default `str` `Ord` for any key containing a
/// supplementary-plane character (U+10000 and above): UTF-16 encodes such a
/// character as a surrogate pair whose first code unit is always in
/// `0xD800..=0xDBFF`, which sorts *before* any BMP code unit `>= 0xE000` even
/// though the supplementary character's Unicode scalar value is numerically
/// larger. See `utf16_supplementary_vs_bmp_key_ordering` below.
fn utf16_units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn write_string(s: &str, out: &mut String) -> Result<(), JcsError> {
    let encoded =
        serde_json::to_string(s).map_err(|error| JcsError::StringEncodingFailed(error.to_string()))?;
    out.push_str(&encoded);
    Ok(())
}

fn write_number(n: &Number, out: &mut String) -> Result<(), JcsError> {
    if let Some(i) = n.as_i64() {
        if !(MIN_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&i) {
            return Err(JcsError::NumberOutOfSafeIntegerRange(n.to_string()));
        }
        out.push_str(&i.to_string());
        return Ok(());
    }
    if let Some(u) = n.as_u64() {
        if u > MAX_SAFE_INTEGER as u64 {
            return Err(JcsError::NumberOutOfSafeIntegerRange(n.to_string()));
        }
        out.push_str(&u.to_string());
        return Ok(());
    }
    // Parsed as f64: either fractional, exponential, or an integer literal
    // written with a decimal point (e.g. "5.0") -- all out of contract.
    Err(JcsError::FractionalOrExponentialNumberRejected(n.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn object_keys_sorted_lexically() {
        let v = json!({"b": 1, "a": 2});
        assert_eq!(canonicalize(&v).unwrap(), r#"{"a":2,"b":1}"#);
    }

    #[test]
    fn key_order_independent_equivalence() {
        let a = json!({"b": 1, "a": 2, "c": 3});
        let b = json!({"c": 3, "a": 2, "b": 1});
        assert_eq!(canonicalize(&a).unwrap(), canonicalize(&b).unwrap());
    }

    #[test]
    fn nested_structures() {
        let v = json!({"z": [3, 2, 1], "a": {"y": 1, "x": 2}});
        assert_eq!(canonicalize(&v).unwrap(), r#"{"a":{"x":2,"y":1},"z":[3,2,1]}"#);
    }

    #[test]
    fn unicode_string_content_preserved() {
        let v = json!({"name": "café"});
        let out = canonicalize(&v).unwrap();
        assert!(out.contains("café"), "unicode content must be preserved: {out}");
    }

    #[test]
    fn safe_integer_boundaries_accepted() {
        let v = json!({"n": MAX_SAFE_INTEGER});
        assert_eq!(canonicalize(&v).unwrap(), format!(r#"{{"n":{MAX_SAFE_INTEGER}}}"#));
        let v = json!({"n": MIN_SAFE_INTEGER});
        assert_eq!(canonicalize(&v).unwrap(), format!(r#"{{"n":{MIN_SAFE_INTEGER}}}"#));
    }

    #[test]
    fn unsafe_integer_magnitude_rejected() {
        let too_big = MAX_SAFE_INTEGER as u64 + 1;
        let v = Value::Object(
            [("n".to_string(), Value::Number(Number::from(too_big)))]
                .into_iter()
                .collect(),
        );
        assert!(matches!(
            canonicalize(&v),
            Err(JcsError::NumberOutOfSafeIntegerRange(_))
        ));

        let too_small = MIN_SAFE_INTEGER - 1;
        let v2 = Value::Object(
            [("n".to_string(), Value::Number(Number::from(too_small)))]
                .into_iter()
                .collect(),
        );
        assert!(matches!(
            canonicalize(&v2),
            Err(JcsError::NumberOutOfSafeIntegerRange(_))
        ));
    }

    #[test]
    fn fractional_number_rejected() {
        let v = json!({"n": 1.5});
        assert!(matches!(
            canonicalize(&v),
            Err(JcsError::FractionalOrExponentialNumberRejected(_))
        ));
    }

    #[test]
    fn integer_literal_written_with_decimal_point_rejected() {
        // "5.0" parses to serde_json as a float, not an integer -- rejected,
        // consistent with treating any non-integer JSON literal as out of
        // contract rather than silently reinterpreting it.
        let v: Value = serde_json::from_str(r#"{"n":5.0}"#).unwrap();
        assert!(matches!(
            canonicalize(&v),
            Err(JcsError::FractionalOrExponentialNumberRejected(_))
        ));
    }

    #[test]
    fn utf16_supplementary_vs_bmp_key_ordering() {
        // U+10000 (supplementary plane) encodes in UTF-16 as a surrogate pair
        // whose first code unit is 0xD800, which is LESS than U+E000's single
        // code unit 0xE000 -- even though U+10000's Unicode scalar value
        // (0x10000) is numerically GREATER than U+E000's (0xE000). JCS
        // requires UTF-16 code-unit order, which this test asserts directly
        // rather than relying on Rust's scalar-value `str` `Ord`.
        let mut map = serde_json::Map::new();
        map.insert("\u{E000}".to_string(), json!(1));
        map.insert("\u{10000}".to_string(), json!(2));
        let v = Value::Object(map);
        let out = canonicalize(&v).unwrap();
        let pos_supplementary = out.find('\u{10000}').expect("supplementary key present");
        let pos_bmp = out.find('\u{E000}').expect("BMP key present");
        assert!(
            pos_supplementary < pos_bmp,
            "canonical form must place the supplementary-plane key first: {out}"
        );
    }

    #[test]
    fn digest_is_stable_for_key_order_equivalent_content() {
        let a = json!({"b": 1, "a": 2});
        let b = json!({"a": 2, "b": 1});
        assert_eq!(digest_sha256_hex(&a).unwrap(), digest_sha256_hex(&b).unwrap());
    }

    #[test]
    fn digest_changes_when_content_changes() {
        let a = json!({"a": 1});
        let b = json!({"a": 2});
        assert_ne!(digest_sha256_hex(&a).unwrap(), digest_sha256_hex(&b).unwrap());
    }

    /// Stage 107A cross-crate conformance: `dserver/src/core/jcs.rs` runs the
    /// identical assertions against the same fixture file.
    #[test]
    fn shared_cross_crate_conformance_vectors() {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("scripts")
            .join("fixtures")
            .join("jcs-conformance")
            .join("vectors.json");
        let text = std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", fixture_path.display()));
        let doc: Value = serde_json::from_str(&text).unwrap();
        let vectors = doc["vectors"].as_array().unwrap();
        assert!(!vectors.is_empty());
        for vector in vectors {
            let name = vector["name"].as_str().unwrap();
            let value = &vector["value"];
            let expected_canonical = vector["canonical"].as_str().unwrap();
            let expected_sha256 = vector["sha256"].as_str().unwrap();
            assert_eq!(canonicalize(value).unwrap(), expected_canonical, "vector '{name}' canonical form");
            assert_eq!(digest_sha256_hex(value).unwrap(), expected_sha256, "vector '{name}' digest");
        }
    }
}
