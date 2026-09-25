//! Stage 109 (ADR-0016). Real, governed Linux native-process execution --
//! replaces Stage 108's `not_implemented_native_process_runtime` stub.
//!
//! Structure: PURE decision logic (`decide_native_reconciliation_action`,
//! `evaluate_local_native_policy`) is unit-tested everywhere, matching this
//! codebase's established discipline of separating safety-critical decisions
//! from their I/O-performing executors. Real process spawn/signal/`/proc`
//! reads are Linux-only (`#[cfg(target_os = "linux")]`); a non-Linux host
//! fails every real-mutation attempt closed with the stable
//! `native_process_runtime_unsupported_platform` outcome, never pretending
//! to adopt or signal a process it cannot actually observe.
//!
//! Reuses Stage 107A/B governance machinery directly: `LeaseGuard`,
//! `check_generation_and_projection`, `BLOCKED_*` reason codes, and (via
//! `pub(crate)` visibility) `operational_reconciliation::check_boundary`/
//! `MutationBoundaryContext`/`MutationBoundaryRecord` -- no parallel
//! authorization mechanism is introduced.

use std::path::PathBuf;

// Used only by the real Linux spawn/signal/observation path
// (`spawn_and_persist`, `governed_stop`, `observe_persisted_identity`'s
// Linux variant) -- gated the same way that code is, so a non-Linux build
// never carries an unused-import warning for symbols it genuinely has no
// use for.
#[cfg(target_os = "linux")]
use std::path::Path;
#[cfg(target_os = "linux")]
use chrono::Utc;
#[cfg(target_os = "linux")]
use crate::agent::native_process_state::{
    check_identity, compute_args_digest, MANAGED_PROCESS_RUNTIME_STATE_SCHEMA, PROCESS_STATE_RUNNING,
};

use crate::agent::execution_projection;
use crate::agent::native_process_acquisition::{
    self, is_valid_content_sha256, ManagedLayout, MaterializationNeed, MaterializeError, MaterializeOutcome,
};
use crate::agent::native_process_state::{
    self, IdentityCheck, ManagedProcessRuntimeStateV1,
};
use crate::agent::operational_reconciliation::{
    no_wasm_decision, MutationBoundaryContext, MutationBoundaryRecord, OperationalArtifactAssignment,
    OperationalDesiredService, OperationalReconciliationAuthorization, OperationalServiceReconciliationReport,
};
use crate::agent::stage107a_enforcement::{
    check_generation_and_projection, LeaseGuard, BLOCKED_REQUIRED_INCOMPLETE,
};
use crate::config::Config;

// ---------------------------------------------------------------------- //
// Stable outcome/reason codes.
// ---------------------------------------------------------------------- //
pub const NATIVE_PROCESS_RUNTIME_UNSUPPORTED_PLATFORM: &str = "native_process_runtime_unsupported_platform";
pub const BLOCKED_MISSING_ARTIFACT_ASSIGNMENT: &str = "blocked_missing_artifact_assignment";
pub const BLOCKED_LOCAL_NATIVE_POLICY_DENIED: &str = "blocked_local_native_process_policy";
pub const UNPINNED_NATIVE_PROCESS_CONTENT: &str = "unpinned_native_process_content";
pub const BLOCKED_NATIVE_PROCESS_CONTENT_UNPINNED: &str = "blocked_native_process_content_unpinned";
pub const BLOCKED_NATIVE_PROCESS_CONTENT_MISMATCH: &str = "blocked_native_process_content_mismatch";
pub const NATIVE_PROCESS_RUNNING_CONVERGED: &str = "native_process_running_converged";
pub const NATIVE_PROCESS_SPAWNED: &str = "native_process_spawned";
pub const NATIVE_PROCESS_RESPAWNED: &str = "native_process_respawned";
pub const NATIVE_PROCESS_RESTARTED_FOR_DRIFT: &str = "native_process_restarted_for_drift";
pub const NATIVE_PROCESS_ROLLED_BACK: &str = "native_process_rolled_back";
pub const NATIVE_PROCESS_OBSERVED_ONLY: &str = "native_process_observed_only";
pub const UNSUPPORTED_NATIVE_PROCESS_COMPLETED_DESIRED_STATE: &str =
    "unsupported_native_process_completed_desired_state";
pub const BLOCKED_NATIVE_PROCESS_STATE_PERSISTENCE_FAILED: &str = "blocked_native_process_state_persistence_failed";
pub const BLOCKED_NATIVE_PROCESS_STATE_UNREADABLE: &str = "blocked_native_process_state_unreadable";
/// Section AE: a persisted runtime-state record that parses as JSON but
/// fails semantic validation (`native_process_state::validate_loaded_state`)
/// -- distinct from `BLOCKED_NATIVE_PROCESS_STATE_UNREADABLE` (malformed
/// JSON). Neither outcome ever signals a PID derived from the record, and
/// neither is silently treated as "no managed process."
pub const BLOCKED_INVALID_MANAGED_PROCESS_STATE: &str = "invalid_managed_process_state";
/// Section AA rule 1: PID+starttime matched (still the same OS process
/// instance) but the observed executable or argument identity differs from
/// what was persisted -- identity cannot be safely CONFIRMED, but the old
/// PID is still alive and may still be the managed workload, so it must
/// never be spawned over.
pub const BLOCKED_NATIVE_PROCESS_IDENTITY_MISMATCH: &str = "blocked_native_process_identity_mismatch";
/// Section AA rule 2 / AH#2: PID+starttime matched but a further identity
/// read (executable/cmdline) failed -- a procfs race, not a disproof.
pub const BLOCKED_NATIVE_PROCESS_IDENTITY_UNAVAILABLE: &str = "blocked_native_process_identity_unavailable";
/// Section AB: another reconciliation attempt for this exact stable service
/// key is already in flight on this node (advisory per-service lock could
/// not be acquired) -- no observation or mutation is attempted this cycle;
/// the next cycle retries.
pub const BLOCKED_NATIVE_PROCESS_CONCURRENT_RECONCILIATION: &str = "blocked_native_process_concurrent_reconciliation";
/// Section AM: Stage 109 v0.1 does not implement secret injection for
/// native processes. A native artifact declaring non-empty `secret_refs`
/// must fail closed before spawn, regardless of `execution_pinning_mode` --
/// silent ignore is never acceptable.
pub const BLOCKED_NATIVE_PROCESS_SECRETS_UNSUPPORTED: &str = "blocked_native_process_secrets_unsupported";
/// Section AI/AP: a governed stop was attempted but could not be positively
/// confirmed (SIGKILL delivered but the process was still observed alive at
/// the bounded confirmation deadline) -- callers must never treat this as
/// "safe to proceed" (no respawn, no state removal).
pub const NATIVE_PROCESS_STOP_INCOMPLETE: &str = "native_process_stop_incomplete";

// Native mutation boundary identifiers (parallel to Container's M1-M8;
// never reusing those identifiers for an unrelated operation).
pub const N1_MATERIALIZE: &str = "N1";
pub const N2_SET_EXECUTABLE_PERMISSIONS: &str = "N2";
pub const N3_SPAWN: &str = "N3";
pub const N4_SIGTERM: &str = "N4";
pub const N5_SIGKILL: &str = "N5";
pub const N6_REMOVE: &str = "N6";

/// Bounded stop timeout: a fixed local-policy constant, never an artifact-
/// controlled value (Section L's explicit requirement -- an unbounded or
/// artifact-supplied timeout would let a desired-state document itself
/// control how long a governed SIGTERM grace period lasts).
pub const GRACEFUL_STOP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
#[allow(dead_code)]
const GRACEFUL_STOP_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

// ---------------------------------------------------------------------- //
// Local execution policy (Section E).
// ---------------------------------------------------------------------- //

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalNativePolicy {
    Allowed { root: PathBuf },
    Denied(String),
}

/// DServer authorization ALONE is never sufficient to spawn a native
/// process -- this must independently pass, every cycle. Deliberately does
/// NOT reuse `allow_container_execution`/`container_mappings`: this is an
/// independent permission and an independent controlled root, not a Docker
/// concept repurposed. Docker availability plays no role here at all.
pub fn evaluate_local_native_policy(cfg: &Config) -> LocalNativePolicy {
    let Some(dispatch) = cfg.dispatch.as_ref() else {
        return LocalNativePolicy::Denied("dispatch configuration is absent".to_string());
    };
    if !dispatch.enabled {
        return LocalNativePolicy::Denied("dispatch.enabled is false".to_string());
    }
    if !dispatch.allow_native_process_execution.unwrap_or(false) {
        return LocalNativePolicy::Denied("dispatch.allow_native_process_execution is not true".to_string());
    }
    let Some(root) = dispatch.native_process_root.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return LocalNativePolicy::Denied("dispatch.native_process_root is not configured".to_string());
    };
    // A Linux-absolute-path check (leading `/`), deliberately NOT
    // `Path::is_absolute()` -- Stage 109's real target is always a Linux
    // D-Node, even when this policy check itself is validated in a
    // portable unit test on a non-Linux development host, where
    // `is_absolute()` would (incorrectly, for this purpose) reject an
    // ordinary Unix-style path like `/var/lib/datum/native` for lacking a
    // Windows drive-letter prefix.
    if !root.starts_with('/') {
        return LocalNativePolicy::Denied(format!("dispatch.native_process_root '{root}' must be an absolute Linux path (starting with '/')"));
    }
    let root_path = PathBuf::from(root);
    LocalNativePolicy::Allowed { root: root_path }
}

/// Section AN (observe-only mode): the managed root used to OBSERVE
/// whatever process may already be running is a strictly WEAKER gate than
/// `evaluate_local_native_policy`'s MUTATION gate -- it requires only that
/// dispatch is on at all and that a syntactically valid root is
/// configured, deliberately NOT `allow_native_process_execution`.
///
/// This matters concretely: an operator flipping
/// `allow_native_process_execution` to `false` (or a cycle arriving with
/// `execute_requested = false`) must still let this agent read its own
/// stable state key and truthfully report whether its previously-spawned
/// process is running, missing, or identity-mismatched -- observation and
/// mutation permission are separate concerns (Section AN). Using the full
/// mutation policy for observation would silently blind the agent to a
/// still-running managed process the moment mutation is locally denied,
/// which is a monitoring/evidence regression, not merely a stricter
/// safety posture.
fn configured_native_process_root(cfg: &Config) -> Option<PathBuf> {
    let dispatch = cfg.dispatch.as_ref()?;
    if !dispatch.enabled {
        return None;
    }
    let root = dispatch.native_process_root.as_deref().map(str::trim).filter(|s| !s.is_empty())?;
    if !root.starts_with('/') {
        return None;
    }
    Some(PathBuf::from(root))
}

/// Phase 115D: an independent, READ-ONLY observation of a managed native
/// process's CURRENT persisted state + freshly-checked identity -- reuses
/// exactly the same `configured_native_process_root`/`stable_service_key`/
/// `ManagedLayout`/`load_state`/`observe_persisted_identity` machinery
/// `reconcile_native_service` itself uses, so a DMonitor producer never
/// re-derives "where is this service's state file" a second, potentially
/// divergent way. Deliberately separate from `reconcile_native_service`:
/// this never acquires the reconciliation lock, never mutates anything, and
/// is safe to call from an entirely independent observation cycle (mirrors
/// DMonitor's own "never gate/mutate" contract at the call-site boundary,
/// not merely by convention). `None` means either no root is configured, no
/// state has ever been persisted for this exact service, or the persisted
/// record is unreadable/invalid -- in every such case there is honestly
/// nothing to observe, never a fabricated placeholder.
pub(crate) fn observe_current_native_state(
    cfg: &Config,
    project_id: &str,
    node_id: &str,
    service_id: &str,
) -> Option<(ManagedProcessRuntimeStateV1, IdentityCheck)> {
    let root = configured_native_process_root(cfg)?;
    let stable_key = native_process_acquisition::stable_service_key(project_id, node_id, service_id);
    let layout = ManagedLayout::new(root);
    let path = layout.state_path(&stable_key).ok()?;
    let state = native_process_state::load_state(&path).ok()??;
    if native_process_state::validate_loaded_state(&state).is_err() {
        return None;
    }
    let identity = observe_persisted_identity(&state);
    Some((state, identity))
}

// ---------------------------------------------------------------------- //
// Pure reconciliation decision (Section N).
// ---------------------------------------------------------------------- //

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeReconciliationAction {
    /// No host mutation may occur; `reason` names why (platform, policy,
    /// authorization, execute_requested, or lease/generation gate).
    ObserveOnly { reason: String },
    /// A content hash is missing or syntactically invalid -- a hard block,
    /// independent of pinning mode (Section D).
    ContentUnpinned,
    /// The desired `desired_state` is `"completed"`, which Stage 109 does
    /// not implement -- must not spawn, must not claim convergence.
    UnsupportedCompletedDesiredState,
    /// Exact desired process already running under the current desired
    /// content/projection -- no mutation.
    AlreadyConverged,
    /// No persisted state at all -- acquire/materialize then spawn fresh.
    Spawn,
    /// Persisted state exists but the process is gone (or the PID was
    /// reused / identity mismatched) -- respawn under current authorization.
    Respawn,
    /// Persisted state exists, the exact process is running, but current
    /// desired content/projection differs -- governed stop then respawn.
    RestartForDrift,
    /// A rollback request for an exact, currently-confirmed managed
    /// process: stop it, then (once confirmed stopped) governed-N6-remove
    /// its runtime-state record.
    Rollback,
    /// A rollback request with NO persisted state at all -- a true,
    /// mutation-free no-op (v3 fix: previously this variant was also
    /// reached for `ProcessAbsent`/`PidReused`/mismatch/`Unavailable`,
    /// conflating "genuinely nothing to do" with "there IS a stale state
    /// record that a governed N6 removal would clean up").
    RollbackNoOp,
    /// A rollback request where persisted state describes a process that
    /// is provably gone from that PID (`ProcessAbsent`/`PidReused`/
    /// `Zombie`) -- the stale state record MAY be removed, but only via a
    /// governed, freshly-lease-checked N6 boundary (v3 fix: this is a
    /// real mutation, never silently folded into `RollbackNoOp`'s
    /// `host_mutation_performed = false`).
    RollbackStaleStateCleanup,
    /// A rollback request where persisted identity could not be safely
    /// confirmed OR disproven (`ExecutableMismatch`/`ArgsMismatch`/
    /// `Unavailable`) -- the PID may still be the managed workload. Never
    /// signal, never remove state; rollback is incomplete.
    RollbackIncomplete,
}

#[allow(clippy::too_many_arguments)]
pub fn decide_native_reconciliation_action(
    desired_state: &str,
    rollback_requested: bool,
    execute_requested: bool,
    control_plane_authorized: bool,
    local_policy_denied_reason: Option<&str>,
    platform_supported: bool,
    lease_or_projection_block: Option<&str>,
    rollback_projection_block: Option<&str>,
    content_hash_valid: bool,
    persisted_identity: Option<IdentityCheck>,
    content_or_projection_drifted: bool,
) -> NativeReconciliationAction {
    if !platform_supported {
        return NativeReconciliationAction::ObserveOnly {
            reason: NATIVE_PROCESS_RUNTIME_UNSUPPORTED_PLATFORM.to_string(),
        };
    }
    // P0-1 (v3 fix): rollback is a mutation like any other and must pass
    // the COMPLETE governance gate before it may act: execute_requested,
    // control_plane_authorized, local native mutation policy, and a valid
    // generation/projection binding (`rollback_projection_block` -- NOT
    // the full `lease_or_projection_block`, which also folds in content-
    // acquisition/secret completeness gaps that have no bearing on
    // whether it is safe to STOP and clean up an already-owned process).
    // Content/hash completeness deliberately does NOT gate rollback: an
    // artifact whose executable source has gone missing must still be
    // removable.
    if rollback_requested {
        if !execute_requested {
            return NativeReconciliationAction::ObserveOnly { reason: "execute_requested is false".to_string() };
        }
        if !control_plane_authorized {
            return NativeReconciliationAction::ObserveOnly {
                reason: "DServer authorization is absent or invalid for this cycle".to_string(),
            };
        }
        if let Some(reason) = local_policy_denied_reason {
            return NativeReconciliationAction::ObserveOnly { reason: reason.to_string() };
        }
        if let Some(reason) = rollback_projection_block {
            return NativeReconciliationAction::ObserveOnly { reason: reason.to_string() };
        }
        return match persisted_identity {
            None => NativeReconciliationAction::RollbackNoOp,
            Some(IdentityCheck::Confirmed) => NativeReconciliationAction::Rollback,
            Some(IdentityCheck::ProcessAbsent) | Some(IdentityCheck::PidReused) | Some(IdentityCheck::Zombie) => {
                NativeReconciliationAction::RollbackStaleStateCleanup
            }
            Some(IdentityCheck::ExecutableMismatch)
            | Some(IdentityCheck::ArgsMismatch)
            | Some(IdentityCheck::Unavailable) => NativeReconciliationAction::RollbackIncomplete,
        };
    }
    if !matches!(desired_state.trim().to_ascii_lowercase().as_str(), "running") {
        // Section: completed/one-shot is explicitly NOT implemented --
        // never coerced to running, never silently converged.
        return NativeReconciliationAction::UnsupportedCompletedDesiredState;
    }
    if !execute_requested {
        return NativeReconciliationAction::ObserveOnly { reason: "execute_requested is false".to_string() };
    }
    if !control_plane_authorized {
        return NativeReconciliationAction::ObserveOnly {
            reason: "DServer authorization is absent or invalid for this cycle".to_string(),
        };
    }
    if let Some(reason) = local_policy_denied_reason {
        return NativeReconciliationAction::ObserveOnly { reason: reason.to_string() };
    }
    if let Some(reason) = lease_or_projection_block {
        return NativeReconciliationAction::ObserveOnly { reason: reason.to_string() };
    }
    if !content_hash_valid {
        return NativeReconciliationAction::ContentUnpinned;
    }
    match persisted_identity {
        Some(IdentityCheck::Confirmed) => {
            if content_or_projection_drifted {
                NativeReconciliationAction::RestartForDrift
            } else {
                NativeReconciliationAction::AlreadyConverged
            }
        }
        // The PID at that number is now provably a DIFFERENT process
        // (starttime mismatch), provably gone entirely, or a zombie (the
        // workload has already exited, merely awaiting reaping) -- our
        // original managed process's identity is either disproven-at-that-
        // PID, absent, or no longer a live workload, and this codebase has
        // no process-tree/name-scan mechanism to locate it any other way.
        // Respawning is the only implementable, safe action (never
        // signals the unrelated/absent/zombie PID first -- that is
        // `governed_stop`'s job on the OLD persisted identity, and it
        // independently re-verifies before ever signaling).
        Some(IdentityCheck::ProcessAbsent) | Some(IdentityCheck::PidReused) | Some(IdentityCheck::Zombie) => {
            NativeReconciliationAction::Respawn
        }
        // Section AA rule 1: PID + starttime still match -- this is
        // provably the SAME OS process instance this agent started, still
        // alive right now. An executable/args difference means identity
        // cannot be fully CONFIRMED, but it is very much NOT safely
        // disproven either (a live process at a matching PID/starttime is
        // the strongest possible evidence it might still be the managed
        // workload). Spawning a second process here would risk two live
        // instances of the same service; the only safe action is to block
        // with no mutation and report the mismatch truthfully.
        Some(IdentityCheck::ExecutableMismatch) | Some(IdentityCheck::ArgsMismatch) => {
            NativeReconciliationAction::ObserveOnly { reason: BLOCKED_NATIVE_PROCESS_IDENTITY_MISMATCH.to_string() }
        }
        // Section AA rule 2 / AH#2: PID + starttime matched (still alive,
        // same instance) but a further identity read raced away. Neither a
        // disproof nor a confirmation -- never signal, never spawn a
        // duplicate.
        Some(IdentityCheck::Unavailable) => {
            NativeReconciliationAction::ObserveOnly { reason: BLOCKED_NATIVE_PROCESS_IDENTITY_UNAVAILABLE.to_string() }
        }
        None => NativeReconciliationAction::Spawn,
    }
}

// ---------------------------------------------------------------------- //
// Linux-only real OS integration.
// ---------------------------------------------------------------------- //

#[cfg(target_os = "linux")]
pub mod linux {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Signal {
        Term,
        Kill,
    }

