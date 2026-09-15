//! Local, per-installation D-Node runtime identity (ADR-0021, independent
//! review v4, G1). This answers a question canonical DServer authorization
//! cannot answer by itself: "which logical D-Node IS THIS PROCESS?"
//!
//! Root cause this closes: prior to G1, the governed CLI accepted a raw
//! `--node-id` argument and used the SAME caller-supplied value both as the
//! node DServer was asked to authorize AND as the local identity checked
//! against that authorization inside `invoke::invoke_dcode` -- a circular
//! proof. It showed "DServer authorized the node ID the caller requested,"
//! never "this process is actually the D-Node named by that authorization."
//! A process physically running as `cloud-01` could pass `--node-id fog-01`,
//! be correctly authorized for `fog-01`, pass the local equality check, and
//! execute a module DMap actually placed on `fog-01` -- while physically
//! running in the wrong runtime.
//!
//! G1A (existing-source audit, see `local-dnode-identity-audit.txt` for the
//! full table): no existing source in this repository is safe to reuse here.
//! `config::AgentConfig.node_id` is Sentinel/agent OPERATIONAL configuration
//! (mutable by editing a config file, sits next to a free-text `stage`
//! field, and is the Sentinel's own registration identity) -- turning it into
//! canonical D-Node authority would violate the explicit architectural rule
//! that D-Node and Sentinel remain separate concepts. `agent::bootstrap`'s
//! `resolved_node_id` is likewise LIVE, negotiated Sentinel-bootstrap state
//! (resource_id/instance_id/hardware_id resolution against DServer's
//! historical resource registry), not a stable local structural identity
//! independent of Sentinel lease/bootstrap. Neither is reused. This module
//! introduces the smallest new, deliberately separate mechanism instead.
//!
//! This is OPERATIONAL local state, exactly like `dcode::module_store`:
//! - NOT part of the canonical `datum.dgraph/1` / `datum.dmap/1` /
//!   `datum.dcontinuum/1` contracts;
//! - NOT Sentinel registration/lease/liveness;
//! - carries no platform/capacity/stage/telemetry fields -- only a bare
//!   `node_id`;
//! - explicitly initialized once (`smartsentinel-dcode-init`), never
//!   implicitly created by the first invocation request;
//! - idempotent on identical re-initialization, fails closed on a
//!   conflicting one;
//! - validated on every read; a corrupted or malformed file fails closed and
//!   is NEVER silently repaired or defaulted.
//!
//! One physical host MAY represent multiple logical D-Nodes: each distinct
//! `dcode-runtime-root` (the `--store-root`/`--dnode-root` directory the
//! governed CLI is pointed at) carries its own independent local identity
//! file, so running two governed invocations against two different roots on
//! the same host resolves two different `local_node_id` values.
//!
//! Independent review v5 (H3): the init flow below (load/check-absent ->
//! write-temp -> atomic rename) has a TOCTOU race between two CONCURRENT
//! processes/threads both targeting the same root -- both could observe
//! "absent" before either writes, defeating the "conflicting re-init fails
//! closed" guarantee. DATUM targets Linux, so this module does not rely on
//! any assumption about `rename`'s behavior toward an existing destination on
//! other platforms being safe. `init_local_identity` now serializes the
//! entire read-check-write-rename-verify sequence across processes/threads
//! via a dedicated exclusive lock file (`dnode-local-identity.lock`, reusing
//! this codebase's existing `fs2::FileExt` pattern -- see
//! `agent::operational_evidence::append_operational_evidence` for the same
//! open+`lock_exclusive`+work+`unlock` shape). The lock is intentionally
//! BLOCKING (not `try_lock_exclusive`): the loser of a race must WAIT for the
//! winner to finish, then make its own decision against whatever is now
//! actually on disk, so exactly one `node_id` value wins deterministically
//! and every other concurrent caller either matches it (idempotent success)
//! or fails closed against it (conflicting re-init) -- never a scenario where
//! two callers could both believe they "won." The lock lives in its own
//! dedicated file, never inside the canonical identity file itself, and is
//! unrelated to any Sentinel instance lock.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

pub const DNODE_LOCAL_IDENTITY_SCHEMA: &str = "datum.dnode-local-identity/1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DNodeLocalIdentityV1 {
    pub schema: String,
    pub node_id: String,
}

fn identity_path(root: &Path) -> PathBuf {
    root.join("dnode-local-identity.json")
}

fn identity_lock_path(root: &Path) -> PathBuf {
    root.join("dnode-local-identity.lock")
}

