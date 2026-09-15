//! Isolated D-Code execution worker (ADR-0021, independent review v2 R5).
//! NEVER invoked directly by an operator -- spawned as a short-lived child
//! process by `datum::dcode::worker::invoke_isolated`. Reads one
//! `WorkerRequest` (JSON) from stdin, does the ACTUAL Wasmtime `Engine`/
//! `Store`/`Instance` invocation in-process (`dcode::runtime`), and writes
//! exactly one `WorkerResponse` (JSON) to stdout. This is deliberately the
//! ONLY place canonical D-Code guest bytes are ever actually executed: if
//! Wasmtime crashes natively here (the documented Windows fuel/epoch
//! interrupt defect, `resource-limits-audit.txt`), only THIS short-lived
//! process is affected -- the parent SmartSentinel process that spawned it
//! observes an abnormal exit status and continues running.

use std::io::Read;

use datum::dcode::module_store::DCodeModuleStore;
use datum::dcode::runtime::DCodeModuleCache;
use datum::dcode::worker::{WorkerRequest, WorkerResponse};

fn respond_and_exit(response: WorkerResponse, exit_code: i32) -> ! {
    let json = serde_json::to_string(&response).unwrap_or_else(|_| {
        r#"{"outcome":"resource_limit","message":"dcode worker failed to serialize its own response"}"#.to_string()
    });
    println!("{json}");
    std::process::exit(exit_code);
}

fn main() {
    let mut input = Vec::new();
    if let Err(error) = std::io::stdin().read_to_end(&mut input) {
        respond_and_exit(
            WorkerResponse::ResourceLimit { message: format!("dcode worker failed to read stdin: {error}") },
            1,
        );
    }

    let request: WorkerRequest = match serde_json::from_slice(&input) {
        Ok(request) => request,
        Err(error) => {
            respond_and_exit(
                WorkerResponse::ResourceLimit { message: format!("dcode worker received a malformed request: {error}") },
                1,
            );
        }
    };

    let input_bytes = {
        use base64::Engine as _;
        match base64::engine::general_purpose::STANDARD.decode(&request.input_b64) {
            Ok(bytes) => bytes,
            Err(error) => {
                respond_and_exit(
                    WorkerResponse::ResourceLimit { message: format!("dcode worker received malformed input_b64: {error}") },
                    1,
                );
            }
        }
    };

    let store = DCodeModuleStore::new(&request.module_store_root);
    let module_bytes = match store.load(&request.digest) {
        Ok(bytes) => bytes,
        Err(error) => respond_and_exit(WorkerResponse::ValidationFailed { message: error }, 0),
    };

    let cache = match DCodeModuleCache::new() {
        Ok(cache) => cache,
        Err(error) => {
            respond_and_exit(
                WorkerResponse::ResourceLimit { message: format!("dcode worker failed to build engine: {error}") },
                1,
            );
        }
    };

    // This is the call that can crash natively on this platform under
    // certain fuel/epoch conditions (see the module doc comment) -- if it
    // does, this SHORT-LIVED process exits abnormally, and the PARENT
    // (`dcode::worker::invoke_isolated`) observes that safely.
    let outcome = cache.invoke(&request.digest, &module_bytes, &request.limits, &input_bytes);
    respond_and_exit(WorkerResponse::from(outcome), 0);
}
