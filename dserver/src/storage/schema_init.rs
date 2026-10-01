//! Shared, process-local SQLite schema-initialization synchronization,
//! used by every `storage::` module whose tables can live in the same
//! physical database file as another module's (currently
//! `datum_event_store` and `datum_alert_store`, which share one file by
//! convention -- see `datum_alert_store`'s own module doc comment).
//!
//! # Why one shared lock, not one per module
//!
//! Phase 123 originally used its own private `Mutex<()>` to serialize
//! `CREATE TABLE`/`CREATE INDEX` DDL against `SQLITE_LOCKED` (a
//! schema-lock conflict distinct from `SQLITE_BUSY`, which
//! `Connection::busy_timeout` does not cover -- discovered empirically by
//! Phase 123's own concurrency tests). Phase 124 initially added its OWN
//! separate, identically-named-but-distinct `Mutex<()>`, reasoning that
//! each module's own `CREATE TABLE IF NOT EXISTS` DDL is independently
//! idempotent regardless of interleaving with the other module's. An
//! independent adversarial review (Phase 124 freeze review, finding F3)
//! correctly identified this reasoning as **incomplete**: idempotency
//! guarantees the END STATE is correct if both DDL batches eventually
//! succeed, but does NOT prevent a transient `SQLITE_LOCKED` failure if
//! two separate connections attempt schema DDL concurrently against the
//! SAME underlying database file -- exactly the same class of race the
//! original single-module fix addressed, now reachable ACROSS modules
//! instead of only across threads within one module. Since
//! `datum_event_store` and `datum_alert_store` share one physical file by
//! design (see `datum_alert_store`'s "Durability" doc section for why),
//! they must share ONE lock, not two independent ones with the same name
//! but no actual coordination.
//!
//! # Multi-process scope (unchanged, not solved here)
//!
//! This `Mutex` is PROCESS-LOCAL. It guarantees safe concurrent schema
//! initialization across THREADS within one DServer process, for every
//! module that uses it. It provides NO protection across independent OS
//! processes racing to initialize schema against a freshly shared
//! database file for the first time -- that remains an explicit, already
//! Phase-123-documented Phase 126 acceptance criterion, not something
//! this refactor changes or attempts to solve.

use std::sync::Mutex;

/// Guards every `CREATE TABLE`/`CREATE INDEX`/schema-defining DDL
/// statement any `storage::` module runs against the shared event/alert
/// database file. See the module doc comment for why this replaced two
/// independent, non-coordinating per-module locks.
pub(crate) static SCHEMA_INIT_LOCK: Mutex<()> = Mutex::new(());
