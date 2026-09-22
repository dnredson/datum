//! Real OS-process isolation for D-Code execution (ADR-0021, 111.7,
//! independent review v2 R5). The known, reproducible Wasmtime crash on this
//! platform (`STATUS_STACK_BUFFER_OVERRUN` on an automatic fuel/epoch
//! interrupt, `dcode::runtime`'s doc comment and `resource-limits-audit.txt`)
//! is a real sandbox-boundary failure: a canonical, untrusted D-Code module
//! must never be able to crash the SmartSentinel PARENT process. v1 only
//! documented this; it did not contain it. This module does.
//!
//! Architecture: `invoke_isolated` spawns a short-lived
//! `smartsentinel-dcode-worker` CHILD process per invocation. The child
//! links `dcode::runtime`/`dcode::module_store` and does the ACTUAL Wasmtime
//! `Engine`/`Store`/`Instance` work in-process -- exactly the code that could
//! crash on this platform. If it does crash (or panics, or hangs), the
//! PARENT observes only an abnormal exit status or a timeout; it never
//! shares an address space with Wasmtime, so a native crash there cannot
//! touch it. The parent enforces `artifact.limits.timeout_ms` itself via OS-
//! level process supervision (`kill`+`wait`, cross-platform via
//! `std::process::Child`) -- NOT Wasmtime epoch interruption, which is
//! exactly the in-process mechanism this module exists to route around.
//!
//! No locks from `api::ddeploy`/`api::dnode_registry`/`api::dcode` are ever
//! held across this call (there are none in the DATUM process at all --
//! those locks are dserver-side); this module performs no network I/O and no
//! host mutation beyond spawning/killing its own child process.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::dcode::invoke::DCodeExecutor;
use crate::dcode::runtime::{ExecutionOutcome, ExecutionOutput, InvocationLimits};

