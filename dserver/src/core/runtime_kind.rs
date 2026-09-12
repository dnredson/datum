//! `RuntimeKind` (Stage 108, ADR-0015). The canonical wire vocabulary for how
//! a `ServiceArtifact` executes: exactly `"container"` or `"native_process"`.
//!
//! This is a DISPATCH SEAM, not a `RuntimeDriver` abstraction: it exists so
//! `ServiceRuntimeSpec.kind` / `OperationalDesiredService.runtime` /
//! `OperationalArtifactAssignment.runtime_kind` can be parsed and compared
//! consistently everywhere they are read, instead of each call site
//! re-implementing its own string comparison (Stage 108's P0-4 finding: the
//! pre-Stage-108 codebase had at least four independent
//! `== "container"`-style checks across two crates that were never kept in
//! sync with each other by construction).
//!
//! Deliberately NOT shared via a common crate with `DATUM/src/agent/runtime_kind.rs`
//! -- this project's established discipline (ADR-0006/ADR-0011) is independent,
//! hand-written types on each side of the wire, with parity proven by shared
//! fixtures and cross-crate tests, never by type sharing. See
//! `execution_projection`'s cross-crate mapper fixture tests for the same
//! pattern applied to projection digests.
//!
//! Unknown values fail closed: `parse` returns `Err` for anything outside the
//! two canonical wire strings, including case variants and aliases. No
//! silent lowercasing or fallback to `Container`.

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
    /// case-insensitivity. A pre-Stage-108 caller relying on
    /// `eq_ignore_ascii_case` is not preserved: exact-match is strictly
    /// fail-closed and no existing contract requires the looser behaviour.
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
        assert!(RuntimeKind::parse("NATIVE_PROCESS").is_err(), "no case-insensitive aliasing");
        assert!(RuntimeKind::parse("process").is_err());
    }

    #[test]
    fn wire_round_trips() {
        assert_eq!(RuntimeKind::parse(RuntimeKind::Container.wire()).unwrap(), RuntimeKind::Container);
        assert_eq!(RuntimeKind::parse(RuntimeKind::NativeProcess.wire()).unwrap(), RuntimeKind::NativeProcess);
    }

    #[test]
    fn unknown_runtime_kind_message_names_both_canonical_values() {
        let error = RuntimeKind::parse("vm").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("container"));
        assert!(message.contains("native_process"));
        assert!(message.contains("vm"));
    }
}
