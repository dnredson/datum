//! Canonical D-Serv D-Code / WASM v0.1 (ADR-0021, roadmap phase 111).
//!
//! This module is DELIBERATELY a separate top-level module from
//! `wasm_runtime` (the closed Controlled WASM Deployment / "Management
//! WASM" phase, ADR-0005). Nothing here imports from `wasm_runtime`, and
//! nothing in `wasm_runtime` is renamed or reinterpreted as D-Code. Where a
//! low-level mechanism's SEMANTICS genuinely match (SHA-256 content
//! identity, a fuel-accounting Wasmtime `Engine`/`Module`/`Store`, rejecting
//! any module with a host import), the PATTERN is reused -- documented in
//! `dcode-vs-management-wasm-audit.txt` -- but the code is written fresh
//! here so this module never depends on `wasm_runtime` changing underneath
//! it, and so the canonical invocation entrypoint (`invoke::invoke_dcode`)
//! is visibly, structurally separate in source, not merely by convention.
//!
//! D-Code implements ADR-0003's concrete `datum-dnode/0` calling convention
//! exactly (`abi::DNODE_ABI_V0_REQUIRED_EXPORTS`, `datum_handle`'s packed
//! `i64` return) -- this is NOT the Management WASM generic scalar-argument
//! ABI, and it is NOT an invented alternative: ADR-0003 already defines a
//! concrete function-level convention, so this slice implements THAT one.

pub mod abi;
pub mod authorization;
pub mod client;
pub mod dserv_artifact;
pub mod envelope;
pub mod evidence;
pub mod identity;
pub mod invoke;
pub mod local_identity;
pub mod module_store;
pub mod runtime;
pub mod worker;
