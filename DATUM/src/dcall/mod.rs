//! Canonical D-Call realization v0.1 (ADR-0022). See `delivery::dispatch_validated_emit`
//! for the D-Engine-facing library seam; see `listener` for the cross-D-Node
//! HTTP delivery endpoint; see `grant` for the wire-mirrored delivery-grant
//! type; see `evidence` for the sanitized delivery-evidence record; see
//! `client` for the DServer control-plane HTTP client.

pub mod client;
pub mod delivery;
pub mod evidence;
pub mod grant;
pub mod listener;
pub mod mqtt;
