//! ADR-0003's `datum-dnode/0` ABI -- the ONLY concrete D-Node calling
//! convention this repository's ADRs define. v0.1 D-Code implements this
//! exactly, not an invented alternative.

/// The single D-Node ABI v0.1 supports.
pub const DNODE_ABI_V0: &str = "datum-dnode/0";

/// ADR-0003's required export surface. A module missing any of these fails
/// validation before it is ever instantiated.
pub const DNODE_ABI_V0_REQUIRED_EXPORTS: &[&str] =
    &["memory", "datum_abi_version", "datum_alloc", "datum_dealloc", "datum_handle"];

/// ADR-0003's `datum_abi_version` return value.
pub const DNODE_ABI_V0_VERSION_NUMBER: i32 = 0;

/// v0.1 policy (ADR-0003 Decision, ADR-0021): a D-Code module declares ZERO
/// host imports. This gives a deterministic minimal capability surface, a
/// strong sandbox boundary, and no accidental filesystem/network access --
/// there is no WASI, no clock, no environment, no socket, no filesystem
/// exposed to a D-Code guest in v0.1.
pub fn has_forbidden_imports(module: &wasmtime::Module) -> Option<(String, String)> {
    module
        .imports()
        .next()
        .map(|import| (import.module().to_string(), import.name().to_string()))
}
