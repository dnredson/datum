//! `RuntimeKind` (Stage 108, ADR-0015) -- SmartSentinel agent side.
//!
//! Independently defined mirror of `dserver::core::runtime_kind::RuntimeKind`
//! (same two canonical wire strings, same fail-closed parsing rules). This
//! crate never imports dserver's types (separate Cargo workspaces, by
//! design); parity between the two independent parsers is proven by the
//! `runtime_kind_wire_values_match_dserver` test below (which asserts the
//! exact literal wire strings) plus the cross-crate execution-projection
//! fixture test in `execution_projection.rs`.
//!
//! This is a DISPATCH SEAM, not a `RuntimeDriver` abstraction (Stage 108
//! P1-1 correction) -- see `operational_reconciliation.rs`'s per-service
//! loop, which matches on this enum once, early, before any container-
//! specific observation.

use std::fmt;

pub const RUNTIME_KIND_CONTAINER: &str = "container";
pub const RUNTIME_KIND_NATIVE_PROCESS: &str = "native_process";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeKind {
    Container,
    NativeProcess,
}

impl RuntimeKind {
    /// Exact match against the canonical wire strings only -- no aliases, no
    /// case-insensitivity. This deliberately replaces the pre-Stage-108
    /// `eq_ignore_ascii_case("container")` gates in
    /// `operational_reconciliation.rs`: exact-match is strictly fail-closed
    /// and nothing depended on the looser behaviour.
    pub fn parse(wire: &str) -> Result<Self, UnknownRuntimeKind> {
        match wire {
            RUNTIME_KIND_CONTAINER => Ok(Self::Container),
            RUNTIME_KIND_NATIVE_PROCESS => Ok(Self::NativeProcess),
            other => Err(UnknownRuntimeKind(other.to_string())),
        }
    }

    pub fn wire(self) -> &'static str {
        match self {
            Self::Container => RUNTIME_KIND_CONTAINER,
            Self::NativeProcess => RUNTIME_KIND_NATIVE_PROCESS,
        }
    }
}

impl fmt::Display for RuntimeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.wire())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownRuntimeKind(pub String);

impl fmt::Display for UnknownRuntimeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown runtime kind '{}'; expected '{RUNTIME_KIND_CONTAINER}' or '{RUNTIME_KIND_NATIVE_PROCESS}'", self.0)
    }
}

impl std::error::Error for UnknownRuntimeKind {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_container() {
        assert_eq!(RuntimeKind::parse("container").unwrap(), RuntimeKind::Container);
    }

    #[test]
    fn parses_native_process() {
        assert_eq!(RuntimeKind::parse("native_process").unwrap(), RuntimeKind::NativeProcess);
    }

    #[test]
    fn rejects_unknown_values() {
        assert!(RuntimeKind::parse("vm").is_err());
        assert!(RuntimeKind::parse("").is_err());
        assert!(RuntimeKind::parse("Container").is_err(), "no case-insensitive aliasing");
        assert!(RuntimeKind::parse("process").is_err());
    }

    #[test]
    fn wire_round_trips() {
        assert_eq!(RuntimeKind::parse(RuntimeKind::Container.wire()).unwrap(), RuntimeKind::Container);
        assert_eq!(RuntimeKind::parse(RuntimeKind::NativeProcess.wire()).unwrap(), RuntimeKind::NativeProcess);
    }

    /// Wire-vocabulary parity (Stage 108's decided, non-deferred
    /// cross-crate strategy): both crates must recognise the EXACT same
    /// literal strings. This test pins the literal values this crate
    /// accepts; `dserver::core::runtime_kind`'s own test module pins the
    /// same literals independently -- a divergence in either file's
    /// constants would show up as a failing assertion here or there, not as
    /// a silent runtime mismatch.
    #[test]
    fn runtime_kind_wire_values_match_dserver() {
        assert_eq!(RUNTIME_KIND_CONTAINER, "container");
        assert_eq!(RUNTIME_KIND_NATIVE_PROCESS, "native_process");
    }
}
