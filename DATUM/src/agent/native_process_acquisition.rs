//! Stage 109 (ADR-0016). Managed filesystem layout and content-addressed
//! materialization for native-process executables.
//!
//! Portable: plain `std::fs`/`std::path` logic, no platform-specific code and
//! no `cfg(target_os = ...)` gating -- this module is exercised by ordinary
//! unit tests on any development host, not only Linux. Real process spawn/
//! signal/adoption (Linux-only) lives in `native_process_runtime.rs`.
//!
//! Layout under `native_process_root`:
//!
//! ```text
//! <root>/
//!   artifacts/
//!     <content-sha256>/
//!       <entrypoint>        (the verified, materialized executable bytes,
//!                            at the artifact's OWN declared relative
//!                            `entrypoint` path -- e.g. "run" or "bin/run",
//!                            never a fixed filename)
//!   state/
//!     <stable-service-key>.json
//!   logs/
//!     <stable-service-key>/
//!       stdout.log
//!       stderr.log
//! ```
//!
//! Every path under `native_process_root` is derived from either a
//! content-sha256 (already validated as 64 lowercase hex characters before
//! it is ever used as a path component), a stable service key computed by
//! this module from `(project_id, node_id, service_id)`, or the artifact's
//! own declared `entrypoint`/`working_directory` -- the latter two are
//! always validated for containment (`resolve_contained_path`) before use,
//! never concatenated unsanitized into a path.

use std::io::Write;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

pub const NATIVE_PROCESS_ACQUISITION_KIND: &str = "native_process_executable";

/// SHA-256 (hex, lowercase, unprefixed) of `bytes` -- the same bare-hex
/// content-hash domain `ServiceArtifactAcquisition.sha256` and
/// `ContainerConfigurationMount.declared_content_sha256` already use
/// (distinct from the `sha256:<hex>`-prefixed canonical DATUM contract
/// digest domain ADR-0006 governs).
pub fn sha256_hex_of_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut s = String::with_capacity(64);
    for byte in digest {
        s.push_str(&format!("{byte:02x}"));
    }
    s
}

/// The repository's canonical raw-content-hash representation: exactly 64
/// lowercase hex characters, no `sha256:` prefix (mirrors
/// `service_artifact_registry::validate_artifact`'s existing
/// `control_plane.acquisition.sha256` check).
pub fn is_valid_content_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// v3 fix (P1-2, agent-side mirror of DServer's authoring-time contract):
/// Stage 109 v0.1's native acquisition transport is a declared LOCAL LINUX
/// FILE -- `acquisition.source` must be a non-empty, NUL-free, bounded,
/// POSIX-absolute path. Deliberately pure string logic (not
/// `std::path::Path::is_absolute()`, matching this module's existing
/// `native_process_root`-style path checks): this module is exercised by
/// portable unit tests on any development host, and `is_absolute()`
/// would (incorrectly, for this purpose) reject an ordinary Unix-style
/// path like `/opt/datum/sources/x.bin` on a host with no concept of a
/// bare `/`-rooted path being absolute without a drive letter.
pub fn is_valid_posix_absolute_source_path(value: &str) -> bool {
    if value.is_empty() || value.len() > 4096 || value.contains('\0') || !value.starts_with('/') {
        return false;
    }
    value
        .split('/')
        .skip(1)
        .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

/// v4 fix (P1-2, agent-side WIRE validation -- `operational_reconciliation::validate_context`):
/// a Stage 109 `entrypoint`/`working_directory` is, by contract, a
/// relative path inside the materialized artifact directory. This is
/// deliberately a SEPARATE, pure-string check from
/// `validate_relative_path_component` (which is `std::path`-based and
/// used for the real, Linux-only materialized-path containment logic in
/// this module) -- wire-level validation must not depend on
/// `Path::is_absolute()`'s host-dependent semantics (it incorrectly
/// treats a bare Unix-style `/foo` path as non-absolute on a host with no
/// drive-letter concept), matching the identical reasoning already
/// applied to `is_valid_posix_absolute_source_path` above and to
/// DServer's own mirrored validators.
pub fn is_valid_posix_relative_path(value: &str) -> bool {
    if value.trim().is_empty() || value.len() > 4096 || value.contains('\0') {
        return false;
    }
    if value.starts_with('/') {
        return false;
    }
    value.split('/').all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathContainmentError(pub String);

impl std::fmt::Display for PathContainmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for PathContainmentError {}

/// Validate that `candidate` is a safe, relative path suitable for joining
/// under a managed root: non-empty, NUL-free, not absolute, and containing
/// only ordinary (`Normal`) path components -- no `..`, no bare `.`, no root
/// or prefix (Windows drive-letter) components. This rejects traversal,
/// absolute-path escape, and empty/invalid components uniformly.
pub fn validate_relative_path_component(candidate: &str, label: &str) -> Result<(), PathContainmentError> {
    let trimmed = candidate.trim();
    if trimmed.is_empty() {
        return Err(PathContainmentError(format!("{label} must not be empty")));
    }
    if candidate.contains('\0') {
        return Err(PathContainmentError(format!("{label} must not contain a NUL byte")));
    }
    let path = Path::new(candidate);
    if path.is_absolute() {
        return Err(PathContainmentError(format!(
            "{label} must be a relative path, got absolute path '{candidate}'"
        )));
    }
    if path.components().count() == 0 {
        return Err(PathContainmentError(format!("{label} must contain at least one path component")));
    }
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            Component::CurDir => {
                return Err(PathContainmentError(format!(
                    "{label} must not contain a '.' path component"
                )));
            }
            Component::ParentDir => {
                return Err(PathContainmentError(format!(
                    "{label} must not contain a '..' path component"
                )));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(PathContainmentError(format!(
                    "{label} must not contain a root or drive-prefix component"
                )));
            }
        }
    }
    Ok(())
}