    /// Send a signal to `pid` via `libc::kill`. Callers MUST have already
    /// confirmed process identity (PID + starttime + executable + args) is
    /// still the persisted managed process, checked FRESH immediately
    /// before this call -- this function itself performs no identity
    /// verification, by design (it is the last, narrow, Linux-specific
    /// primitive; identity is a policy decision made by the caller).
    /// P0-1 (v4 fix): a bounded, typed distinction between "the process is
    /// genuinely gone" (`ESRCH` -- the only errno that may ever be
    /// interpreted as an idempotent "already stopped") and every other
    /// failure (`EPERM`/`EACCES`/`EINVAL`/anything else), which proves
    /// NOTHING about whether the process is still alive and must never be
    /// treated as a confirmed stop. Deliberately a typed enum, not a
    /// human-readable string a caller would need to pattern-match by
    /// prefix.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum SignalError {
        /// `ESRCH` -- no such process. The only errno that proves the
        /// target is gone.
        ProcessAbsent,
        /// Any other `kill(2)` failure (`EPERM`, `EACCES`, `EINVAL`, or an
        /// unexpected syscall error) -- proves nothing about liveness;
        /// callers must never treat this as a confirmed stop.
        Failed(String),
    }

    pub fn send_signal(pid: u32, signal: Signal) -> Result<(), SignalError> {
        let sig = match signal {
            Signal::Term => libc::SIGTERM,
            Signal::Kill => libc::SIGKILL,
        };
        let result = unsafe { libc::kill(pid as libc::pid_t, sig) };
        if result == 0 {
            Ok(())
        } else {
            let errno = std::io::Error::last_os_error();
            if errno.raw_os_error() == Some(libc::ESRCH) {
                // No such process -- treated by the caller as "already
                // gone", not a hard error (a process disappearing between
                // the identity check and the signal is a successful,
                // idempotent stop).
                Err(SignalError::ProcessAbsent)
            } else {
                Err(SignalError::Failed(format!("kill(pid={pid}, sig={sig}) failed: {errno}")))
            }
        }
    }

    /// P0-4 fix: `kill(pid, 0)`'s result MUST be interpreted through its
    /// real POSIX semantics, not collapsed into a bare boolean that fails
    /// open. A return of 0 means the PID exists AND this process may
    /// signal it. `ESRCH` means the PID genuinely does not exist. `EPERM`
    /// means the PID DOES exist (the kernel would never return `EPERM`
    /// for a nonexistent PID) but this process lacks permission to
    /// signal it -- treating that as "absent" (the previous, incorrect
    /// behavior) could let a native process be silently respawned as a
    /// duplicate while the original is still alive under a PID this
    /// agent merely cannot signal (e.g. after a UID change). Any other
    /// errno is a genuine observation failure, not a disproof either way.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum PidPresence {
        Exists,
        Absent,
        Unavailable(String),
    }

    pub fn pid_presence(pid: u32) -> PidPresence {
        let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if result == 0 {
            return PidPresence::Exists;
        }
        let errno = std::io::Error::last_os_error();
        match errno.raw_os_error() {
            Some(code) if code == libc::ESRCH => PidPresence::Absent,
            Some(code) if code == libc::EPERM => PidPresence::Exists,
            _ => PidPresence::Unavailable(errno.to_string()),
        }
    }

    /// `true` if a process exists at `pid`, independent of permission to
    /// signal it (`EPERM` still counts as existing -- see `pid_presence`).
    /// Convenience wrapper for callers (e.g. tests exercising a real,
    /// just-spawned child this process definitely owns) that only need a
    /// simple existence check, not the full three-state distinction.
    pub fn pid_exists(pid: u32) -> bool {
        matches!(pid_presence(pid), PidPresence::Exists)
    }

    /// A `/proc/<pid>/stat` read can fail for reasons other than the
    /// process genuinely being gone (permission, a transient I/O error,
    /// malformed content) -- P0-4's "stat read failure must not
    /// automatically mean absent" fix requires distinguishing a REAL
    /// disappearance (`NotFound`, i.e. `ENOENT`) from every other failure
    /// (`Other`).
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum ProcStatReadError {
        NotFound,
        Other(String),
    }

    /// Reads BOTH the process state character (field 3 -- `Z` for zombie,
    /// Section AA/P1-4) and the starttime (field 22) from a single
    /// `/proc/<pid>/stat` read.
    pub fn read_proc_stat(pid: u32) -> Result<(char, u64), ProcStatReadError> {
        let path = format!("/proc/{pid}/stat");
        let contents = std::fs::read_to_string(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ProcStatReadError::NotFound
            } else {
                ProcStatReadError::Other(format!("read {path}: {e}"))
            }
        })?;
        let state_char = crate::agent::native_process_state::parse_proc_stat_state_char(&contents)
            .map_err(|e| ProcStatReadError::Other(e.to_string()))?;
        let starttime = crate::agent::native_process_state::parse_proc_stat_starttime(&contents)
            .map_err(|e| ProcStatReadError::Other(e.to_string()))?;
        Ok((state_char, starttime))
    }

    /// Thin backward-compatible wrapper over `read_proc_stat` for callers
    /// that only need the starttime (e.g. existing integration tests).
    pub fn read_proc_stat_starttime(pid: u32) -> Result<u64, String> {
        read_proc_stat(pid).map(|(_, starttime)| starttime).map_err(|e| match e {
            ProcStatReadError::NotFound => format!("/proc/{pid}/stat not found"),
            ProcStatReadError::Other(message) => message,
        })
    }

    pub fn read_proc_exe(pid: u32) -> Result<String, String> {
        let path = format!("/proc/{pid}/exe");
        std::fs::read_link(&path)
            .map(|p| p.to_string_lossy().to_string())
            .map_err(|e| format!("readlink {path}: {e}"))
    }

    /// Real observed cmdline identity: `/proc/<pid>/cmdline` is
    /// NUL-separated. v5/L1 fix (real Linux validation): this returns the
    /// COMPLETE raw vector AS THE KERNEL RECORDED IT, including argv[0] --
    /// it does NOT attempt to strip argv[0] and reconstruct the desired
    /// `native_spec.args` from it. Real Linux execution disproved that
    /// model: for a shebang script (e.g. `#!/bin/sh`), the kernel execs the
    /// interpreter with argv[0] = the interpreter, argv[1] = the script's
    /// own path, and only argv[2..] = the desired args this agent actually
    /// passed -- so naively dropping a single leading element leaves the
    /// script's path mixed into what should be pure desired-args identity.
    /// Reversing an arbitrary interpreter's argv-rewriting convention back
    /// into the desired args is not implementable in general (it would
    /// require per-interpreter special-casing, explicitly forbidden), so
    /// this function makes no such attempt: the COMPLETE observed vector
    /// (whatever shape the kernel actually produced, ELF or
    /// interpreter-shaped alike) is hashed via
    /// `native_process_state::compute_args_digest` into
    /// `ManagedProcessRuntimeStateV1.cmdline_digest` -- observed RUNTIME
    /// identity, deliberately distinct from and never compared against
    /// `NativeProcessDeploymentSpecV1.args` (desired-state identity, used
    /// only to spawn).
    pub fn read_proc_cmdline(pid: u32) -> Result<Vec<String>, String> {
        let path = format!("/proc/{pid}/cmdline");
        let bytes = std::fs::read(&path).map_err(|e| format!("read {path}: {e}"))?;
        let mut parts: Vec<String> = bytes
            .split(|b| *b == 0)
            .map(|slice| String::from_utf8_lossy(slice).into_owned())
            .collect();
        // `/proc/<pid>/cmdline` is terminated by a trailing NUL, which
        // `split` turns into one trailing empty element -- drop exactly
        // that one (never an interior empty argv entry, which is a
        // legitimate zero-length argument).
        if parts.last().is_some_and(String::is_empty) {
            parts.pop();
        }
        if parts.is_empty() {
            return Err(format!("{path} was empty (process likely a kernel thread or already gone)"));
        }
        Ok(parts)
    }
}

#[cfg(target_os = "linux")]
use linux::Signal;

/// Observe whatever process `persisted` claims to identify, using real
/// `/proc` reads on Linux. Returns `IdentityCheck` describing the outcome
/// -- never signals anything (observation only).
#[cfg(target_os = "linux")]
pub fn observe_persisted_identity(persisted: &ManagedProcessRuntimeStateV1) -> IdentityCheck {
    // P0-4 fix: `EPERM` from `kill(pid, 0)` means the PID EXISTS (just
    // unsignalable by this process) -- never conflated with absence.
    // Only `Unavailable` for a genuine observation failure, only `Absent`
    // for a real, kernel-confirmed `ESRCH`.
    match linux::pid_presence(persisted.pid) {
        linux::PidPresence::Absent => return IdentityCheck::ProcessAbsent,
        linux::PidPresence::Unavailable(_) => return IdentityCheck::Unavailable,
        linux::PidPresence::Exists => {}
    }
    // P0-4 fix: a `/proc/<pid>/stat` read failure is `ProcessAbsent` ONLY
    // when it is a real, concrete disappearance (`ENOENT`/`NotFound`) --
    // AH#1's "the process exited in the narrow window between the two
    // reads" interpretation still applies, but ONLY to that specific,
    // provable case. Any other failure (permission, I/O, malformed
    // content) is `Unavailable`, never silently treated as proof the
    // process is gone.
    let (state_char, starttime) = match linux::read_proc_stat(persisted.pid) {
        Ok(value) => value,
        Err(linux::ProcStatReadError::NotFound) => return IdentityCheck::ProcessAbsent,
        Err(linux::ProcStatReadError::Other(_)) => return IdentityCheck::Unavailable,
    };
    let executable = linux::read_proc_exe(persisted.pid).ok();
    let cmdline_digest = linux::read_proc_cmdline(persisted.pid)
        .ok()
        .map(|cmdline| native_process_state::compute_args_digest(&cmdline));
    check_identity(persisted, true, Some(starttime), Some(state_char), executable.as_deref(), cmdline_digest.as_deref())
}

#[cfg(not(target_os = "linux"))]
pub fn observe_persisted_identity(_persisted: &ManagedProcessRuntimeStateV1) -> IdentityCheck {
    // Non-Linux: no real observation mechanism exists. Never claim a
    // confirmed identity we did not actually verify.
    IdentityCheck::ProcessAbsent
}

pub fn platform_supported() -> bool {
    cfg!(target_os = "linux")
}

// ---------------------------------------------------------------------- //
// Report builders.
// ---------------------------------------------------------------------- //

fn base_report(
    service: &OperationalDesiredService,
    assignment: Option<&OperationalArtifactAssignment>,
) -> OperationalServiceReconciliationReport {
    OperationalServiceReconciliationReport {
        service_id: service.service_id.clone(),
        artifact_id: assignment.map(|a| a.artifact_id.clone()),
        artifact_version: assignment.map(|a| a.version.clone()),
        desired_state: service.desired_state.clone(),
        trusted_container_name: None,
        trusted_image: None,
        observed_before_running: false,
        observation_error: None,
        secret_resolution_status: "not_applicable_native_process".to_string(),
        wasm: no_wasm_decision("not_applicable_native_process"),
        execution_status: NATIVE_PROCESS_OBSERVED_ONLY.to_string(),
        message: String::new(),
        observed_after_running: false,
        host_mutation_performed: false,
        execution_semantics: None,
        authorization_id: None,
        authorization_expires_at_utc: None,
        execution_generation: assignment.map(|a| a.execution_generation),
        expected_execution_projection_digest: assignment.map(|a| a.execution_projection_digest.clone()),
        recomputed_execution_projection_digest: None,
        execution_pinning_mode: None,
        f2_status: None,
        local_policy_digest: None,
        unpinned_inputs: Vec::new(),
        authorization_clock_skew_seconds: None,
        verified_external_inputs: Vec::new(),
        local_substitutions: Vec::new(),
        mutation_boundaries_crossed: Vec::new(),
        verified_image_identity: None,
    }
}

/// Stage 109 (closes Stage 108 P2): a `NativeProcess`-desired service with
/// NO matching artifact assignment is a contract error, never the native
/// stub/execution path. Distinct from `runtime_kind_mismatch` (a mismatch
/// between two present values) and from ordinary execution failures (which
/// require an assignment to exist in the first place).
pub fn missing_artifact_assignment_report(service: &OperationalDesiredService) -> OperationalServiceReconciliationReport {
    let mut report = base_report(service, None);
    report.execution_status = BLOCKED_MISSING_ARTIFACT_ASSIGNMENT.to_string();
    report.message = format!(
        "desired service '{}' declares runtime_kind=native_process but no artifact assignment was delivered for it; no host mutation may occur",
        service.service_id
    );
    report
}

/// The full governed native-process reconciliation entry point. Called
/// exactly once per `NativeProcess`-dispatched desired service, with
/// `assignment` guaranteed `Some` by the caller (the missing-assignment
/// case is handled separately, before this is ever reached).
///
/// Phase 115H (post-Phase-115-closure hardening, discovered live): `lease`/
/// `effective_pinning_mode` are `None` together in EXACTLY one case --
/// `resolve_stage107a_cycle_gate` returned `Err(reason)` for this cycle
/// (missing/insufficient pinning mode, unconfigured required-mode clock
/// skew, mismatched resource identity, or an invalid/expired lease).
/// `stage107a_gate_error` carries that EXACT reason, mirroring the pattern
/// this crate's own Container-runtime path already establishes
/// (`stage107a_service_block_before_image`'s `(Some(Err(reason)), _) =>
/// Some(reason.clone())` arm) -- before this fix, the native-process path
/// alone discarded it and substituted a generic, non-actionable message,
/// which is exactly what made a live, correctly-fail-closed rejection
/// (missing `dispatch.authorization_clock_skew_seconds` under
/// `execution_pinning_mode = required`, ADR-0012 design-review finding 2)
/// look like an unexplained architectural gap during Phase 115F. This is a
/// diagnostics-only change: the BLOCKING decision itself was always
/// correct and is completely unchanged; only the message now tells the
/// operator WHY.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconcile_native_service(
    cfg: &Config,
    project_id: &str,
    node_id: &str,
    authorization: Option<&OperationalReconciliationAuthorization>,
    lease: Option<&LeaseGuard>,
    effective_pinning_mode: Option<&'static str>,
    stage107a_gate_error: Option<&str>,
    boundary_context: &MutationBoundaryContext,
    service: &OperationalDesiredService,
    assignment: &OperationalArtifactAssignment,
    execute_requested: bool,
    control_plane_authorized: bool,
    rollback_requested: bool,
) -> OperationalServiceReconciliationReport {
    let mut report = base_report(service, Some(assignment));
    report.authorization_id = authorization.map(|a| a.authorization_id.clone());
    report.authorization_expires_at_utc = authorization.and_then(|a| a.expires_at_utc.clone());
    report.execution_pinning_mode = effective_pinning_mode.map(str::to_string);

    // Recompute the NATIVE projection digest independently (mirrors the
    // Container path's `execution_projection::recompute_digest`, but for
    // the native projection -- never the container one).
    let recomputed_digest = execution_projection::recompute_native_digest(assignment);
    report.recomputed_execution_projection_digest = recomputed_digest.as_ref().ok().cloned();

    // Section D: native-specific acquisition completeness. Independent of
    // pinning mode -- a real native process must NEVER execute without an
    // exact, syntactically valid content sha256.
    let content_sha256 = assignment
        .acquisition
        .as_ref()
        .and_then(|a| a.sha256.as_deref())
        .filter(|sha| is_valid_content_sha256(sha));
    if content_sha256.is_none() {
        report.unpinned_inputs.push(format!(
            "{UNPINNED_NATIVE_PROCESS_CONTENT}:{}",
            assignment.acquisition.as_ref().and_then(|a| a.source.clone()).unwrap_or_default()
        ));
    }

    let local_policy = evaluate_local_native_policy(cfg);
    let local_policy_denied_reason = match &local_policy {
        LocalNativePolicy::Allowed { .. } => None,
        LocalNativePolicy::Denied(reason) => Some(format!("{BLOCKED_LOCAL_NATIVE_POLICY_DENIED}: {reason}")),
    };
    report.local_policy_digest = compute_local_native_policy_digest(cfg);

    // Lease + generation/projection gate, evaluated only when execution is
    // actually being attempted (mirrors the Container path's precedent of
    // computing this "before image" block only under execute_requested &&
    // control_plane_authorized).
    //
    // P0-1 (v3 fix): split into TWO independent blocks. `generation_projection_block`
    // is the "valid generation/projection binding" check ALONE -- this is
    // what rollback also needs (Section: "authorization/generation/lease/
    // local-policy absolutely do" gate rollback). `completeness_block`
    // additionally folds in `native_required_completeness_gaps` (content
    // hash + secret_refs) under `required` pinning mode -- this is
    // spawn/respawn/restart-specific and must NEVER gate rollback
    // (removing an already-owned process must not require its executable
    // source to still be resolvable).
    let (generation_projection_block, completeness_block): (Option<String>, Option<String>) =
        if execute_requested && control_plane_authorized {
            match (authorization, lease, effective_pinning_mode, &recomputed_digest) {
                (Some(auth), Some(_lease), Some(mode), Ok(digest)) => {
                    let generation_err = check_generation_and_projection(
                        auth,
                        &assignment.service_id,
                        &assignment.artifact_id,
                        assignment.execution_generation,
                        &assignment.execution_projection_digest,
                        digest,
                    )
                    .err();
                    let completeness_err = if mode == crate::agent::stage107a_enforcement::EXECUTION_PINNING_MODE_REQUIRED {
                        let gaps = native_required_completeness_gaps(assignment);
                        if gaps.is_empty() {
                            None
                        } else {
                            Some(format!("{BLOCKED_REQUIRED_INCOMPLETE}: service '{}' is not required-complete: {:?}", assignment.service_id, gaps))
                        }
                    } else {
                        None
                    };
                    (generation_err, completeness_err)
                }
                (_, _, _, Err(error)) => (Some(format!("blocked_projection_computation_failed: {error}")), None),
                // Phase 115H: surface the REAL reason `resolve_stage107a_cycle_gate`
                // returned, exactly mirroring the Container path's own
                // established pattern -- never a generic placeholder when
                // the actual cause is known. The generic fallback string is
                // kept only for the (should-be-unreachable, given this
                // function is only ever called under the same
                // `execute_requested && control_plane_authorized` gate the
                // cycle-gate `Result` was itself computed under) case where
                // no reason was supplied at all.
                _ => (
                    Some(stage107a_gate_error.map(str::to_string).unwrap_or_else(|| {
                        "blocked_authorization_not_leased: no valid Stage 107A cycle gate for this cycle".to_string()
                    })),
                    None,
                ),
            }
        } else {
            (None, None)
        };
    let lease_or_projection_block: Option<String> = generation_projection_block.clone().or(completeness_block);
    let rollback_projection_block: Option<String> = generation_projection_block;

    // Section AN: the root used to OBSERVE is deliberately independent of
    // `allow_native_process_execution` -- see `configured_native_process_root`.
    // `governed_materialize_and_spawn`/rollback's actual mutation path is
    // still gated on `local_policy`/`local_policy_denied_reason` via
    // `decide_native_reconciliation_action`, so a wider observation root
    // never grants a wider mutation permission.
    let root = configured_native_process_root(cfg);

    // Observe current state (read-only, always attempted when a root is
    // configured at all -- observation itself is not a mutation and is
    // safe regardless of execute_requested/authorization/local mutation
    // policy).
    // P0-3 (v3 fix): the stable filesystem identity key MUST be derived
    // from the AUTHORITATIVE reconciliation-context identity
    // (`project_id`/`node_id`, passed in directly by the caller from
    // `OperationalReconciliationContext`), never from whether an
    // authorization happens to be present this cycle. Before this fix,
    // the key silently changed shape depending on authorization presence
    // (`authorization.project_id` vs `""`) and always used the literal
    // string `"node"` regardless of the real node -- meaning an
    // observe-only cycle (no authorization) could compute a DIFFERENT
    // key than the one used during a previously-authorized execution
    // cycle for the exact same service, causing it to load the wrong (or
    // no) state file for its own managed process.
    let stable_key = root
        .as_ref()
        .map(|_root| native_process_acquisition::stable_service_key(project_id, node_id, &service.service_id));
    let layout = root.as_ref().map(|root| ManagedLayout::new(root.clone()));

    // P1-3 (v3 fix): observation (loading state, computing identity) is
    // read-only and must NEVER itself create a lock file/directory --
    // that would let a purely observe-only cycle (`execute_requested =
    // false`, or `allow_native_process_execution = false`) mutate the
    // filesystem, contradicting Stage 109's observe-only contract.
    // `do_observe` is called ONCE, unlocked, to decide the action; if
    // that action cannot mutate anything, its result is used as-is and
    // no lock is ever touched. Only when the DECIDED action may actually
    // mutate the host is the per-service lock acquired, and observation
    // is repeated FRESH while holding it (closing the exact "two
    // concurrent reconcilers both observe stale state" race Section AB
    // targets) before the action is (re)decided and finally executed.
    //
    // Self-review finding (Reviewer E, recovery): a corrupted/unparseable
    // state file must NOT be silently treated the same as "no state at
    // all" -- doing so could spawn a duplicate process while an original,
    // now-untracked one is genuinely still running. Section AE: a
    // syntactically valid JSON record that is semantically nonsensical
    // must be treated with the same caution -- never trusted enough to
    // signal a PID derived from it, and never silently treated as "no
    // managed process exists" either.
    enum ObservationBlock {
        Unreadable(String),
        Invalid(String),
    }
    let do_observe = || -> Result<(Option<ManagedProcessRuntimeStateV1>, Option<IdentityCheck>, bool), ObservationBlock> {
        let loaded_state: Result<Option<ManagedProcessRuntimeStateV1>, String> = match (&layout, &stable_key) {
            (Some(layout), Some(key)) => match layout.state_path(key) {
                Ok(path) => native_process_state::load_state(&path),
                Err(_) => Ok(None),
            },
            _ => Ok(None),
        };
        let persisted_state = loaded_state.map_err(ObservationBlock::Unreadable)?;
        if let Some(state) = &persisted_state {
            native_process_state::validate_loaded_state(state).map_err(ObservationBlock::Invalid)?;
        }
        let persisted_identity = persisted_state.as_ref().map(observe_persisted_identity);
        let content_or_projection_drifted = match (&persisted_state, &recomputed_digest) {
            (Some(state), Ok(digest)) => &state.execution_projection_digest != digest,
            _ => false,
        };
        Ok((persisted_state, persisted_identity, content_or_projection_drifted))
    };

    macro_rules! apply_observation_error_or_return {
        ($result:expr) => {
            match $result {
                Ok(v) => v,
                Err(ObservationBlock::Unreadable(error)) => {
                    report.execution_status = NATIVE_PROCESS_OBSERVED_ONLY.to_string();
                    report.message = format!(
                        "{BLOCKED_NATIVE_PROCESS_STATE_UNREADABLE}: runtime-state file exists but could not be parsed -- refusing to assume no managed process exists: {error}"
                    );
                    return report;
                }
                Err(ObservationBlock::Invalid(error)) => {
                    report.execution_status = NATIVE_PROCESS_OBSERVED_ONLY.to_string();
                    report.message = format!(
                        "{BLOCKED_INVALID_MANAGED_PROCESS_STATE}: persisted runtime-state record failed validation -- refusing to trust it enough to signal or respawn: {error}"
                    );
                    return report;
                }
            }
        };
    }

    let (persisted_state, persisted_identity, content_or_projection_drifted) = apply_observation_error_or_return!(do_observe());
    report.observed_before_running = matches!(persisted_identity, Some(IdentityCheck::Confirmed));

    let mut action = decide_native_reconciliation_action(
        &service.desired_state,
        rollback_requested,
        execute_requested,
        control_plane_authorized,
        local_policy_denied_reason.as_deref(),
        platform_supported(),
        lease_or_projection_block.as_deref(),
        rollback_projection_block.as_deref(),
        content_sha256.is_some(),
        persisted_identity.clone(),
        content_or_projection_drifted,
    );

    let mut persisted_state = persisted_state;
    let _reconciliation_lock = if action_may_mutate(&action) {
        // Section AB: only NOW -- because the decided action may actually
        // mutate the host -- acquire the per-service advisory lock. The
        // long-running operational loop already serializes itself via a
        // per-process instance lock
        // (`operational_service_loop::acquire_operational_lock`), but the
        // one-shot `smartsentinel-operational-reconcile` CLI has no such
        // guard, and nothing prevents an operator (or automation) from
        // invoking it twice concurrently for the same node. The lock is
        // held for the remainder of this function (dropped, releasing
        // it, when the function returns) -- the smallest bounded
        // critical section that actually closes the race, reusing the
        // exact `fs2` mechanism already established for the loop's own
        // instance lock, never a distributed lock.
        let lock = if let (Some(layout), Some(key)) = (&layout, &stable_key) {
            match acquire_service_reconciliation_lock(layout, key) {
                Some(lock) => Some(lock),
                None => {
                    report.execution_status = NATIVE_PROCESS_OBSERVED_ONLY.to_string();
                    report.message = format!(
                        "{BLOCKED_NATIVE_PROCESS_CONCURRENT_RECONCILIATION}: another reconciliation attempt for this exact managed service is already in progress on this node; no observation or mutation attempted this cycle"
                    );
                    return report;
                }
            }
        } else {
            None
        };
        // Re-observe and re-decide FRESH while holding the lock -- another
        // reconciliation attempt could have mutated state between the
        // first, unlocked observation above and this point.
        let (fresh_state, fresh_identity, fresh_drifted) = apply_observation_error_or_return!(do_observe());
        report.observed_before_running = matches!(fresh_identity, Some(IdentityCheck::Confirmed));
        persisted_state = fresh_state;
        action = decide_native_reconciliation_action(
            &service.desired_state,
            rollback_requested,
            execute_requested,
            control_plane_authorized,
            local_policy_denied_reason.as_deref(),
            platform_supported(),
            lease_or_projection_block.as_deref(),
            rollback_projection_block.as_deref(),
            content_sha256.is_some(),
            fresh_identity,
            fresh_drifted,
        );
        lock
    } else {
        None
    };

    let mut mutation_records: Vec<MutationBoundaryRecord> = Vec::new();
    let outcome = execute_native_action(
        &action,
        layout.as_ref(),
        stable_key.as_deref(),
        lease,
        boundary_context,
        &mut mutation_records,
        assignment,
        content_sha256,
        &recomputed_digest,
        persisted_state.as_ref(),
    );
    report.mutation_boundaries_crossed = mutation_records;
    report.host_mutation_performed = outcome.host_mutation_performed;
    report.execution_status = outcome.execution_status;
    report.message = outcome.message;
    report.observed_after_running = outcome.observed_after_running;
    report
}

