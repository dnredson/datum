//! Stage 109 (ADR-0016). `ManagedProcessRuntimeStateV1` -- OBSERVED LOCAL
//! RUNTIME STATE for a managed native process.
//!
//! This is deliberately NOT desired state: it never enters `ServiceArtifact`,
//! `NativeProcessDeploymentSpecV1`, `DGraph`, `DMap`, or the execution
//! projection. An ordinary process restart (or SmartSentinel-restart
//! adoption) changes `pid`/`proc_start_time_ticks`/`started_at_utc`/`state`
//! freely; none of that may ever increment `execution_generation` or change
//! `execution_projection_digest` -- those live entirely in
//! `ArtifactReconciliationAssignment`/`OperationalArtifactAssignment` and are
//! server-owned, never derived from this type.
//!
//! Persisted locally by the agent as one JSON file per managed service under
//! `<native_process_root>/state/<stable-service-key>.json`
//! (`native_process_acquisition::ManagedLayout::state_path`) -- a fresh
//! SmartSentinel process reads this file, never an in-memory `Child` handle,
//! to recover which process (if any) it previously started.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MANAGED_PROCESS_RUNTIME_STATE_SCHEMA: &str = "datum.managed-process-runtime-state/1";

pub const PROCESS_STATE_RUNNING: &str = "running";
pub const PROCESS_STATE_STOPPED: &str = "stopped";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedProcessRuntimeStateV1 {
    pub schema: String,
    pub service_id: String,
    pub artifact_id: String,
    /// Server-owned generation/digest THIS running process was spawned
    /// under -- copied here purely as observed-history evidence (e.g. "which
    /// desired version is this PID actually running"), never treated as an
    /// input to recomputing either value; only
    /// `ArtifactReconciliationAssignment`/`OperationalArtifactAssignment`
    /// authoritatively carry them.
    pub execution_generation: u64,
    pub execution_projection_digest: String,
    pub content_sha256: String,
    pub pid: u32,
    pub proc_start_time_ticks: u64,
    pub observed_executable: String,
    /// SHA-256 (hex) of the COMPLETE observed `/proc/<pid>/cmdline` vector
    /// (including argv[0]) at spawn time, via `compute_args_digest`. This is
    /// runtime identity, not desired-state identity -- it is deliberately
    /// NOT derived from `NativeProcessDeploymentSpecV1.args` (the desired
    /// argv used to spawn), because a real `/proc/<pid>/cmdline` is
    /// interpreter-shaped for a shebang script (e.g. `["/bin/sh",
    /// "<script-path>", ...desired args]`), not simply the desired argv
    /// with argv[0] removed. Every later identity check
    /// (`native_process_runtime::observe_persisted_identity`) re-reads the
    /// complete raw `/proc/<pid>/cmdline` and compares its digest against
    /// this value exactly -- never against the desired args alone.
    pub cmdline_digest: String,
    pub started_at_utc: String,
    pub state: String,
}