/// Resolve `relative` against `root`, asserting the joined path stays
/// lexically within `root` AND that no component of the resolved path is
/// (or passes through) a symlink (Section AG items E/F). `relative` is
/// validated first (`validate_relative_path_component`).
///
/// Lexical containment alone is not sufficient: every path under the
/// managed root is indeed constructed by this module from a validated
/// content-sha256 or a stable, agent-computed key, but the managed root is
/// an ordinary directory on a real filesystem, and something OTHER than
/// this module (disk corruption, an operator mistake, a compromised
/// co-located process) could have replaced a path component with a
/// symlink pointing outside the managed root entirely. A symlinked final
/// path or symlinked ancestor directory is rejected outright, never
/// silently followed.
pub fn resolve_contained_path(root: &Path, relative: &str, label: &str) -> Result<PathBuf, PathContainmentError> {
    validate_relative_path_component(relative, label)?;
    let joined = root.join(relative);
    if !joined.starts_with(root) {
        return Err(PathContainmentError(format!(
            "{label} '{relative}' resolves outside the managed root"
        )));
    }
    if let Some(symlinked_at) = first_symlink_component(root, &joined) {
        return Err(PathContainmentError(format!(
            "{label} '{relative}' passes through a symlink at '{}'; refusing to follow a symlink under the managed root",
            symlinked_at.display()
        )));
    }
    Ok(joined)
}

/// Walk from `root` down to `joined` (inclusive), returning the first
/// ancestor (or `joined` itself) that already exists on disk as a symlink.
/// Components that do not exist yet are skipped -- there is nothing to
/// escape through until something is actually created there, and every
/// creation under the managed root goes through this same function first.
fn first_symlink_component(root: &Path, joined: &Path) -> Option<PathBuf> {
    let relative = joined.strip_prefix(root).ok()?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        if let Ok(metadata) = std::fs::symlink_metadata(&current) {
            if metadata.file_type().is_symlink() {
                return Some(current);
            }
        }
    }
    None
}

/// Deterministic, filesystem-safe per-service state/log key. Built ONLY from
/// already-validated identity strings (never raw artifact-controlled path
/// fragments): non-alphanumeric characters are replaced with `_`, and the
/// full original triple is preserved via a content hash suffix so two
/// distinct `(project_id, node_id, service_id)` triples can never collide
/// onto the same key even after sanitization.
pub fn stable_service_key(project_id: &str, node_id: &str, service_id: &str) -> String {
    fn sanitize(value: &str) -> String {
        value
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
            .collect()
    }
    let joined = format!("{project_id}\u{0}{node_id}\u{0}{service_id}");
    let suffix = &sha256_hex_of_bytes(joined.as_bytes())[..16];
    format!("{}-{}-{}-{}", sanitize(project_id), sanitize(node_id), sanitize(service_id), suffix)
}