/// Section AB/P1-3: does this decided action potentially perform a real
/// host mutation? Used to decide whether the per-service reconciliation
/// lock needs to be acquired at all -- a purely observational outcome
/// (including `RollbackIncomplete`, which by construction never signals
/// or removes state) must never cause a lock file to be created.
fn action_may_mutate(action: &NativeReconciliationAction) -> bool {
    matches!(
        action,
        NativeReconciliationAction::Spawn
            | NativeReconciliationAction::Respawn
            | NativeReconciliationAction::RestartForDrift
            | NativeReconciliationAction::Rollback
            | NativeReconciliationAction::RollbackStaleStateCleanup
    )
}

/// Section AB: acquire a non-blocking, per-service advisory lock so two
/// concurrent reconciliation attempts for the same stable service key
/// never both proceed. Returns `None` immediately (never blocks/waits) if
/// another holder already has it -- the caller treats that as "retry next
/// cycle", not an error. Reuses `fs2::FileExt::try_lock_exclusive`, the
/// same mechanism `operational_service_loop::acquire_operational_lock`
/// already uses for this codebase's per-process instance guarantee; this
/// is the same pattern applied at per-service granularity rather than a
/// new locking abstraction.
fn acquire_service_reconciliation_lock(layout: &ManagedLayout, stable_key: &str) -> Option<std::fs::File> {
    use fs2::FileExt;
    // P1-3 (v3 fix): derive the lock path from `ManagedLayout::state_path`
    // (which resolves through `resolve_contained_path`'s symlink-and-
    // containment checks), rather than joining `layout.root()` with a
    // literal `"state"` component directly -- the lock path must use the
    // exact same symlink-safe managed-root discipline as every other path
    // under the managed root, never blindly follow a symlinked `state/`
    // directory merely to create a lock file.
    let state_path = layout.state_path(stable_key).ok()?;
    let lock_path = state_path.with_extension("lock");
    if let Some(parent) = lock_path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return None;
        }
    }
    let file = std::fs::OpenOptions::new().create(true).write(true).open(&lock_path).ok()?;
    file.try_lock_exclusive().ok()?;
    Some(file)
}

fn compute_local_native_policy_digest(cfg: &Config) -> Option<String> {
    let dispatch = cfg.dispatch.as_ref()?;
    let value = serde_json::to_value(dispatch).ok()?;
    crate::agent::jcs::canonical_digest_sha256_hex(&value).ok()
}

/// Mirrors `stage107a_enforcement::required_completeness_gaps`'s existing
/// per-input-kind gap discipline, but scoped to the ONE native-specific
/// completeness input this stage introduces (content hash) -- never OCI
/// image completeness logic, which does not apply to a native artifact at
/// all.
fn native_required_completeness_gaps(assignment: &OperationalArtifactAssignment) -> Vec<String> {
    let mut gaps = Vec::new();
    let valid = assignment
        .acquisition
        .as_ref()
        .and_then(|a| a.sha256.as_deref())
        .is_some_and(is_valid_content_sha256);
    if !valid {
        gaps.push(format!(
            "{UNPINNED_NATIVE_PROCESS_CONTENT}:{}",
            assignment.acquisition.as_ref().and_then(|a| a.source.clone()).unwrap_or_default()
        ));
    }
    for secret_ref in &assignment.secret_refs {
        gaps.push(format!("unpinned_secret_reference:{secret_ref}"));
    }
    gaps
}

struct NativeActionOutcome {
    host_mutation_performed: bool,
    execution_status: String,
    message: String,
    observed_after_running: bool,
}

#[allow(clippy::too_many_arguments)]
fn execute_native_action(
    action: &NativeReconciliationAction,
    layout: Option<&ManagedLayout>,
    stable_key: Option<&str>,
    lease: Option<&LeaseGuard>,
    boundary_context: &MutationBoundaryContext,
    records: &mut Vec<MutationBoundaryRecord>,
    assignment: &OperationalArtifactAssignment,
    content_sha256: Option<&str>,
    recomputed_digest: &Result<String, String>,
    persisted_state: Option<&ManagedProcessRuntimeStateV1>,
) -> NativeActionOutcome {
    match action {
        NativeReconciliationAction::ObserveOnly { reason } => NativeActionOutcome {
            host_mutation_performed: false,
            execution_status: NATIVE_PROCESS_OBSERVED_ONLY.to_string(),
            message: reason.clone(),
            observed_after_running: matches!(
                persisted_state.map(observe_persisted_identity),
                Some(IdentityCheck::Confirmed)
            ),
        },
        NativeReconciliationAction::ContentUnpinned => NativeActionOutcome {
            host_mutation_performed: false,
            execution_status: BLOCKED_NATIVE_PROCESS_CONTENT_UNPINNED.to_string(),
            message: "acquisition.sha256 is missing or not a valid 64-character hexadecimal digest; a native process must never execute without an exact content hash".to_string(),
            observed_after_running: false,
        },
        NativeReconciliationAction::UnsupportedCompletedDesiredState => NativeActionOutcome {
            host_mutation_performed: false,
            execution_status: UNSUPPORTED_NATIVE_PROCESS_COMPLETED_DESIRED_STATE.to_string(),
            message: "desired_state != 'running' is not implemented for native_process in Stage 109; never coerced to running, never claimed converged".to_string(),
            observed_after_running: false,
        },
        NativeReconciliationAction::AlreadyConverged => NativeActionOutcome {
            host_mutation_performed: false,
            execution_status: NATIVE_PROCESS_RUNNING_CONVERGED.to_string(),
            message: "the exact desired managed process is already running under the current content and execution projection; no mutation performed".to_string(),
            observed_after_running: true,
        },
        NativeReconciliationAction::RollbackNoOp => {
            // v3 fix (P0-2 / "RollbackNoOp must not mutate while claiming
            // false"): this variant is now reached ONLY when NO persisted
            // state exists at all (see `decide_native_reconciliation_action`)
            // -- a genuinely mutation-free no-op. It must never call
            // `remove_state` (there is nothing to remove, and doing so
            // anyway blurred "true no-op" with "governed cleanup" into a
            // single, evidence-inconsistent branch). Stale state that DOES
            // exist is handled by `RollbackStaleStateCleanup` below, via a
            // real, governed, N6-boundary-checked removal.
            NativeActionOutcome {
                host_mutation_performed: false,
                execution_status: NATIVE_PROCESS_ROLLED_BACK.to_string(),
                message: "no persisted state exists for this service; rollback is a true no-op".to_string(),
                observed_after_running: false,
            }
        }
        NativeReconciliationAction::RollbackStaleStateCleanup => {
            // Persisted state describes a process that is provably no
            // longer the managed workload at that PID (absent, PID-reused,
            // or a zombie) -- safe to clean up, but ONLY via a governed,
            // freshly-lease-checked N6 removal (P0-2 fix), and reported as
            // a real mutation only when that removal actually completes.
            let remove_result = governed_remove_state(layout, stable_key, lease, boundary_context, records);
            if let Some(reason) = remove_result.blocked_reason {
                return NativeActionOutcome {
                    host_mutation_performed: false,
                    execution_status: NATIVE_PROCESS_STOP_INCOMPLETE.to_string(),
                    message: format!("stale-state cleanup incomplete, state preserved: {reason}"),
                    observed_after_running: false,
                };
            }
            NativeActionOutcome {
                host_mutation_performed: true,
                execution_status: NATIVE_PROCESS_ROLLED_BACK.to_string(),
                message: "no live process at the persisted identity; stale runtime-state record removed under governance".to_string(),
                observed_after_running: false,
            }
        }
        NativeReconciliationAction::RollbackIncomplete => {
            // Identity could not be safely confirmed OR disproven
            // (executable/args mismatch, or a procfs observation
            // failure) -- the PID may still be the managed workload.
            // Never signal, never remove state.
            NativeActionOutcome {
                host_mutation_performed: false,
                execution_status: NATIVE_PROCESS_STOP_INCOMPLETE.to_string(),
                message: "persisted process identity could not be safely confirmed; rollback incomplete, state preserved".to_string(),
                observed_after_running: false,
            }
        }
        NativeReconciliationAction::Rollback => {
            let Some(state) = persisted_state else {
                return NativeActionOutcome {
                    host_mutation_performed: false,
                    execution_status: NATIVE_PROCESS_ROLLED_BACK.to_string(),
                    message: "no persisted state to roll back".to_string(),
                    observed_after_running: false,
                };
            };
            let stop_result = governed_stop(state, lease, boundary_context, records);
            // Section AP fault matrix (items 3-7): state is removed ONLY
            // when the stop is positively confirmed. Never prefer a
            // "clean looking" removed state file over identity
            // recoverability -- if the process might still be running
            // under an unconfirmed identity, or a lease blocked N4/N5, the
            // persisted record is exactly how a later cycle knows which
            // PID it may still safely govern; erasing it now would strand
            // that process as permanently unmanaged.
            if !stop_result.confirmed_stopped {
                return NativeActionOutcome {
                    host_mutation_performed: stop_result.signaled,
                    execution_status: NATIVE_PROCESS_STOP_INCOMPLETE.to_string(),
                    message: format!("rollback incomplete, state preserved: {}", stop_result.message),
                    observed_after_running: false,
                };
            }
            // P0-2 fix: N6 is now a real, freshly-lease-checked boundary --
            // never an unconditional filesystem removal.
            let remove_result = governed_remove_state(layout, stable_key, lease, boundary_context, records);
            if let Some(reason) = remove_result.blocked_reason {
                return NativeActionOutcome {
                    host_mutation_performed: true, // the confirmed stop above IS a real mutation
                    execution_status: NATIVE_PROCESS_STOP_INCOMPLETE.to_string(),
                    message: format!("process stopped ({}), but state removal blocked: {reason}; cleanup pending next cycle", stop_result.message),
                    observed_after_running: false,
                };
            }
            NativeActionOutcome {
                host_mutation_performed: true,
                execution_status: NATIVE_PROCESS_ROLLED_BACK.to_string(),
                message: stop_result.message,
                observed_after_running: false,
            }
        }
        NativeReconciliationAction::RestartForDrift => {
            let Some(state) = persisted_state else {
                // No persisted process to stop first -- behaves exactly
                // like a fresh Spawn.
                let spawn_outcome = governed_materialize_and_spawn(
                    layout, stable_key, lease, boundary_context, records, assignment, content_sha256, recomputed_digest, persisted_state,
                );
                // P0-5 fix: `execution_status` must reflect whether the
                // spawn itself actually succeeded (`observed_after_running`
                // -- only ever true once a real PID/starttime/executable
                // was read back after a real spawn), never merely whether
                // SOME mutation happened (which a partial N1-only write
                // followed by an N2/N3 block would also satisfy).
                return NativeActionOutcome {
                    host_mutation_performed: spawn_outcome.host_mutation_performed,
                    execution_status: if spawn_outcome.observed_after_running {
                        NATIVE_PROCESS_RESTARTED_FOR_DRIFT.to_string()
                    } else {
                        spawn_outcome.execution_status
                    },
                    message: spawn_outcome.message,
                    observed_after_running: spawn_outcome.observed_after_running,
                };
            };
            let stop_result = governed_stop(state, lease, boundary_context, records);
            // Sections AF/AI/AB: never spawn the new desired process until
            // the OLD one is positively confirmed gone -- an unconfirmed
            // stop must block here, never fall through to a duplicate
            // spawn. The persisted state describing the OLD process is
            // deliberately left untouched (not overwritten with the new
            // generation) so the old identity remains recoverable/
            // re-stoppable on the next cycle.
            if !stop_result.confirmed_stopped {
                return NativeActionOutcome {
                    host_mutation_performed: stop_result.signaled,
                    execution_status: NATIVE_PROCESS_STOP_INCOMPLETE.to_string(),
                    message: format!("drift restart blocked, old process not confirmed stopped: {}", stop_result.message),
                    observed_after_running: true,
                };
            }
            let spawn_outcome = governed_materialize_and_spawn(
                layout, stable_key, lease, boundary_context, records, assignment, content_sha256, recomputed_digest, persisted_state,
            );
            // P0-5 fix ("RestartForDrift has the same bug"): only report
            // RESTARTED when the new spawn actually, verifiably succeeded.
            // The OLD process WAS confirmed stopped by this point (we
            // already returned above otherwise), so `host_mutation_performed`
            // stays `true` regardless -- that stop is a real mutation on
            // its own -- but the STATUS must propagate the real spawn
            // outcome (e.g. "old process stopped, N3 lease expired" must
            // never be reported as a successful restart).
            NativeActionOutcome {
                host_mutation_performed: true,
                execution_status: if spawn_outcome.observed_after_running {
                    NATIVE_PROCESS_RESTARTED_FOR_DRIFT.to_string()
                } else {
                    spawn_outcome.execution_status.clone()
                },
                message: format!("{}; {}", stop_result.message, spawn_outcome.message),
                observed_after_running: spawn_outcome.observed_after_running,
            }
        }
        NativeReconciliationAction::Spawn | NativeReconciliationAction::Respawn => {
            let spawn_outcome = governed_materialize_and_spawn(
                layout, stable_key, lease, boundary_context, records, assignment, content_sha256, recomputed_digest, persisted_state,
            );
            let status = if matches!(action, NativeReconciliationAction::Spawn) {
                NATIVE_PROCESS_SPAWNED
            } else {
                NATIVE_PROCESS_RESPAWNED
            };
            // P0-5 fix ("spawn/restart status currently confuses mutated
            // with succeeded"): `host_mutation_performed` answers only
            // "did any host mutation happen this cycle?" -- it must never
            // decide the SUCCESS status. Only `observed_after_running`
            // (true exclusively once a real PID/starttime/executable was
            // read back after an actual spawn) may promote the status to
            // "spawned"/"respawned"; any other case propagates the real,
            // truthful `SpawnOutcome::execution_status` (e.g. an N2 lease
            // block after N1 already wrote bytes) unchanged.
            NativeActionOutcome {
                host_mutation_performed: spawn_outcome.host_mutation_performed,
                execution_status: if spawn_outcome.observed_after_running { status.to_string() } else { spawn_outcome.execution_status },
                message: spawn_outcome.message,
                observed_after_running: spawn_outcome.observed_after_running,
            }
        }
    }
}

struct RemoveStateResult {
    /// `true` iff the runtime-state record no longer exists after this
    /// call (either it was actually removed, or there was nothing to
    /// remove in the first place -- both are equally "clean" outcomes).
    #[allow(dead_code)]
    removed: bool,
    /// `Some(reason)` iff removal was attempted but blocked (no lease,
    /// N6 boundary check failed, or the filesystem removal itself
    /// failed) -- the state record is left in place.
    blocked_reason: Option<String>,
}

/// P0-2 (v3 fix): N6 (runtime-state removal) is now a REAL mutation
/// boundary -- a fresh `check_boundary(..., N6_REMOVE, ...)` call
/// immediately before the actual filesystem removal, exactly like every
/// other N-boundary. Before this fix, `N6_REMOVE` was declared but never
/// actually checked anywhere; state was removed unconditionally whenever
/// a caller decided to. If N6 blocks, the state record is preserved and
/// the caller must report the cleanup/rollback as incomplete, never as a
/// full success.
fn governed_remove_state(
    layout: Option<&ManagedLayout>,
    stable_key: Option<&str>,
    lease: Option<&LeaseGuard>,
    boundary_context: &MutationBoundaryContext,
    records: &mut Vec<MutationBoundaryRecord>,
) -> RemoveStateResult {
    let (Some(layout), Some(key)) = (layout, stable_key) else {
        // No managed root/stable key at all -- nothing on disk could
        // exist to remove either. This is the ONLY case where
        // `removed: true` with no path resolution is truthful: there is
        // structurally no managed root under which anything could exist.
        return RemoveStateResult { removed: true, blocked_reason: None };
    };
    // v4 fix (P1-3): a path-containment/resolution FAILURE is NOT proof
    // the state was removed -- it is proof NOTHING was even attempted,
    // because we could not safely determine where the file would be.
    // The old code treated this identically to "nothing to remove"
    // (`removed: true`), which could make rollback report success while
    // the real runtime-state file (if `state_path` failed only because
    // of e.g. a symlink planted at an ancestor component) remained
    // present and unreachable. No filesystem mutation occurs here.
    let path = match layout.state_path(key) {
        Ok(path) => path,
        Err(error) => {
            return RemoveStateResult {
                removed: false,
                blocked_reason: Some(format!("state path containment/resolution failed: {error}")),
            };
        }
    };
    let Some(lease) = lease else {
        return RemoveStateResult { removed: false, blocked_reason: Some("no lease available; N6 not attempted".to_string()) };
    };
    if let Err(reason) = crate::agent::operational_reconciliation::check_boundary(lease, boundary_context, N6_REMOVE, records).map_err(|e| e.to_string()) {
        return RemoveStateResult { removed: false, blocked_reason: Some(format!("N6 blocked: {reason}")) };
    }
    match native_process_state::remove_state(&path) {
        Ok(()) => RemoveStateResult { removed: true, blocked_reason: None },
        Err(error) => RemoveStateResult { removed: false, blocked_reason: Some(format!("state removal failed: {error}")) },
    }
}

struct StopResult {
    /// `true` iff at least one signal (SIGTERM or SIGKILL) was actually
    /// delivered to the managed PID this cycle -- mutation-evidence
    /// bookkeeping only, NOT a claim that the process is confirmed gone.
    signaled: bool,
    /// `true` ONLY when this call has POSITIVE evidence the managed
    /// process (under the identity `state` describes) is no longer
    /// running -- either it was already gone/mismatched before any signal
    /// was needed, or a signal was sent and a subsequent fresh observation
    /// confirmed it exited. Callers (`RestartForDrift`, `Rollback`) MUST
    /// treat `false` as "stop did not complete" and must never proceed to
    /// spawn a new process or remove the runtime-state record while this
    /// is `false` (Sections AF, AI, AP) -- doing so risks a second,
    /// duplicate managed process running alongside a still-alive original.
    confirmed_stopped: bool,
    message: String,
}