/// SHA-256 (hex) of a string vector -- NUL-separated, so elements
/// containing spaces or embedded separators can never collide with a
/// different vector that happens to concatenate to the same string. Used
/// both to hash a complete observed `/proc/<pid>/cmdline` vector (the
/// `cmdline_digest` domain) and, historically, a desired argv -- the
/// algorithm itself is agnostic to which kind of vector it is given.
pub fn compute_args_digest(args: &[String]) -> String {
    let mut hasher = Sha256::new();
    for (index, arg) in args.iter().enumerate() {
        if index > 0 {
            hasher.update([0u8]);
        }
        hasher.update(arg.as_bytes());
    }
    let digest = hasher.finalize();
    let mut s = String::with_capacity(64);
    for byte in digest {
        s.push_str(&format!("{byte:02x}"));
    }
    s
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcStatParseError(pub String);

impl std::fmt::Display for ProcStatParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ProcStatParseError {}

/// Parse `/proc/<pid>/stat`'s field 22 (`starttime`, clock ticks since boot)
/// out of the raw file contents.
///
/// Field 2 (`comm`, the executable's basename) is wrapped in parentheses and
/// MAY ITSELF CONTAIN SPACES AND PARENTHESES (a process can rename itself to
/// almost anything via `prctl(PR_SET_NAME)`/argv[0] tricks) -- naively
/// splitting the whole line on whitespace from the start would misalign
/// every field after `comm` for such a process. This parser instead locates
/// the LAST `)` in the line (the `comm` field is always immediately followed
/// by a space then the numeric `state` field, so the true closing paren is
/// the rightmost one: no numeric field can itself contain `)`), then indexes
/// forward from there.
///
/// Pure and portable -- takes the file's contents as a `&str`, never reads
/// `/proc` itself, so it is unit-tested identically on every development
/// host, not only Linux.
/// Shared groundwork for both `parse_proc_stat_starttime` and
/// `parse_proc_stat_state_char`: locate the `comm` field's closing `)`
/// (the LAST one, since `comm` may itself contain spaces/parentheses) and
/// split everything after it into fields 3.. (`fields[0]` is field 3, the
/// `state` field; field N, N >= 3, is `fields[N - 3]`).
fn proc_stat_fields_after_comm(stat_contents: &str) -> Result<Vec<&str>, ProcStatParseError> {
    let open = stat_contents
        .find('(')
        .ok_or_else(|| ProcStatParseError("malformed /proc/<pid>/stat: no '(' found".to_string()))?;
    let close = stat_contents
        .rfind(')')
        .ok_or_else(|| ProcStatParseError("malformed /proc/<pid>/stat: no ')' found".to_string()))?;
    if close <= open {
        return Err(ProcStatParseError(
            "malformed /proc/<pid>/stat: ')' does not appear after '('".to_string(),
        ));
    }
    let after_comm = &stat_contents[close + 1..];
    Ok(after_comm.split_whitespace().collect())
}

pub fn parse_proc_stat_starttime(stat_contents: &str) -> Result<u64, ProcStatParseError> {
    let fields = proc_stat_fields_after_comm(stat_contents)?;
    const STARTTIME_FIELD_NUMBER: usize = 22;
    let index = STARTTIME_FIELD_NUMBER - 3;
    let raw = fields.get(index).ok_or_else(|| {
        ProcStatParseError(format!(
            "malformed /proc/<pid>/stat: expected at least {} fields after the comm field, found {}",
            index + 1,
            fields.len()
        ))
    })?;
    raw.parse::<u64>()
        .map_err(|error| ProcStatParseError(format!("malformed /proc/<pid>/stat starttime field '{raw}': {error}")))
}

/// Field 3 (`state`): a single character (`R` running, `S` sleeping, `Z`
/// zombie, etc.). Used (Section AA "Zombie" invariant / P1-4) to detect a
/// managed process that has already exited but not yet been reaped -- a
/// zombie retains its PID/starttime (so `IdentityCheck` would otherwise
/// report `Confirmed`), but it is NOT a running workload and must never be
/// reported as converged/running, nor escalated to SIGKILL.
pub fn parse_proc_stat_state_char(stat_contents: &str) -> Result<char, ProcStatParseError> {
    let fields = proc_stat_fields_after_comm(stat_contents)?;
    let raw = fields
        .first()
        .ok_or_else(|| ProcStatParseError("malformed /proc/<pid>/stat: missing state field".to_string()))?;
    raw.chars()
        .next()
        .ok_or_else(|| ProcStatParseError("malformed /proc/<pid>/stat: empty state field".to_string()))
}

/// The outcome of comparing a persisted `ManagedProcessRuntimeStateV1`
/// against freshly-observed process facts on adoption/re-verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityCheck {
    /// PID, starttime, executable, and args digest all match -- this is
    /// exactly the process this agent (or a prior instance of it) started.
    Confirmed,
    /// No process exists at this PID at all.
    ProcessAbsent,
    /// A process exists at this PID, but its starttime does not match the
    /// persisted value -- the PID was reused by an unrelated process. The
    /// managed process must be treated as absent; this PID must NEVER be
    /// signaled.
    PidReused,
    /// The PID+starttime pair matches, but the observed executable path
    /// differs from what was persisted -- never adopt.
    ExecutableMismatch,
    /// The PID+starttime pair matches, but the observed complete
    /// `/proc/<pid>/cmdline` digest differs from what was persisted --
    /// never adopt. (Variant name retained from v0.1/v2/v3/v4 for call-site
    /// stability; the underlying model is the observed cmdline, not a
    /// desired-args-only comparison -- see the v5 Linux-closure ADR
    /// addendum.)
    ArgsMismatch,
    /// PID+starttime matched (this is provably the same process generation
    /// this agent started), but a subsequent `/proc` read needed to fully
    /// re-confirm identity (executable or cmdline) failed -- a procfs race,
    /// not a disproof of identity. Never signal, never spawn a duplicate;
    /// preserve state and report the uncertainty truthfully.
    Unavailable,
    /// PID+starttime matched, but `/proc/<pid>/stat`'s state field is `Z`
    /// (zombie) -- the process has already exited and is only awaiting
    /// reaping. NOT "running/converged" (Section AA's explicit invariant),
    /// but also not something to SIGKILL (a zombie cannot be signaled
    /// meaningfully) -- safe for governed stale-state cleanup instead.
    Zombie,
}