pub struct ManagedLayout {
    root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterializeOutcome {
    /// Content already materialized under its content-addressed path with
    /// the expected hash AND already executable -- reused exactly as-is,
    /// with NO mutation at all: no bytes rewritten (N1 not crossed), no
    /// permission change (N2 not crossed) (Section AG-B).
    AlreadyPresent,
    /// Content already materialized with the expected hash, but its
    /// executable permission bit is missing/wrong. v4 fix (P1-1): this
    /// call did NOT repair it -- `materialize()` NEVER sets executable
    /// permissions under any circumstance, including this internal
    /// reclassification case (which exists specifically to detect a
    /// TOCTOU where a concurrent actor, possibly a DIFFERENT service
    /// sharing this content-addressed path, supplied the correct bytes
    /// between the caller's own classification and this call). The
    /// caller MUST perform its own fresh N2 `check_boundary` and then
    /// call `repair_executable_permission` itself -- the ONLY function
    /// permitted to ever mutate the executable bit, and only ever
    /// reached after a real, separate N2 check. (Renamed from
    /// `AlreadyPresentPermissionRepaired`, which incorrectly implied
    /// this function had already performed the repair.)
    AlreadyPresentPermissionRepairNeeded,
    /// Freshly written this call (nothing existed at the content-addressed
    /// path before), atomically (temp file + rename), then re-hashed and
    /// confirmed to match. `N1` applies to THIS write. The file is
    /// deliberately left NON-EXECUTABLE (P0-6 fix) -- setting the
    /// executable bit is `N2`'s job, performed as a separate, freshly
    /// lease-checked step by the caller (`governed_materialize_and_spawn`
    /// via `repair_executable_permission`), never bundled into this
    /// write.
    Written,
    /// Something existed at the content-addressed path, but it did NOT
    /// hash to the path's own declared sha256 (corruption) -- governed
    /// self-heal: content-addressing makes this unambiguous (the path
    /// names its own hash), so it is safe to atomically replace the
    /// corrupted bytes with the already independently-verified
    /// `source_bytes` via the same temp-file-then-rename mechanism used
    /// for a fresh write, rather than permanently refusing forever
    /// (Section AG-D). `N1` applies to THIS write; like `Written`, the
    /// file is left non-executable until the caller's separate `N2` step.
    RepairedCorruption,
}

/// A read-only (non-mutating) classification of what, if anything, would
/// need to happen to materialize `expected_sha256`/`entrypoint` -- computed
/// BEFORE any lease/boundary check, so a caller can skip checking (and
/// recording) a mutation boundary entirely when nothing would actually be
/// mutated (Section AG-B; Section M's "check the lease before EACH ACTUAL
/// mutation" implies no boundary check is owed for a true no-op reuse).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterializationNeed {
    /// Nothing to do: correct content, correct permissions already.
    None,
    /// Correct content already on disk, but its executable bit needs
    /// repair (Unix only; always `None` in this variant's place on other
    /// platforms, since there is no POSIX executable bit to be wrong).
    PermissionRepairOnly,
    /// No content at the expected path yet -- a fresh write is needed.
    FreshWrite,
    /// Content exists at the expected path but does not hash to it --
    /// corrupted; needs governed repair-replacement.
    RepairCorruption { actual_hash: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterializeError {
    Containment(String),
    Io(String),
    /// The source bytes' own hash did not match the declared/authorized
    /// sha256 -- never written into the content-addressed area under the
    /// WRONG hash.
    SourceHashMismatch { expected: String, actual: String },
    /// Something is already on disk at the content-addressed path for this
    /// hash, but re-hashing it does NOT match the hash the path itself
    /// names -- corruption. Never executed; never silently overwritten.
    ExistingContentCorrupted { path: String, expected: String, actual: String },
}

impl std::fmt::Display for MaterializeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MaterializeError::Containment(m) => write!(f, "path containment violation: {m}"),
            MaterializeError::Io(m) => write!(f, "io error: {m}"),
            MaterializeError::SourceHashMismatch { expected, actual } => write!(
                f,
                "acquired content hash mismatch: expected sha256 {expected}, actual {actual}"
            ),
            MaterializeError::ExistingContentCorrupted { path, expected, actual } => write!(
                f,
                "materialized content at '{path}' is corrupted: content-addressed path names sha256 {expected}, actual bytes hash to {actual}"
            ),
        }
    }
}

impl std::error::Error for MaterializeError {}

impl From<PathContainmentError> for MaterializeError {
    fn from(value: PathContainmentError) -> Self {
        MaterializeError::Containment(value.0)
    }
}