/// Explicitly initializes (or re-confirms) this installation's local D-Node
/// identity. Re-initializing with the IDENTICAL `node_id` is a no-op success
/// (idempotent). Re-initializing with a DIFFERENT `node_id` than what is
/// already recorded fails closed -- this mechanism never silently overwrites
/// an existing local identity. Serialized across concurrent processes/threads
/// via an exclusive file lock (H3); see the module doc comment above.
pub fn init_local_identity(root: &Path, node_id: &str) -> Result<(), String> {
    if node_id.trim().is_empty() {
        return Err("local D-Node identity node_id must not be empty".to_string());
    }

    fs::create_dir_all(root).map_err(|error| format!("create D-Node local identity root {}: {error}", root.display()))?;

    let lock_file_path = identity_lock_path(root);
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(&lock_file_path)
        .map_err(|error| format!("open D-Node local identity lock {}: {error}", lock_file_path.display()))?;
    lock_file
        .lock_exclusive()
        .map_err(|error| format!("acquire D-Node local identity lock {}: {error}", lock_file_path.display()))?;

    let result = init_local_identity_while_locked(root, node_id);

    let _ = FileExt::unlock(&lock_file);
    result
}

/// The actual init logic -- executed ONLY while `init_local_identity` holds
/// the exclusive cross-process lock above. Never call this directly.
fn init_local_identity_while_locked(root: &Path, node_id: &str) -> Result<(), String> {
    if let Ok(existing) = load_local_identity(root) {
        if existing.node_id == node_id {
            return Ok(());
        }
        return Err(format!(
            "local D-Node identity already initialized as '{}' at {}; refusing to silently overwrite with '{node_id}' -- this is a conflicting re-initialization, not an update",
            existing.node_id,
            identity_path(root).display()
        ));
    }
    // `load_local_identity` above already distinguished "file absent" from
    // "file present but corrupt" by returning Err in both cases; re-check
    // explicitly here so a CORRUPT existing file is never silently
    // overwritten by init either -- only a genuinely ABSENT file may be
    // freshly created.
    let path = identity_path(root);
    if path.is_file() {
        return Err(format!(
            "a local D-Node identity file already exists at {} but failed to load/validate; refusing to overwrite -- inspect or remove it manually before re-initializing",
            path.display()
        ));
    }

    let identity = DNodeLocalIdentityV1 { schema: DNODE_LOCAL_IDENTITY_SCHEMA.to_string(), node_id: node_id.to_string() };
    let bytes = serde_json::to_vec_pretty(&identity).map_err(|error| format!("serialize local D-Node identity: {error}"))?;

    // Independent review v5 (H3): the tmp filename embeds this process's PID,
    // which two THREADS in the same process would share -- harmless here
    // because the exclusive lock above already guarantees only one
    // process/thread executes this whole function body at a time, so the tmp
    // path is always free again by the time a second caller reaches this
    // point.
    let tmp_path = root.join(format!(".dnode-local-identity.{}.tmp", std::process::id()));
    {
        let mut file = fs::File::create(&tmp_path)
            .map_err(|error| format!("create temp local D-Node identity file {}: {error}", tmp_path.display()))?;
        file.write_all(&bytes)
            .map_err(|error| format!("write temp local D-Node identity file {}: {error}", tmp_path.display()))?;
        file.sync_all().map_err(|error| format!("fsync temp local D-Node identity file {}: {error}", tmp_path.display()))?;
    }
    fs::rename(&tmp_path, &path).map_err(|error| {
        let _ = fs::remove_file(&tmp_path);
        format!("atomically install local D-Node identity {}: {error}", path.display())
    })?;

    // H3: verify what is actually on disk after the rename, still while
    // holding the lock -- defense in depth against any platform-specific
    // rename anomaly silently producing a different final identity than the
    // one this call intended to install.
    let verify = load_local_identity(root)?;
    if verify.node_id != node_id {
        return Err(format!(
            "local D-Node identity verification after write mismatched: wrote '{node_id}', read back '{}'",
            verify.node_id
        ));
    }
    Ok(())
}

