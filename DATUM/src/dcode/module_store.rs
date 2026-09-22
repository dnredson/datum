//! D-Node-local content-addressed D-Code module store (ADR-0021, 111.4).
//! OPERATIONAL, not canonical: this is not part of the D-Graph/D-Map/
//! D-Continuum contract, and no module path ever participates in canonical
//! D-Code identity -- only the SHA-256 digest does. Distribution (fetching
//! a module onto a node from elsewhere) is explicitly out of scope for
//! v0.1; a module is installed locally and explicitly.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use crate::dcode::identity::{sha256_hex, verify_module_bytes_digest};

pub struct DCodeModuleStore {
    root: PathBuf,
}

impl DCodeModuleStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn sha256_dir(&self) -> PathBuf {
        self.root.join("dcode").join("sha256")
    }

    fn module_path(&self, digest: &str) -> PathBuf {
        self.sha256_dir().join(format!("{digest}.wasm"))
    }

    /// Verifies `bytes` matches `declared_sha256`, then atomically installs
    /// them under `<root>/dcode/sha256/<64hex>.wasm` (write-to-temp +
    /// rename, so a process crash mid-write can never leave a PARTIAL file
    /// at the final content-addressed path). Re-installing byte-identical
    /// content already present is a no-op; since the path itself IS the
    /// digest, two DIFFERENT contents can never collide under the same
    /// path.
    pub fn install(&self, bytes: &[u8], declared_sha256: &str) -> Result<String, String> {
        let digest = verify_module_bytes_digest(bytes, declared_sha256)?;
        let final_path = self.module_path(&digest);
        if final_path.is_file() {
            // Idempotent: verify what's already there is not corrupted,
            // rather than trusting its mere presence.
            let existing = fs::read(&final_path).map_err(|error| {
                format!(
                    "read existing D-Code module {}: {error}",
                    final_path.display()
                )
            })?;
            if sha256_hex(&existing) == digest {
                return Ok(digest);
            }
            return Err(format!(
                "D-Code module store corruption detected: stored file at {} no longer hashes to its own filename digest",
                final_path.display()
            ));
        }

        let dir = self.sha256_dir();
        fs::create_dir_all(&dir).map_err(|error| {
            format!("create D-Code module store dir {}: {error}", dir.display())
        })?;
        let tmp_path = dir.join(format!(".{digest}.{}.tmp", std::process::id()));
        {
            let mut file = fs::File::create(&tmp_path).map_err(|error| {
                format!(
                    "create temp D-Code module file {}: {error}",
                    tmp_path.display()
                )
            })?;
            file.write_all(bytes).map_err(|error| {
                format!(
                    "write temp D-Code module file {}: {error}",
                    tmp_path.display()
                )
            })?;
            file.sync_all().map_err(|error| {
                format!(
                    "fsync temp D-Code module file {}: {error}",
                    tmp_path.display()
                )
            })?;
        }
        fs::rename(&tmp_path, &final_path).map_err(|error| {
            let _ = fs::remove_file(&tmp_path);
            format!(
                "atomically install D-Code module {}: {error}",
                final_path.display()
            )
        })?;

        Ok(digest)
    }

    /// Loads module bytes by digest, RECOMPUTING and verifying the digest
    /// against the path it was loaded from -- a corrupted stored file (its
    /// content no longer matches its own filename) fails to load rather
    /// than being trusted.
    pub fn load(&self, digest: &str) -> Result<Vec<u8>, String> {
        let normalized = crate::dcode::identity::normalize_expected_sha256(digest)?;
        let path = self.module_path(&normalized);
        let bytes = fs::read(&path)
            .map_err(|error| format!("read D-Code module {}: {error}", path.display()))?;
        let actual = sha256_hex(&bytes);
        if actual != normalized {
            return Err(format!(
                "D-Code module store corruption detected: {} contains bytes hashing to '{actual}', not its own filename digest '{normalized}'",
                path.display()
            ));
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn fresh_store() -> DCodeModuleStore {
        DCodeModuleStore::new(
            std::env::temp_dir().join(format!("datum-dcode-store-{}", Uuid::new_v4())),
        )
    }

    #[test]
    fn correct_digest_installs_and_loads() {
        let store = fresh_store();
        let bytes = b"\0asm-fake-module-bytes".to_vec();
        let digest = sha256_hex(&bytes);
        let installed = store
            .install(&bytes, &digest)
            .expect("install should succeed");
        assert_eq!(installed, digest);
        assert_eq!(store.load(&digest).unwrap(), bytes);
    }

    #[test]
    fn wrong_declared_digest_is_rejected() {
        let store = fresh_store();
        let bytes = b"module-bytes".to_vec();
        let error = store.install(&bytes, &"0".repeat(64)).unwrap_err();
        assert!(error.contains("mismatch"));
    }

    #[test]
    fn identical_reinstall_is_idempotent() {
        let store = fresh_store();
        let bytes = b"module-bytes".to_vec();
        let digest = sha256_hex(&bytes);
        store.install(&bytes, &digest).unwrap();
        store.install(&bytes, &digest).unwrap();
    }

    #[test]
    fn corrupted_stored_module_is_rejected_on_load() {
        let store = fresh_store();
        let bytes = b"module-bytes".to_vec();
        let digest = sha256_hex(&bytes);
        store.install(&bytes, &digest).unwrap();
        // Corrupt the stored file in place.
        std::fs::write(store.module_path(&digest), b"tampered").unwrap();
        let error = store.load(&digest).unwrap_err();
        assert!(error.contains("corruption"));
    }

    #[test]
    fn loading_an_uninstalled_digest_fails() {
        let store = fresh_store();
        let error = store.load(&"a".repeat(64)).unwrap_err();
        assert!(error.contains("read D-Code module"));
    }

    #[test]
    fn no_temp_file_remains_after_a_successful_install() {
        let store = fresh_store();
        let bytes = b"module-bytes".to_vec();
        let digest = sha256_hex(&bytes);
        store.install(&bytes, &digest).unwrap();
        let entries: Vec<_> = std::fs::read_dir(store.sha256_dir()).unwrap().collect();
        assert_eq!(
            entries.len(),
            1,
            "only the final content-addressed file should remain"
        );
    }
}