impl ManagedLayout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn artifact_directory(&self, content_sha256: &str) -> Result<PathBuf, PathContainmentError> {
        if !is_valid_content_sha256(content_sha256) {
            return Err(PathContainmentError(format!(
                "content_sha256 '{content_sha256}' is not a valid 64-character lowercase hex digest"
            )));
        }
        resolve_contained_path(&self.root, &format!("artifacts/{content_sha256}"), "content-addressed artifact directory")
    }

    /// The materialized executable's real path for `content_sha256` and the
    /// artifact's OWN declared `entrypoint` (Section F): `entrypoint` is a
    /// relative path INSIDE the content-addressed artifact directory,
    /// validated for containment exactly like every other managed-root path
    /// -- never an artifact-controlled absolute host path, never allowed to
    /// escape via `..`. Callers spawn THIS resolved path, never
    /// `acquisition.source`.
    pub fn materialized_entrypoint_path(&self, content_sha256: &str, entrypoint: &str) -> Result<PathBuf, PathContainmentError> {
        let dir = self.artifact_directory(content_sha256)?;
        resolve_contained_path(&dir, entrypoint, "entrypoint")
    }

    pub fn state_path(&self, stable_key: &str) -> Result<PathBuf, PathContainmentError> {
        resolve_contained_path(&self.root, &format!("state/{stable_key}.json"), "runtime-state path")
    }

    pub fn logs_dir(&self, stable_key: &str) -> Result<PathBuf, PathContainmentError> {
        resolve_contained_path(&self.root, &format!("logs/{stable_key}"), "logs directory")
    }

    /// Read-only classification (Section AG-B/C/D): what, if anything, a
    /// subsequent `materialize` call for this `(expected_sha256,
    /// entrypoint)` pair would actually need to do. Never mutates
    /// anything; safe to call before deciding which (if any) mutation
    /// boundary/lease check is owed.
    pub fn classify_materialization_need(
        &self,
        expected_sha256: &str,
        entrypoint: &str,
    ) -> Result<MaterializationNeed, MaterializeError> {
        let entrypoint_path = self.materialized_entrypoint_path(expected_sha256, entrypoint)?;
        if !entrypoint_path.exists() {
            return Ok(MaterializationNeed::FreshWrite);
        }
        let existing = std::fs::read(&entrypoint_path)
            .map_err(|e| MaterializeError::Io(format!("read existing materialized entrypoint: {e}")))?;
        let existing_hash = sha256_hex_of_bytes(&existing);
        if !existing_hash.eq_ignore_ascii_case(expected_sha256) {
            return Ok(MaterializationNeed::RepairCorruption { actual_hash: existing_hash });
        }
        if !is_executable(&entrypoint_path) {
            return Ok(MaterializationNeed::PermissionRepairOnly);
        }
        Ok(MaterializationNeed::None)
    }

    /// Section AG-C: repair ONLY the executable permission bit of an
    /// already-correctly-hashed materialized file -- never touches the
    /// bytes, never reads/needs the acquisition source. Callers must only
    /// invoke this when `classify_materialization_need` returned
    /// `PermissionRepairOnly` (re-checked here defensively: refuses if the
    /// on-disk content no longer hashes to `expected_sha256`, e.g. a
    /// concurrent change raced in between).
    pub fn repair_executable_permission(&self, expected_sha256: &str, entrypoint: &str) -> Result<(), MaterializeError> {
        let entrypoint_path = self.materialized_entrypoint_path(expected_sha256, entrypoint)?;
        let existing = std::fs::read(&entrypoint_path)
            .map_err(|e| MaterializeError::Io(format!("read existing materialized entrypoint: {e}")))?;
        let existing_hash = sha256_hex_of_bytes(&existing);
        if !existing_hash.eq_ignore_ascii_case(expected_sha256) {
            return Err(MaterializeError::ExistingContentCorrupted {
                path: entrypoint_path.to_string_lossy().to_string(),
                expected: expected_sha256.to_string(),
                actual: existing_hash,
            });
        }
        set_executable_permissions(&entrypoint_path).map_err(MaterializeError::Io)?;
        // P0-6 (v3 fix): "set executable permission" is followed by
        // "verify executable permission" -- never assume `chmod` took
        // effect without re-checking, the same defense-in-depth
        // discipline already applied to content hashes.
        if !is_executable(&entrypoint_path) {
            return Err(MaterializeError::Io(format!(
                "executable permission verification failed after chmod: '{}' is still not executable",
                entrypoint_path.display()
            )));
        }
        Ok(())
    }

    /// Idempotently materialize `source_bytes` (already read/acquired by the
    /// caller, from whatever acquisition mechanism `acquisition.kind`
    /// names) under the content-addressed artifact directory for
    /// `expected_sha256`.
    ///
    /// - Rejects `source_bytes` outright if they do not hash to
    ///   `expected_sha256` -- never writes content under the wrong hash.
    /// - Delegates to `classify_materialization_need` for the reuse/
    ///   repair/fresh-write decision (Section AG-B/C/D):
    ///     - `None`: returns `AlreadyPresent`, touching nothing.
    ///     - `PermissionRepairOnly`: NEVER repairs it (v4/P1-1 fix -- this
    ///       function never sets executable permissions, under any
    ///       circumstance); returns `AlreadyPresentPermissionRepairNeeded`
    ///       so the caller performs its own fresh N2 check and repair.
    ///       Bytes untouched either way.
    ///     - `FreshWrite`/`RepairCorruption`: writes atomically (a temp
    ///       file in the same directory, then rename into place),
    ///       deliberately leaving the file NON-EXECUTABLE (setting the
    ///       executable bit is exclusively N2's job, performed by the
    ///       caller via `repair_executable_permission` after its own
    ///       fresh boundary check -- never by this function), then
    ///       re-hashes the final materialized file to confirm it matches
    ///       before returning `Written` or `RepairedCorruption`
    ///       respectively. Never executed before this re-verification
    ///       passes.
    pub fn materialize(
        &self,
        expected_sha256: &str,
        entrypoint: &str,
        source_bytes: &[u8],
    ) -> Result<MaterializeOutcome, MaterializeError> {
        let actual_source_hash = sha256_hex_of_bytes(source_bytes);
        if !actual_source_hash.eq_ignore_ascii_case(expected_sha256) {
            return Err(MaterializeError::SourceHashMismatch {
                expected: expected_sha256.to_string(),
                actual: actual_source_hash,
            });
        }
        let entrypoint_path = self.materialized_entrypoint_path(expected_sha256, entrypoint)?;
        let dir = entrypoint_path.parent().expect("entrypoint path always has a parent under the managed root").to_path_buf();

        let was_repairing_corruption = match self.classify_materialization_need(expected_sha256, entrypoint)? {
            MaterializationNeed::None => return Ok(MaterializeOutcome::AlreadyPresent),
            // v4 fix (P1-1): `materialize()` MUST NEVER set executable
            // permissions -- not "normally does not," NEVER, including
            // here. This branch exists only to report the TOCTOU-safe
            // truth (correct bytes are already present; the executable
            // bit still needs setting) back to the caller, which must
            // perform its own fresh N2 boundary check and then call
            // `repair_executable_permission` itself. The only path that
            // is EVER allowed to mutate the executable bit is one reached
            // after a real, separate N2 check.
            MaterializationNeed::PermissionRepairOnly => {
                return Ok(MaterializeOutcome::AlreadyPresentPermissionRepairNeeded);
            }
            MaterializationNeed::FreshWrite => false,
            MaterializationNeed::RepairCorruption { .. } => true,
        };

        // P0-6 (v3 fix): the freshly-written/repaired file is deliberately
        // left NON-EXECUTABLE here -- setting the executable bit is N2's
        // job, not N1's. A boundary check does not retroactively govern a
        // mutation that already happened; `governed_materialize_and_spawn`
        // performs its own fresh N2 lease check and THEN calls
        // `repair_executable_permission` (or the platform no-op) as an
        // explicit, separate step. If N2 blocks, the file that N1 wrote
        // remains on disk but non-executable, and `Command::spawn()`
        // would refuse to execute it (POSIX `EACCES`) even if somehow
        // reached -- defense in depth on top of N3's own lease check.
        std::fs::create_dir_all(&dir).map_err(|e| MaterializeError::Io(format!("create artifact directory: {e}")))?;
        let tmp_path = dir.join(format!(".entrypoint.tmp.{}", std::process::id()));
        {
            let mut file = std::fs::File::create(&tmp_path)
                .map_err(|e| MaterializeError::Io(format!("create temp materialization file: {e}")))?;
            file.write_all(source_bytes)
                .map_err(|e| MaterializeError::Io(format!("write temp materialization file: {e}")))?;
            file.sync_all().map_err(|e| MaterializeError::Io(format!("sync temp materialization file: {e}")))?;
        }
        std::fs::rename(&tmp_path, &entrypoint_path)
            .map_err(|e| MaterializeError::Io(format!("rename materialized entrypoint into place: {e}")))?;

        let final_bytes = std::fs::read(&entrypoint_path)
            .map_err(|e| MaterializeError::Io(format!("read final materialized entrypoint: {e}")))?;
        let final_hash = sha256_hex_of_bytes(&final_bytes);
        if !final_hash.eq_ignore_ascii_case(expected_sha256) {
            return Err(MaterializeError::ExistingContentCorrupted {
                path: entrypoint_path.to_string_lossy().to_string(),
                expected: expected_sha256.to_string(),
                actual: final_hash,
            });
        }
        Ok(if was_repairing_corruption { MaterializeOutcome::RepairedCorruption } else { MaterializeOutcome::Written })
    }

    /// Re-verify a previously materialized executable's on-disk bytes
    /// against `expected_sha256` -- used immediately before every spawn
    /// (fresh start or restart), never trusting "materialize() succeeded
    /// earlier" alone.
    pub fn verify_materialized(&self, expected_sha256: &str, entrypoint: &str) -> Result<(), MaterializeError> {
        let path = self.materialized_entrypoint_path(expected_sha256, entrypoint)?;
        let bytes = std::fs::read(&path).map_err(|e| MaterializeError::Io(format!("read materialized entrypoint: {e}")))?;
        let actual = sha256_hex_of_bytes(&bytes);
        if actual.eq_ignore_ascii_case(expected_sha256) {
            Ok(())
        } else {
            Err(MaterializeError::ExistingContentCorrupted {
                path: path.to_string_lossy().to_string(),
                expected: expected_sha256.to_string(),
                actual,
            })
        }
    }
}