/// Independent review v3 (F2) correction: this file previously added a
/// 250ms `WATCHDOG_MARGIN_MS` grace period ON TOP OF `limits.timeout_ms`
/// before killing the child -- meaning `artifact.limits.timeout_ms = 200`
/// did NOT mean "killed at/before 200ms"; the effective allowed lifetime was
/// ~450ms. A fixed grace is legitimate as a TEST ASSERTION tolerance (a test
/// asserting "killed within roughly timeout_ms" may still want slack for
/// scheduler jitter) but is NOT acceptable as a silent expansion of the
/// canonical, host-enforced execution limit itself (ADR-0003/ADR-0021).
/// `timeout_ms` is now a hard parent-observed deadline for the WHOLE
/// isolated invocation -- INCLUDING process spawn and IPC overhead -- with
/// the deadline clock starting no later than the `Command::spawn()` call
/// below and NO execution grace added on top of the configured limit. This
/// is deliberately fail-closed: a very small `timeout_ms` may reject an
/// otherwise-fast module because spawn/IPC overhead consumed part of its
/// budget -- that is strictly safer than silently granting extra runtime a
/// canonical artifact never declared. If platform evidence ever proves a
/// separate startup allowance is strictly necessary, it must be implemented
/// as an EXPLICIT, separately-named startup-phase timeout with its own
/// READY handshake -- never folded into the guest's declared execution
/// budget the way a grace constant used to be here. Any test-assertion
/// tolerance for scheduler jitter now lives entirely in the test files that
/// need it (see `dcode_worker_isolation.rs`'s `TEST_TIMEOUT_TOLERANCE_MS`),
/// never in this production enforcement path.

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkerRequest {
    pub module_store_root: PathBuf,
    pub digest: String,
    pub limits: InvocationLimits,
    pub input_b64: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum WorkerResponse {
    /// Independent review v5 (H1): `output_b64` is `None` for the canonical
    /// "no output" case (`ExecutionOutput::NoOutput`) and `Some(..)` (base64,
    /// possibly encoding zero bytes) for `ExecutionOutput::Bytes` -- these
    /// remain two distinct wire representations across the worker IPC
    /// boundary, exactly as they are two distinct `ExecutionOutput` variants
    /// in-process.
    Success {
        output_b64: Option<String>,
    },
    ValidationFailed {
        message: String,
    },
    ResourceLimit {
        message: String,
    },
    Trap {
        message: String,
    },
}

impl From<ExecutionOutcome> for WorkerResponse {
    fn from(outcome: ExecutionOutcome) -> Self {
        match outcome {
            ExecutionOutcome::Success(ExecutionOutput::NoOutput) => {
                WorkerResponse::Success { output_b64: None }
            }
            ExecutionOutcome::Success(ExecutionOutput::Bytes(output)) => WorkerResponse::Success {
                output_b64: Some(base64_encode(&output)),
            },
            ExecutionOutcome::ValidationFailed(message) => {
                WorkerResponse::ValidationFailed { message }
            }
            ExecutionOutcome::ResourceLimit(message) => WorkerResponse::ResourceLimit { message },
            ExecutionOutcome::Trap(message) => WorkerResponse::Trap { message },
        }
    }
}

impl WorkerResponse {
    fn into_execution_outcome(self) -> ExecutionOutcome {
        match self {
            WorkerResponse::Success { output_b64: None } => {
                ExecutionOutcome::Success(ExecutionOutput::NoOutput)
            }
            WorkerResponse::Success {
                output_b64: Some(b64),
            } => match base64_decode(&b64) {
                Ok(bytes) => ExecutionOutcome::Success(ExecutionOutput::Bytes(bytes)),
                Err(error) => ExecutionOutcome::ResourceLimit(format!(
                    "dcode worker returned an unparseable success output: {error}"
                )),
            },
            WorkerResponse::ValidationFailed { message } => {
                ExecutionOutcome::ValidationFailed(message)
            }
            WorkerResponse::ResourceLimit { message } => ExecutionOutcome::ResourceLimit(message),
            WorkerResponse::Trap { message } => ExecutionOutcome::Trap(message),
        }
    }
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn base64_decode(value: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|error| error.to_string())
}

enum WaitOutcome {
    Exited(ExitStatus),
    TimedOut,
}

/// Independent review v3 (F2): takes an ABSOLUTE `deadline`, not a duration
/// measured from when this function happens to be called -- the caller's
/// clock starts at process spawn, so any time already spent serializing the
/// request or writing to stdin before this function is even entered is
/// correctly counted against the deadline, never granted back as free time.
fn wait_with_timeout(child: &mut Child, deadline: Instant) -> WaitOutcome {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return WaitOutcome::Exited(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    return WaitOutcome::TimedOut;
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(_) => {
                if Instant::now() >= deadline {
                    return WaitOutcome::TimedOut;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

/// Executes exactly one D-Code invocation in an isolated child process. The
/// PARENT (this function's caller) is guaranteed to return an
/// `ExecutionOutcome` regardless of whether the child exits cleanly, traps,
/// panics, crashes natively, or hangs past `limits.timeout_ms` -- it never
/// propagates a child crash/panic into this process.
pub fn invoke_isolated(
    worker_binary: &Path,
    module_store_root: &Path,
    digest: &str,
    limits: &InvocationLimits,
    input: &[u8],
) -> ExecutionOutcome {
    // F2: the deadline clock starts HERE, no later than spawn -- it is never
    // reset or extended afterward, so process-spawn/IPC overhead is counted
    // against `limits.timeout_ms`, not granted on top of it.
    let deadline_started = Instant::now();
    let mut child = match Command::new(worker_binary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return ExecutionOutcome::ResourceLimit(format!(
                "failed to spawn dcode worker process '{}': {error}",
                worker_binary.display()
            ))
        }
    };

    let request = WorkerRequest {
        module_store_root: module_store_root.to_path_buf(),
        digest: digest.to_string(),
        limits: *limits,
        input_b64: base64_encode(input),
    };
    let request_json = match serde_json::to_vec(&request) {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return ExecutionOutcome::ResourceLimit(format!(
                "failed to serialize dcode worker request: {error}"
            ));
        }
    };

    // Optional hardening (independent review v4): write stdin on a
    // background thread rather than blocking the main thread on
    // `write_all`. `request_json` is bounded (its `input_b64` component
    // encodes an already envelope-wrapped payload, itself capped by the
    // artifact's own `max_message_bytes`), and the worker binary is now
    // ALWAYS the trusted sibling install (G2A) whose `main` reads stdin to
    // completion before touching Wasmtime at all -- so this was already a
    // low-risk blocking window even before this change. Moving it to a
    // thread removes the risk entirely: the main thread reaches
    // `wait_with_timeout` immediately, so a hung/misbehaving child can never
    // delay the start of deadline enforcement, only its own outcome once the
    // deadline is reached. If the child exits early (e.g. it panics before
    // reading), the write simply fails on a broken pipe inside this thread
    // and is ignored -- `wait_with_timeout` below independently observes the
    // child's actual exit status either way.
    if let Some(mut stdin) = child.stdin.take() {
        thread::spawn(move || {
            let _ = stdin.write_all(&request_json);
            // `stdin` drops here, closing the pipe so the child's blocking
            // read-to-end on stdin observes EOF.
        });
    }

    let mut stdout_pipe = child.stdout.take().expect("stdout was piped");
    let mut stderr_pipe = child.stderr.take().expect("stderr was piped");

    let (stdout_tx, stdout_rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buffer);
        let _ = stdout_tx.send(buffer);
    });
    let (stderr_tx, stderr_rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buffer = String::new();
        let _ = stderr_pipe.read_to_string(&mut buffer);
        let _ = stderr_tx.send(buffer);
    });

    // F2: hard deadline = spawn time + declared timeout_ms, exactly -- no
    // added grace. `deadline_started` was captured immediately before
    // `Command::spawn()` above, so spawn + serialize + stdin-write overhead
    // already incurred by this point is counted against the budget.
    let deadline = deadline_started + Duration::from_millis(limits.timeout_ms);
    match wait_with_timeout(&mut child, deadline) {
        WaitOutcome::TimedOut => {
            let _ = child.kill();
            let _ = child.wait();
            ExecutionOutcome::ResourceLimit(format!(
                "dcode invocation exceeded wall-clock timeout_ms={} (worker process killed and reaped)",
                limits.timeout_ms
            ))
        }
        WaitOutcome::Exited(status) => {
            let stdout_bytes = stdout_rx
                .recv_timeout(Duration::from_secs(5))
                .unwrap_or_default();
            let stderr_text = stderr_rx
                .recv_timeout(Duration::from_secs(5))
                .unwrap_or_default();

            if !status.success() {
                return ExecutionOutcome::ResourceLimit(format!(
                    "dcode worker process exited abnormally (status={status:?}); this is safely contained -- the parent process was never affected; stderr={}",
                    stderr_text.trim()
                ));
            }
            match serde_json::from_slice::<WorkerResponse>(&stdout_bytes) {
                Ok(response) => response.into_execution_outcome(),
                Err(error) => ExecutionOutcome::ResourceLimit(format!(
                    "dcode worker exited successfully but produced no valid response ({error}); stderr={}",
                    stderr_text.trim()
                )),
            }
        }
    }
}