/// Pure comparison: given the persisted state and freshly observed facts
/// (already read from `/proc` by the caller), decide the identity outcome.
/// Never signals or touches the OS itself -- that is
/// `native_process_runtime`'s job, and it must call this first.
///
/// `observed_executable`/`observed_cmdline_digest` are `None` ONLY when the
/// corresponding `/proc` read was actually attempted and failed (e.g. a
/// procfs race where `/proc/<pid>/exe` or `/proc/<pid>/cmdline` disappears
/// between the PID-existence check and this read) -- NEVER used to mean
/// "skip this check." A failed read after PID+starttime already matched is
/// a materially different situation from a clean mismatch: the process is
/// provably still the same PID generation (starttime matches), so it is
/// NOT absent and NOT a PID-reuse case, yet its identity cannot be fully
/// re-confirmed either. Collapsing that into `Confirmed` (by skipping the
/// check) would let a procfs race silently authorize a signal against a
/// process whose identity was never actually re-verified; collapsing it
/// into a mismatch variant would wrongly suggest a REAL executable/args
/// difference was observed, when none was -- so this is its own
/// `IdentityCheck::Unavailable` outcome (never signal, never spawn a
/// duplicate, preserve state, report uncertainty truthfully).
/// `observed_pid_exists = false` means the PID is PROVABLY gone (e.g. a
/// concrete ESRCH from `kill(pid, 0)`, or `/proc/<pid>/stat` returning
/// `ENOENT` -- a real disappearance, not merely "a read failed"). A stat
/// read that fails for ANY OTHER reason (permission, I/O, malformed
/// content) must be passed as `observed_starttime: None` while
/// `observed_pid_exists` stays `true` -- P0-4/stat-read-failure fix: this
/// is `Unavailable`, never automatically `ProcessAbsent`, because the PID
/// itself was never actually disproven to exist.
pub fn check_identity(
    persisted: &ManagedProcessRuntimeStateV1,
    observed_pid_exists: bool,
    observed_starttime: Option<u64>,
    observed_state_char: Option<char>,
    observed_executable: Option<&str>,
    observed_cmdline_digest: Option<&str>,
) -> IdentityCheck {
    if !observed_pid_exists {
        return IdentityCheck::ProcessAbsent;
    }
    let Some(starttime) = observed_starttime else {
        return IdentityCheck::Unavailable;
    };
    if starttime != persisted.proc_start_time_ticks {
        return IdentityCheck::PidReused;
    }
    if observed_state_char == Some('Z') {
        return IdentityCheck::Zombie;
    }
    let Some(executable) = observed_executable else {
        return IdentityCheck::Unavailable;
    };
    if executable != persisted.observed_executable {
        return IdentityCheck::ExecutableMismatch;
    }
    let Some(cmdline_digest) = observed_cmdline_digest else {
        return IdentityCheck::Unavailable;
    };
    if cmdline_digest != persisted.cmdline_digest {
        return IdentityCheck::ArgsMismatch;
    }
    IdentityCheck::Confirmed
}

