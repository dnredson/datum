//! D-Code module content identity: `sha256:<64 lowercase hex>` of the exact
//! bytes actually executed, always recomputed, never trusted from a
//! filename, path, mtime, or caller claim. A digest mismatch fails BEFORE
//! compilation/instantiation.
//!
//! This is a deliberate, independent ~15-line duplication of the SHA-256
//! hex helper that already exists in `wasm_runtime::integrity` -- see
//! `dcode-vs-management-wasm-audit.txt` for why: this module must have zero
//! `use` of anything under `wasm_runtime` (Management WASM), and standard
//! SHA-256 hashing is trivial, well-tested-by-`sha2`-itself code, not
//! meaningfully risky to duplicate.

use sha2::{Digest, Sha256};

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing SHA-256 hex to String cannot fail");
    }
    output
}

/// Accepts a bare 64-hex-char digest or a `sha256:`-prefixed one, either
/// case; returns the normalized lowercase bare-hex form.
pub fn normalize_expected_sha256(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    let digest = trimmed.strip_prefix("sha256:").unwrap_or(trimmed);
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("expected SHA-256 must contain exactly 64 hexadecimal characters, optionally prefixed with 'sha256:'".to_string());
    }
    Ok(digest.to_ascii_lowercase())
}

/// Verifies `bytes` matches `expected_sha256`, returning the normalized
/// digest on success.
pub fn verify_module_bytes_digest(bytes: &[u8], expected_sha256: &str) -> Result<String, String> {
    let expected = normalize_expected_sha256(expected_sha256)?;
    let actual = sha256_hex(bytes);
    if actual != expected {
        return Err(format!("D-Code module SHA-256 mismatch: expected {expected}, calculated {actual}"));
    }
    Ok(actual)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_known_abc_vector() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn matching_digest_verifies() {
        assert_eq!(verify_module_bytes_digest(b"abc", &sha256_hex(b"abc")).unwrap(), sha256_hex(b"abc"));
    }

    #[test]
    fn prefixed_uppercase_digest_is_accepted() {
        let expected = format!("sha256:{}", sha256_hex(b"abc").to_ascii_uppercase());
        assert_eq!(verify_module_bytes_digest(b"abc", &expected).unwrap(), sha256_hex(b"abc"));
    }

    #[test]
    fn mismatched_digest_is_rejected() {
        let error = verify_module_bytes_digest(b"abc", &"0".repeat(64)).unwrap_err();
        assert!(error.contains("mismatch"));
    }

    #[test]
    fn malformed_digest_is_rejected() {
        let error = verify_module_bytes_digest(b"abc", "not-a-digest").unwrap_err();
        assert!(error.contains("64 hexadecimal characters"));
    }

    #[test]
    fn source_guard_keeps_dcode_identity_free_of_management_wasm_imports() {
        let source = include_str!("identity.rs");
        let implementation = source.split("#[cfg(test)]").next().expect("implementation section should exist");
        for forbidden in ["use crate::wasm_runtime", "use super::super::wasm_runtime", "wasmtime_wasi", "WasiCtx"] {
            assert!(!implementation.contains(forbidden), "dcode::identity must not reference '{forbidden}'");
        }
    }
}