/// Independent review fix v5 (K1C): the ONE centralized trusted-worker-path
/// resolver in the crate -- the sibling `smartsentinel-dcode-worker`
/// executable next to whatever binary is currently running. Previously this
/// exact logic was duplicated in `smartsentinel-dcode-invoke.rs`,
/// `smartsentinel-dcall-listen.rs`, and `dcall::delivery`; all three now call
/// this instead. Only correct for a real INSTALLED binary -- a `cargo test`
/// binary's `current_exe()` does not live alongside the real worker binary
/// the way an installation does (Cargo puts test binaries under
/// `target/.../deps`, one level below where `[[bin]]` targets are placed),
/// which is exactly why the `dcall-test-support`-feature-gated test seams
/// exist to let a test supply `env!("CARGO_BIN_EXE_smartsentinel-dcode-worker")`
/// explicitly instead.
pub fn trusted_sibling_worker_binary_path() -> PathBuf {
    let mut path = std::env::current_exe().expect("current_exe should resolve");
    path.set_file_name(if cfg!(windows) {
        "smartsentinel-dcode-worker.exe"
    } else {
        "smartsentinel-dcode-worker"
    });
    path
}

/// The REAL production `DCodeExecutor` -- spawns a fresh isolated worker
/// child process per invocation (see `invoke_isolated` above). Used
/// exclusively by the governed CLI binaries; never by `dcode::invoke`'s own
/// unit tests (which use the faster, non-isolating `InProcessExecutor`).
pub struct IsolatedWorkerExecutor<'a> {
    pub worker_binary: &'a Path,
    pub module_store_root: &'a Path,
}