/// Load a persisted `ManagedProcessRuntimeStateV1` from `path`, if present.
/// `Ok(None)` means no managed process has ever been recorded for this
/// service (never an error) -- the caller treats it as "no state to adopt."
///
/// Deliberately does NOT run `validate_loaded_state` itself -- a
/// syntactically well-formed-but-semantically-invalid record is a
/// different, distinguishable outcome from "file unreadable/malformed
/// JSON" (Section AE), and callers that need to tell the two apart (to
/// report a stable, distinct `invalid_managed_process_state` truthfully)
/// call `validate_loaded_state` themselves immediately after a successful
/// load.
pub fn load_state(path: &std::path::Path) -> Result<Option<ManagedProcessRuntimeStateV1>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("read runtime-state file '{}': {e}", path.display()))?;
    let state: ManagedProcessRuntimeStateV1 =
        serde_json::from_str(&text).map_err(|e| format!("parse runtime-state file '{}': {e}", path.display()))?;
    Ok(Some(state))
}

/// Treat a successfully-JSON-parsed `ManagedProcessRuntimeStateV1` as
/// untrusted local input (Section AE) -- not an enterprise-security
/// exercise, basic recovery correctness: a record can be syntactically
/// valid JSON yet semantically nonsensical (e.g. hand-edited, produced by
/// a future/incompatible schema version, or corrupted in a way that
/// happens to still deserialize) and must never be silently trusted enough
/// to signal a PID derived from it or to be treated as `Confirmed` without
/// ever being compared against a real observation.
///
/// Returns `Err` (never panics) describing the first validation failure
/// found. Does not delete or otherwise mutate anything -- validation is
/// read-only; the caller decides what governed action (if any) is safe to
/// take on an invalid record.
pub fn validate_loaded_state(state: &ManagedProcessRuntimeStateV1) -> Result<(), String> {
    if state.schema != MANAGED_PROCESS_RUNTIME_STATE_SCHEMA {
        return Err(format!(
            "unrecognized schema '{}' (expected '{MANAGED_PROCESS_RUNTIME_STATE_SCHEMA}')",
            state.schema
        ));
    }
    if state.service_id.trim().is_empty() {
        return Err("service_id is empty".to_string());
    }
    if state.artifact_id.trim().is_empty() {
        return Err("artifact_id is empty".to_string());
    }
    if state.pid == 0 {
        return Err("pid is 0, which is never a valid managed process id".to_string());
    }
    if !crate::agent::native_process_acquisition::is_valid_content_sha256(&state.content_sha256) {
        return Err(format!(
            "content_sha256 '{}' is not a valid 64-character lowercase hex digest",
            state.content_sha256
        ));
    }
    let digest_ok = state
        .execution_projection_digest
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()));
    if !digest_ok {
        return Err(format!(
            "execution_projection_digest '{}' is not a valid 'sha256:<64-hex>' canonical digest",
            state.execution_projection_digest
        ));
    }
    if state.observed_executable.trim().is_empty() {
        return Err("observed_executable is empty".to_string());
    }
    if state.cmdline_digest.len() != 64 || !state.cmdline_digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("cmdline_digest '{}' is not a valid 64-character hex digest", state.cmdline_digest));
    }
    if state.started_at_utc.trim().is_empty() {
        return Err("started_at_utc is empty".to_string());
    }
    if state.state != PROCESS_STATE_RUNNING && state.state != PROCESS_STATE_STOPPED {
        return Err(format!(
            "state '{}' is not a recognized value ('{PROCESS_STATE_RUNNING}' or '{PROCESS_STATE_STOPPED}')",
            state.state
        ));
    }
    Ok(())
}

/// Persist `state` atomically (temp file + rename) to `path`, creating the
/// parent directory if needed.
pub fn persist_state(path: &std::path::Path, state: &ManagedProcessRuntimeStateV1) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create runtime-state directory: {e}"))?;
    }
    let text = serde_json::to_string_pretty(state).map_err(|e| format!("serialize runtime-state: {e}"))?;
    let tmp_path = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp_path, text).map_err(|e| format!("write temp runtime-state file: {e}"))?;
    std::fs::rename(&tmp_path, path).map_err(|e| format!("rename runtime-state file into place: {e}"))?;
    Ok(())
}