/// Loads and validates this installation's local D-Node identity. Fails
/// closed (never silently repairs or defaults) on: a missing file, malformed
/// JSON, an unsupported schema, or an empty `node_id`.
pub fn load_local_identity(root: &Path) -> Result<DNodeLocalIdentityV1, String> {
    let path = identity_path(root);
    let bytes = fs::read(&path).map_err(|error| format!("read local D-Node identity {}: {error}", path.display()))?;
    let identity: DNodeLocalIdentityV1 = serde_json::from_slice(&bytes)
        .map_err(|error| format!("local D-Node identity {} is not valid JSON: {error}", path.display()))?;
    if identity.schema != DNODE_LOCAL_IDENTITY_SCHEMA {
        return Err(format!(
            "local D-Node identity {} declares unsupported schema '{}', expected '{DNODE_LOCAL_IDENTITY_SCHEMA}'",
            path.display(),
            identity.schema
        ));
    }
    if identity.node_id.trim().is_empty() {
        return Err(format!("local D-Node identity {} has an empty node_id", path.display()));
    }
    Ok(identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn fresh_root() -> PathBuf {
        std::env::temp_dir().join(format!("datum-dnode-local-identity-{}", Uuid::new_v4()))
    }

    #[test]
    fn init_then_load_round_trips() {
        let root = fresh_root();
        init_local_identity(&root, "fog-01").unwrap();
        let identity = load_local_identity(&root).unwrap();
        assert_eq!(identity.node_id, "fog-01");
        assert_eq!(identity.schema, DNODE_LOCAL_IDENTITY_SCHEMA);
    }

    #[test]
    fn identical_reinit_is_idempotent() {
        let root = fresh_root();
        init_local_identity(&root, "fog-01").unwrap();
        init_local_identity(&root, "fog-01").unwrap();
        assert_eq!(load_local_identity(&root).unwrap().node_id, "fog-01");
    }

    /// G1C-4: a conflicting local identity re-init fails closed.
    #[test]
    fn conflicting_reinit_fails_closed() {
        let root = fresh_root();
        init_local_identity(&root, "fog-01").unwrap();
        let error = init_local_identity(&root, "cloud-01").unwrap_err();
        assert!(error.contains("conflicting"));
        // The original identity must remain untouched.
        assert_eq!(load_local_identity(&root).unwrap().node_id, "fog-01");
    }

    #[test]
    fn empty_node_id_is_rejected() {
        let root = fresh_root();
        let error = init_local_identity(&root, "").unwrap_err();
        assert!(error.contains("must not be empty"));
    }

    #[test]
    fn loading_never_initialized_identity_fails_closed() {
        let root = fresh_root();
        let error = load_local_identity(&root).unwrap_err();
        assert!(error.contains("read local D-Node identity"));
    }

    /// G1C-5: a corrupted local identity file fails closed rather than being
    /// silently repaired or defaulted.
    #[test]
    fn corrupted_identity_file_fails_closed_on_load() {
        let root = fresh_root();
        init_local_identity(&root, "fog-01").unwrap();
        fs::write(identity_path(&root), b"not json at all").unwrap();
        let error = load_local_identity(&root).unwrap_err();
        assert!(error.contains("not valid JSON"));
    }

    #[test]
    fn unsupported_schema_fails_closed_on_load() {
        let root = fresh_root();
        fs::create_dir_all(&root).unwrap();
        fs::write(identity_path(&root), br#"{"schema":"datum.dnode-local-identity/9","node_id":"fog-01"}"#).unwrap();
        let error = load_local_identity(&root).unwrap_err();
        assert!(error.contains("unsupported schema"));
    }

    #[test]
    fn empty_node_id_in_file_fails_closed_on_load() {
        let root = fresh_root();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            identity_path(&root),
            format!(r#"{{"schema":"{DNODE_LOCAL_IDENTITY_SCHEMA}","node_id":""}}"#),
        )
        .unwrap();
        let error = load_local_identity(&root).unwrap_err();
        assert!(error.contains("empty node_id"));
    }

    /// G1C-6: one host MAY represent multiple logical D-Nodes -- proven here
    /// as two entirely independent roots, each with its own distinct local
    /// identity, neither influencing the other.
    #[test]
    fn two_distinct_runtime_roots_hold_two_independent_local_identities() {
        let root_a = fresh_root();
        let root_b = fresh_root();
        init_local_identity(&root_a, "fog-01").unwrap();
        init_local_identity(&root_b, "cloud-01").unwrap();
        assert_eq!(load_local_identity(&root_a).unwrap().node_id, "fog-01");
        assert_eq!(load_local_identity(&root_b).unwrap().node_id, "cloud-01");
    }

    // -- H3: cross-process/thread initialization race hardening -------------

    /// H3 regression 1: many THREADS racing `init_local_identity` against the
    /// SAME root with the IDENTICAL `node_id` all succeed (idempotent), and
    /// exactly the expected identity is left on disk. A `Barrier` maximizes
    /// the chance every thread actually contends for the lock at the same
    /// instant, rather than merely running sequentially by luck.
    #[test]
    fn concurrent_identical_init_remains_idempotent() {
        use std::sync::{Arc, Barrier};
        let root = Arc::new(fresh_root());
        const THREADS: usize = 8;
        let barrier = Arc::new(Barrier::new(THREADS));
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let root = Arc::clone(&root);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    init_local_identity(&root, "fog-01")
                })
            })
            .collect();
        let results: Vec<Result<(), String>> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        for result in &results {
            assert!(result.is_ok(), "every identical concurrent init must succeed, got: {results:?}");
        }
        assert_eq!(load_local_identity(&root).unwrap().node_id, "fog-01");
    }

    /// H3 regression 2: two THREADS racing `init_local_identity` against the
    /// SAME root with DIFFERENT `node_id`s -- exactly one must succeed, the
    /// other must fail closed as a conflicting re-initialization, and
    /// whichever `node_id` is actually persisted must match the one whose
    /// call returned `Ok`.
    #[test]
    fn concurrent_conflicting_init_results_in_exactly_one_winner() {
        use std::sync::{Arc, Barrier};
        let root = Arc::new(fresh_root());
        let barrier = Arc::new(Barrier::new(2));

        let root_a = Arc::clone(&root);
        let barrier_a = Arc::clone(&barrier);
        let handle_a = std::thread::spawn(move || {
            barrier_a.wait();
            init_local_identity(&root_a, "fog-01")
        });

        let root_b = Arc::clone(&root);
        let barrier_b = Arc::clone(&barrier);
        let handle_b = std::thread::spawn(move || {
            barrier_b.wait();
            init_local_identity(&root_b, "cloud-01")
        });

        let result_a = handle_a.join().unwrap();
        let result_b = handle_b.join().unwrap();

        let ok_count = [&result_a, &result_b].iter().filter(|r| r.is_ok()).count();
        assert_eq!(ok_count, 1, "exactly one of two conflicting concurrent inits must win, got: a={result_a:?} b={result_b:?}");

        let persisted = load_local_identity(&root).unwrap().node_id;
        let winner_requested = if result_a.is_ok() { "fog-01" } else { "cloud-01" };
        assert_eq!(persisted, winner_requested, "the persisted identity must be exactly what the WINNING call requested");

        let loser_error = if result_a.is_ok() { result_b.unwrap_err() } else { result_a.unwrap_err() };
        assert!(loser_error.contains("conflicting"), "the losing call's error must identify itself as a conflict: {loser_error}");
    }

    /// H3 regression 3: after a concurrent race, the final identity file is
    /// well-formed, parseable JSON -- never a half-written or corrupted
    /// artifact of two writers interleaving.
    #[test]
    fn final_identity_file_after_a_race_is_valid_json() {
        use std::sync::{Arc, Barrier};
        let root = Arc::new(fresh_root());
        const THREADS: usize = 6;
        let barrier = Arc::new(Barrier::new(THREADS));
        let handles: Vec<_> = (0..THREADS)
            .map(|i| {
                let root = Arc::clone(&root);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    // A mix of identical and conflicting requests -- exactly
                    // one distinct value can ultimately persist, but every
                    // call must complete cleanly either way.
                    let node_id = if i % 2 == 0 { "fog-01" } else { "cloud-01" };
                    init_local_identity(&root, node_id)
                })
            })
            .collect();
        for handle in handles {
            let _ = handle.join().unwrap();
        }

        let raw = fs::read(identity_path(&root)).expect("identity file must exist after the race");
        let parsed: Result<DNodeLocalIdentityV1, _> = serde_json::from_slice(&raw);
        assert!(parsed.is_ok(), "final identity file must be valid JSON, got bytes: {:?}", String::from_utf8_lossy(&raw));
    }

    /// H3 regression 4: after a concurrent race, no partial/temporary
    /// identity file is left behind -- every `.dnode-local-identity.*.tmp`
    /// was either renamed into place or cleaned up on error.
    #[test]
    fn no_partial_temp_identity_survives_a_race() {
        use std::sync::{Arc, Barrier};
        let root = Arc::new(fresh_root());
        const THREADS: usize = 6;
        let barrier = Arc::new(Barrier::new(THREADS));
        let handles: Vec<_> = (0..THREADS)
            .map(|i| {
                let root = Arc::clone(&root);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    let node_id = if i % 2 == 0 { "fog-01" } else { "cloud-01" };
                    init_local_identity(&root, node_id)
                })
            })
            .collect();
        for handle in handles {
            let _ = handle.join().unwrap();
        }

        let leftover_tmp_files: Vec<_> = fs::read_dir(root.as_path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                name.starts_with(".dnode-local-identity.") && name.ends_with(".tmp")
            })
            .collect();
        assert!(leftover_tmp_files.is_empty(), "no partial temp identity file may survive a race, found: {leftover_tmp_files:?}");
    }
}