impl DCodeExecutor for IsolatedWorkerExecutor<'_> {
    fn execute(&self, digest: &str, limits: &InvocationLimits, input: &[u8]) -> ExecutionOutcome {
        invoke_isolated(
            self.worker_binary,
            self.module_store_root,
            digest,
            limits,
            input,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_guard_confirms_process_isolation_lives_here_not_in_runtime() {
        // `dcode::runtime`'s own source guard asserts it does NOT contain
        // `std::process::Command` -- this module is where that
        // responsibility actually lives instead.
        let source = include_str!("worker.rs");
        assert!(
            source.contains("std::process::"),
            "worker.rs must own process spawning/supervision"
        );
    }

    #[test]
    fn worker_response_round_trips_success_through_json() {
        let response = WorkerResponse::from(ExecutionOutcome::Success(ExecutionOutput::Bytes(
            b"hello".to_vec(),
        )));
        let json = serde_json::to_string(&response).unwrap();
        let parsed: WorkerResponse = serde_json::from_str(&json).unwrap();
        match parsed.into_execution_outcome() {
            ExecutionOutcome::Success(ExecutionOutput::Bytes(bytes)) => assert_eq!(bytes, b"hello"),
            other => panic!("expected Success(Bytes), got {other:?}"),
        }
    }

    /// H1 regression: the canonical "no output" case round-trips through the
    /// worker IPC JSON boundary as a distinct representation (`output_b64:
    /// None`), never conflated with `Bytes(vec![])`.
    #[test]
    fn worker_response_round_trips_no_output_through_json_as_a_distinct_case() {
        let response = WorkerResponse::from(ExecutionOutcome::Success(ExecutionOutput::NoOutput));
        let json = serde_json::to_string(&response).unwrap();
        assert!(
            json.contains("\"output_b64\":null"),
            "no-output must serialize as a null output_b64, got: {json}"
        );
        let parsed: WorkerResponse = serde_json::from_str(&json).unwrap();
        match parsed.into_execution_outcome() {
            ExecutionOutcome::Success(ExecutionOutput::NoOutput) => {}
            other => panic!("expected Success(NoOutput), got {other:?}"),
        }
    }

    #[test]
    fn spawning_a_nonexistent_worker_binary_fails_safely_without_panicking() {
        let outcome = invoke_isolated(
            Path::new("this-binary-does-not-exist-anywhere"),
            Path::new("."),
            &"a".repeat(64),
            &InvocationLimits {
                fuel_per_invocation: 1,
                timeout_ms: 100,
                max_memory_pages: 1,
                max_message_bytes: 1,
            },
            b"",
        );
        match outcome {
            ExecutionOutcome::ResourceLimit(message) => {
                assert!(message.contains("failed to spawn"))
            }
            other => panic!("expected ResourceLimit, got {other:?}"),
        }
    }
}