#[cfg(unix)]
fn set_executable_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path).map_err(|e| format!("stat temp file for permissions: {e}"))?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).map_err(|e| format!("set executable permissions: {e}"))
}

#[cfg(not(unix))]
fn set_executable_permissions(_path: &Path) -> Result<(), String> {
    // No POSIX executable bit on this platform; Stage 109's real spawn path
    // is Linux-only (see native_process_runtime.rs's platform boundary) --
    // this is a harmless no-op elsewhere, exercised only by portable
    // materialization tests, never by real execution.
    Ok(())
}

/// Section AG-C: is the file at `path` already executable by its owner?
/// Unix-only meaningfully -- on any other platform there is no POSIX
/// executable bit to be wrong, so this always reports `true` (nothing to
/// repair), matching `set_executable_permissions`'s no-op there.
#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|metadata| metadata.permissions().mode() & 0o100 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;

    fn temp_dir(label: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "datum-native-acq-test-{label}-{}-{}",
            std::process::id(),
            sha256_hex_of_bytes(label.as_bytes())
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sha256_matches_known_abc_vector() {
        assert_eq!(
            sha256_hex_of_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn valid_content_sha256_accepts_64_lowercase_hex() {
        assert!(is_valid_content_sha256(&"a".repeat(64)));
        assert!(!is_valid_content_sha256(&"A".repeat(64)));
        assert!(!is_valid_content_sha256(&"a".repeat(63)));
        assert!(!is_valid_content_sha256("sha256:aaaa"));
    }

    // ---- v4 (P1-2): wire-level POSIX path validators -------------------- //

    #[test]
    fn is_valid_posix_relative_path_accepts_ordinary_relative_shapes() {
        assert!(is_valid_posix_relative_path("run"));
        assert!(is_valid_posix_relative_path("bin/run"));
        assert!(is_valid_posix_relative_path("bin/nested/run"));
    }

    #[test]
    fn is_valid_posix_relative_path_rejects_absolute_and_traversal_shapes() {
        assert!(!is_valid_posix_relative_path("/etc/passwd"));
        assert!(!is_valid_posix_relative_path("../escape"));
        assert!(!is_valid_posix_relative_path("bin/../escape"));
        assert!(!is_valid_posix_relative_path("./run"));
        assert!(!is_valid_posix_relative_path(""));
        assert!(!is_valid_posix_relative_path("   "));
        assert!(!is_valid_posix_relative_path("bin//run"));
        assert!(!is_valid_posix_relative_path("bad\0path"));
    }

    #[test]
    fn is_valid_posix_absolute_source_path_accepts_and_rejects_expected_shapes() {
        assert!(is_valid_posix_absolute_source_path("/opt/datum/sources/native-svc.bin"));
        assert!(!is_valid_posix_absolute_source_path("relative/sources/native-svc.bin"));
        assert!(!is_valid_posix_absolute_source_path("/opt/../etc/passwd"));
        assert!(!is_valid_posix_absolute_source_path(""));
    }

    #[test]
    fn relative_path_component_accepts_ordinary_relative_path() {
        validate_relative_path_component("bin/run", "entrypoint").unwrap();
    }

    #[test]
    fn relative_path_component_rejects_absolute_path() {
        assert!(validate_relative_path_component("/etc/passwd", "entrypoint").is_err());
    }

    #[test]
    fn relative_path_component_rejects_parent_traversal() {
        assert!(validate_relative_path_component("../../etc/passwd", "entrypoint").is_err());
        assert!(validate_relative_path_component("bin/../../etc/passwd", "entrypoint").is_err());
    }

    #[test]
    fn relative_path_component_rejects_empty_and_nul() {
        assert!(validate_relative_path_component("", "entrypoint").is_err());
        assert!(validate_relative_path_component("bad\0path", "entrypoint").is_err());
    }

    #[test]
    fn relative_path_component_rejects_bare_current_dir_component() {
        assert!(validate_relative_path_component("./run", "entrypoint").is_err());
    }

    #[test]
    fn resolve_contained_path_stays_inside_root() {
        let root = temp_dir("contain-ok");
        let resolved = resolve_contained_path(&root, "artifacts/abc", "x").unwrap();
        assert!(resolved.starts_with(&root));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn stable_service_key_is_deterministic_and_distinguishes_inputs() {
        let a = stable_service_key("proj", "fog-01", "svc-a");
        let b = stable_service_key("proj", "fog-01", "svc-a");
        let c = stable_service_key("proj", "fog-01", "svc-b");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn materialize_writes_and_reuses_idempotently() {
        let root = temp_dir("materialize-ok");
        let layout = ManagedLayout::new(&root);
        let bytes = b"#!/bin/sh\necho hi\n";
        let hash = sha256_hex_of_bytes(bytes);

        let first = layout.materialize(&hash, "run", bytes).unwrap();
        assert_eq!(first, MaterializeOutcome::Written);

        // v6 fix (real Linux validation): on Unix, N1's fresh write (above)
        // deliberately leaves the file NON-EXECUTABLE -- setting the
        // executable bit is N2's job, a separate, freshly-checked step
        // (`repair_executable_permission`), never bundled into `materialize()`
        // itself. A second `materialize()` call before that separate N2 step
        // therefore correctly reports `AlreadyPresentPermissionRepairNeeded`
        // (correct bytes, permission still needs repair) -- NOT `AlreadyPresent`
        // (which would incorrectly claim nothing needs to happen). Only AFTER
        // performing the real N2 repair does a third call see a true, full
        // no-op reuse. On a non-Unix host (this test's remaining assertions
        // stay portable) `is_executable`/`set_executable_permissions` are
        // no-ops that always report "already executable" -- there is no real
        // POSIX permission bit to be wrong, so a second call there correctly
        // reports `AlreadyPresent` directly, with no separate repair step.
        let second = layout.materialize(&hash, "run", bytes).unwrap();
        #[cfg(unix)]
        {
            assert_eq!(second, MaterializeOutcome::AlreadyPresentPermissionRepairNeeded);
            layout.repair_executable_permission(&hash, "run").unwrap();
            let third = layout.materialize(&hash, "run", bytes).unwrap();
            assert_eq!(third, MaterializeOutcome::AlreadyPresent);
        }
        #[cfg(not(unix))]
        {
            assert_eq!(second, MaterializeOutcome::AlreadyPresent);
        }

        let path = layout.materialized_entrypoint_path(&hash, "run").unwrap();
        let mut on_disk = Vec::new();
        std::fs::File::open(&path).unwrap().read_to_end(&mut on_disk).unwrap();
        assert_eq!(on_disk, bytes);

        layout.verify_materialized(&hash, "run").unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn materialize_leaves_a_fresh_write_non_executable_until_the_separate_n2_step() {
        use std::os::unix::fs::PermissionsExt;
        let root = temp_dir("materialize-n1-n2-split");
        let layout = ManagedLayout::new(&root);
        let bytes = b"#!/bin/sh\necho hi\n";
        let hash = sha256_hex_of_bytes(bytes);

        let outcome = layout.materialize(&hash, "run", bytes).unwrap();
        assert_eq!(outcome, MaterializeOutcome::Written);
        let path = layout.materialized_entrypoint_path(&hash, "run").unwrap();
        let mode_after_n1 = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode_after_n1 & 0o100, 0, "N1 must never leave the file executable");

        assert_eq!(
            layout.classify_materialization_need(&hash, "run").unwrap(),
            MaterializationNeed::PermissionRepairOnly
        );

        layout.repair_executable_permission(&hash, "run").unwrap();
        let mode_after_n2 = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_ne!(mode_after_n2 & 0o100, 0, "N2 must make the file executable");

        // Bytes must be byte-for-byte unchanged by the separate N2 step.
        let on_disk = std::fs::read(&path).unwrap();
        assert_eq!(on_disk, bytes);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn materialize_rejects_source_bytes_not_matching_declared_hash() {
        let root = temp_dir("materialize-mismatch");
        let layout = ManagedLayout::new(&root);
        let bytes = b"actual content";
        let wrong_hash = "a".repeat(64);
        let error = layout.materialize(&wrong_hash, "run", bytes).unwrap_err();
        assert!(matches!(error, MaterializeError::SourceHashMismatch { .. }));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn materialize_detects_and_governed_repairs_corrupted_existing_content() {
        let root = temp_dir("materialize-corrupt");
        let layout = ManagedLayout::new(&root);
        let bytes = b"original bytes";
        let hash = sha256_hex_of_bytes(bytes);
        layout.materialize(&hash, "run", bytes).unwrap();

        // Corrupt the on-disk file directly (simulating disk corruption /
        // tampering). `verify_materialized` (the read-only defense-in-depth
        // check called immediately before every spawn) must detect this
        // and refuse -- corrupted content is NEVER executed as-is.
        let path = layout.materialized_entrypoint_path(&hash, "run").unwrap();
        std::fs::write(&path, b"corrupted").unwrap();
        let verify_error = layout.verify_materialized(&hash, "run").unwrap_err();
        assert!(matches!(verify_error, MaterializeError::ExistingContentCorrupted { .. }));

        // Section AG-D: `materialize` itself, given the SAME already
        // independently-verified correct `source_bytes`, performs a
        // governed self-heal -- content-addressing makes this safe (the
        // path names its own hash, so a mismatch there is unambiguous
        // corruption of OUR bytes, never another service's legitimate
        // content) -- rather than refusing forever.
        let outcome = layout.materialize(&hash, "run", bytes).unwrap();
        assert_eq!(outcome, MaterializeOutcome::RepairedCorruption);
        layout.verify_materialized(&hash, "run").unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn materialize_never_repairs_corruption_using_the_wrong_bytes() {
        // The repair path is gated by the SAME source-hash check as a
        // fresh write -- corruption is never "fixed" using bytes that
        // don't themselves match the declared hash.
        let root = temp_dir("materialize-corrupt-wrong-bytes");
        let layout = ManagedLayout::new(&root);
        let bytes = b"original bytes";
        let hash = sha256_hex_of_bytes(bytes);
        layout.materialize(&hash, "run", bytes).unwrap();
        let path = layout.materialized_entrypoint_path(&hash, "run").unwrap();
        std::fs::write(&path, b"corrupted").unwrap();

        let error = layout.materialize(&hash, "run", b"not the right bytes either").unwrap_err();
        assert!(matches!(error, MaterializeError::SourceHashMismatch { .. }));
        // The corrupted file on disk must remain untouched by the rejected
        // attempt.
        let on_disk = std::fs::read(&path).unwrap();
        assert_eq!(on_disk, b"corrupted");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn content_addressed_path_rejects_invalid_hash_shape() {
        let root = temp_dir("bad-hash-shape");
        let layout = ManagedLayout::new(&root);
        assert!(layout.materialized_entrypoint_path("not-a-hash", "run").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- Section F: entrypoint/working-directory containment ------------- //

    #[test]
    fn valid_relative_entrypoint_resolves_inside_the_artifact_directory() {
        let root = temp_dir("entrypoint-valid");
        let layout = ManagedLayout::new(&root);
        let hash = "e".repeat(64);
        let path = layout.materialized_entrypoint_path(&hash, "bin/run").unwrap();
        let expected_dir = layout.artifact_directory(&hash).unwrap();
        assert!(path.starts_with(&expected_dir));
        assert_eq!(path, expected_dir.join("bin").join("run"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn absolute_entrypoint_is_rejected() {
        let root = temp_dir("entrypoint-absolute");
        let layout = ManagedLayout::new(&root);
        let hash = "e".repeat(64);
        assert!(layout.materialized_entrypoint_path(&hash, "/etc/passwd").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn entrypoint_parent_traversal_escape_is_rejected() {
        let root = temp_dir("entrypoint-escape");
        let layout = ManagedLayout::new(&root);
        let hash = "e".repeat(64);
        assert!(layout.materialized_entrypoint_path(&hash, "../../etc/passwd").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn materialize_writes_to_the_declared_entrypoint_relative_path() {
        let root = temp_dir("entrypoint-custom-path");
        let layout = ManagedLayout::new(&root);
        let bytes = b"#!/bin/sh\necho hi\n";
        let hash = sha256_hex_of_bytes(bytes);
        layout.materialize(&hash, "bin/run", bytes).unwrap();
        let path = layout.materialized_entrypoint_path(&hash, "bin/run").unwrap();
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn working_directory_escape_via_resolve_contained_path_is_rejected() {
        let root = temp_dir("workdir-escape");
        let layout = ManagedLayout::new(&root);
        let hash = "e".repeat(64);
        let artifact_dir = layout.artifact_directory(&hash).unwrap();
        assert!(resolve_contained_path(&artifact_dir, "../../etc", "working_directory").is_err());
        assert!(resolve_contained_path(&artifact_dir, "/etc", "working_directory").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- Section AG-E/F: symlink containment ----------------------------- //

    #[cfg(unix)]
    #[test]
    fn resolve_contained_path_rejects_a_final_path_that_is_a_symlink() {
        let root = temp_dir("symlink-final");
        let outside = temp_dir("symlink-final-outside");
        std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
        let error = resolve_contained_path(&root, "escape", "x").unwrap_err();
        assert!(error.0.contains("symlink"), "error was: {}", error.0);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[cfg(unix)]
    #[test]
    fn resolve_contained_path_rejects_a_symlinked_ancestor_directory() {
        let root = temp_dir("symlink-ancestor");
        let outside = temp_dir("symlink-ancestor-outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("artifacts")).unwrap();
        let error = resolve_contained_path(&root, "artifacts/deadbeef/run", "x").unwrap_err();
        assert!(error.0.contains("symlink"), "error was: {}", error.0);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[cfg(unix)]
    #[test]
    fn materialize_refuses_to_write_through_a_symlinked_artifact_directory() {
        let root = temp_dir("symlink-materialize");
        let outside = temp_dir("symlink-materialize-outside");
        std::fs::create_dir_all(&outside).unwrap();
        let layout = ManagedLayout::new(&root);
        let bytes = b"#!/bin/sh\necho hi\n";
        let hash = sha256_hex_of_bytes(bytes);
        std::fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("artifacts")).unwrap();
        let error = layout.materialize(&hash, "run", bytes).unwrap_err();
        assert!(matches!(error, MaterializeError::Containment(_)));
        // Confirm nothing was ever written into the symlink target either.
        assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    // ---- Section AG-C: permission repair ---------------------------------- //

    #[cfg(unix)]
    #[test]
    fn materialize_never_repairs_permission_itself_even_when_bytes_are_already_correct() {
        // v4 fix (P1-1): `materialize()` must NEVER set executable
        // permissions, under any circumstance -- including this internal
        // reclassification case (a defense against a TOCTOU where a
        // concurrent actor sharing this content-addressed path already
        // supplied the correct bytes). Only `repair_executable_permission`,
        // called by the CALLER after its own fresh N2 boundary check, may
        // ever mutate the executable bit.
        use std::os::unix::fs::PermissionsExt;
        let root = temp_dir("permission-repair-toctou");
        let layout = ManagedLayout::new(&root);
        let bytes = b"#!/bin/sh\necho hi\n";
        let hash = sha256_hex_of_bytes(bytes);
        layout.materialize(&hash, "run", bytes).unwrap();
        let path = layout.materialized_entrypoint_path(&hash, "run").unwrap();

        // Strip the executable bit (simulating a restrictive umask/restore
        // that lost it, or a concurrent actor that wrote correct bytes but
        // left them non-executable) while leaving the correct bytes
        // untouched.
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o644);
        std::fs::set_permissions(&path, perms).unwrap();

        let need = layout.classify_materialization_need(&hash, "run").unwrap();
        assert_eq!(need, MaterializationNeed::PermissionRepairOnly);

        // Calling `materialize()` directly (as if it were still mid-N1,
        // with N2 not yet reached) must NOT touch the permission bit.
        let outcome = layout.materialize(&hash, "run", bytes).unwrap();
        assert_eq!(outcome, MaterializeOutcome::AlreadyPresentPermissionRepairNeeded);

        let mode_after_materialize = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode_after_materialize & 0o100, 0, "materialize() must NEVER set the executable bit, even in this reclassification case");

        // Only the SEPARATE, N2-gated call actually repairs it.
        layout.repair_executable_permission(&hash, "run").unwrap();
        let repaired_mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_ne!(repaired_mode & 0o100, 0, "repair_executable_permission must set the executable bit");
        // Bytes must be byte-for-byte unchanged throughout.
        let on_disk = std::fs::read(&path).unwrap();
        assert_eq!(on_disk, bytes);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn classify_materialization_need_reports_none_when_already_correct_and_executable() {
        let root = temp_dir("classify-none");
        let layout = ManagedLayout::new(&root);
        let bytes = b"#!/bin/sh\necho hi\n";
        let hash = sha256_hex_of_bytes(bytes);
        layout.materialize(&hash, "run", bytes).unwrap();
        // P0-6 (v3 fix): a fresh write (N1) alone leaves the file
        // non-executable -- the N2 step must run separately before a
        // subsequent classification reports a true no-op reuse.
        layout.repair_executable_permission(&hash, "run").unwrap();
        assert_eq!(
            layout.classify_materialization_need(&hash, "run").unwrap(),
            MaterializationNeed::None
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn classify_materialization_need_reports_fresh_write_when_absent() {
        let root = temp_dir("classify-fresh");
        let layout = ManagedLayout::new(&root);
        let hash = "c".repeat(64);
        assert_eq!(
            layout.classify_materialization_need(&hash, "run").unwrap(),
            MaterializationNeed::FreshWrite
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn working_directory_valid_relative_path_resolves_inside_artifact_directory() {
        let root = temp_dir("workdir-valid");
        let layout = ManagedLayout::new(&root);
        let hash = "e".repeat(64);
        let artifact_dir = layout.artifact_directory(&hash).unwrap();
        let resolved = resolve_contained_path(&artifact_dir, "data", "working_directory").unwrap();
        assert!(resolved.starts_with(&artifact_dir));
        let _ = std::fs::remove_dir_all(&root);
    }
}