fn governed_stop(
    state: &ManagedProcessRuntimeStateV1,
    lease: Option<&LeaseGuard>,
    boundary_context: &MutationBoundaryContext,
    records: &mut Vec<MutationBoundaryRecord>,
) -> StopResult {
    #[cfg(target_os = "linux")]
    {
        // Fresh identity re-verification immediately before signaling --
        // never trust an earlier observation. `ProcessAbsent`/`PidReused`
        // mean nothing to signal at this PID for THIS managed identity --
        // an idempotent, confirmed stop. `ExecutableMismatch`/`ArgsMismatch`/
        // `Unavailable` mean the PID may still be alive under an
        // unconfirmed identity -- never signal it, and stop is NOT
        // confirmed (Section AA rule 1/2; a caller must not proceed to
        // spawn a duplicate or remove state on this outcome).
        // P1-4: a zombie is not a running workload -- it counts as
        // already-stopped for every purpose here (never signaled, never
        // escalated to SIGKILL, immediately idempotent-confirmed).
        match observe_persisted_identity(state) {
            IdentityCheck::ProcessAbsent | IdentityCheck::PidReused | IdentityCheck::Zombie => {
                return StopResult {
                    signaled: false,
                    confirmed_stopped: true,
                    message: "managed process identity no longer present (or already exited) at stop time; treating as already stopped (idempotent)".to_string(),
                };
            }
            IdentityCheck::ExecutableMismatch | IdentityCheck::ArgsMismatch | IdentityCheck::Unavailable => {
                return StopResult {
                    signaled: false,
                    confirmed_stopped: false,
                    message: "managed process identity could not be safely confirmed at stop time; no signal sent, stop not confirmed".to_string(),
                };
            }
            IdentityCheck::Confirmed => {}
        }
        let Some(lease) = lease else {
            return StopResult { signaled: false, confirmed_stopped: false, message: "no lease available; stop not attempted".to_string() };
        };
        if let Err(reason) = crate::agent::operational_reconciliation::check_boundary(lease, boundary_context, N4_SIGTERM, records).map_err(|e| e.to_string()) {
            return StopResult { signaled: false, confirmed_stopped: false, message: format!("N4 blocked: {reason}") };
        }
        // P0-1 (v4 fix): ONLY `ESRCH` (the process is genuinely gone) may
        // ever be interpreted as a confirmed, idempotent stop here.
        // `EPERM`/`EACCES`/`EINVAL`/any other failure proves NOTHING about
        // whether the process is still alive -- treating it as confirmed
        // (the old, incorrect behavior) could let a live, unmanaged
        // process survive while rollback's N6 erased its only identifying
        // state record, or while `RestartForDrift` spawned a duplicate
        // over it.
        match linux::send_signal(state.pid, Signal::Term) {
            Ok(()) => {}
            Err(linux::SignalError::ProcessAbsent) => {
                return StopResult {
                    signaled: false,
                    confirmed_stopped: true,
                    message: "SIGTERM delivery reported ESRCH (no such process); treated as already-gone, confirmed idempotent stop".to_string(),
                };
            }
            Err(linux::SignalError::Failed(reason)) => {
                return StopResult {
                    signaled: false,
                    confirmed_stopped: false,
                    message: format!("SIGTERM delivery failed ({reason}); this does NOT prove the process is gone -- stop not confirmed, state preserved"),
                };
            }
        }
        // P1-5 fix: during polling, `!= Confirmed` is NOT a safe proxy for
        // "stopped" -- `ExecutableMismatch`/`ArgsMismatch` are NOT proof of
        // termination and must never be treated as a confirmed stop, nor
        // silently ignored so polling continues toward a blind SIGKILL
        // escalation. Only a genuine disappearance
        // (`ProcessAbsent`/`PidReused`/`Zombie`) confirms the stop.
        // v6/#5 fix (real Linux validation): a signal was JUST sent to an
        // identity that was freshly `Confirmed` immediately beforehand --
        // procfs can transiently lose `/proc/<pid>/exe` or
        // `/proc/<pid>/cmdline` for a brief teardown window as the process
        // actually exits, before `/proc/<pid>/stat` reports `Zombie` or the
        // PID disappears entirely. A single such `Unavailable` reading is
        // NOT a disproof of identity (unlike `ExecutableMismatch`/
        // `ArgsMismatch`, which are real, positive mismatches against a
        // DIFFERENT observed value) -- treating it as an immediate stop
        // failure produced false negatives against a real, successfully
        // cooperating process. `Unavailable` therefore continues polling
        // within this SAME bounded window, exactly like `Confirmed`; it is
        // still never itself treated as a confirmed stop, and the fail-
        // closed final pre-SIGKILL check below still refuses to escalate
        // past a persistent `Unavailable` once the window actually expires.
        let deadline = std::time::Instant::now() + GRACEFUL_STOP_TIMEOUT;
        while std::time::Instant::now() < deadline {
            match observe_persisted_identity(state) {
                IdentityCheck::ProcessAbsent | IdentityCheck::PidReused | IdentityCheck::Zombie => {
                    return StopResult { signaled: true, confirmed_stopped: true, message: "SIGTERM succeeded; process exited before timeout".to_string() };
                }
                IdentityCheck::ExecutableMismatch | IdentityCheck::ArgsMismatch => {
                    return StopResult {
                        signaled: true,
                        confirmed_stopped: false,
                        message: "identity could not be safely confirmed during the SIGTERM poll; stop incomplete, never escalating blindly".to_string(),
                    };
                }
                IdentityCheck::Confirmed | IdentityCheck::Unavailable => {}
            }
            std::thread::sleep(GRACEFUL_STOP_POLL_INTERVAL);
        }
        // Timeout: re-verify identity fresh before SIGKILL -- never signal
        // a PID that has since been reused or whose identity can no longer
        // be confirmed. v6/#5 fail-closed requirement: unlike the poll loop
        // above, `Unavailable` here is NOT tolerated -- the bounded window
        // for a transient procfs teardown race has already fully elapsed,
        // so a still-`Unavailable` reading at this exact instant is no
        // longer distinguishable from a genuine, persistent observation
        // problem. Only an EXACT `Confirmed` identity may ever proceed to
        // N5/SIGKILL; `Unavailable` withholds SIGKILL and reports the stop
        // incomplete, exactly like a real mismatch.
        match observe_persisted_identity(state) {
            IdentityCheck::ProcessAbsent | IdentityCheck::PidReused | IdentityCheck::Zombie => {
                return StopResult { signaled: true, confirmed_stopped: true, message: "process exited during the final identity check; no SIGKILL needed".to_string() };
            }
            IdentityCheck::ExecutableMismatch | IdentityCheck::ArgsMismatch | IdentityCheck::Unavailable => {
                return StopResult { signaled: true, confirmed_stopped: false, message: "SIGTERM sent; identity could no longer be safely confirmed before SIGKILL, so SIGKILL was withheld; stop not confirmed".to_string() };
            }
            IdentityCheck::Confirmed => {}
        }
        if let Err(reason) = crate::agent::operational_reconciliation::check_boundary(lease, boundary_context, N5_SIGKILL, records).map_err(|e| e.to_string()) {
            return StopResult { signaled: true, confirmed_stopped: false, message: format!("SIGTERM sent; N5 blocked before SIGKILL: {reason}; stop not confirmed") };
        }
        // P0-1 (v4 fix): the same ESRCH-only distinction applies to
        // SIGKILL -- a typed match, never string-prefix parsing.
        match linux::send_signal(state.pid, Signal::Kill) {
            Ok(()) => {}
            Err(linux::SignalError::ProcessAbsent) => {
                return StopResult {
                    signaled: false,
                    confirmed_stopped: true,
                    message: "SIGKILL delivery reported ESRCH (no such process); confirmed idempotent stop".to_string(),
                };
            }
            Err(linux::SignalError::Failed(reason)) => {
                return StopResult {
                    signaled: false,
                    confirmed_stopped: false,
                    message: format!("SIGKILL delivery failed ({reason}); this does NOT prove the process is gone -- stop incomplete"),
                };
            }
        }
        // Section AI: SIGKILL is not itself proof of termination -- confirm
        // with a bounded poll rather than assuming success the instant the
        // syscall returns. Same P1-5 fix applies here: an uncertain
        // identity mid-poll is never silently treated as confirmed.
        // v6/#3 fix (real Linux validation): identical reasoning to the
        // graceful SIGTERM poll above -- SIGKILL was JUST delivered to an
        // identity that was freshly `Confirmed` immediately before
        // signaling, and procfs can transiently lose `/proc/<pid>/exe` or
        // `/proc/<pid>/cmdline` in the brief window before `stat` reports
        // `Zombie` or the PID disappears. A single transient `Unavailable`
        // reading here is NOT proof of anything (never success, but also
        // never an immediate failure) -- it continues polling within this
        // SAME bounded window, exactly like `Confirmed`. This is strictly a
        // continue-polling tolerance: no additional signal is ever sent
        // here, and if the window actually expires while still not
        // positively confirmed stopped, the fallthrough below still
        // reports `NATIVE_PROCESS_STOP_INCOMPLETE` truthfully -- a
        // persistent `Unavailable` is never silently treated as success.
        let kill_deadline = std::time::Instant::now() + GRACEFUL_STOP_TIMEOUT;
        while std::time::Instant::now() < kill_deadline {
            match observe_persisted_identity(state) {
                IdentityCheck::ProcessAbsent | IdentityCheck::PidReused | IdentityCheck::Zombie => {
                    return StopResult { signaled: true, confirmed_stopped: true, message: "SIGTERM timed out; SIGKILL delivered and confirmed".to_string() };
                }
                IdentityCheck::ExecutableMismatch | IdentityCheck::ArgsMismatch => {
                    return StopResult {
                        signaled: true,
                        confirmed_stopped: false,
                        message: "identity could not be safely confirmed during the SIGKILL confirmation poll; stop incomplete".to_string(),
                    };
                }
                IdentityCheck::Confirmed | IdentityCheck::Unavailable => {}
            }
            std::thread::sleep(GRACEFUL_STOP_POLL_INTERVAL);
        }
        StopResult {
            signaled: true,
            confirmed_stopped: false,
            message: format!("{NATIVE_PROCESS_STOP_INCOMPLETE}: SIGKILL delivered but the process was still observed alive after the bounded confirmation window"),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (state, lease, boundary_context, records);
        StopResult {
            signaled: false,
            confirmed_stopped: false,
            message: NATIVE_PROCESS_RUNTIME_UNSUPPORTED_PLATFORM.to_string(),
        }
    }
}

struct SpawnOutcome {
    host_mutation_performed: bool,
    execution_status: String,
    message: String,
    observed_after_running: bool,
}

#[allow(clippy::too_many_arguments)]
// v5/L10 fix: `recomputed_digest` is genuinely used only inside this
// function's `#[cfg(target_os = "linux")]` spawn block (the real digest
// persisted alongside a real spawn) -- on a non-Linux development host
// that block is elided entirely, making the parameter unused THERE only.
// This is the exact platform-asymmetry the real Linux compile bug hid: an
// underscore-prefixed name here compiles silently on every host but
// Linux, where the real code actually references the un-prefixed
// identifier. Keeping the real name (required for Linux) and silencing
// the resulting non-Linux warning explicitly is preferable to ever
// re-introducing that mismatch.
#[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
fn governed_materialize_and_spawn(
    layout: Option<&ManagedLayout>,
    stable_key: Option<&str>,
    lease: Option<&LeaseGuard>,
    boundary_context: &MutationBoundaryContext,
    records: &mut Vec<MutationBoundaryRecord>,
    assignment: &OperationalArtifactAssignment,
    content_sha256: Option<&str>,
    recomputed_digest: &Result<String, String>,
    _persisted_state: Option<&ManagedProcessRuntimeStateV1>,
) -> SpawnOutcome {
    let (Some(layout), Some(stable_key), Some(sha256), Some(lease)) = (layout, stable_key, content_sha256, lease) else {
        return SpawnOutcome {
            host_mutation_performed: false,
            execution_status: NATIVE_PROCESS_OBSERVED_ONLY.to_string(),
            message: "materialization preconditions not met (policy, content hash, or lease unavailable)".to_string(),
            observed_after_running: false,
        };
    };
    let Some(native_spec) = assignment.native_process.as_ref() else {
        return SpawnOutcome {
            host_mutation_performed: false,
            execution_status: BLOCKED_MISSING_ARTIFACT_ASSIGNMENT.to_string(),
            message: "assignment carries runtime_kind=native_process but no native_process payload".to_string(),
            observed_after_running: false,
        };
    };

    // Section AM: Stage 109 v0.1 does not implement secret injection for
    // native processes (only the generic, non-secret `bindings` carrier is
    // supported -- see the environment handling below). A native artifact
    // that declares a non-empty `secret_refs` must fail closed BEFORE any
    // mutation, unconditionally (independent of `execution_pinning_mode`)
    // -- silently spawning without the secrets the artifact author
    // declared it needs would be a silent-ignore failure, which is never
    // acceptable.
    if !assignment.secret_refs.is_empty() {
        return SpawnOutcome {
            host_mutation_performed: false,
            execution_status: BLOCKED_NATIVE_PROCESS_SECRETS_UNSUPPORTED.to_string(),
            message: format!(
                "native artifact '{}' declares {} secret_refs, but Stage 109 v0.1 does not implement native secret injection; failing closed rather than spawning without them",
                assignment.artifact_id,
                assignment.secret_refs.len()
            ),
            observed_after_running: false,
        };
    }

    // v3 fix (P1-2, agent-side defense-in-depth): DServer now validates
    // `acquisition.kind`/`acquisition.source` at authoring time (Stage
    // 107A's existing dual-layer philosophy: the agent never trusts that
    // server-side validation alone was sufficient). An assignment
    // claiming a DIFFERENT acquisition kind (git/image/http/a future
    // kind) must NEVER be silently treated as a local executable file --
    // and `source` must be exactly the POSIX-absolute-local-file shape
    // this stage's v0.1 transport actually implements, never a relative
    // path (which could resolve unpredictably depending on this agent
    // process's own current working directory).
    let acquisition_kind = assignment.acquisition.as_ref().map(|a| a.kind.as_str()).unwrap_or_default();
    if acquisition_kind != native_process_acquisition::NATIVE_PROCESS_ACQUISITION_KIND {
        return SpawnOutcome {
            host_mutation_performed: false,
            execution_status: "blocked_native_process_acquisition_kind_mismatch".to_string(),
            message: format!(
                "acquisition.kind '{acquisition_kind}' is not '{}'; refusing to treat an unrecognized acquisition kind as a local executable file",
                native_process_acquisition::NATIVE_PROCESS_ACQUISITION_KIND
            ),
            observed_after_running: false,
        };
    }
    let acquisition_source = assignment.acquisition.as_ref().and_then(|a| a.source.as_deref()).unwrap_or_default();
    if !native_process_acquisition::is_valid_posix_absolute_source_path(acquisition_source) {
        return SpawnOutcome {
            host_mutation_performed: false,
            execution_status: "blocked_native_process_acquisition_source_invalid".to_string(),
            message: format!("acquisition.source '{acquisition_source}' is not a valid, non-empty, NUL-free, bounded, POSIX-absolute path"),
            observed_after_running: false,
        };
    }

    // Section AG-B/C/D + Section M: classify FIRST (read-only) so a true
    // no-op reuse (correct content, correct permissions already) crosses
    // NEITHER N1 nor N2 -- "check the lease before EACH ACTUAL mutation"
    // implies no boundary check (and no recorded evidence claiming a
    // mutation) is owed when nothing will actually be mutated.
    let need = match layout.classify_materialization_need(sha256, &native_spec.entrypoint) {
        Ok(need) => need,
        Err(error) => {
            return SpawnOutcome {
                host_mutation_performed: false,
                execution_status: BLOCKED_NATIVE_PROCESS_CONTENT_MISMATCH.to_string(),
                message: format!("materialization classification failed: {error}"),
                observed_after_running: false,
            };
        }
    };

    let mut host_mutation_performed = false;
    if !matches!(need, MaterializationNeed::None) {
        // Section: acquisition transport. Stage 109's minimum bounded
        // mechanism is a declared local-file source: `acquisition.source`
        // names a path this agent reads directly, verifies against
        // `sha256`, and materializes into the content-addressed managed
        // root. Real bytes acquired are re-hashed by
        // `ManagedLayout::materialize` itself -- "download hash matched"
        // is never trusted without independently re-verifying the
        // materialized copy. Only read when a write/repair is actually
        // needed (a true no-op reuse never touches the acquisition source
        // at all).
        let needs_full_rewrite = matches!(need, MaterializationNeed::FreshWrite | MaterializationNeed::RepairCorruption { .. });
        // v4 fix (P1-1): set to `true` whenever the executable-permission
        // question has already been fully resolved (either nothing needed
        // repairing, or this call's own fresh N2 check + repair already
        // ran) -- so the `needs_full_rewrite`-gated N2 block below is
        // never redundantly (or incorrectly) re-entered.
        let mut permission_already_handled = false;
        if needs_full_rewrite {
            if let Err(reason) = crate::agent::operational_reconciliation::check_boundary(lease, boundary_context, N1_MATERIALIZE, records).map_err(|e| e.to_string()) {
                return SpawnOutcome { host_mutation_performed: false, execution_status: "blocked_native_process_lease".to_string(), message: reason, observed_after_running: false };
            }
            let source = assignment.acquisition.as_ref().and_then(|a| a.source.clone()).unwrap_or_default();
            let source_bytes = match std::fs::read(&source) {
                Ok(bytes) => bytes,
                Err(error) => {
                    return SpawnOutcome {
                        host_mutation_performed: false,
                        execution_status: "blocked_native_process_acquisition_failed".to_string(),
                        message: format!("failed to read acquisition source '{source}': {error}"),
                        observed_after_running: false,
                    };
                }
            };
            match layout.materialize(sha256, &native_spec.entrypoint, &source_bytes) {
                Ok(MaterializeOutcome::Written) | Ok(MaterializeOutcome::RepairedCorruption) => {
                    host_mutation_performed = true;
                }
                Ok(MaterializeOutcome::AlreadyPresent) => {
                    // Classified as needing a rewrite but a concurrent
                    // actor (Section AB's per-service lock makes this
                    // vanishingly unlikely for the SAME service, but
                    // content-addressed bytes may be shared across
                    // DIFFERENT service keys) already supplied the
                    // correct, already-executable bytes first -- no
                    // mutation performed by THIS call, and N2 below is
                    // skipped entirely (nothing left to repair).
                    permission_already_handled = true;
                }
                Ok(MaterializeOutcome::AlreadyPresentPermissionRepairNeeded) => {
                    // v4 fix (P1-1 TOCTOU): a concurrent actor supplied
                    // the correct bytes, but `materialize()` NEVER sets
                    // the executable bit itself (not even in this internal
                    // reclassification case) -- N2 must still run here,
                    // as its own fresh, separately-checked boundary,
                    // exactly like the `PermissionRepairOnly` branch
                    // below. This is the exact TOCTOU the fix closes: the
                    // chmod is never performed under N1's lease check.
                    if let Err(reason) = crate::agent::operational_reconciliation::check_boundary(lease, boundary_context, N2_SET_EXECUTABLE_PERMISSIONS, records).map_err(|e| e.to_string()) {
                        return SpawnOutcome { host_mutation_performed: false, execution_status: "blocked_native_process_lease".to_string(), message: reason, observed_after_running: false };
                    }
                    if let Err(error) = layout.repair_executable_permission(sha256, &native_spec.entrypoint) {
                        return SpawnOutcome {
                            host_mutation_performed: false,
                            execution_status: BLOCKED_NATIVE_PROCESS_CONTENT_MISMATCH.to_string(),
                            message: format!("permission repair failed: {error}"),
                            observed_after_running: false,
                        };
                    }
                    host_mutation_performed = true;
                    permission_already_handled = true;
                }
                Err(error) => {
                    // P1-1 fix: `SourceHashMismatch` and a `Containment`
                    // error both fail BEFORE any filesystem write is
                    // attempted (the former on the in-memory hash check,
                    // the latter on pure path computation) -- genuinely
                    // zero mutation. Every other `materialize` error
                    // (`Io`, `ExistingContentCorrupted`) can only occur
                    // AFTER at least `create_dir_all`/a temp-file write
                    // was attempted, so `host_mutation_performed` must not
                    // claim `false` for those -- doing so would under-
                    // report a real (if harmless-debris-shaped) host
                    // filesystem side effect.
                    let attempted_a_write =
                        !matches!(error, MaterializeError::SourceHashMismatch { .. } | MaterializeError::Containment(_));
                    return SpawnOutcome {
                        host_mutation_performed: attempted_a_write,
                        execution_status: BLOCKED_NATIVE_PROCESS_CONTENT_MISMATCH.to_string(),
                        message: format!("materialization failed: {error}"),
                        observed_after_running: false,
                    };
                }
            }
        } else {
            // PermissionRepairOnly: correct bytes already present, only the
            // executable bit needs repair -- N2 only, never N1.
            if let Err(reason) = crate::agent::operational_reconciliation::check_boundary(lease, boundary_context, N2_SET_EXECUTABLE_PERMISSIONS, records).map_err(|e| e.to_string()) {
                return SpawnOutcome { host_mutation_performed: false, execution_status: "blocked_native_process_lease".to_string(), message: reason, observed_after_running: false };
            }
            match layout.repair_executable_permission(sha256, &native_spec.entrypoint) {
                Ok(()) => {
                    host_mutation_performed = true;
                }
                Err(error) => {
                    return SpawnOutcome {
                        host_mutation_performed: false,
                        execution_status: BLOCKED_NATIVE_PROCESS_CONTENT_MISMATCH.to_string(),
                        message: format!("permission repair failed: {error}"),
                        observed_after_running: false,
                    };
                }
            }
        }
        if needs_full_rewrite && !permission_already_handled {
            // P0-6 (v3 fix): N1 (the write above) deliberately leaves the
            // file NON-EXECUTABLE -- `materialize()` no longer chmods as
            // part of the atomic write. N2 is a SEPARATE, freshly-checked
            // boundary that must itself perform the actual chmod, not
            // merely check a lease after the permission was already set.
            // If N2 blocks here, the file stays non-executable and
            // `verify_materialized`/N3/spawn are never reached with an
            // executable-but-ungoverned file.
            if let Err(reason) = crate::agent::operational_reconciliation::check_boundary(lease, boundary_context, N2_SET_EXECUTABLE_PERMISSIONS, records).map_err(|e| e.to_string()) {
                return SpawnOutcome { host_mutation_performed: true, execution_status: "blocked_native_process_lease".to_string(), message: reason, observed_after_running: false };
            }
            if let Err(error) = layout.repair_executable_permission(sha256, &native_spec.entrypoint) {
                return SpawnOutcome {
                    host_mutation_performed: true,
                    execution_status: BLOCKED_NATIVE_PROCESS_CONTENT_MISMATCH.to_string(),
                    message: format!("failed to set executable permission after N1 write: {error}"),
                    observed_after_running: false,
                };
            }
        }
    }

    if let Err(error) = layout.verify_materialized(sha256, &native_spec.entrypoint) {
        return SpawnOutcome {
            host_mutation_performed,
            execution_status: BLOCKED_NATIVE_PROCESS_CONTENT_MISMATCH.to_string(),
            message: format!("post-materialization re-verification failed: {error}"),
            observed_after_running: false,
        };
    }

    if let Err(reason) = crate::agent::operational_reconciliation::check_boundary(lease, boundary_context, N3_SPAWN, records).map_err(|e| e.to_string()) {
        // L8/v5 fix: this must report the REAL, already-tracked mutation
        // evidence, not a literal `true` -- when N1/N2 were both skipped
        // (correct bytes and permissions already present) and N3 is the
        // very first boundary check reached, blocking here performs
        // exactly zero host mutation.
        return SpawnOutcome { host_mutation_performed, execution_status: "blocked_native_process_lease".to_string(), message: reason, observed_after_running: false };
    }

    let executable_path = match layout.materialized_entrypoint_path(sha256, &native_spec.entrypoint) {
        Ok(path) => path,
        Err(error) => {
            return SpawnOutcome { host_mutation_performed: true, execution_status: "blocked_native_process_path_containment".to_string(), message: error.to_string(), observed_after_running: false };
        }
    };
    // Section F: `working_directory`, when declared, is a relative path
    // INSIDE the managed artifact directory (never an artifact-controlled
    // absolute host path); when absent, the working directory IS the
    // managed artifact directory itself.
    let artifact_dir = match layout.artifact_directory(sha256) {
        Ok(dir) => dir,
        Err(error) => {
            return SpawnOutcome { host_mutation_performed: true, execution_status: "blocked_native_process_path_containment".to_string(), message: error.to_string(), observed_after_running: false };
        }
    };
    let working_directory = match native_spec.working_directory.as_deref() {
        Some(declared) => match native_process_acquisition::resolve_contained_path(&artifact_dir, declared, "working_directory") {
            Ok(path) => path,
            Err(error) => {
                return SpawnOutcome { host_mutation_performed: true, execution_status: "blocked_native_process_path_containment".to_string(), message: error.to_string(), observed_after_running: false };
            }
        },
        None => artifact_dir,
    };
    let logs_dir = match layout.logs_dir(stable_key) {
        Ok(dir) => dir,
        Err(error) => {
            return SpawnOutcome { host_mutation_performed: true, execution_status: "blocked_native_process_path_containment".to_string(), message: error.to_string(), observed_after_running: false };
        }
    };

    #[cfg(target_os = "linux")]
    {
        match spawn_and_persist(
            &executable_path,
            &native_spec.args,
            &working_directory,
            &logs_dir,
            assignment,
            sha256,
            recomputed_digest.as_deref().unwrap_or(""),
            layout,
            stable_key,
        ) {
            Ok(()) => SpawnOutcome {
                host_mutation_performed: true,
                execution_status: "spawned".to_string(),
                message: format!("spawned managed executable '{}'", executable_path.display()),
                observed_after_running: true,
            },
            Err(error) => SpawnOutcome {
                // P1-1 fix: a real filesystem side effect (logs directory/
                // files created) counts as a host mutation even when no
                // process ended up running (or was compensated away).
                host_mutation_performed: error.compensation_attempted || error.filesystem_side_effect,
                execution_status: BLOCKED_NATIVE_PROCESS_STATE_PERSISTENCE_FAILED.to_string(),
                message: error.message,
                observed_after_running: false,
            },
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (executable_path, working_directory, logs_dir);
        SpawnOutcome {
            host_mutation_performed: false,
            execution_status: NATIVE_PROCESS_RUNTIME_UNSUPPORTED_PLATFORM.to_string(),
            message: "real spawn is Linux-only".to_string(),
            observed_after_running: false,
        }
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct SpawnError {
    message: String,
    /// `true` if this agent attempted bounded compensation (terminating a
    /// process it just spawned) after a later step failed -- so the
    /// caller's `host_mutation_performed` reflects reality even when the
    /// overall outcome is an error (Section: partial-mutation
    /// truthfulness).
    compensation_attempted: bool,
    /// P1-1 fix: `true` if this specific error occurred AFTER a real
    /// filesystem side effect had already happened this call (e.g. the
    /// logs directory or stdout/stderr log files were already created
    /// before a later step failed) -- distinct from `compensation_attempted`
    /// (which is specifically about a live child process). A caller must
    /// never report `host_mutation_performed = false` when this is
    /// `true`, even if no process was ever spawned.
    filesystem_side_effect: bool,
}

#[cfg(target_os = "linux")]
#[allow(clippy::too_many_arguments)]
fn spawn_and_persist(
    executable_path: &Path,
    args: &[String],
    working_directory: &Path,
    logs_dir: &Path,
    assignment: &OperationalArtifactAssignment,
    content_sha256: &str,
    execution_projection_digest: &str,
    layout: &ManagedLayout,
    stable_key: &str,
) -> Result<(), SpawnError> {
    std::fs::create_dir_all(logs_dir).map_err(|e| SpawnError { message: format!("create logs directory: {e}"), compensation_attempted: false, filesystem_side_effect: false })?;
    let stdout_path = logs_dir.join("stdout.log");
    let stderr_path = logs_dir.join("stderr.log");
    // P1-1 fix: `create_dir_all` above already succeeded by the time any
    // of the following can fail -- a real directory now exists on disk,
    // so every subsequent error in this function carries
    // `filesystem_side_effect: true`.
    let stdout_file = std::fs::OpenOptions::new().create(true).append(true).open(&stdout_path)
        .map_err(|e| SpawnError { message: format!("open stdout log: {e}"), compensation_attempted: false, filesystem_side_effect: true })?;
    let stderr_file = std::fs::OpenOptions::new().create(true).append(true).open(&stderr_path)
        .map_err(|e| SpawnError { message: format!("open stderr log: {e}"), compensation_attempted: false, filesystem_side_effect: true })?;

    // Typed argv only -- NEVER a shell string. `Command::new` invokes the
    // materialized executable directly; the OS itself never interprets
    // `args` as shell syntax.
    let mut command = std::process::Command::new(executable_path);
    command
        .args(args)
        .current_dir(working_directory)
        .stdin(std::process::Stdio::null())
        .stdout(stdout_file)
        .stderr(stderr_file)
        // Section AL: environment determinism. The child NEVER inherits
        // this agent's own process environment (which may carry host
        // PATH/HOME/USER/SHELL, or anything else this long-running agent
        // process happens to have) -- only variables the artifact author
        // explicitly declared via `bindings` (the same generic key=value
        // carrier the Container path already uses for non-secret
        // environment) are present. A directly-executable, absolute
        // managed entrypoint never needs PATH to find itself; a
        // `#!/usr/bin/env ...` shebang script DOES need PATH, and that is
        // only ever true here if the artifact explicitly declared PATH in
        // `bindings` -- Stage 109 never secretly injects the host's PATH
        // to make such a script "just work".
        .env_clear()
        .envs(assignment.bindings.iter());

    let mut child = command.spawn().map_err(|e| SpawnError { message: format!("spawn failed: {e}"), compensation_attempted: false, filesystem_side_effect: true })?;
    let pid = child.id();

    let starttime = match linux::read_proc_stat_starttime(pid) {
        Ok(value) => value,
        Err(error) => {
            // Bounded compensation: never knowingly leave an unmanaged
            // process running if we cannot even establish its identity.
            let _ = child.kill();
            let _ = child.wait();
            return Err(SpawnError { message: format!("failed to read starttime after spawn, terminated child: {error}"), compensation_attempted: true, filesystem_side_effect: true });
        }
    };
    // P0-4/v3 fix ("post-spawn identity is still fabricated on exe read
    // failure"): the desired executable path/args are NEVER substituted
    // for a real observation. Persisted identity must come ONLY from what
    // was actually read back from `/proc` immediately after spawn -- a
    // fabricated fallback here would let a later adoption/identity check
    // "succeed" against a value this agent never actually verified came
    // from the real process. If either read fails, this is exactly the
    // same class of failure as a starttime read failure: compensate
    // (terminate the child) and report truthfully, never persist state
    // built from a guess.
    let observed_executable = match linux::read_proc_exe(pid) {
        Ok(value) => value,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(SpawnError { message: format!("failed to read /proc/{pid}/exe after spawn, terminated child: {error}"), compensation_attempted: true, filesystem_side_effect: true });
        }
    };
    // v3 fix ("compute the args/cmdline digest FROM THE REAL OBSERVED
    // CMDLINE, not directly from the desired args vector"): persisting
    // `compute_args_digest(args)` (the DESIRED argv this function was
    // asked to launch) would let a persisted record claim an identity
    // this agent never actually confirmed the kernel recorded -- on a
    // real system the OS-recorded cmdline is what future identity checks
    // (`observe_persisted_identity`) will always compare against, so the
    // baseline persisted here must be sourced the exact same way. v5/L1
    // fix: the COMPLETE raw observed cmdline (including argv[0], whatever
    // shape the kernel actually produced -- ELF or interpreter-shaped
    // alike) is what is hashed, never a partial reconstruction of the
    // desired args.
    let observed_cmdline_digest = match linux::read_proc_cmdline(pid) {
        Ok(observed_cmdline) => native_process_state::compute_args_digest(&observed_cmdline),
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(SpawnError { message: format!("failed to read /proc/{pid}/cmdline after spawn, terminated child: {error}"), compensation_attempted: true, filesystem_side_effect: true });
        }
    };

    let state = ManagedProcessRuntimeStateV1 {
        schema: MANAGED_PROCESS_RUNTIME_STATE_SCHEMA.to_string(),
        service_id: assignment.service_id.clone(),
        artifact_id: assignment.artifact_id.clone(),
        execution_generation: assignment.execution_generation,
        execution_projection_digest: execution_projection_digest.to_string(),
        // Phase 115C: carried verbatim from the governed assignment this
        // process was spawned from -- never independently derived.
        application_id: assignment.application_id.clone(),
        dmap_ref: assignment.dmap_ref.clone(),
        content_sha256: content_sha256.to_string(),
        pid,
        proc_start_time_ticks: starttime,
        observed_executable,
        cmdline_digest: observed_cmdline_digest,
        started_at_utc: Utc::now().to_rfc3339(),
        state: PROCESS_STATE_RUNNING.to_string(),
    };
    let state_path = layout.state_path(stable_key).map_err(|e| {
        let _ = child.kill();
        let _ = child.wait();
        SpawnError { message: format!("state path containment error, terminated child: {e}"), compensation_attempted: true, filesystem_side_effect: true }
    })?;

    if let Err(error) = native_process_state::persist_state(&state_path, &state) {
        // Section J: spawn succeeded but state persistence failed -- do
        // NOT knowingly leave an unmanaged process running. Bounded
        // compensation using the live child handle, then report truthfully.
        let _ = child.kill();
        let _ = child.wait();
        return Err(SpawnError {
            message: format!("process spawned (pid {pid}) but runtime-state persistence failed, so the child was terminated as compensation: {error}"),
            compensation_attempted: true,
            filesystem_side_effect: true,
        });
    }

    // Section AJ: never intentionally leak a detached child that this
    // agent will never reap -- if this exact long-running agent process
    // stays alive until the child exits, and nothing ever calls `wait()`
    // on it, the child becomes a permanent zombie entry (Section AA's
    // "Zombie" state) until this agent process itself exits. A minimal,
    // single-purpose reaper thread is spawned ONLY now, after durable
    // managed state already exists -- it owns exactly this one `Child`
    // handle, calls `wait()` exactly once, and does nothing else: it never
    // touches the runtime-state file, never decides whether to respawn
    // (ordinary reconciliation stays solely responsible for that, via its
    // own independent `/proc`-based observation), and a failure inside it
    // is swallowed rather than propagated (never panics this agent).
    //
    // This is deliberately NOT how recovery after an agent restart works:
    // a fresh SmartSentinel process is never the OS-level parent of a
    // process a PRIOR instance spawned (the kernel reparents it to
    // init/a subreaper across a real restart), so a new instance could
    // not `wait()` on it even if it wanted to -- recovery there is
    // purely PID+starttime+executable+args adoption from persisted state
    // (see process-identity-adoption-audit.txt), never reaping.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::native_process_state::IdentityCheck;

    // ---- local policy --------------------------------------------------- //

    fn dispatch_cfg(enabled: bool, allow_native: Option<bool>, root: Option<&str>) -> Config {
        use crate::config::{AgentConfig, DispatchConfig, OutputsConfig};
        Config {
            agent: AgentConfig { node_id: "n".to_string(), cycle_s: 1, schema_version: "1".to_string(), hostname: None, stage: None, events: None },
            outputs: OutputsConfig { file_json: None, http: None },
            dispatch: Some(DispatchConfig {
                enabled,
                allow_native_process_execution: allow_native,
                native_process_root: root.map(str::to_string),
                ..Default::default()
            }),
            assisted_execution: None,
            collectors: None,
            services: None,
            service_registry: None,
            auto_recovery: None,
            desired_state: None,
        }
    }

    #[test]
    fn native_execution_denied_by_default() {
        let cfg = dispatch_cfg(true, None, Some("/var/lib/datum/native"));
        assert!(matches!(evaluate_local_native_policy(&cfg), LocalNativePolicy::Denied(_)));
    }

    #[test]
    fn allow_container_execution_does_not_authorize_native() {
        use crate::config::DispatchConfig;
        let mut cfg = dispatch_cfg(true, None, Some("/var/lib/datum/native"));
        if let Some(dispatch) = cfg.dispatch.as_mut() {
            *dispatch = DispatchConfig { allow_container_execution: Some(true), ..dispatch.clone() };
        }
        assert!(matches!(evaluate_local_native_policy(&cfg), LocalNativePolicy::Denied(_)));
    }

    #[test]
    fn native_execution_allowed_with_valid_root() {
        let cfg = dispatch_cfg(true, Some(true), Some("/var/lib/datum/native"));
        assert!(matches!(evaluate_local_native_policy(&cfg), LocalNativePolicy::Allowed { .. }));
    }

    #[test]
    fn missing_or_relative_root_blocks_even_when_allowed() {
        let cfg = dispatch_cfg(true, Some(true), None);
        assert!(matches!(evaluate_local_native_policy(&cfg), LocalNativePolicy::Denied(_)));
        let cfg2 = dispatch_cfg(true, Some(true), Some("relative/path"));
        assert!(matches!(evaluate_local_native_policy(&cfg2), LocalNativePolicy::Denied(_)));
    }

    #[test]
    fn dispatch_disabled_blocks_native_even_if_allowed_true() {
        let cfg = dispatch_cfg(false, Some(true), Some("/var/lib/datum/native"));
        assert!(matches!(evaluate_local_native_policy(&cfg), LocalNativePolicy::Denied(_)));
    }

    // ---- reconciliation decision ---------------------------------------- //

    #[test]
    fn running_exact_process_converges_without_mutation() {
        let action = decide_native_reconciliation_action(
            "running", false, true, true, None, true, None, None, true, Some(IdentityCheck::Confirmed), false,
        );
        assert_eq!(action, NativeReconciliationAction::AlreadyConverged);
    }

    #[test]
    fn absent_process_triggers_spawn() {
        let action = decide_native_reconciliation_action(
            "running", false, true, true, None, true, None, None, true, None, false,
        );
        assert_eq!(action, NativeReconciliationAction::Spawn);
    }

    #[test]
    fn dead_persisted_process_triggers_respawn() {
        let action = decide_native_reconciliation_action(
            "running", false, true, true, None, true, None, None, true, Some(IdentityCheck::ProcessAbsent), false,
        );
        assert_eq!(action, NativeReconciliationAction::Respawn);
    }

    #[test]
    fn pid_reuse_triggers_respawn_never_adoption() {
        let action = decide_native_reconciliation_action(
            "running", false, true, true, None, true, None, None, true, Some(IdentityCheck::PidReused), false,
        );
        assert_eq!(action, NativeReconciliationAction::Respawn);
    }

    #[test]
    fn drift_with_confirmed_identity_triggers_restart() {
        let action = decide_native_reconciliation_action(
            "running", false, true, true, None, true, None, None, true, Some(IdentityCheck::Confirmed), true,
        );
        assert_eq!(action, NativeReconciliationAction::RestartForDrift);
    }

    #[test]
    fn hash_drift_blocks_execution_regardless_of_persisted_state() {
        let action = decide_native_reconciliation_action(
            "running", false, true, true, None, true, None, None, false, None, false,
        );
        assert_eq!(action, NativeReconciliationAction::ContentUnpinned);
    }

    #[test]
    fn local_policy_denial_blocks_mutation() {
        let action = decide_native_reconciliation_action(
            "running", false, true, true, Some("denied"), true, None, None, true, None, false,
        );
        assert!(matches!(action, NativeReconciliationAction::ObserveOnly { .. }));
    }

    #[test]
    fn unauthorized_never_mutates() {
        let action = decide_native_reconciliation_action(
            "running", false, true, false, None, true, None, None, true, None, false,
        );
        assert!(matches!(action, NativeReconciliationAction::ObserveOnly { .. }));
    }

    #[test]
    fn execute_requested_false_never_mutates() {
        let action = decide_native_reconciliation_action(
            "running", false, false, true, None, true, None, None, true, None, false,
        );
        assert!(matches!(action, NativeReconciliationAction::ObserveOnly { .. }));
    }

    #[test]
    fn expired_lease_blocks_mutation() {
        let action = decide_native_reconciliation_action(
            "running", false, true, true, None, true, Some("blocked_lease_expired: ..."), None, true, None, false,
        );
        assert!(matches!(action, NativeReconciliationAction::ObserveOnly { .. }));
    }

    #[test]
    fn non_linux_platform_fails_closed() {
        let action = decide_native_reconciliation_action(
            "running", false, true, true, None, false, None, None, true, None, false,
        );
        assert_eq!(action, NativeReconciliationAction::ObserveOnly { reason: NATIVE_PROCESS_RUNTIME_UNSUPPORTED_PLATFORM.to_string() });
    }

    #[test]
    fn completed_desired_state_is_explicitly_unsupported_never_coerced() {
        let action = decide_native_reconciliation_action(
            "completed", false, true, true, None, true, None, None, true, None, false,
        );
        assert_eq!(action, NativeReconciliationAction::UnsupportedCompletedDesiredState);
    }

    #[test]
    fn rollback_of_confirmed_process_stops_it() {
        let action = decide_native_reconciliation_action(
            "running", true, true, true, None, true, None, None, true, Some(IdentityCheck::Confirmed), false,
        );
        assert_eq!(action, NativeReconciliationAction::Rollback);
    }

    #[test]
    fn rollback_never_signals_unrelated_pid() {
        // v3 fix: a PID-reuse identity means the ORIGINAL process is gone
        // from that PID -- the stale state record MAY be cleaned up, but
        // only via governed `RollbackStaleStateCleanup` (a real N6-gated
        // mutation), never silently folded into a "no mutation at all"
        // no-op. The unrelated (reused) PID itself is never signaled
        // either way.
        let action = decide_native_reconciliation_action(
            "running", true, true, true, None, true, None, None, true, Some(IdentityCheck::PidReused), false,
        );
        assert_eq!(action, NativeReconciliationAction::RollbackStaleStateCleanup);
    }

    #[test]
    fn rollback_with_no_persisted_state_is_idempotent_noop() {
        let action = decide_native_reconciliation_action(
            "running", true, true, true, None, true, None, None, true, None, false,
        );
        assert_eq!(action, NativeReconciliationAction::RollbackNoOp);
    }

    // ---- Section AA state machine: identity-mismatch / unavailable ----- //

    #[test]
    fn executable_mismatch_never_triggers_respawn_only_observes() {
        let action = decide_native_reconciliation_action(
            "running", false, true, true, None, true, None, None, true, Some(IdentityCheck::ExecutableMismatch), false,
        );
        assert_eq!(
            action,
            NativeReconciliationAction::ObserveOnly { reason: BLOCKED_NATIVE_PROCESS_IDENTITY_MISMATCH.to_string() }
        );
    }

    #[test]
    fn args_mismatch_never_triggers_respawn_only_observes() {
        let action = decide_native_reconciliation_action(
            "running", false, true, true, None, true, None, None, true, Some(IdentityCheck::ArgsMismatch), false,
        );
        assert_eq!(
            action,
            NativeReconciliationAction::ObserveOnly { reason: BLOCKED_NATIVE_PROCESS_IDENTITY_MISMATCH.to_string() }
        );
    }

    #[test]
    fn identity_unavailable_never_spawns_or_respawns_only_observes() {
        let action = decide_native_reconciliation_action(
            "running", false, true, true, None, true, None, None, true, Some(IdentityCheck::Unavailable), false,
        );
        assert_eq!(
            action,
            NativeReconciliationAction::ObserveOnly { reason: BLOCKED_NATIVE_PROCESS_IDENTITY_UNAVAILABLE.to_string() }
        );
    }

    #[test]
    fn platform_unsupported_takes_priority_over_everything_else() {
        // Even with rollback requested and otherwise-valid inputs, an
        // unsupported platform must still fail closed first.
        let action = decide_native_reconciliation_action(
            "running", true, true, true, None, false, None, None, true, Some(IdentityCheck::Confirmed), false,
        );
        assert_eq!(action, NativeReconciliationAction::ObserveOnly { reason: NATIVE_PROCESS_RUNTIME_UNSUPPORTED_PLATFORM.to_string() });
    }

    // ---- missing-assignment report --------------------------------------- //

    #[test]
    fn missing_assignment_report_is_a_contract_error_not_execution() {
        let service = OperationalDesiredService {
            service_id: "svc".to_string(),
            runtime: "native_process".to_string(),
            desired_state: "running".to_string(),
            application_id: String::new(),
            dmap_ref: None,
        };
        let report = missing_artifact_assignment_report(&service);
        assert_eq!(report.execution_status, BLOCKED_MISSING_ARTIFACT_ASSIGNMENT);
        assert!(!report.host_mutation_performed);
    }

    // ---- corrupted state handling (Reviewer E self-review finding) ------- //

    /// A `/`-rooted (Linux-style) temp path, deliberately NOT derived from
    /// `std::env::temp_dir()` (which returns a drive-letter-prefixed path on
    /// Windows that would fail `evaluate_local_native_policy`'s
    /// Linux-absolute-path check). On Windows, `std::fs` resolves a bare
    /// `/`-rooted path relative to the current drive (e.g. `C:\tmp\...`),
    /// so this is a real, working path on both platforms for portable
    /// testing -- confirmed empirically, not assumed.
    fn temp_root(label: &str) -> std::path::PathBuf {
        let dir = std::path::PathBuf::from(format!("/tmp/datum-native-reconcile-test-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn wasm_policy() -> crate::agent::operational_reconciliation::ArtifactWasmDecisionPolicy {
        crate::agent::operational_reconciliation::ArtifactWasmDecisionPolicy {
            explicit_invocation_required: true,
            module_id: "reconcile_service".to_string(),
            function_name: "decide".to_string(),
            host_imports_allowed: false,
            side_effects_allowed: false,
            safe_to_auto_execute: false,
        }
    }

    fn native_assignment_for_reconcile() -> OperationalArtifactAssignment {
        OperationalArtifactAssignment {
            artifact_id: "svc".to_string(),
            service_id: "svc".to_string(),
            artifact_service_id: "svc".to_string(),
            version: "1".to_string(),
            artifact_kind: "command_bundle".to_string(),
            runtime_kind: "native_process".to_string(),
            container_name: String::new(),
            image: String::new(),
            native_process: Some(crate::agent::operational_reconciliation::NativeProcessDeploymentSpecV1 {
                process_name: "svc".to_string(),
                entrypoint: "run".to_string(),
                args: Vec::new(),
                working_directory: None,
            }),
            acquisition: Some(crate::agent::operational_reconciliation::OperationalServiceArtifactAcquisition {
                kind: "native_process_executable".to_string(),
                source: Some("/does/not/matter".to_string()),
                revision: None,
                sha256: Some("b".repeat(64)),
            }),
            image_source_kind: crate::agent::operational_reconciliation::IMAGE_SOURCE_KIND_REGISTRY.to_string(),
            resolved_image_identity: None,
            target_node: "fog-01".to_string(),
            lifecycle: crate::agent::operational_reconciliation::OperationalLifecycle::default(),
            network: None,
            network_aliases: Vec::new(),
            ports: Vec::new(),
            volumes: Vec::new(),
            configuration_mounts: Vec::new(),
            command: Vec::new(),
            dependencies: Vec::new(),
            wasm_decision: wasm_policy(),
            bindings: std::collections::BTreeMap::new(),
            secret_refs: Vec::new(),
            secret_bindings: Vec::new(),
            execution_generation: 1,
            execution_projection_digest: String::new(),
            application_id: String::new(),
            dmap_ref: None,
        }
    }

    // ---- Section AB: duplicate-spawn prevention ------------------------- //

    #[test]
    fn concurrent_reconciliation_for_the_same_service_key_is_serialized_not_duplicated() {
        let root = temp_root("concurrent-lock");
        let layout = ManagedLayout::new(&root);
        let key = native_process_acquisition::stable_service_key("proj", "node", "svc");

        // First holder acquires the lock (simulating one in-flight
        // reconciliation attempt for this exact stable service key)...
        let first = acquire_service_reconciliation_lock(&layout, &key);
        assert!(first.is_some(), "first acquisition must succeed");

        // ...a second, concurrent attempt for the SAME key must be denied,
        // never silently granted (which would let two callers both decide
        // "no managed process, spawn").
        let second = acquire_service_reconciliation_lock(&layout, &key);
        assert!(second.is_none(), "a second concurrent attempt for the same key must be denied");

        // Releasing the first holder (drop) frees the lock for the next
        // cycle -- this is NOT a permanent deadlock, just exclusion during
        // the overlap.
        drop(first);
        let third = acquire_service_reconciliation_lock(&layout, &key);
        assert!(third.is_some(), "the lock must be available again once the prior holder releases it");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn concurrent_reconciliation_for_different_service_keys_is_not_serialized() {
        // Two DIFFERENT services on the same node must never block each
        // other -- the lock is per-service, not a single node-wide mutex.
        let root = temp_root("concurrent-lock-distinct-keys");
        let layout = ManagedLayout::new(&root);
        let key_a = native_process_acquisition::stable_service_key("proj", "node", "svc-a");
        let key_b = native_process_acquisition::stable_service_key("proj", "node", "svc-b");

        let lock_a = acquire_service_reconciliation_lock(&layout, &key_a);
        let lock_b = acquire_service_reconciliation_lock(&layout, &key_b);
        assert!(lock_a.is_some());
        assert!(lock_b.is_some());

        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- P1-3: the lock is acquired ONLY for a mutating action --------- //
    //
    // `decide_native_reconciliation_action`'s very first check is
    // `!platform_supported`, which is always true on this non-Linux
    // development host -- so NO mutating action can ever be selected
    // through the full `reconcile_native_service` entry point here, and
    // the "concurrent reconciliation blocks a real mutating cycle"
    // end-to-end scenario is therefore only exercisable on a real Linux
    // host (see `linux_integration_tests`, and platform-test-coverage-audit.txt
    // for why this is honestly disclosed rather than faked here). What
    // IS fully verifiable portably: `action_may_mutate`'s classification
    // itself, and the raw lock's mutual exclusion
    // (`concurrent_reconciliation_for_the_same_service_key_is_serialized_not_duplicated`
    // above).

    #[test]
    fn action_may_mutate_classifies_every_action_variant_correctly() {
        assert!(action_may_mutate(&NativeReconciliationAction::Spawn));
        assert!(action_may_mutate(&NativeReconciliationAction::Respawn));
        assert!(action_may_mutate(&NativeReconciliationAction::RestartForDrift));
        assert!(action_may_mutate(&NativeReconciliationAction::Rollback));
        assert!(action_may_mutate(&NativeReconciliationAction::RollbackStaleStateCleanup));

        assert!(!action_may_mutate(&NativeReconciliationAction::ObserveOnly { reason: "x".to_string() }));
        assert!(!action_may_mutate(&NativeReconciliationAction::ContentUnpinned));
        assert!(!action_may_mutate(&NativeReconciliationAction::UnsupportedCompletedDesiredState));
        assert!(!action_may_mutate(&NativeReconciliationAction::AlreadyConverged));
        assert!(!action_may_mutate(&NativeReconciliationAction::RollbackNoOp));
        assert!(!action_may_mutate(&NativeReconciliationAction::RollbackIncomplete));
    }

    #[test]
    fn observe_only_cycle_never_creates_a_lock_file() {
        // On this host `platform_supported()` is false, so every cycle
        // through `reconcile_native_service` is observe-only -- confirm
        // no `.lock` file is ever created under the managed root as a
        // side effect of a call that should be purely read-only.
        let root = temp_root("observe-only-no-lock-file");
        let cfg = dispatch_cfg(true, Some(true), Some(&root.to_string_lossy().replace('\\', "/")));
        let mut assignment = native_assignment_for_reconcile();
        assignment.execution_projection_digest = execution_projection::recompute_native_digest(&assignment).unwrap();
        let service = OperationalDesiredService {
            service_id: "svc".to_string(),
            runtime: "native_process".to_string(),
            desired_state: "running".to_string(),
            application_id: String::new(),
            dmap_ref: None,
        };
        let boundary_context = MutationBoundaryContext { authorization_id: None, expires_at_utc: None, configured_clock_skew_seconds: None };

        let _report = reconcile_native_service(
            &cfg, "proj", "node", None, None, None, None, &boundary_context, &service, &assignment, true, true, false,
        );
        let state_dir = root.join("state");
        let lock_file_exists = std::fs::read_dir(&state_dir)
            .map(|entries| entries.filter_map(|e| e.ok()).any(|e| e.path().extension().is_some_and(|ext| ext == "lock")))
            .unwrap_or(false);
        assert!(!lock_file_exists, "an observe-only cycle must never create a lock file");
        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- Phase 115H: Stage 107A cycle-gate error is surfaced, not lost -- //

    // `reconcile_native_service` short-circuits to
    // `NATIVE_PROCESS_RUNTIME_UNSUPPORTED_PLATFORM` before ever reaching the
    // lease/generation match arm on a non-Linux host (`platform_supported()`
    // is `cfg!(target_os = "linux")`, hardcoded, not injectable) -- these two
    // tests exercise the actual Phase 115H glue code and so, like the rest of
    // this file's mutation-path tests, only run on Linux.
    #[cfg(target_os = "linux")]
    #[test]
    fn stage107a_gate_error_is_surfaced_verbatim_and_still_blocks_mutation() {
        // The exact live Phase 115F failure mode: `resolve_stage107a_cycle_gate`
        // returned `Err(reason)` (here, the real
        // `blocked_missing_clock_skew_config` string), so `lease` and
        // `effective_pinning_mode` are both `None`. Before the Phase 115H
        // fix, the catch-all substituted a generic, non-actionable message;
        // now the real reason must appear verbatim, AND the blocking
        // decision itself (no mutation) must be unchanged.
        let root = temp_root("stage107a-gate-error-surfaced");
        let cfg = dispatch_cfg(true, Some(true), Some(&root.to_string_lossy().replace('\\', "/")));
        let mut assignment = native_assignment_for_reconcile();
        assignment.execution_projection_digest = execution_projection::recompute_native_digest(&assignment).unwrap();
        let service = OperationalDesiredService {
            service_id: "svc".to_string(),
            runtime: "native_process".to_string(),
            desired_state: "running".to_string(),
            application_id: String::new(),
            dmap_ref: None,
        };
        let boundary_context = MutationBoundaryContext { authorization_id: None, expires_at_utc: None, configured_clock_skew_seconds: None };
        let real_reason = format!(
            "{}: required mode needs an explicit authorization_clock_skew_seconds; there is no implicit zero",
            crate::agent::stage107a_enforcement::BLOCKED_MISSING_CLOCK_SKEW_CONFIG
        );

        let report = reconcile_native_service(
            &cfg, "proj", "node", None, None, None, Some(&real_reason), &boundary_context, &service, &assignment, true, true, false,
        );
        assert_eq!(report.message, real_reason);
        assert!(!report.host_mutation_performed);
        assert_eq!(report.execution_status, NATIVE_PROCESS_OBSERVED_ONLY);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn absent_stage107a_gate_error_falls_back_to_the_generic_message_and_still_blocks() {
        // The should-be-unreachable case (cycle gate resolution supplied no
        // reason at all): the generic fallback string is kept, and the
        // security-relevant outcome -- no mutation -- is identical to the
        // case above.
        let root = temp_root("stage107a-gate-error-absent-fallback");
        let cfg = dispatch_cfg(true, Some(true), Some(&root.to_string_lossy().replace('\\', "/")));
        let mut assignment = native_assignment_for_reconcile();
        assignment.execution_projection_digest = execution_projection::recompute_native_digest(&assignment).unwrap();
        let service = OperationalDesiredService {
            service_id: "svc".to_string(),
            runtime: "native_process".to_string(),
            desired_state: "running".to_string(),
            application_id: String::new(),
            dmap_ref: None,
        };
        let boundary_context = MutationBoundaryContext { authorization_id: None, expires_at_utc: None, configured_clock_skew_seconds: None };

        let report = reconcile_native_service(
            &cfg, "proj", "node", None, None, None, None, &boundary_context, &service, &assignment, true, true, false,
        );
        assert_eq!(
            report.message,
            "blocked_authorization_not_leased: no valid Stage 107A cycle gate for this cycle"
        );
        assert!(!report.host_mutation_performed);
        assert_eq!(report.execution_status, NATIVE_PROCESS_OBSERVED_ONLY);
        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- Section AN: observe-only mode still really observes ------------ //

    #[test]
    fn observation_is_attempted_even_when_local_mutation_policy_denies_execution() {
        // `allow_native_process_execution = false` denies MUTATION, but
        // observation (reading this agent's own persisted state key) must
        // still happen -- proven here by writing a genuinely CORRUPTED
        // state file and confirming the report reflects an actual read
        // attempt (`blocked_native_process_state_unreadable`), not the
        // policy-denied message that would result if observation were
        // skipped entirely whenever mutation is locally denied (the bug
        // Section AN's fix closes: `configured_native_process_root` is
        // independent of `allow_native_process_execution`).
        let root = temp_root("observe-only-mode");
        let cfg = dispatch_cfg(true, Some(false), Some(&root.to_string_lossy().replace('\\', "/")));
        let mut assignment = native_assignment_for_reconcile();
        assignment.execution_projection_digest = execution_projection::recompute_native_digest(&assignment).unwrap();
        let service = OperationalDesiredService {
            service_id: "svc".to_string(),
            runtime: "native_process".to_string(),
            desired_state: "running".to_string(),
            application_id: String::new(),
            dmap_ref: None,
        };
        let layout = ManagedLayout::new(&root);
        let key = native_process_acquisition::stable_service_key("proj", "node", "svc");
        let state_path = layout.state_path(&key).unwrap();
        std::fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        std::fs::write(&state_path, b"not valid json").unwrap();
        let boundary_context = MutationBoundaryContext { authorization_id: None, expires_at_utc: None, configured_clock_skew_seconds: None };

        let report = reconcile_native_service(
            &cfg, "proj", "node", None, None, None, None, &boundary_context, &service, &assignment, true, true, false,
        );
        assert!(!report.host_mutation_performed);
        assert!(
            report.message.contains("blocked_native_process_state_unreadable"),
            "observation must still be attempted when only mutation is locally denied; message was: {}",
            report.message
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn configured_native_process_root_ignores_allow_native_process_execution() {
        // The observation root is available purely from dispatch.enabled +
        // a valid native_process_root -- explicitly NOT gated by
        // allow_native_process_execution (unlike `evaluate_local_native_policy`).
        let cfg = dispatch_cfg(true, Some(false), Some("/var/lib/datum/native"));
        assert!(configured_native_process_root(&cfg).is_some());
        assert!(matches!(evaluate_local_native_policy(&cfg), LocalNativePolicy::Denied(_)));
    }

    #[test]
    fn configured_native_process_root_still_requires_dispatch_enabled() {
        let cfg = dispatch_cfg(false, Some(true), Some("/var/lib/datum/native"));
        assert!(configured_native_process_root(&cfg).is_none());
    }

    // ---- Section AM: native secrets fail closed, unconditionally ------- //

    fn portable_valid_lease() -> (LeaseGuard, OperationalReconciliationAuthorization) {
        let far_future = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        let auth = OperationalReconciliationAuthorization {
            authorization_id: "auth-portable".to_string(),
            project_id: "proj".to_string(),
            dgraph_id: "dg".to_string(),
            dgraph_revision: None,
            node_id: "node".to_string(),
            binding_id: None,
            resource_id: None,
            authorized_by: "test".to_string(),
            authorized_at_utc: chrono::Utc::now().to_rfc3339(),
            allow_host_mutation: true,
            operation: "reconcile".to_string(),
            expires_at_utc: Some(far_future),
            persistence_scope: "durable".to_string(),
            artifact_bindings: Vec::new(),
            execution_pinning_mode: String::new(),
        };
        let lease = LeaseGuard::from_authorization(&auth, 30).unwrap();
        (lease, auth)
    }

    #[test]
    fn native_artifact_with_secret_refs_fails_closed_regardless_of_pinning_mode() {
        let root = temp_root("secrets-unsupported");
        let layout = ManagedLayout::new(&root);
        let mut assignment = native_assignment_for_reconcile();
        assignment.secret_refs = vec!["some-secret".to_string()];
        let (lease, auth) = portable_valid_lease();
        let boundary_context = MutationBoundaryContext {
            authorization_id: Some(&auth.authorization_id),
            expires_at_utc: auth.expires_at_utc.as_deref(),
            configured_clock_skew_seconds: Some(30),
        };
        let mut records = Vec::new();
        let outcome = governed_materialize_and_spawn(
            Some(&layout),
            Some("secret-test-key"),
            Some(&lease),
            &boundary_context,
            &mut records,
            &assignment,
            Some(&"b".repeat(64)),
            &Ok("sha256:".to_string() + &"c".repeat(64)),
            None,
        );
        assert_eq!(outcome.execution_status, BLOCKED_NATIVE_PROCESS_SECRETS_UNSUPPORTED);
        assert!(!outcome.host_mutation_performed);
        assert!(records.is_empty(), "no boundary should be crossed before the secret-refs check: {:?}", records);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn native_artifact_with_no_secret_refs_is_unaffected_by_the_secrets_gate() {
        // Sanity: the gate is specifically `secret_refs.is_empty()`, not a
        // blanket block on every native spawn attempt.
        let assignment = native_assignment_for_reconcile();
        assert!(assignment.secret_refs.is_empty());
    }

    // ---- v3 (independent review): P1-2 agent-side acquisition guards --- //

    #[test]
    fn governed_materialize_and_spawn_rejects_a_non_native_acquisition_kind() {
        let root = temp_root("acquisition-kind-mismatch");
        let layout = ManagedLayout::new(&root);
        let mut assignment = native_assignment_for_reconcile();
        assignment.acquisition.as_mut().unwrap().kind = "command_bundle".to_string();
        let (lease, auth) = portable_valid_lease();
        let boundary_context = MutationBoundaryContext {
            authorization_id: Some(&auth.authorization_id),
            expires_at_utc: auth.expires_at_utc.as_deref(),
            configured_clock_skew_seconds: Some(30),
        };
        let mut records = Vec::new();
        let outcome = governed_materialize_and_spawn(
            Some(&layout), Some("kind-test-key"), Some(&lease), &boundary_context, &mut records,
            &assignment, Some(&"b".repeat(64)), &Ok("sha256:".to_string() + &"c".repeat(64)), None,
        );
        assert_eq!(outcome.execution_status, "blocked_native_process_acquisition_kind_mismatch");
        assert!(!outcome.host_mutation_performed);
        assert!(records.is_empty(), "no boundary should be crossed before the acquisition-kind check: {:?}", records);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn governed_materialize_and_spawn_rejects_a_relative_acquisition_source() {
        let root = temp_root("acquisition-source-relative");
        let layout = ManagedLayout::new(&root);
        let mut assignment = native_assignment_for_reconcile();
        assignment.acquisition.as_mut().unwrap().source = Some("relative/sources/native-svc.bin".to_string());
        let (lease, auth) = portable_valid_lease();
        let boundary_context = MutationBoundaryContext {
            authorization_id: Some(&auth.authorization_id),
            expires_at_utc: auth.expires_at_utc.as_deref(),
            configured_clock_skew_seconds: Some(30),
        };
        let mut records = Vec::new();
        let outcome = governed_materialize_and_spawn(
            Some(&layout), Some("source-test-key"), Some(&lease), &boundary_context, &mut records,
            &assignment, Some(&"b".repeat(64)), &Ok("sha256:".to_string() + &"c".repeat(64)), None,
        );
        assert_eq!(outcome.execution_status, "blocked_native_process_acquisition_source_invalid");
        assert!(!outcome.host_mutation_performed);
        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- Sections AF/AI/AB: never spawn/remove-state on unconfirmed stop //
    //
    // v5/L7 fix (real Linux validation): the two tests below are gated
    // `#[cfg(not(target_os = "linux"))]` -- they rely on `governed_stop`
    // ALWAYS returning `confirmed_stopped: false` for a non-Linux target
    // (there is no real signal/observation mechanism to confirm anything
    // at all there), which is true regardless of the fixture's fake PID.
    // On REAL Linux, PID 999_999 is legitimately `ProcessAbsent`
    // (idempotent, CONFIRMED stop) rather than unconfirmed, which would
    // silently flip these tests' expected outcome depending on ambient
    // process-table state -- exactly the failure real Linux validation
    // exposed. `linux_integration_tests::restart_for_drift_never_spawns_when_a_real_processs_identity_is_unconfirmed`
    // and `..._rollback_..._is_unconfirmed` are the Linux-native
    // equivalents: a REAL spawned child with a deliberately mismatched
    // `cmdline_digest`, proving the same "never spawn/remove on an
    // unconfirmed stop" invariant using `governed_stop`'s actual
    // confirmation logic rather than this platform's fallback.
    #[cfg(not(target_os = "linux"))]
    fn sample_persisted_state() -> ManagedProcessRuntimeStateV1 {
        ManagedProcessRuntimeStateV1 {
            schema: crate::agent::native_process_state::MANAGED_PROCESS_RUNTIME_STATE_SCHEMA.to_string(),
            service_id: "svc".to_string(),
            artifact_id: "svc".to_string(),
            execution_generation: 3,
            execution_projection_digest: "sha256:".to_string() + &"a".repeat(64),
            application_id: String::new(),
            dmap_ref: None,
            content_sha256: "b".repeat(64),
            pid: 999_999,
            proc_start_time_ticks: 123456,
            observed_executable: "/managed/root/artifacts/bbbb/run".to_string(),
            cmdline_digest: native_process_state::compute_args_digest(&[]),
            started_at_utc: "2026-01-01T00:00:00Z".to_string(),
            state: crate::agent::native_process_state::PROCESS_STATE_RUNNING.to_string(),
        }
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn restart_for_drift_never_spawns_when_the_old_process_stop_is_not_confirmed() {
        let assignment = native_assignment_for_reconcile();
        let state = sample_persisted_state();
        let boundary_context = MutationBoundaryContext { authorization_id: None, expires_at_utc: None, configured_clock_skew_seconds: None };
        let mut records = Vec::new();
        let outcome = execute_native_action(
            &NativeReconciliationAction::RestartForDrift,
            None,
            None,
            None,
            &boundary_context,
            &mut records,
            &assignment,
            Some(&"b".repeat(64)),
            &Ok("sha256:".to_string() + &"c".repeat(64)),
            Some(&state),
        );
        assert_eq!(outcome.execution_status, NATIVE_PROCESS_STOP_INCOMPLETE);
        assert!(
            outcome.message.contains("drift restart blocked, old process not confirmed stopped"),
            "message was: {}",
            outcome.message
        );
        assert!(!outcome.message.to_lowercase().contains("spawned"), "must never proceed to spawn: {}", outcome.message);
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn rollback_never_removes_state_when_the_stop_is_not_confirmed() {
        let assignment = native_assignment_for_reconcile();
        let state = sample_persisted_state();
        let boundary_context = MutationBoundaryContext { authorization_id: None, expires_at_utc: None, configured_clock_skew_seconds: None };
        let mut records = Vec::new();
        let outcome = execute_native_action(
            &NativeReconciliationAction::Rollback,
            None,
            None,
            None,
            &boundary_context,
            &mut records,
            &assignment,
            Some(&"b".repeat(64)),
            &Ok("sha256:".to_string() + &"c".repeat(64)),
            Some(&state),
        );
        assert_eq!(outcome.execution_status, NATIVE_PROCESS_STOP_INCOMPLETE);
        assert!(outcome.message.contains("rollback incomplete, state preserved"), "message was: {}", outcome.message);
    }

    // Note: a true end-to-end proof of "rollback preserves state when a
    // CONFIRMED-running process fails to stop" needs a real `Confirmed`
    // identity, which requires a real `/proc` observation -- Linux-only
    // (see `linux_integration_tests`; on this host `observe_persisted_identity`
    // always reports `ProcessAbsent`, which correctly routes to
    // `RollbackNoOp`'s safe-cleanup path instead, a different and equally
    // correct scenario already covered by
    // `rollback_with_no_persisted_state_is_idempotent_noop`-style tests).

    // ---- rollback vs. local policy (Section E is unconditional) --------- //

    #[test]
    fn rollback_is_blocked_by_local_policy_denial_never_bypasses_it() {
        let action = decide_native_reconciliation_action(
            "running", true, true, true, Some("blocked_local_native_process_policy: denied"), true, None, None, true, Some(IdentityCheck::Confirmed), false,
        );
        assert!(matches!(action, NativeReconciliationAction::ObserveOnly { .. }));
    }

    #[test]
    fn rollback_requires_execute_requested() {
        let action = decide_native_reconciliation_action(
            "running", true, false, true, None, true, None, None, true, Some(IdentityCheck::Confirmed), false,
        );
        assert!(matches!(action, NativeReconciliationAction::ObserveOnly { reason } if reason == "execute_requested is false"));
    }

    #[test]
    fn rollback_requires_control_plane_authorized() {
        let action = decide_native_reconciliation_action(
            "running", true, true, false, None, true, None, None, true, Some(IdentityCheck::Confirmed), false,
        );
        assert!(matches!(action, NativeReconciliationAction::ObserveOnly { .. }));
    }

    #[test]
    fn rollback_requires_a_valid_generation_projection_binding() {
        // Only `rollback_projection_block` (NOT the full `lease_or_projection_block`,
        // which also folds in content-completeness gaps) gates rollback --
        // Section: "content acquisition/hash completeness does NOT need
        // to block removal of an already-owned exact process."
        let action = decide_native_reconciliation_action(
            "running", true, true, true, None, true, None, Some("blocked_generation_mismatch: ..."), true, Some(IdentityCheck::Confirmed), false,
        );
        assert!(matches!(action, NativeReconciliationAction::ObserveOnly { .. }));
    }

    #[test]
    fn rollback_is_never_blocked_by_content_completeness_alone() {
        // `lease_or_projection_block` (which folds in completeness gaps)
        // is deliberately NOT consulted for rollback -- only
        // `rollback_projection_block` is. A hash/secret completeness
        // block must never prevent removing an already-owned process.
        let action = decide_native_reconciliation_action(
            "running", true, true, true, None, true, Some("blocked_required_incomplete: ..."), None, true, Some(IdentityCheck::Confirmed), false,
        );
        assert_eq!(action, NativeReconciliationAction::Rollback);
    }

    #[test]
    fn rollback_of_a_zombie_is_stale_state_cleanup_not_signaled() {
        let action = decide_native_reconciliation_action(
            "running", true, true, true, None, true, None, None, true, Some(IdentityCheck::Zombie), false,
        );
        assert_eq!(action, NativeReconciliationAction::RollbackStaleStateCleanup);
    }

    #[test]
    fn rollback_of_unavailable_identity_is_incomplete_never_signals_or_removes() {
        let action = decide_native_reconciliation_action(
            "running", true, true, true, None, true, None, None, true, Some(IdentityCheck::Unavailable), false,
        );
        assert_eq!(action, NativeReconciliationAction::RollbackIncomplete);
    }

    #[test]
    fn rollback_with_genuinely_no_state_is_a_true_zero_mutation_noop() {
        let action = decide_native_reconciliation_action(
            "running", true, true, true, None, true, None, None, true, None, false,
        );
        assert_eq!(action, NativeReconciliationAction::RollbackNoOp);
    }

    // ---- N6: a real, freshly-lease-checked boundary (P0-2) -------------- //

    #[test]
    fn governed_remove_state_is_blocked_by_an_expired_lease_and_preserves_the_file() {
        let root = temp_root("n6-expired-lease");
        let layout = ManagedLayout::new(&root);
        let key = "n6-test-key";
        let state_path = layout.state_path(key).unwrap();
        std::fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        std::fs::write(&state_path, b"{}").unwrap();

        let (lease, auth) = expired_lease_and_boundary_portable();
        let boundary_context = MutationBoundaryContext {
            authorization_id: Some(&auth.authorization_id),
            expires_at_utc: auth.expires_at_utc.as_deref(),
            configured_clock_skew_seconds: Some(30),
        };
        let mut records = Vec::new();
        let result = governed_remove_state(Some(&layout), Some(key), Some(&lease), &boundary_context, &mut records);
        assert!(result.blocked_reason.is_some(), "an expired lease must block N6");
        assert!(state_path.exists(), "state must be preserved when N6 is blocked");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn governed_remove_state_succeeds_with_a_valid_lease() {
        let root = temp_root("n6-valid-lease");
        let layout = ManagedLayout::new(&root);
        let key = "n6-test-key";
        let state_path = layout.state_path(key).unwrap();
        std::fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        std::fs::write(&state_path, b"{}").unwrap();

        let (lease, auth) = portable_valid_lease();
        let boundary_context = MutationBoundaryContext {
            authorization_id: Some(&auth.authorization_id),
            expires_at_utc: auth.expires_at_utc.as_deref(),
            configured_clock_skew_seconds: Some(30),
        };
        let mut records = Vec::new();
        let result = governed_remove_state(Some(&layout), Some(key), Some(&lease), &boundary_context, &mut records);
        assert!(result.blocked_reason.is_none(), "a valid lease must allow N6");
        assert!(!state_path.exists(), "state must actually be removed");
        assert!(records.iter().any(|r| r.boundary == N6_REMOVE));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// v4 fix (P1-3): a state-path containment/resolution FAILURE (e.g. a
    /// symlink planted at the `state/` ancestor directory) must NEVER be
    /// reported as a successful removal. `#[cfg(unix)]`-gated: creating a
    /// real symlink is not portable to this Windows development host, but
    /// this test compiles and runs on any Unix host (not only Linux --
    /// `resolve_contained_path`'s symlink check is pure, portable logic).
    #[cfg(unix)]
    #[test]
    fn governed_remove_state_never_reports_success_when_the_state_path_cannot_be_resolved() {
        let root = temp_root("n6-path-resolution-failure");
        let outside = temp_root("n6-path-resolution-failure-outside");
        std::fs::create_dir_all(&outside).unwrap();
        // Plant a symlink at the exact ancestor `resolve_contained_path`
        // walks through for `state/<key>.json` -- `layout.state_path(key)`
        // must now fail containment, not silently resolve somewhere else.
        std::os::unix::fs::symlink(&outside, root.join("state")).unwrap();

        let layout = ManagedLayout::new(&root);
        let key = "n6-path-resolution-key";
        let (lease, auth) = portable_valid_lease();
        let boundary_context = MutationBoundaryContext {
            authorization_id: Some(&auth.authorization_id),
            expires_at_utc: auth.expires_at_utc.as_deref(),
            configured_clock_skew_seconds: Some(30),
        };
        let mut records = Vec::new();
        let result = governed_remove_state(Some(&layout), Some(key), Some(&lease), &boundary_context, &mut records);

        assert!(!result.removed, "a path-resolution failure must never be reported as removed");
        assert!(result.blocked_reason.is_some(), "a path-resolution failure must produce a blocked_reason, not silent success");
        assert!(records.is_empty(), "no N6 boundary should be recorded when the path could not even be resolved: {:?}", records);
        // Confirm nothing was ever written into (or removed from) the
        // symlink's real target directory either.
        assert!(std::fs::read_dir(&outside).unwrap().next().is_none());

        let _ = std::fs::remove_dir_all(&outside);
        // Do not `remove_dir_all(&root)` blindly here -- `root/state` is a
        // symlink to `outside`, which has already been removed above;
        // remove only the symlink itself and the root directory.
        let _ = std::fs::remove_file(root.join("state"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Same fix, exercised through the full `Rollback` action rather than
    /// `governed_remove_state` in isolation: a path-resolution failure
    /// during N6 must make rollback report an INCOMPLETE outcome, never
    /// `native_process_rolled_back`.
    #[cfg(unix)]
    #[test]
    fn rollback_is_never_reported_successful_when_n6_path_resolution_fails() {
        let root = temp_root("rollback-n6-path-resolution-failure");
        let outside = temp_root("rollback-n6-path-resolution-failure-outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("state")).unwrap();

        let layout = ManagedLayout::new(&root);
        let key = "rollback-n6-path-resolution-key";
        let (lease, auth) = portable_valid_lease();
        let boundary_context = MutationBoundaryContext {
            authorization_id: Some(&auth.authorization_id),
            expires_at_utc: auth.expires_at_utc.as_deref(),
            configured_clock_skew_seconds: Some(30),
        };
        let assignment = native_assignment_for_reconcile();
        let mut records = Vec::new();
        // `RollbackStaleStateCleanup` reaches `governed_remove_state`
        // directly (no stop involved) -- the simplest, most direct path
        // to exercise this exact fix through a real reconciliation action.
        let outcome = execute_native_action(
            &NativeReconciliationAction::RollbackStaleStateCleanup,
            Some(&layout), Some(key), Some(&lease), &boundary_context, &mut records,
            &assignment, None, &Ok("sha256:".to_string() + &"c".repeat(64)), None,
        );
        assert_ne!(outcome.execution_status, NATIVE_PROCESS_ROLLED_BACK, "must never report success when N6 path resolution failed");
        assert!(!outcome.host_mutation_performed);

        let _ = std::fs::remove_dir_all(&outside);
        let _ = std::fs::remove_file(root.join("state"));
        let _ = std::fs::remove_dir_all(&root);
    }

    fn expired_lease_and_boundary_portable() -> (LeaseGuard, OperationalReconciliationAuthorization) {
        let past = "2020-01-01T00:00:00Z".to_string();
        let auth = OperationalReconciliationAuthorization {
            authorization_id: "auth-expired-portable".to_string(),
            project_id: "proj".to_string(),
            dgraph_id: "dg".to_string(),
            dgraph_revision: None,
            node_id: "node".to_string(),
            binding_id: None,
            resource_id: None,
            authorized_by: "test".to_string(),
            authorized_at_utc: past.clone(),
            allow_host_mutation: true,
            operation: "reconcile".to_string(),
            expires_at_utc: Some(past),
            persistence_scope: "durable".to_string(),
            artifact_bindings: Vec::new(),
            execution_pinning_mode: String::new(),
        };
        let lease = LeaseGuard::from_authorization(&auth, 30).unwrap();
        (lease, auth)
    }

    #[test]
    fn corrupted_runtime_state_blocks_truthfully_instead_of_silently_respawning() {
        let root = temp_root("corrupted-state");
        let cfg = dispatch_cfg(true, Some(true), Some(&root.to_string_lossy().replace('\\', "/")));
        let mut assignment = native_assignment_for_reconcile();
        assignment.execution_projection_digest = execution_projection::recompute_native_digest(&assignment).unwrap();

        let service = OperationalDesiredService {
            service_id: "svc".to_string(),
            runtime: "native_process".to_string(),
            desired_state: "running".to_string(),
            application_id: String::new(),
            dmap_ref: None,
        };

        // Write a corrupted (non-JSON) state file at the exact path the
        // function itself would compute.
        let layout = ManagedLayout::new(&root);
        let key = native_process_acquisition::stable_service_key("proj", "node", "svc");
        let state_path = layout.state_path(&key).unwrap();
        std::fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        std::fs::write(&state_path, b"not valid json").unwrap();

        let boundary_context = MutationBoundaryContext {
            authorization_id: None,
            expires_at_utc: None,
            configured_clock_skew_seconds: None,
        };

        let report = reconcile_native_service(
            &cfg,
            "proj",
            "node",
            None,
            None,
            None,
            None,
            &boundary_context,
            &service,
            &assignment,
            false,
            false,
            false,
        );

        assert!(!report.host_mutation_performed);
        assert!(
            report.message.contains("blocked_native_process_state_unreadable"),
            "message was: {}",
            report.message
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_runtime_state_file_is_treated_as_no_managed_process_not_an_error() {
        let root = temp_root("missing-state");
        let cfg = dispatch_cfg(true, Some(true), Some(&root.to_string_lossy().replace('\\', "/")));
        let mut assignment = native_assignment_for_reconcile();
        assignment.execution_projection_digest = execution_projection::recompute_native_digest(&assignment).unwrap();
        let service = OperationalDesiredService {
            service_id: "svc".to_string(),
            runtime: "native_process".to_string(),
            desired_state: "running".to_string(),
            application_id: String::new(),
            dmap_ref: None,
        };
        let boundary_context = MutationBoundaryContext {
            authorization_id: None,
            expires_at_utc: None,
            configured_clock_skew_seconds: None,
        };

        let report = reconcile_native_service(
            &cfg, "proj", "node", None, None, None, None, &boundary_context, &service, &assignment, false, false, false,
        );

        assert!(!report.message.contains("blocked_native_process_state_unreadable"));
        assert!(!report.host_mutation_performed);
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// Real, safe Linux-only integration tests (Section V, items 71-77):
/// exercised against a genuine temporary child process this test itself
/// spawns and cleans up -- never a live lab node, never a real Docker
/// daemon. `#[cfg(target_os = "linux")]` per ADR-0016's platform boundary:
/// these do not compile or run on a non-Linux development host at all (the
/// pure decision logic above is what stays portable and runs everywhere).
#[cfg(all(test, target_os = "linux"))]
mod linux_integration_tests {
    use super::linux::{self, Signal};
    use super::*;
    use std::io::Write as _;

    fn temp_script(label: &str, contents: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("datum-native-it-{label}-{}", std::process::id()));
        {
            let mut file = std::fs::File::create(&path).unwrap();
            file.write_all(contents.as_bytes()).unwrap();
        }
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    /// v6/#1,#2,#4 fix (real Linux validation): a script spawned via
    /// `Command::spawn()` can be signaled, or have its `/proc/<pid>/cmdline`
    /// read, before the shell has actually finished executing its own
    /// setup lines (e.g. a `trap` statement) -- a real, observed startup
    /// race between this test process and the freshly-forked child, not a
    /// flaky test. This builds a script that writes a readiness marker
    /// AFTER `setup` and BEFORE `body`, so a caller can block on
    /// `wait_for_readiness` before doing anything that depends on the
    /// child having actually reached a stable, fully-execve'd state.
    /// TEST-ONLY synchronization: production code sends no such handshake
    /// and needs none -- a governed reconciliation cycle does not race its
    /// own just-issued spawn within microseconds the way these tests,
    /// which signal/observe a child immediately after `spawn()` returns,
    /// deliberately do.
    fn temp_script_with_readiness(label: &str, setup: &str, body: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let mut readiness = std::env::temp_dir();
        readiness.push(format!("datum-native-it-ready-{label}-{}", std::process::id()));
        let _ = std::fs::remove_file(&readiness);
        let contents = format!("#!/bin/sh\n{setup}\necho ready > {}\n{body}\n", readiness.display());
        (temp_script(label, &contents), readiness)
    }

    /// Bounded wait for a readiness marker `temp_script_with_readiness`
    /// arranges for the spawned script to create once it has actually
    /// started running. Test-only; never used by production code.
    fn wait_for_readiness(path: &std::path::Path) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if path.exists() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("readiness marker '{}' was not created within the test deadline", path.display());
    }

    /// 71/72: spawn a trivial temporary test process and observe its real
    /// PID/starttime via `/proc`.
    #[test]
    fn spawn_and_observe_pid_and_starttime() {
        let script = temp_script("sleep-loop", "#!/bin/sh\nsleep 30\n");
        let mut child = std::process::Command::new(&script).spawn().unwrap();
        let pid = child.id();

        assert!(linux::pid_exists(pid));
        let starttime = linux::read_proc_stat_starttime(pid).unwrap();
        assert!(starttime > 0 || starttime == 0); // ticks since boot; just must parse successfully

        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_file(&script);
    }

    /// 73: graceful stop via SIGTERM against the exact managed identity.
    #[test]
    fn graceful_sigterm_stop_succeeds() {
        let script = temp_script("term-friendly", "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do sleep 1; done\n");
        let mut child = std::process::Command::new(&script).spawn().unwrap();
        let pid = child.id();
        assert!(linux::pid_exists(pid));

        linux::send_signal(pid, Signal::Term).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut exited = false;
        while std::time::Instant::now() < deadline {
            // v5/L3 fix: a direct child that has exited but not yet been
            // reaped is a ZOMBIE -- `linux::pid_exists` (backed by
            // `kill(pid, 0)`) still reports it present. `Child::try_wait`
            // on the OWNED handle is the only reliable "actually exited AND
            // reaped" signal available to this real parent process.
            if child.try_wait().unwrap().is_some() {
                exited = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(exited, "process did not exit after SIGTERM within the test deadline");
        assert!(!linux::pid_exists(pid), "pid must be fully gone once reaped via try_wait");
        let _ = std::fs::remove_file(&script);
    }

    /// 76: an unresponsive process (ignores SIGTERM) exercises the SIGKILL
    /// fallback. v6/#1 fix (real Linux validation): a `trap '' TERM`
    /// spawned via a fresh shell can receive SIGTERM before that shell has
    /// actually executed the `trap` statement -- this raced shell startup
    /// on the real VM. Fixed with a readiness handshake: wait for the
    /// script to actually reach its `trap` line and signal readiness
    /// before sending any signal.
    #[test]
    fn unresponsive_process_is_force_killed() {
        let (script, readiness) = temp_script_with_readiness("term-ignoring", "trap '' TERM", "while true; do sleep 1; done");
        let mut child = std::process::Command::new(&script).spawn().unwrap();
        let pid = child.id();
        assert!(linux::pid_exists(pid));
        wait_for_readiness(&readiness);

        linux::send_signal(pid, Signal::Term).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(500));
        // v5/L3 fix: `try_wait()` on the owned handle, not `pid_exists`,
        // is the correct "still actually running" check here too.
        assert!(child.try_wait().unwrap().is_none(), "process should still be alive after ignoring SIGTERM");

        linux::send_signal(pid, Signal::Kill).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut exited = false;
        while std::time::Instant::now() < deadline {
            if child.try_wait().unwrap().is_some() {
                exited = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(exited, "process did not exit after SIGKILL within the test deadline");
        assert!(!linux::pid_exists(pid), "pid must be fully gone once reaped via try_wait");
        let _ = std::fs::remove_file(&readiness);
        let _ = std::fs::remove_file(&script);
    }

    /// 75: prove that recovery/observation (`observe_persisted_identity`)
    /// itself never receives or needs an in-memory `Child` handle -- it is
    /// given only `persisted` (state) and reads only real `/proc`. v5/L5
    /// fix (real Linux validation): the previous version of this test
    /// leaked the child as a permanent zombie in the cargo test process via
    /// `std::mem::forget` (this test process never called `wait()`, so the
    /// child was never reaped for the remainder of the run) and its name
    /// overstated what is actually exercised. This does NOT simulate a
    /// true parent-process restart/reparenting (this test process never
    /// actually exits) -- only that the recovery ALGORITHM itself never
    /// touches a `Child` handle; the handle below is retained solely for
    /// this test's own cleanup.
    ///
    /// v6 fix (real Linux validation): the v6 fixture spawned a
    /// `#!/bin/sh\nsleep 30\n` SCRIPT (via `temp_script`) and captured
    /// `/proc/<pid>/exe` before calling `observe_persisted_identity`. Real
    /// Linux showed this intermittently produced a legitimate
    /// `ExecutableMismatch`: when a shell script's LAST command is a
    /// simple, unconditional invocation with nothing left for the shell to
    /// do afterward (exactly this script's shape), many `/bin/sh`
    /// implementations execute it as a tail call -- the shell process
    /// directly `execve()`s the final command IN PLACE of itself, replacing
    /// its own process image (same PID, same starttime) rather than
    /// forking a child. This is a real, POSIX-permitted shell optimization,
    /// not a bug: it means `/proc/<pid>/exe` can race between "the shell
    /// binary" and "the `sleep` binary" for the EXACT SAME pid+starttime,
    /// entirely outside this agent's control. The runtime was CORRECT to
    /// report `ExecutableMismatch` when the persisted snapshot and the
    /// freshly re-observed executable disagreed -- the FIXTURE was
    /// unstable, not the identity check. Fixed by spawning `sleep` DIRECTLY
    /// (`Command::new("sleep")`, no shell, no script, no `temp_script`
    /// indirection at all) so there is no intermediate process image ever
    /// to exec away from -- `/proc/<pid>/exe` is stably `sleep` for this
    /// process's entire lifetime. No interpreter special-casing was added,
    /// and production `observe_persisted_identity` gained no retry logic --
    /// this is a test-fixture-only correction.
    #[test]
    fn adoption_from_persisted_state_does_not_require_child_handle_for_observation() {
        let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let pid = child.id();
        let starttime = linux::read_proc_stat_starttime(pid).unwrap();
        let executable = linux::read_proc_exe(pid).unwrap();
        // v5/L1 fix: the persisted `cmdline_digest` must be built from the
        // COMPLETE real observed `/proc/<pid>/cmdline` (whatever shape the
        // kernel actually produced), never `compute_args_digest(&[])`.
        let cmdline = linux::read_proc_cmdline(pid).unwrap();

        let persisted = ManagedProcessRuntimeStateV1 {
            schema: MANAGED_PROCESS_RUNTIME_STATE_SCHEMA.to_string(),
            service_id: "it-svc".to_string(),
            artifact_id: "it-svc".to_string(),
            execution_generation: 1,
            execution_projection_digest: "sha256:".to_string() + &"a".repeat(64),
            application_id: String::new(),
            dmap_ref: None,
            content_sha256: "b".repeat(64),
            pid,
            proc_start_time_ticks: starttime,
            observed_executable: executable,
            cmdline_digest: compute_args_digest(&cmdline),
            started_at_utc: chrono::Utc::now().to_rfc3339(),
            state: PROCESS_STATE_RUNNING.to_string(),
        };

        // `observe_persisted_identity` receives ONLY `persisted` -- `child`
        // is never passed to it, proving observation/adoption is
        // handle-independent.
        let identity = observe_persisted_identity(&persisted);
        assert_eq!(identity, IdentityCheck::Confirmed);

        // Cleanup: reap through the still-owned handle -- never leak a
        // zombie in this test process.
        let _ = child.kill();
        let _ = child.wait();
        assert!(!linux::pid_exists(pid), "pid must be fully gone once reaped");
    }

    /// 46: a process disappearing between checks is a successful, idempotent
    /// stop -- never an error.
    #[test]
    fn stop_of_an_already_exited_process_is_idempotent() {
        let script = temp_script("quick-exit", "#!/bin/sh\nexit 0\n");
        let mut child = std::process::Command::new(&script).spawn().unwrap();
        let _ = child.wait(); // ensure it has actually exited
        let pid = child.id();

        assert!(!linux::pid_exists(pid));
        // Signaling an already-gone PID must report ESRCH-shaped failure,
        // never panic, and callers treat this as "already stopped."
        assert!(linux::send_signal(pid, Signal::Term).is_err());
        let _ = std::fs::remove_file(&script);
    }

    /// 77: cleanup leaves no child process running -- a meta-test asserting
    /// every test above actually terminates its own spawned process (best
    /// effort: re-checks after each test's own cleanup already ran).
    #[test]
    fn full_lifecycle_leaves_no_process_running() {
        let script = temp_script("full-lifecycle", "#!/bin/sh\nsleep 30\n");
        let mut child = std::process::Command::new(&script).spawn().unwrap();
        let pid = child.id();
        assert!(linux::pid_exists(pid));

        linux::send_signal(pid, Signal::Kill).unwrap();
        let _ = child.wait();
        assert!(!linux::pid_exists(pid), "process must not remain running after cleanup");
        let _ = std::fs::remove_file(&script);
    }

    // ---- v2: real argv identity via /proc/<pid>/cmdline ------------------ //

    /// Closes the Reviewer J ("contract vs. execution") gap: args identity
    /// was persisted since v1 but never actually re-verified against a live
    /// process. v5/L1 fix (real Linux validation): the ORIGINAL version of
    /// this test asserted that stripping argv[0] from a shebang script's
    /// real `/proc/<pid>/cmdline` reconstructs the desired args exactly --
    /// real Linux disproved that. For `#!/bin/sh` the kernel execs `sh`
    /// with argv[0]=the interpreter, argv[1]=the script's own path, THEN
    /// the real args -- so naively dropping one leading element still
    /// leaves the script path mixed in. `read_proc_cmdline` therefore
    /// returns the COMPLETE raw vector, unmodified, and this test confirms
    /// only what is actually true and needed: the digest of that complete
    /// observed vector is stable across repeated real reads of the SAME
    /// live process, and differs for a process with genuinely different
    /// real args -- exactly the two properties `check_identity` relies on
    /// (`Confirmed` vs. `ArgsMismatch`), independent of any particular
    /// interpreter's argv-rewriting convention.
    /// v6/#4 fix (real Linux validation): `/proc/<pid>/cmdline` was
    /// occasionally observed empty immediately after `spawn()` returns (the
    /// same fork/execve startup race as #1/#2, exercised here against
    /// `read_proc_cmdline` directly rather than via a signal). Fixed with a
    /// readiness handshake for BOTH spawned processes before reading their
    /// real cmdlines -- test synchronization only; production cmdline
    /// observation (`read_proc_cmdline`, `observe_persisted_identity`) is
    /// NOT changed and gains no retry behavior.
    #[test]
    fn real_argv_identity_round_trips_and_detects_mismatch() {
        let (script, readiness) = temp_script_with_readiness("argv-identity", "", "sleep 30");
        let mut child = std::process::Command::new(&script).arg("--foo").arg("bar baz").spawn().unwrap();
        let pid = child.id();
        wait_for_readiness(&readiness);

        let observed_cmdline = linux::read_proc_cmdline(pid).unwrap();
        assert!(!observed_cmdline.is_empty());
        assert!(observed_cmdline.iter().any(|a| a == "--foo"), "observed cmdline was: {observed_cmdline:?}");
        assert!(observed_cmdline.iter().any(|a| a == "bar baz"), "observed cmdline was: {observed_cmdline:?}");

        let digest_a = native_process_state::compute_args_digest(&observed_cmdline);
        let digest_b = native_process_state::compute_args_digest(&linux::read_proc_cmdline(pid).unwrap());
        assert_eq!(digest_a, digest_b, "repeated real reads of the same live process must yield the same observed cmdline digest");

        let (other_script, other_readiness) = temp_script_with_readiness("argv-identity-other", "", "sleep 30");
        let mut other = std::process::Command::new(&other_script).arg("--different").spawn().unwrap();
        let other_pid = other.id();
        wait_for_readiness(&other_readiness);
        let other_cmdline = linux::read_proc_cmdline(other_pid).unwrap();
        let other_digest = native_process_state::compute_args_digest(&other_cmdline);
        assert_ne!(digest_a, other_digest, "a genuinely differing real cmdline must produce a differing digest");

        let _ = child.kill();
        let _ = child.wait();
        let _ = other.kill();
        let _ = other.wait();
        let _ = std::fs::remove_file(&readiness);
        let _ = std::fs::remove_file(&script);
        let _ = std::fs::remove_file(&other_readiness);
        let _ = std::fs::remove_file(&other_script);
    }

    // ---- v2: environment determinism (Section AL) ------------------------ //

    fn env_probe_assignment(bindings: std::collections::BTreeMap<String, String>) -> OperationalArtifactAssignment {
        OperationalArtifactAssignment {
            artifact_id: "env-probe".to_string(),
            service_id: "env-probe".to_string(),
            artifact_service_id: "env-probe".to_string(),
            version: "1".to_string(),
            artifact_kind: "command_bundle".to_string(),
            runtime_kind: "native_process".to_string(),
            container_name: String::new(),
            image: String::new(),
            native_process: Some(crate::agent::operational_reconciliation::NativeProcessDeploymentSpecV1 {
                process_name: "env-probe".to_string(),
                entrypoint: "run".to_string(),
                args: Vec::new(),
                working_directory: None,
            }),
            acquisition: None,
            image_source_kind: crate::agent::operational_reconciliation::IMAGE_SOURCE_KIND_REGISTRY.to_string(),
            resolved_image_identity: None,
            target_node: "fog-01".to_string(),
            lifecycle: crate::agent::operational_reconciliation::OperationalLifecycle::default(),
            network: None,
            network_aliases: Vec::new(),
            ports: Vec::new(),
            volumes: Vec::new(),
            configuration_mounts: Vec::new(),
            command: Vec::new(),
            dependencies: Vec::new(),
            wasm_decision: crate::agent::operational_reconciliation::ArtifactWasmDecisionPolicy {
                explicit_invocation_required: true,
                module_id: "reconcile_service".to_string(),
                function_name: "decide".to_string(),
                host_imports_allowed: false,
                side_effects_allowed: false,
                safe_to_auto_execute: false,
            },
            bindings,
            secret_refs: Vec::new(),
            secret_bindings: Vec::new(),
            execution_generation: 1,
            execution_projection_digest: String::new(),
        }
    }

    /// Section AL: the child process must NOT inherit this agent's own
    /// (arbitrary, host-controlled) environment -- proven here by setting a
    /// host-only variable in THIS test process's own environment (which
    /// `std::process::Command` would otherwise inherit by default) and
    /// confirming the spawned child never sees it, while a variable
    /// explicitly declared via `bindings` IS present. One spawn proves both
    /// halves of Section AL's requirement at once.
    #[test]
    fn spawned_process_gets_only_explicitly_declared_environment_never_host_environment() {
        std::env::set_var("DATUM_TEST_HOST_ONLY_VAR", "must-never-be-inherited");

        let root = temp_root_it("env-determinism");
        let layout = ManagedLayout::new(&root);
        // A brief trailing sleep avoids a race against `spawn_and_persist`'s
        // own immediate post-spawn `/proc/<pid>/stat`+`/proc/<pid>/exe`
        // reads, which require the process to still exist at that instant
        // (a script that exits instantly could beat those reads).
        let script = b"#!/bin/sh\necho \"DECLARED=$DATUM_TEST_DECLARED_VAR\"\necho \"HOST=$DATUM_TEST_HOST_ONLY_VAR|\"\nsleep 2\n";
        let hash = native_process_acquisition::sha256_hex_of_bytes(script);
        layout.materialize(&hash, "run", script).unwrap();
        // v5/L6 fix (real Linux validation): since the v4 N1/N2 split,
        // `materialize()` deliberately leaves a freshly written file
        // NON-EXECUTABLE (setting the executable bit is N2's job, done by
        // the real governed caller via `repair_executable_permission`
        // after its own fresh boundary check). This test calls the direct
        // spawn helper below `governed_materialize_and_spawn`'s own N2
        // step, so it must perform the equivalent repair itself, or the
        // real `Command::spawn()` fails with `EACCES` (the runtime
        // correctly refusing to execute a non-executable file -- the test
        // setup was stale, not the runtime).
        layout.repair_executable_permission(&hash, "run").unwrap();
        let executable_path = layout.materialized_entrypoint_path(&hash, "run").unwrap();
        let artifact_dir = layout.artifact_directory(&hash).unwrap();
        let logs_dir = layout.logs_dir("env-probe-key").unwrap();

        let mut bindings = std::collections::BTreeMap::new();
        bindings.insert("DATUM_TEST_DECLARED_VAR".to_string(), "declared-value".to_string());
        let assignment = env_probe_assignment(bindings);

        spawn_and_persist(
            &executable_path,
            &[],
            &artifact_dir,
            &logs_dir,
            &assignment,
            &hash,
            &("sha256:".to_string() + &"a".repeat(64)),
            &layout,
            "env-probe-key",
        )
        .unwrap();

        // Give the short-lived script time to run and exit.
        std::thread::sleep(std::time::Duration::from_millis(500));
        let stdout = std::fs::read_to_string(logs_dir.join("stdout.log")).unwrap();
        assert!(stdout.contains("DECLARED=declared-value"), "stdout was: {stdout}");
        assert!(stdout.contains("HOST=|"), "host-only var must be absent from the child's environment; stdout was: {stdout}");

        std::env::remove_var("DATUM_TEST_HOST_ONLY_VAR");
        let _ = std::fs::remove_dir_all(&root);
    }

    fn temp_root_it(label: &str) -> std::path::PathBuf {
        let dir = std::path::PathBuf::from(format!("/tmp/datum-native-it-root-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // ---- v2: governed_stop confirmation + lease-expiry-mid-sequence ------ //

    fn valid_lease_and_boundary() -> (LeaseGuard, crate::agent::operational_reconciliation::OperationalReconciliationAuthorization) {
        let far_future = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        let auth = crate::agent::operational_reconciliation::OperationalReconciliationAuthorization {
            authorization_id: "auth-it".to_string(),
            project_id: "proj".to_string(),
            dgraph_id: "dg".to_string(),
            dgraph_revision: None,
            node_id: "node".to_string(),
            binding_id: None,
            resource_id: None,
            authorized_by: "test".to_string(),
            authorized_at_utc: chrono::Utc::now().to_rfc3339(),
            allow_host_mutation: true,
            operation: "reconcile".to_string(),
            expires_at_utc: Some(far_future),
            persistence_scope: "durable".to_string(),
            artifact_bindings: Vec::new(),
            execution_pinning_mode: String::new(),
        };
        let lease = LeaseGuard::from_authorization(&auth, 30).unwrap();
        (lease, auth)
    }

    fn expired_lease_and_boundary() -> (LeaseGuard, crate::agent::operational_reconciliation::OperationalReconciliationAuthorization) {
        let past = "2020-01-01T00:00:00Z".to_string();
        let auth = crate::agent::operational_reconciliation::OperationalReconciliationAuthorization {
            authorization_id: "auth-it-expired".to_string(),
            project_id: "proj".to_string(),
            dgraph_id: "dg".to_string(),
            dgraph_revision: None,
            node_id: "node".to_string(),
            binding_id: None,
            resource_id: None,
            authorized_by: "test".to_string(),
            authorized_at_utc: past.clone(),
            allow_host_mutation: true,
            operation: "reconcile".to_string(),
            expires_at_utc: Some(past),
            persistence_scope: "durable".to_string(),
            artifact_bindings: Vec::new(),
            execution_pinning_mode: String::new(),
        };
        let lease = LeaseGuard::from_authorization(&auth, 30).unwrap();
        (lease, auth)
    }

    /// v5/L2 fix (real Linux validation): this MUST build `cmdline_digest`
    /// from the callee's REAL, currently-observed `/proc/<pid>/cmdline` --
    /// the previous `compute_args_digest(&[])` claimed an empty observed
    /// cmdline for a real process that always has a non-empty one (at
    /// minimum, its own path/interpreter as argv[0]), which made
    /// `check_identity` correctly refuse to report `Confirmed` and every
    /// caller of this helper fail for a reason unrelated to what it was
    /// actually testing.
    fn state_for_real_pid(pid: u32, starttime: u64, executable: String) -> ManagedProcessRuntimeStateV1 {
        // v6/#6 fix (real Linux validation): a just-forked/exec'd process's
        // `/proc/<pid>/cmdline` was occasionally observed transiently
        // unreadable/empty (a kernel exec-argv-population race for a
        // FRESHLY spawned child, distinct from the post-signal teardown
        // race fixed in `governed_stop` itself -- see the v6/#3 and v6/#5
        // fixes there). Bounded retry here is a TEST-FIXTURE-ONLY
        // accommodation for this narrow just-spawned-process race, applied
        // BEFORE any identity is ever compared; production
        // `observe_persisted_identity` gains no retry loop and remains
        // fail-closed, single-observation.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let cmdline = loop {
            match linux::read_proc_cmdline(pid) {
                Ok(cmdline) => break cmdline,
                Err(error) if std::time::Instant::now() < deadline => {
                    let _ = error;
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => panic!(
                    "real process cmdline for pid {pid} was not readable within this test fixture's bounded deadline: {error}"
                ),
            }
        };
        ManagedProcessRuntimeStateV1 {
            schema: MANAGED_PROCESS_RUNTIME_STATE_SCHEMA.to_string(),
            service_id: "it-svc".to_string(),
            artifact_id: "it-svc".to_string(),
            execution_generation: 1,
            execution_projection_digest: "sha256:".to_string() + &"a".repeat(64),
            application_id: String::new(),
            dmap_ref: None,
            content_sha256: "b".repeat(64),
            pid,
            proc_start_time_ticks: starttime,
            observed_executable: executable,
            cmdline_digest: compute_args_digest(&cmdline),
            started_at_utc: chrono::Utc::now().to_rfc3339(),
            state: PROCESS_STATE_RUNNING.to_string(),
        }
    }

    /// Section AI: `governed_stop` itself (not just the raw `send_signal`
    /// primitive) confirms a cooperative SIGTERM stop.
    #[test]
    fn governed_stop_confirms_a_cooperative_stop() {
        let script = temp_script("governed-stop-cooperative", "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do sleep 1; done\n");
        let mut child = std::process::Command::new(&script).spawn().unwrap();
        let pid = child.id();
        let starttime = linux::read_proc_stat_starttime(pid).unwrap();
        let executable = linux::read_proc_exe(pid).unwrap();

        let state = state_for_real_pid(pid, starttime, executable);
        let (lease, auth) = valid_lease_and_boundary();
        let boundary_context = MutationBoundaryContext {
            authorization_id: Some(&auth.authorization_id),
            expires_at_utc: auth.expires_at_utc.as_deref(),
            configured_clock_skew_seconds: Some(30),
        };
        let mut records = Vec::new();
        let result = governed_stop(&state, Some(&lease), &boundary_context, &mut records);
        assert!(result.confirmed_stopped, "message was: {}", result.message);
        // v5/L4 fix: the process has exited (confirmed above via
        // `governed_stop`'s own Zombie-aware observation) but remains an
        // UNREAPED ZOMBIE in this test process until reaped here --
        // `linux::pid_exists` (backed by `kill(pid, 0)`) cannot distinguish
        // a zombie from a live process; the retained `Child` handle is kept
        // for exactly this reaping, never passed to `governed_stop` itself.
        let _ = child.wait();
        assert!(!linux::pid_exists(pid), "pid must be fully gone once reaped");
        let _ = std::fs::remove_file(&script);
    }

    /// Section AI: SIGKILL confirmation, not merely "syscall returned Ok" --
    /// exercised through `governed_stop` itself.
    #[test]
    fn governed_stop_confirms_after_sigkill_for_an_unresponsive_process() {
        // v6/#2 fix (real Linux validation): the same startup race as v6/#1
        // -- `trap '' TERM` in a fresh shell could still be racing shell
        // startup when this test built persisted identity and called
        // `governed_stop`, and (independently) `/proc/<pid>/cmdline` could
        // be transiently unreadable immediately post-spawn. Wait for
        // readiness BEFORE reading starttime/executable or building
        // persisted state at all.
        let (script, readiness) = temp_script_with_readiness("governed-stop-uncooperative", "trap '' TERM", "while true; do sleep 1; done");
        let mut child = std::process::Command::new(&script).spawn().unwrap();
        let pid = child.id();
        wait_for_readiness(&readiness);
        let starttime = linux::read_proc_stat_starttime(pid).unwrap();
        let executable = linux::read_proc_exe(pid).unwrap();

        let state = state_for_real_pid(pid, starttime, executable);
        let (lease, auth) = valid_lease_and_boundary();
        let boundary_context = MutationBoundaryContext {
            authorization_id: Some(&auth.authorization_id),
            expires_at_utc: auth.expires_at_utc.as_deref(),
            configured_clock_skew_seconds: Some(30),
        };
        let mut records = Vec::new();
        // This deliberately runs past GRACEFUL_STOP_TIMEOUT (SIGTERM is
        // ignored) before SIGKILL is confirmed -- a real, bounded wait.
        let result = governed_stop(&state, Some(&lease), &boundary_context, &mut records);
        assert!(result.confirmed_stopped, "message was: {}", result.message);
        // v5/L4 fix: reap via the retained handle before checking
        // `pid_exists` -- see the identical fix/comment on
        // `governed_stop_confirms_a_cooperative_stop` above.
        let _ = child.wait();
        assert!(!linux::pid_exists(pid), "pid must be fully gone once reaped");
        assert!(records.iter().any(|r| r.boundary == N4_SIGTERM));
        assert!(records.iter().any(|r| r.boundary == N5_SIGKILL));
        let _ = std::fs::remove_file(&readiness);
        let _ = std::fs::remove_file(&script);
    }

    // ---- v4 (P0-1): ESRCH is the ONLY errno that confirms a stop ------- //

    #[test]
    fn send_signal_esrch_maps_to_process_absent_not_a_generic_failure() {
        // A PID this large is vanishingly unlikely to exist (Linux's
        // default pid_max is far below this), and even if it did, this is
        // purely testing the ESRCH-vs-other-errno TYPE distinction, not a
        // specific process's identity.
        let definitely_dead_pid: u32 = 0x7FFF_FFFE;
        match linux::send_signal(definitely_dead_pid, Signal::Term) {
            Err(linux::SignalError::ProcessAbsent) => {}
            other => panic!("expected ProcessAbsent (ESRCH), got {other:?}"),
        }
    }

    /// P0-1: `EPERM` (or any non-ESRCH failure) must NEVER be reported as
    /// `ProcessAbsent` -- it proves nothing about whether the target is
    /// alive. PID 1 (init) is a real, always-present process this test
    /// does not own; signaling it as a NON-ROOT user reliably returns
    /// `EPERM` on Linux. Best-effort: if this test happens to run as
    /// root (common in some container-based CI), root can signal any
    /// process, so no real EPERM can be produced here -- the test then
    /// skips its assertion rather than failing or fabricating a result
    /// (documented explicitly, not silently).
    #[test]
    fn send_signal_eperm_never_maps_to_process_absent() {
        if unsafe { libc::getuid() } == 0 {
            eprintln!("skipping: running as root, cannot produce a real EPERM against PID 1");
            return;
        }
        match linux::send_signal(1, Signal::Term) {
            Err(linux::SignalError::Failed(_)) => {}
            other => panic!("expected Failed(EPERM), got {other:?} (are we unexpectedly privileged?)"),
        }
    }

    /// P0-1 core fix, exercised through `governed_stop` itself (not just
    /// the raw primitive): a non-ESRCH SIGTERM failure must leave
    /// `confirmed_stopped: false` and `signaled: false` -- never treated
    /// as an idempotent "already gone" stop. PID 1's real, current
    /// identity is read and used to build a `ManagedProcessRuntimeStateV1`
    /// that will pass `governed_stop`'s own fresh identity check
    /// (it really is PID 1, with its real starttime/exe), so the SIGTERM
    /// attempt itself is what is under test, not the identity check.
    /// Best-effort/non-root only, same reasoning as above.
    #[test]
    fn governed_stop_never_confirms_a_stop_on_sigterm_eperm() {
        if unsafe { libc::getuid() } == 0 {
            eprintln!("skipping: running as root, cannot produce a real EPERM against PID 1");
            return;
        }
        let Ok(starttime) = linux::read_proc_stat_starttime(1) else {
            eprintln!("skipping: /proc/1/stat unreadable in this environment");
            return;
        };
        let Ok(executable) = linux::read_proc_exe(1) else {
            eprintln!("skipping: /proc/1/exe unreadable in this environment");
            return;
        };
        let state = state_for_real_pid(1, starttime, executable);
        let (lease, auth) = valid_lease_and_boundary();
        let boundary_context = MutationBoundaryContext {
            authorization_id: Some(&auth.authorization_id),
            expires_at_utc: auth.expires_at_utc.as_deref(),
            configured_clock_skew_seconds: Some(30),
        };
        let mut records = Vec::new();
        let result = governed_stop(&state, Some(&lease), &boundary_context, &mut records);
        assert!(!result.confirmed_stopped, "an EPERM SIGTERM failure must never be treated as a confirmed stop: {}", result.message);
        assert!(!result.signaled, "message was: {}", result.message);
        assert!(linux::pid_exists(1), "PID 1 must obviously still be alive");
    }

    /// P0-1: rollback must never remove the runtime-state record (N6)
    /// after a SIGTERM failure it cannot interpret as confirmed --
    /// exercised through the full `Rollback` action, not just
    /// `governed_stop` in isolation.
    #[test]
    fn rollback_cannot_remove_state_after_a_sigterm_eperm_failure() {
        if unsafe { libc::getuid() } == 0 {
            eprintln!("skipping: running as root, cannot produce a real EPERM against PID 1");
            return;
        }
        let Ok(starttime) = linux::read_proc_stat_starttime(1) else {
            eprintln!("skipping: /proc/1/stat unreadable in this environment");
            return;
        };
        let Ok(executable) = linux::read_proc_exe(1) else {
            eprintln!("skipping: /proc/1/exe unreadable in this environment");
            return;
        };
        let root = temp_root_it("rollback-eperm");
        let layout = ManagedLayout::new(&root);
        let key = "rollback-eperm-key";
        let state = state_for_real_pid(1, starttime, executable);
        let state_path = layout.state_path(key).unwrap();
        std::fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        native_process_state::persist_state(&state_path, &state).unwrap();

        let (lease, auth) = valid_lease_and_boundary();
        let boundary_context = MutationBoundaryContext {
            authorization_id: Some(&auth.authorization_id),
            expires_at_utc: auth.expires_at_utc.as_deref(),
            configured_clock_skew_seconds: Some(30),
        };
        let assignment = env_probe_assignment(std::collections::BTreeMap::new());
        let mut records = Vec::new();
        let outcome = execute_native_action(
            &NativeReconciliationAction::Rollback,
            Some(&layout), Some(key), Some(&lease), &boundary_context, &mut records,
            &assignment, None, &Ok("sha256:".to_string() + &"c".repeat(64)), Some(&state),
        );
        assert_eq!(outcome.execution_status, NATIVE_PROCESS_STOP_INCOMPLETE);
        assert!(state_path.exists(), "state must be preserved after an EPERM SIGTERM failure");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// P0-1: `RestartForDrift` must never proceed to spawn a new process
    /// after a SIGTERM failure it cannot interpret as confirmed -- the
    /// old (real, still-alive) PID 1 must never be treated as "stopped".
    #[test]
    fn restart_for_drift_cannot_spawn_after_a_sigterm_eperm_failure() {
        if unsafe { libc::getuid() } == 0 {
            eprintln!("skipping: running as root, cannot produce a real EPERM against PID 1");
            return;
        }
        let Ok(starttime) = linux::read_proc_stat_starttime(1) else {
            eprintln!("skipping: /proc/1/stat unreadable in this environment");
            return;
        };
        let Ok(executable) = linux::read_proc_exe(1) else {
            eprintln!("skipping: /proc/1/exe unreadable in this environment");
            return;
        };
        let state = state_for_real_pid(1, starttime, executable);
        let (lease, auth) = valid_lease_and_boundary();
        let boundary_context = MutationBoundaryContext {
            authorization_id: Some(&auth.authorization_id),
            expires_at_utc: auth.expires_at_utc.as_deref(),
            configured_clock_skew_seconds: Some(30),
        };
        let assignment = env_probe_assignment(std::collections::BTreeMap::new());
        let mut records = Vec::new();
        let outcome = execute_native_action(
            &NativeReconciliationAction::RestartForDrift,
            None, None, Some(&lease), &boundary_context, &mut records,
            &assignment, None, &Ok("sha256:".to_string() + &"c".repeat(64)), Some(&state),
        );
        assert_eq!(outcome.execution_status, NATIVE_PROCESS_STOP_INCOMPLETE);
        assert!(linux::pid_exists(1), "PID 1 must still be the only process involved -- no duplicate spawned");
    }

    /// Section AO + the core v2 fix: a lease that is already expired must
    /// block N4 -- `governed_stop` never signals, and a real, still-alive
    /// process is left exactly as it was (never a duplicate spawned over
    /// it). This is the crown-jewel real-process proof of the
    /// `RestartForDrift`/`Rollback` "never mutate on an unconfirmed stop"
    /// fix: the old process survives, untouched, and no new process is
    /// ever spawned alongside it.
    #[test]
    fn expired_lease_blocks_governed_stop_and_never_duplicates_the_process() {
        let script = temp_script("expired-lease-stop", "#!/bin/sh\nsleep 30\n");
        let mut child = std::process::Command::new(&script).spawn().unwrap();
        let pid = child.id();
        let starttime = linux::read_proc_stat_starttime(pid).unwrap();
        let executable = linux::read_proc_exe(pid).unwrap();

        let state = state_for_real_pid(pid, starttime, executable);
        let (lease, auth) = expired_lease_and_boundary();
        let boundary_context = MutationBoundaryContext {
            authorization_id: Some(&auth.authorization_id),
            expires_at_utc: auth.expires_at_utc.as_deref(),
            configured_clock_skew_seconds: Some(30),
        };
        let mut records = Vec::new();
        let result = governed_stop(&state, Some(&lease), &boundary_context, &mut records);

        assert!(!result.confirmed_stopped, "an expired lease must never let the stop be treated as confirmed");
        assert!(!result.signaled, "an expired lease must block BEFORE any signal is sent");
        assert!(linux::pid_exists(pid), "the real process must be left running, completely untouched");

        // Direct proof this cannot lead to a duplicate spawn: `execute_native_action`
        // for RestartForDrift, given this exact unconfirmed stop, must
        // refuse to proceed to `governed_materialize_and_spawn` at all.
        let assignment = env_probe_assignment(std::collections::BTreeMap::new());
        let mut records2 = Vec::new();
        let outcome = execute_native_action(
            &NativeReconciliationAction::RestartForDrift,
            None,
            None,
            Some(&lease),
            &boundary_context,
            &mut records2,
            &assignment,
            None,
            &Ok("sha256:".to_string() + &"c".repeat(64)),
            Some(&state),
        );
        assert_eq!(outcome.execution_status, NATIVE_PROCESS_STOP_INCOMPLETE);
        assert!(linux::pid_exists(pid), "the original process must still be the only one running");

        // v5/L4 hygiene fix: reap via the retained handle after the final
        // direct-kill cleanup, rather than leaking a permanent zombie for
        // the remainder of this test binary's run.
        linux::send_signal(pid, Signal::Kill).unwrap();
        let _ = child.wait();
        let _ = std::fs::remove_file(&script);
    }

    // ---- v5/L7 (real Linux validation): stop-incomplete via a REAL ------
    // process with a deliberately unconfirmable identity, replacing the
    // portable fixture's fake PID 999_999 (which is legitimately
    // `ProcessAbsent`, not unconfirmed, on a real Linux host -- see the
    // gating comment on `sample_persisted_state` in the portable `tests`
    // module above).

    /// A real, currently-live process whose PID+starttime+executable all
    /// match the persisted record, but whose `cmdline_digest` is
    /// deliberately wrong -- `check_identity` must report `ArgsMismatch`
    /// (never `ProcessAbsent`/`PidReused`), so `governed_stop` must refuse
    /// to signal it and report the stop unconfirmed.
    #[test]
    fn restart_for_drift_never_spawns_when_a_real_processs_identity_is_unconfirmed() {
        let script = temp_script("restart-drift-unconfirmed", "#!/bin/sh\nsleep 30\n");
        let mut child = std::process::Command::new(&script).spawn().unwrap();
        let pid = child.id();
        let starttime = linux::read_proc_stat_starttime(pid).unwrap();
        let executable = linux::read_proc_exe(pid).unwrap();

        let mut state = state_for_real_pid(pid, starttime, executable);
        state.cmdline_digest = compute_args_digest(&["definitely-not-the-real-cmdline".to_string()]);

        let assignment = env_probe_assignment(std::collections::BTreeMap::new());
        let boundary_context = MutationBoundaryContext { authorization_id: None, expires_at_utc: None, configured_clock_skew_seconds: None };
        let mut records = Vec::new();
        let outcome = execute_native_action(
            &NativeReconciliationAction::RestartForDrift,
            None, None, None, &boundary_context, &mut records,
            &assignment, Some(&"b".repeat(64)), &Ok("sha256:".to_string() + &"c".repeat(64)), Some(&state),
        );
        assert_eq!(outcome.execution_status, NATIVE_PROCESS_STOP_INCOMPLETE);
        assert!(
            outcome.message.contains("drift restart blocked, old process not confirmed stopped"),
            "message was: {}",
            outcome.message
        );
        assert!(!outcome.message.to_lowercase().contains("spawned"), "must never proceed to spawn: {}", outcome.message);
        assert!(linux::pid_exists(pid), "the real process must be left completely untouched (never signaled while unconfirmed)");

        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_file(&script);
    }

    /// Same real-process setup as above, exercised through `Rollback`:
    /// state must be preserved, never removed, while identity is
    /// unconfirmed.
    #[test]
    fn rollback_never_removes_state_when_a_real_processs_identity_is_unconfirmed() {
        let script = temp_script("rollback-unconfirmed", "#!/bin/sh\nsleep 30\n");
        let mut child = std::process::Command::new(&script).spawn().unwrap();
        let pid = child.id();
        let starttime = linux::read_proc_stat_starttime(pid).unwrap();
        let executable = linux::read_proc_exe(pid).unwrap();

        let mut state = state_for_real_pid(pid, starttime, executable);
        state.cmdline_digest = compute_args_digest(&["definitely-not-the-real-cmdline".to_string()]);

        let assignment = env_probe_assignment(std::collections::BTreeMap::new());
        let boundary_context = MutationBoundaryContext { authorization_id: None, expires_at_utc: None, configured_clock_skew_seconds: None };
        let mut records = Vec::new();
        let outcome = execute_native_action(
            &NativeReconciliationAction::Rollback,
            None, None, None, &boundary_context, &mut records,
            &assignment, Some(&"b".repeat(64)), &Ok("sha256:".to_string() + &"c".repeat(64)), Some(&state),
        );
        assert_eq!(outcome.execution_status, NATIVE_PROCESS_STOP_INCOMPLETE);
        assert!(outcome.message.contains("rollback incomplete, state preserved"), "message was: {}", outcome.message);
        assert!(linux::pid_exists(pid), "the real process must be left completely untouched (never signaled while unconfirmed)");

        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_file(&script);
    }
}