/// Remove a persisted runtime-state file if present. Idempotent: absence is
/// success, never an error (mirrors this codebase's existing reconciliation
/// idempotency discipline).
pub fn remove_state(path: &std::path::Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_file(path).map_err(|e| format!("remove runtime-state file: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_state() -> ManagedProcessRuntimeStateV1 {
        ManagedProcessRuntimeStateV1 {
            schema: MANAGED_PROCESS_RUNTIME_STATE_SCHEMA.to_string(),
            service_id: "svc".to_string(),
            artifact_id: "svc".to_string(),
            execution_generation: 3,
            execution_projection_digest: "sha256:".to_string() + &"a".repeat(64),
            content_sha256: "b".repeat(64),
            pid: 4242,
            proc_start_time_ticks: 123456,
            observed_executable: "/managed/root/artifacts/bbbb/entrypoint".to_string(),
            cmdline_digest: compute_args_digest(&["--config".to_string(), "cfg.toml".to_string()]),
            started_at_utc: "2026-01-01T00:00:00Z".to_string(),
            state: PROCESS_STATE_RUNNING.to_string(),
        }
    }

    // ---- args digest ------------------------------------------------- //

    #[test]
    fn args_digest_is_stable_and_order_sensitive() {
        let a = compute_args_digest(&["--config".to_string(), "x".to_string()]);
        let b = compute_args_digest(&["--config".to_string(), "x".to_string()]);
        let c = compute_args_digest(&["x".to_string(), "--config".to_string()]);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn args_digest_distinguishes_embedded_separators() {
        // Without a NUL separator, ["a b", "c"] and ["a", "b c"] would both
        // naively join to "a b c" -- the NUL-joined digest must NOT collide.
        let d1 = compute_args_digest(&["a b".to_string(), "c".to_string()]);
        let d2 = compute_args_digest(&["a".to_string(), "b c".to_string()]);
        assert_ne!(d1, d2);
    }

    // ---- /proc/<pid>/stat parsing ------------------------------------- //

    #[test]
    fn parses_ordinary_stat_line() {
        // Real-shaped example: pid=100, comm=(bash), state=S, ppid=1, ...,
        // starttime (field 22) = 987654.
        let mut fields = vec!["100", "(bash)", "S", "1", "1", "1", "0", "-1", "4194304"];
        // fields so far: pid,comm,state,ppid,pgrp,session,tty_nr,tpgid,flags = 9 fields (1..=9)
        // need up to field 22: minutes(10) through starttime(22) => 13 more filler fields (10..=21), then starttime at 22.
        for _ in 10..=21 {
            fields.push("0");
        }
        fields.push("987654"); // field 22
        let line = fields.join(" ");
        assert_eq!(parse_proc_stat_starttime(&line).unwrap(), 987654);
    }

    #[test]
    fn parses_comm_containing_spaces_and_parentheses() {
        let mut fields = vec!["100", "(my (weird) proc name)", "S", "1", "1", "1", "0", "-1", "4194304"];
        for _ in 10..=21 {
            fields.push("0");
        }
        fields.push("555555");
        let line = fields.join(" ");
        assert_eq!(parse_proc_stat_starttime(&line).unwrap(), 555555);
    }

    #[test]
    fn rejects_malformed_missing_parens() {
        assert!(parse_proc_stat_starttime("100 bash S 1 1").is_err());
    }

    #[test]
    fn rejects_malformed_too_few_fields() {
        assert!(parse_proc_stat_starttime("100 (bash) S 1 1").is_err());
    }

    #[test]
    fn rejects_malformed_non_numeric_starttime() {
        let mut fields = vec!["100", "(bash)", "S", "1", "1", "1", "0", "-1", "4194304"];
        for _ in 10..=21 {
            fields.push("0");
        }
        fields.push("not-a-number");
        let line = fields.join(" ");
        assert!(parse_proc_stat_starttime(&line).is_err());
    }

    // ---- identity comparison ------------------------------------------ //

    #[test]
    fn identity_confirmed_when_everything_matches() {
        let persisted = sample_state();
        let outcome = check_identity(
            &persisted,
            true,
            Some(persisted.proc_start_time_ticks),
            Some('R'),
            Some(persisted.observed_executable.as_str()),
            Some(persisted.cmdline_digest.as_str()),
        );
        assert_eq!(outcome, IdentityCheck::Confirmed);
    }

    #[test]
    fn identity_absent_when_pid_does_not_exist() {
        let persisted = sample_state();
        let outcome = check_identity(&persisted, false, None, None, None, None);
        assert_eq!(outcome, IdentityCheck::ProcessAbsent);
    }

    #[test]
    fn identity_pid_reused_when_starttime_differs() {
        let persisted = sample_state();
        let outcome = check_identity(&persisted, true, Some(persisted.proc_start_time_ticks + 1), None, None, None);
        assert_eq!(outcome, IdentityCheck::PidReused);
    }

    #[test]
    fn identity_executable_mismatch_never_adopts() {
        let persisted = sample_state();
        let outcome = check_identity(
            &persisted,
            true,
            Some(persisted.proc_start_time_ticks),
            Some('R'),
            Some("/some/other/binary"),
            None,
        );
        assert_eq!(outcome, IdentityCheck::ExecutableMismatch);
    }

    #[test]
    fn identity_args_mismatch_never_adopts() {
        let persisted = sample_state();
        let outcome = check_identity(
            &persisted,
            true,
            Some(persisted.proc_start_time_ticks),
            Some('R'),
            Some(persisted.observed_executable.as_str()),
            Some("different-digest"),
        );
        assert_eq!(outcome, IdentityCheck::ArgsMismatch);
    }

    // ---- procfs-race "unavailable" (Section AH#2, AA) ------------------ //

    #[test]
    fn identity_unavailable_when_executable_read_failed_after_starttime_matched() {
        let persisted = sample_state();
        // PID exists, starttime matches (same process generation), but the
        // executable read failed (e.g. `/proc/<pid>/exe` raced away) --
        // must be neither Confirmed nor a mismatch variant.
        let outcome = check_identity(&persisted, true, Some(persisted.proc_start_time_ticks), Some('R'), None, None);
        assert_eq!(outcome, IdentityCheck::Unavailable);
    }

    #[test]
    fn identity_unavailable_when_args_read_failed_after_executable_matched() {
        let persisted = sample_state();
        let outcome = check_identity(
            &persisted,
            true,
            Some(persisted.proc_start_time_ticks),
            Some('R'),
            Some(persisted.observed_executable.as_str()),
            None,
        );
        assert_eq!(outcome, IdentityCheck::Unavailable);
    }

    #[test]
    fn identity_unavailable_never_confused_with_pid_reused_or_absent() {
        // Sanity: Unavailable requires PID existence AND a starttime match --
        // it is not merely "we don't have executable/args info."
        let persisted = sample_state();
        assert_eq!(check_identity(&persisted, false, None, None, None, None), IdentityCheck::ProcessAbsent);
        assert_eq!(
            check_identity(&persisted, true, Some(persisted.proc_start_time_ticks + 1), None, None, None),
            IdentityCheck::PidReused
        );
    }

    // ---- P0-4: a stat-read failure is NOT automatically "absent" -------- //

    #[test]
    fn identity_unavailable_when_pid_exists_but_starttime_read_failed_for_a_non_absence_reason() {
        // `observed_pid_exists = true` (kill(pid,0) succeeded, or reported
        // EPERM -- either way the PID slot is occupied) but the starttime
        // read itself failed for a reason OTHER than a proven disappearance
        // (permission/I/O/parse error). This must NOT be silently treated
        // as `ProcessAbsent` -- that would let a native process be
        // respawned as a duplicate while the original might still be
        // alive under a PID this agent merely failed to fully inspect.
        let persisted = sample_state();
        let outcome = check_identity(&persisted, true, None, None, None, None);
        assert_eq!(outcome, IdentityCheck::Unavailable);
    }

    // ---- P1-4: zombie is not "running/converged" ------------------------ //

    #[test]
    fn identity_zombie_when_state_char_is_z_even_with_matching_pid_and_starttime() {
        let persisted = sample_state();
        let outcome = check_identity(
            &persisted,
            true,
            Some(persisted.proc_start_time_ticks),
            Some('Z'),
            Some(persisted.observed_executable.as_str()),
            Some(persisted.cmdline_digest.as_str()),
        );
        assert_eq!(outcome, IdentityCheck::Zombie);
    }

    #[test]
    fn identity_zombie_takes_priority_over_executable_or_args_mismatch() {
        // A zombie process's /proc/<pid>/exe read may itself fail or
        // report something stale -- the zombie determination (from the
        // stat state char) must be checked BEFORE the executable/args
        // comparison, not after, so a zombie is never miscategorized as a
        // plain identity mismatch (which the reconciliation layer treats
        // differently -- see native-runtime-state-machine-audit.txt).
        let persisted = sample_state();
        let outcome = check_identity(&persisted, true, Some(persisted.proc_start_time_ticks), Some('Z'), None, None);
        assert_eq!(outcome, IdentityCheck::Zombie);
    }

    #[test]
    fn parse_proc_stat_state_char_reads_field_3() {
        let mut fields = vec!["100", "(bash)", "Z", "1", "1", "1", "0", "-1", "4194304"];
        for _ in 10..=21 {
            fields.push("0");
        }
        fields.push("987654");
        let line = fields.join(" ");
        assert_eq!(parse_proc_stat_state_char(&line).unwrap(), 'Z');
    }

    #[test]
    fn parse_proc_stat_state_char_handles_comm_with_spaces() {
        let mut fields = vec!["100", "(my (weird) proc name)", "R", "1", "1", "1", "0", "-1", "4194304"];
        for _ in 10..=21 {
            fields.push("0");
        }
        fields.push("555555");
        let line = fields.join(" ");
        assert_eq!(parse_proc_stat_state_char(&line).unwrap(), 'R');
    }

    // ---- managed-state validation (Section AE) -------------------------- //

    #[test]
    fn validate_loaded_state_accepts_a_well_formed_record() {
        assert!(validate_loaded_state(&sample_state()).is_ok());
    }

    #[test]
    fn validate_loaded_state_rejects_zero_pid() {
        let mut state = sample_state();
        state.pid = 0;
        assert!(validate_loaded_state(&state).is_err());
    }

    #[test]
    fn validate_loaded_state_rejects_malformed_content_sha256() {
        let mut state = sample_state();
        state.content_sha256 = "not-a-hash".to_string();
        assert!(validate_loaded_state(&state).is_err());
    }

    #[test]
    fn validate_loaded_state_rejects_malformed_execution_projection_digest() {
        let mut state = sample_state();
        state.execution_projection_digest = "not-canonical".to_string();
        assert!(validate_loaded_state(&state).is_err());
        let mut state2 = sample_state();
        state2.execution_projection_digest = "sha256:tooshort".to_string();
        assert!(validate_loaded_state(&state2).is_err());
    }

    #[test]
    fn validate_loaded_state_rejects_empty_identity_fields() {
        let mut state = sample_state();
        state.service_id = String::new();
        assert!(validate_loaded_state(&state).is_err());
        let mut state2 = sample_state();
        state2.observed_executable = String::new();
        assert!(validate_loaded_state(&state2).is_err());
    }

    #[test]
    fn validate_loaded_state_rejects_unknown_schema_and_state_value() {
        let mut state = sample_state();
        state.schema = "some.other.schema/9".to_string();
        assert!(validate_loaded_state(&state).is_err());
        let mut state2 = sample_state();
        state2.state = "definitely_not_a_real_state".to_string();
        assert!(validate_loaded_state(&state2).is_err());
    }

    #[test]
    fn validate_loaded_state_rejects_malformed_cmdline_digest() {
        let mut state = sample_state();
        state.cmdline_digest = "short".to_string();
        assert!(validate_loaded_state(&state).is_err());
    }

    // ---- persistence --------------------------------------------------- //

    fn temp_path(label: &str) -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("datum-native-state-test-{label}-{}", std::process::id()));
        dir
    }

    #[test]
    fn persist_and_load_round_trips() {
        let path = temp_path("roundtrip");
        let state = sample_state();
        persist_state(&path, &state).unwrap();
        let loaded = load_state(&path).unwrap().expect("state must load back");
        assert_eq!(loaded, state);
        remove_state(&path).unwrap();
        assert!(load_state(&path).unwrap().is_none());
    }

    #[test]
    fn load_missing_state_is_none_not_error() {
        let path = temp_path("missing");
        let _ = std::fs::remove_file(&path);
        assert!(load_state(&path).unwrap().is_none());
    }

    #[test]
    fn remove_missing_state_is_idempotent() {
        let path = temp_path("remove-idempotent");
        let _ = std::fs::remove_file(&path);
        remove_state(&path).unwrap();
        remove_state(&path).unwrap();
    }
}
