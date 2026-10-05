//! Phase 138C (Part B): dedicated SQLite storage for `ReconciliationRequestV1`.
//!
//! Deliberately NOT the generic `storage::control_state` domain-JSON blob
//! store (`load_domain_json`/`persist_domain_json`) -- that mechanism is an
//! unconditional upsert with no compare-and-swap, which would make
//! concurrent claim attempts race (last-write-wins). This module mirrors
//! `storage::operational_evidence`'s own dedicated-table precedent instead,
//! giving genuine SQL-row-level atomicity to the one operation that
//! actually needs it: claiming a request.

use std::{fs, path::Path, thread, time::Duration};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::core::reconciliation_request::{
    ReconciliationRequestStatus, ReconciliationRequestV1, MAX_PENDING_REQUESTS_PER_NODE,
    MAX_RECENT_REQUESTS_PER_PROJECT,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateRequestOutcome {
    Created,
    /// A request with this exact `(project_id, client_request_id)` already
    /// existed -- Part A2/O. The caller is responsible for checking the
    /// existing row's target/mode still matches what was asked for this
    /// time (an incompatible reuse is a caller-level error, not a storage
    /// concern).
    AlreadyExists,
    /// Independent review (Audit 43): the target node already has
    /// `MAX_PENDING_REQUESTS_PER_NODE` requests in a non-terminal
    /// (`pending`/`claimed`) state. No new row is inserted -- an unbounded
    /// request queue is never allowed to grow silently. Never returned for
    /// an idempotent retry of an EXISTING `client_request_id` (that check
    /// runs first and always wins), so a legitimate retry can never be
    /// rejected merely because the node is currently busy.
    NodeQueueFull,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimOutcome {
    Claimed,
    /// No row matched `(request_id, project_id, node_id)` in a claimable
    /// state (already claimed by someone else, already terminal, or
    /// expired) -- Part D1's atomic CAS failure path. The caller re-reads
    /// the request to report the exact reason.
    Conflict,
}

const SCHEMA_INIT_RETRY_ATTEMPTS: u32 = 25;
const SCHEMA_INIT_RETRY_DELAY: Duration = Duration::from_millis(20);

fn open_database(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create SQLite reconciliation-request directory '{}': {error}", parent.display()))?;
    }
    let connection = Connection::open(path)
        .map_err(|error| format!("open SQLite reconciliation-request database '{}': {error}", path.display()))?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| format!("configure SQLite reconciliation-request busy timeout: {error}"))?;

    // Independent review (Audit 7): every call opens a fresh connection and
    // runs this idempotent `CREATE TABLE IF NOT EXISTS`/`PRAGMA` batch
    // (mirroring `storage::operational_evidence`'s own established
    // pattern). On a brand-new database file, `PRAGMA journal_mode=WAL`'s
    // one-time rollback-journal-to-WAL conversion can return "database is
    // locked" immediately when a concurrent connection is also
    // initializing the SAME never-before-seen file, WITHOUT honoring
    // `busy_timeout`'s normal retry-on-SQLITE_BUSY behavior -- proven in
    // practice by this module's own
    // `concurrent_create_with_same_client_request_id_never_errors_and_resolves_to_one_row`
    // test, which failed intermittently with exactly this error before
    // this retry loop existed. Once the file/schema exists (the normal
    // steady-state case), this batch is a fast no-op and the loop exits on
    // its first attempt.
    let mut attempt = 0;
    loop {
        let result = connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA foreign_keys=ON;

             CREATE TABLE IF NOT EXISTS reconciliation_requests (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 reconciliation_request_id TEXT NOT NULL,
                 client_request_id TEXT NOT NULL,
                 reconciliation_batch_id TEXT,
                 project_id TEXT NOT NULL,
                 node_id TEXT NOT NULL,
                 target_dgraph_id TEXT NOT NULL,
                 target_dgraph_revision INTEGER NOT NULL,
                 mode TEXT NOT NULL,
                 requested_by TEXT NOT NULL,
                 created_at_utc TEXT NOT NULL,
                 expires_at_utc TEXT NOT NULL,
                 status TEXT NOT NULL,
                 claim_token TEXT,
                 claimed_at_utc TEXT,
                 claim_expires_at_utc TEXT,
                 linked_attempt_id TEXT,
                 updated_at_utc TEXT NOT NULL,
                 UNIQUE(project_id, reconciliation_request_id),
                 UNIQUE(project_id, client_request_id)
             );

             CREATE INDEX IF NOT EXISTS idx_reconciliation_requests_project_time
                 ON reconciliation_requests(project_id, created_at_utc);
             CREATE INDEX IF NOT EXISTS idx_reconciliation_requests_node_status
                 ON reconciliation_requests(project_id, node_id, status);
             CREATE INDEX IF NOT EXISTS idx_reconciliation_requests_batch
                 ON reconciliation_requests(project_id, reconciliation_batch_id);",
        );
        match result {
            Ok(()) => break,
            Err(error) if attempt < SCHEMA_INIT_RETRY_ATTEMPTS && is_transient_lock_error(&error) => {
                attempt += 1;
                thread::sleep(SCHEMA_INIT_RETRY_DELAY);
            }
            Err(error) => return Err(format!("initialize SQLite reconciliation-request schema: {error}")),
        }
    }
    Ok(connection)
}

fn is_transient_lock_error(error: &rusqlite::Error) -> bool {
    let message = error.to_string();
    message.contains("database is locked") || message.contains("database table is locked")
}

fn row_to_request(row: &rusqlite::Row) -> rusqlite::Result<ReconciliationRequestV1> {
    let status_text: String = row.get("status")?;
    let target_dgraph_revision: i64 = row.get("target_dgraph_revision")?;
    Ok(ReconciliationRequestV1 {
        schema_version: crate::core::reconciliation_request::RECONCILIATION_REQUEST_SCHEMA_VERSION.to_string(),
        reconciliation_request_id: row.get("reconciliation_request_id")?,
        client_request_id: row.get("client_request_id")?,
        reconciliation_batch_id: row.get("reconciliation_batch_id")?,
        project_id: row.get("project_id")?,
        node_id: row.get("node_id")?,
        target_dgraph_id: row.get("target_dgraph_id")?,
        target_dgraph_revision: u64::try_from(target_dgraph_revision).unwrap_or(0),
        mode: row.get("mode")?,
        requested_by: row.get("requested_by")?,
        created_at_utc: row.get("created_at_utc")?,
        expires_at_utc: row.get("expires_at_utc")?,
        status: ReconciliationRequestStatus::parse(&status_text).unwrap_or(ReconciliationRequestStatus::Expired),
        claim_token: row.get("claim_token")?,
        claimed_at_utc: row.get("claimed_at_utc")?,
        claim_expires_at_utc: row.get("claim_expires_at_utc")?,
        linked_attempt_id: row.get("linked_attempt_id")?,
        updated_at_utc: row.get("updated_at_utc")?,
    })
}

const SELECT_COLUMNS: &str = "reconciliation_request_id, client_request_id, reconciliation_batch_id, project_id, node_id,
     target_dgraph_id, target_dgraph_revision, mode, requested_by, created_at_utc, expires_at_utc,
     status, claim_token, claimed_at_utc, claim_expires_at_utc, linked_attempt_id, updated_at_utc";

/// Idempotent creation (Part A2): if `(project_id, client_request_id)`
/// already exists, returns the EXISTING row untouched (never overwrites,
/// never duplicates) -- the caller decides whether the existing row's
/// target/mode matches what was requested this time.
///
/// Independent review (Audit 6/7): the check-then-insert sequence below
/// MUST run inside a `BEGIN IMMEDIATE` transaction, not SQLite's default
/// deferred one. A deferred transaction only acquires SQLite's write lock
/// at the first write statement, so two genuinely concurrent callers can
/// both pass the "not found" `SELECT` before either has written, race past
/// each other, and the loser's `INSERT` then fails with a raw UNIQUE
/// constraint violation -- proven in practice by
/// `concurrent_create_with_same_client_request_id_never_errors_and_resolves_to_one_row`,
/// which failed intermittently (~1 in 4 runs) before this fix. `Immediate`
/// acquires the write lock at `BEGIN`, so the second caller blocks (via
/// `busy_timeout`) until the first fully commits, then its own `SELECT`
/// truthfully observes the just-inserted row and returns `AlreadyExists`
/// -- never a raw SQLite error surfaced to the caller as a 500.
pub fn create_request(db_path: &Path, request: &ReconciliationRequestV1) -> Result<(CreateRequestOutcome, ReconciliationRequestV1), String> {
    let mut connection = open_database(db_path)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| format!("begin SQLite reconciliation-request create transaction: {error}"))?;

    let existing = transaction
        .query_row(
            &format!("SELECT {SELECT_COLUMNS} FROM reconciliation_requests WHERE project_id = ?1 AND client_request_id = ?2"),
            params![request.project_id, request.client_request_id],
            row_to_request,
        )
        .optional()
        .map_err(|error| format!("check existing SQLite reconciliation-request: {error}"))?;

    if let Some(existing) = existing {
        transaction.commit().map_err(|error| format!("commit SQLite reconciliation-request read transaction: {error}"))?;
        return Ok((CreateRequestOutcome::AlreadyExists, existing));
    }

    // Independent review (Audit 43): bounded inside the SAME `BEGIN
    // IMMEDIATE` transaction as the insert decision above, so the count and
    // the insert are atomic with respect to any other concurrent creator --
    // two racing creators for an already-at-the-bound node can never both
    // slip through.
    let live_count: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM reconciliation_requests WHERE project_id = ?1 AND node_id = ?2 AND status IN ('pending','claimed')",
            params![request.project_id, request.node_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("count SQLite reconciliation-requests for node queue bound: {error}"))?;
    if live_count >= MAX_PENDING_REQUESTS_PER_NODE as i64 {
        transaction.commit().map_err(|error| format!("commit SQLite reconciliation-request read transaction: {error}"))?;
        return Ok((CreateRequestOutcome::NodeQueueFull, request.clone()));
    }

    let target_dgraph_revision = i64::try_from(request.target_dgraph_revision)
        .map_err(|_| "target_dgraph_revision exceeds SQLite INTEGER range".to_string())?;
    transaction
        .execute(
            "INSERT INTO reconciliation_requests (
                reconciliation_request_id, client_request_id, reconciliation_batch_id, project_id, node_id,
                target_dgraph_id, target_dgraph_revision, mode, requested_by, created_at_utc, expires_at_utc,
                status, claim_token, claimed_at_utc, claim_expires_at_utc, linked_attempt_id, updated_at_utc
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,NULL,NULL,NULL,NULL,?13)",
            params![
                request.reconciliation_request_id,
                request.client_request_id,
                request.reconciliation_batch_id,
                request.project_id,
                request.node_id,
                request.target_dgraph_id,
                target_dgraph_revision,
                request.mode,
                request.requested_by,
                request.created_at_utc,
                request.expires_at_utc,
                request.status.as_str(),
                request.updated_at_utc,
            ],
        )
        .map_err(|error| format!("insert SQLite reconciliation-request: {error}"))?;
    transaction.commit().map_err(|error| format!("commit SQLite reconciliation-request create transaction: {error}"))?;
    Ok((CreateRequestOutcome::Created, request.clone()))
}

pub fn get_request(db_path: &Path, project_id: &str, reconciliation_request_id: &str) -> Result<Option<ReconciliationRequestV1>, String> {
    let connection = open_database(db_path)?;
    connection
        .query_row(
            &format!("SELECT {SELECT_COLUMNS} FROM reconciliation_requests WHERE project_id = ?1 AND reconciliation_request_id = ?2"),
            params![project_id, reconciliation_request_id],
            row_to_request,
        )
        .optional()
        .map_err(|error| format!("load SQLite reconciliation-request: {error}"))
}

/// Part E2: candidates for the control-slice `pending_request` signal --
/// stored `status='pending'` only (an abandoned `claimed` row becomes
/// agent-actionable again only once its lease lapses, which the CALLER
/// evaluates via `effective_status`; this query intentionally returns only
/// the unambiguous stored-pending rows plus any already-lapsed claims so
/// the caller can apply `effective_status` uniformly). Bounded by
/// `MAX_PENDING_REQUESTS_PER_NODE`.
pub fn list_claimable_for_node(db_path: &Path, project_id: &str, node_id: &str) -> Result<Vec<ReconciliationRequestV1>, String> {
    let connection = open_database(db_path)?;
    let mut statement = connection
        .prepare(&format!(
            "SELECT {SELECT_COLUMNS} FROM reconciliation_requests
             WHERE project_id = ?1 AND node_id = ?2 AND status IN ('pending','claimed')
             ORDER BY created_at_utc ASC LIMIT ?3"
        ))
        .map_err(|error| format!("prepare SQLite reconciliation-request node query: {error}"))?;
    let rows = statement
        .query_map(params![project_id, node_id, MAX_PENDING_REQUESTS_PER_NODE as i64], row_to_request)
        .map_err(|error| format!("query SQLite reconciliation-requests for node: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|error| format!("read SQLite reconciliation-requests for node: {error}"))
}

pub fn list_recent_for_project(db_path: &Path, project_id: &str, limit: usize) -> Result<Vec<ReconciliationRequestV1>, String> {
    let connection = open_database(db_path)?;
    let bounded_limit = limit.min(MAX_RECENT_REQUESTS_PER_PROJECT).max(1);
    let mut statement = connection
        .prepare(&format!("SELECT {SELECT_COLUMNS} FROM reconciliation_requests WHERE project_id = ?1 ORDER BY created_at_utc DESC LIMIT ?2"))
        .map_err(|error| format!("prepare SQLite reconciliation-request recent query: {error}"))?;
    let rows = statement
        .query_map(params![project_id, bounded_limit as i64], row_to_request)
        .map_err(|error| format!("query SQLite recent reconciliation-requests: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|error| format!("read SQLite recent reconciliation-requests: {error}"))
}

pub fn list_for_batch(db_path: &Path, project_id: &str, reconciliation_batch_id: &str) -> Result<Vec<ReconciliationRequestV1>, String> {
    let connection = open_database(db_path)?;
    let mut statement = connection
        .prepare(&format!(
            "SELECT {SELECT_COLUMNS} FROM reconciliation_requests WHERE project_id = ?1 AND reconciliation_batch_id = ?2 ORDER BY node_id ASC"
        ))
        .map_err(|error| format!("prepare SQLite reconciliation-request batch query: {error}"))?;
    let rows = statement
        .query_map(params![project_id, reconciliation_batch_id], row_to_request)
        .map_err(|error| format!("query SQLite reconciliation-requests for batch: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|error| format!("read SQLite reconciliation-requests for batch: {error}"))
}

/// Part D1: the one real atomic compare-and-swap in this module. A single
/// `UPDATE ... WHERE` matches EITHER a fresh `pending` row OR an abandoned
/// `claimed` row whose lease has already lapsed (Part M's reclaim-after-
/// crash path) -- both expressed in SQL so two concurrent claimants
/// produce exactly one `Claimed` and the rest `Conflict`, never both
/// succeeding. The request's own TTL is checked too: an expired request
/// can never be claimed regardless of its stored status.
#[allow(clippy::too_many_arguments)]
pub fn claim_request(
    db_path: &Path,
    project_id: &str,
    node_id: &str,
    reconciliation_request_id: &str,
    claim_token: &str,
    claimed_at_utc: &str,
    claim_expires_at_utc: &str,
    now_utc: &str,
) -> Result<ClaimOutcome, String> {
    let connection = open_database(db_path)?;
    let affected = connection
        .execute(
            "UPDATE reconciliation_requests
             SET status = 'claimed', claim_token = ?1, claimed_at_utc = ?2, claim_expires_at_utc = ?3, updated_at_utc = ?2
             WHERE project_id = ?4 AND node_id = ?5 AND reconciliation_request_id = ?6
               AND expires_at_utc > ?7
               AND (status = 'pending' OR (status = 'claimed' AND claim_expires_at_utc IS NOT NULL AND claim_expires_at_utc <= ?7))",
            params![claim_token, claimed_at_utc, claim_expires_at_utc, project_id, node_id, reconciliation_request_id, now_utc],
        )
        .map_err(|error| format!("claim SQLite reconciliation-request: {error}"))?;
    Ok(if affected == 1 { ClaimOutcome::Claimed } else { ClaimOutcome::Conflict })
}

/// Part D4/K: marks a request `stale` -- best-effort, idempotent (a
/// concurrent duplicate mark is harmless), never used as a mutual-
/// exclusion mechanism (only `claim_request`'s own atomic `UPDATE` is).
/// Only a currently non-terminal row is affected.
pub fn mark_stale(db_path: &Path, project_id: &str, reconciliation_request_id: &str, now_utc: &str) -> Result<(), String> {
    let connection = open_database(db_path)?;
    connection
        .execute(
            "UPDATE reconciliation_requests SET status = 'stale', updated_at_utc = ?1
             WHERE project_id = ?2 AND reconciliation_request_id = ?3 AND status IN ('pending','claimed')",
            params![now_utc, project_id, reconciliation_request_id],
        )
        .map_err(|error| format!("mark SQLite reconciliation-request stale: {error}"))?;
    Ok(())
}

/// Part G: a request transitions to `Attempted` and records its
/// `linked_attempt_id` the first time valid evidence links an attempt to
/// it. Idempotent-safe: only a `claimed` row transitions (an already-
/// `attempted` row linking a SECOND attempt id -- Part G1's reclaim case --
/// updates `linked_attempt_id` to the latest without re-deriving status).
pub fn link_attempt(
    db_path: &Path,
    project_id: &str,
    reconciliation_request_id: &str,
    reconciliation_attempt_id: &str,
    now_utc: &str,
) -> Result<(), String> {
    let connection = open_database(db_path)?;
    connection
        .execute(
            "UPDATE reconciliation_requests
             SET status = 'attempted', linked_attempt_id = ?1, updated_at_utc = ?2
             WHERE project_id = ?3 AND reconciliation_request_id = ?4 AND status IN ('claimed','attempted')",
            params![reconciliation_attempt_id, now_utc, project_id, reconciliation_request_id],
        )
        .map_err(|error| format!("link SQLite reconciliation-request to attempt: {error}"))?;
    Ok(())
}

pub fn request_count(db_path: &Path, project_id: &str) -> Result<u64, String> {
    let connection = open_database(db_path)?;
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM reconciliation_requests WHERE project_id = ?1",
            params![project_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("count SQLite reconciliation-requests: {error}"))?;
    u64::try_from(count).map_err(|_| "SQLite reconciliation-request count returned negative".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::reconciliation_request::{RECONCILIATION_REQUEST_SCHEMA_VERSION, REQUEST_MODE_EXECUTE};
    use uuid::Uuid;

    fn temp_db_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("datum-reconciliation-request-test-{}.db", Uuid::new_v4()))
    }

    fn request(project_id: &str, node_id: &str, request_id: &str, client_request_id: &str) -> ReconciliationRequestV1 {
        ReconciliationRequestV1 {
            schema_version: RECONCILIATION_REQUEST_SCHEMA_VERSION.to_string(),
            reconciliation_request_id: request_id.to_string(),
            client_request_id: client_request_id.to_string(),
            reconciliation_batch_id: None,
            project_id: project_id.to_string(),
            node_id: node_id.to_string(),
            target_dgraph_id: "dgraph:app-1".to_string(),
            target_dgraph_revision: 4,
            mode: REQUEST_MODE_EXECUTE.to_string(),
            requested_by: "operator".to_string(),
            created_at_utc: "2026-10-05T12:00:00Z".to_string(),
            expires_at_utc: "2026-10-05T12:15:00Z".to_string(),
            status: ReconciliationRequestStatus::Pending,
            claim_token: None,
            claimed_at_utc: None,
            claim_expires_at_utc: None,
            linked_attempt_id: None,
            updated_at_utc: "2026-10-05T12:00:00Z".to_string(),
        }
    }

    // Phase 138C independent review (Audit 43): the node queue must be
    // bounded at creation time, not just truncated at read time -- fail
    // explicitly rather than silently growing an unbounded pending queue.
    #[test]
    fn node_queue_full_is_rejected_explicitly_once_the_bound_is_reached() {
        let db = temp_db_path();
        for i in 0..MAX_PENDING_REQUESTS_PER_NODE {
            let (outcome, _) = create_request(
                &db,
                &request("project-a", "node-a", &format!("reconciliation-request:{i}"), &format!("client:{i}")),
            )
            .unwrap();
            assert_eq!(outcome, CreateRequestOutcome::Created);
        }
        let (outcome, _) = create_request(
            &db,
            &request("project-a", "node-a", "reconciliation-request:overflow", "client:overflow"),
        )
        .unwrap();
        assert_eq!(outcome, CreateRequestOutcome::NodeQueueFull, "the bound must be enforced, never silently exceeded");
        assert_eq!(request_count(&db, "project-a").unwrap() as usize, MAX_PENDING_REQUESTS_PER_NODE, "no row may be inserted for a rejected creation");
    }

    // Phase 138C independent review (Audit 43): a retry of an ALREADY
    // existing `client_request_id` must never be rejected by the queue
    // bound -- idempotent retries always win over "node is busy".
    #[test]
    fn node_queue_full_never_blocks_an_idempotent_retry_of_an_existing_request() {
        let db = temp_db_path();
        for i in 0..MAX_PENDING_REQUESTS_PER_NODE {
            create_request(
                &db,
                &request("project-a", "node-a", &format!("reconciliation-request:{i}"), &format!("client:{i}")),
            )
            .unwrap();
        }
        let (outcome, stored) = create_request(
            &db,
            &request("project-a", "node-a", "reconciliation-request:0", "client:0"),
        )
        .unwrap();
        assert_eq!(outcome, CreateRequestOutcome::AlreadyExists);
        assert_eq!(stored.reconciliation_request_id, "reconciliation-request:0");
    }

    // Phase 138C independent review (Audit 43): the bound is per
    // (project_id, node_id) -- a different node (or project) in the same
    // project must never be blocked by another node's full queue.
    #[test]
    fn node_queue_bound_is_scoped_per_node() {
        let db = temp_db_path();
        for i in 0..MAX_PENDING_REQUESTS_PER_NODE {
            create_request(
                &db,
                &request("project-a", "node-a", &format!("reconciliation-request:{i}"), &format!("client:{i}")),
            )
            .unwrap();
        }
        let (outcome, _) = create_request(
            &db,
            &request("project-a", "node-b", "reconciliation-request:other-node", "client:other-node"),
        )
        .unwrap();
        assert_eq!(outcome, CreateRequestOutcome::Created, "a different node's queue must be independent");
    }

    #[test]
    fn create_then_get_round_trips_exactly() {
        let db = temp_db_path();
        let r = request("project-a", "node-a", "reconciliation-request:1", "client:1");
        let (outcome, created) = create_request(&db, &r).unwrap();
        assert_eq!(outcome, CreateRequestOutcome::Created);
        assert_eq!(created, r);
        let loaded = get_request(&db, "project-a", "reconciliation-request:1").unwrap().unwrap();
        assert_eq!(loaded, r);
    }

    #[test]
    fn create_with_same_client_request_id_is_idempotent_never_duplicates() {
        let db = temp_db_path();
        let r1 = request("project-a", "node-a", "reconciliation-request:1", "client:1");
        let mut r2 = request("project-a", "node-a", "reconciliation-request:2", "client:1");
        r2.target_dgraph_revision = 999; // caller would detect this mismatch; storage just returns the original
        let (first_outcome, _) = create_request(&db, &r1).unwrap();
        let (second_outcome, returned) = create_request(&db, &r2).unwrap();
        assert_eq!(first_outcome, CreateRequestOutcome::Created);
        assert_eq!(second_outcome, CreateRequestOutcome::AlreadyExists);
        assert_eq!(returned.reconciliation_request_id, "reconciliation-request:1", "the ORIGINAL request must be returned, never a second row");
        assert_eq!(request_count(&db, "project-a").unwrap(), 1);
    }

    // Phase 138C independent review (Audit 7): two REAL OS threads, each
    // with their own SQLite connection (not a pure-function simulation),
    // racing to create a request with the IDENTICAL `client_request_id`.
    // Exactly one logical request must result, and the loser must receive
    // the idempotent `AlreadyExists` outcome -- never a raw SQLite
    // constraint-violation error surfaced as a 500.
    #[test]
    fn concurrent_create_with_same_client_request_id_never_errors_and_resolves_to_one_row() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let db = temp_db_path();
        let barrier = Arc::new(Barrier::new(2));
        let db_a = db.clone();
        let db_b = db.clone();
        let barrier_a = Arc::clone(&barrier);
        let barrier_b = Arc::clone(&barrier);

        let handle_a = thread::spawn(move || {
            barrier_a.wait();
            create_request(&db_a, &request("project-a", "node-a", "reconciliation-request:1", "client:race"))
        });
        let handle_b = thread::spawn(move || {
            barrier_b.wait();
            create_request(&db_b, &request("project-a", "node-a", "reconciliation-request:2", "client:race"))
        });

        let result_a = handle_a.join().unwrap();
        let result_b = handle_b.join().unwrap();

        assert!(result_a.is_ok(), "concurrent create must never surface a raw SQLite error: {result_a:?}");
        assert!(result_b.is_ok(), "concurrent create must never surface a raw SQLite error: {result_b:?}");

        let outcomes = [result_a.unwrap().0, result_b.unwrap().0];
        let created_count = outcomes.iter().filter(|outcome| **outcome == CreateRequestOutcome::Created).count();
        let already_exists_count = outcomes.iter().filter(|outcome| **outcome == CreateRequestOutcome::AlreadyExists).count();
        assert_eq!(created_count, 1, "exactly one logical request must result from the race");
        assert_eq!(already_exists_count, 1, "the loser must receive the idempotent AlreadyExists outcome");
        assert_eq!(request_count(&db, "project-a").unwrap(), 1, "no duplicate row may ever exist");
    }

    #[test]
    fn two_concurrent_claimants_exactly_one_wins() {
        let db = temp_db_path();
        let r = request("project-a", "node-a", "reconciliation-request:1", "client:1");
        create_request(&db, &r).unwrap();

        let first = claim_request(&db, "project-a", "node-a", "reconciliation-request:1", "token-a", "2026-10-05T12:01:00Z", "2026-10-05T12:03:00Z", "2026-10-05T12:01:00Z").unwrap();
        let second = claim_request(&db, "project-a", "node-a", "reconciliation-request:1", "token-b", "2026-10-05T12:01:01Z", "2026-10-05T12:03:01Z", "2026-10-05T12:01:01Z").unwrap();

        assert_eq!(first, ClaimOutcome::Claimed);
        assert_eq!(second, ClaimOutcome::Conflict, "a second claimant must never also succeed");
        let loaded = get_request(&db, "project-a", "reconciliation-request:1").unwrap().unwrap();
        assert_eq!(loaded.claim_token.as_deref(), Some("token-a"), "the FIRST claimant's token must be the one persisted");
    }

    #[test]
    fn claim_requires_exact_project_and_node_scope() {
        let db = temp_db_path();
        let r = request("project-a", "node-a", "reconciliation-request:1", "client:1");
        create_request(&db, &r).unwrap();

        let wrong_node = claim_request(&db, "project-a", "node-b", "reconciliation-request:1", "token-a", "2026-10-05T12:01:00Z", "2026-10-05T12:03:00Z", "2026-10-05T12:01:00Z").unwrap();
        let wrong_project = claim_request(&db, "project-b", "node-a", "reconciliation-request:1", "token-a", "2026-10-05T12:01:00Z", "2026-10-05T12:03:00Z", "2026-10-05T12:01:00Z").unwrap();
        assert_eq!(wrong_node, ClaimOutcome::Conflict, "a request for node-a must never be claimable via node-b");
        assert_eq!(wrong_project, ClaimOutcome::Conflict);
    }

    #[test]
    fn claim_fails_once_the_request_itself_has_expired() {
        let db = temp_db_path();
        let mut r = request("project-a", "node-a", "reconciliation-request:1", "client:1");
        r.expires_at_utc = "2026-10-05T12:00:30Z".to_string();
        create_request(&db, &r).unwrap();
        let outcome = claim_request(&db, "project-a", "node-a", "reconciliation-request:1", "token-a", "2026-10-05T12:01:00Z", "2026-10-05T12:03:00Z", "2026-10-05T12:01:00Z").unwrap();
        assert_eq!(outcome, ClaimOutcome::Conflict);
    }

    #[test]
    fn abandoned_claim_past_its_lease_can_be_reclaimed_by_a_different_token() {
        let db = temp_db_path();
        let r = request("project-a", "node-a", "reconciliation-request:1", "client:1");
        create_request(&db, &r).unwrap();
        let first = claim_request(&db, "project-a", "node-a", "reconciliation-request:1", "token-a", "2026-10-05T12:01:00Z", "2026-10-05T12:02:00Z", "2026-10-05T12:01:00Z").unwrap();
        assert_eq!(first, ClaimOutcome::Claimed);

        // Still within lease: a second claimant must still lose.
        let too_soon = claim_request(&db, "project-a", "node-a", "reconciliation-request:1", "token-b", "2026-10-05T12:01:30Z", "2026-10-05T12:03:30Z", "2026-10-05T12:01:30Z").unwrap();
        assert_eq!(too_soon, ClaimOutcome::Conflict);

        // Past the original claim's lease: reclaim succeeds (crash recovery, Part M).
        let reclaim = claim_request(&db, "project-a", "node-a", "reconciliation-request:1", "token-c", "2026-10-05T12:05:00Z", "2026-10-05T12:07:00Z", "2026-10-05T12:05:00Z").unwrap();
        assert_eq!(reclaim, ClaimOutcome::Claimed);
        let loaded = get_request(&db, "project-a", "reconciliation-request:1").unwrap().unwrap();
        assert_eq!(loaded.claim_token.as_deref(), Some("token-c"));
    }

    #[test]
    fn mark_stale_only_affects_non_terminal_rows() {
        let db = temp_db_path();
        let r = request("project-a", "node-a", "reconciliation-request:1", "client:1");
        create_request(&db, &r).unwrap();
        mark_stale(&db, "project-a", "reconciliation-request:1", "2026-10-05T12:01:00Z").unwrap();
        let loaded = get_request(&db, "project-a", "reconciliation-request:1").unwrap().unwrap();
        assert_eq!(loaded.status, ReconciliationRequestStatus::Stale);

        // A terminal row is never resurrected by mark_stale being called again.
        mark_stale(&db, "project-a", "reconciliation-request:1", "2026-10-05T12:02:00Z").unwrap();
        let loaded_again = get_request(&db, "project-a", "reconciliation-request:1").unwrap().unwrap();
        assert_eq!(loaded_again.status, ReconciliationRequestStatus::Stale);
    }

    #[test]
    fn link_attempt_transitions_to_attempted_and_records_the_attempt_id() {
        let db = temp_db_path();
        let r = request("project-a", "node-a", "reconciliation-request:1", "client:1");
        create_request(&db, &r).unwrap();
        claim_request(&db, "project-a", "node-a", "reconciliation-request:1", "token-a", "2026-10-05T12:01:00Z", "2026-10-05T12:03:00Z", "2026-10-05T12:01:00Z").unwrap();
        link_attempt(&db, "project-a", "reconciliation-request:1", "reconciliation-attempt:xyz", "2026-10-05T12:01:30Z").unwrap();
        let loaded = get_request(&db, "project-a", "reconciliation-request:1").unwrap().unwrap();
        assert_eq!(loaded.status, ReconciliationRequestStatus::Attempted);
        assert_eq!(loaded.linked_attempt_id.as_deref(), Some("reconciliation-attempt:xyz"));
    }

    #[test]
    fn project_isolation_list_recent_never_crosses_projects() {
        let db = temp_db_path();
        create_request(&db, &request("project-a", "node-a", "reconciliation-request:1", "client:1")).unwrap();
        create_request(&db, &request("project-b", "node-a", "reconciliation-request:2", "client:1")).unwrap();
        let a = list_recent_for_project(&db, "project-a", 10).unwrap();
        let b = list_recent_for_project(&db, "project-b", 10).unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        assert_eq!(a[0].reconciliation_request_id, "reconciliation-request:1");
        assert_eq!(b[0].reconciliation_request_id, "reconciliation-request:2");
    }

    #[test]
    fn list_claimable_for_node_is_node_scoped() {
        let db = temp_db_path();
        create_request(&db, &request("project-a", "node-a", "reconciliation-request:1", "client:1")).unwrap();
        create_request(&db, &request("project-a", "node-b", "reconciliation-request:2", "client:2")).unwrap();
        let for_a = list_claimable_for_node(&db, "project-a", "node-a").unwrap();
        assert_eq!(for_a.len(), 1);
        assert_eq!(for_a[0].reconciliation_request_id, "reconciliation-request:1");
    }

    #[test]
    fn batch_lookup_returns_only_members_of_that_batch() {
        let db = temp_db_path();
        let mut r1 = request("project-a", "node-a", "reconciliation-request:1", "client:1");
        r1.reconciliation_batch_id = Some("reconciliation-batch:1".to_string());
        let mut r2 = request("project-a", "node-b", "reconciliation-request:2", "client:2");
        r2.reconciliation_batch_id = Some("reconciliation-batch:1".to_string());
        let r3 = request("project-a", "node-c", "reconciliation-request:3", "client:3");
        create_request(&db, &r1).unwrap();
        create_request(&db, &r2).unwrap();
        create_request(&db, &r3).unwrap();
        let members = list_for_batch(&db, "project-a", "reconciliation-batch:1").unwrap();
        assert_eq!(members.len(), 2);
    }

    #[test]
    fn persists_across_a_fresh_connection_restart_simulation() {
        let db = temp_db_path();
        let r = request("project-a", "node-a", "reconciliation-request:1", "client:1");
        create_request(&db, &r).unwrap();
        claim_request(&db, "project-a", "node-a", "reconciliation-request:1", "token-a", "2026-10-05T12:01:00Z", "2026-10-05T12:03:00Z", "2026-10-05T12:01:00Z").unwrap();
        // A fresh `get_request` call opens its OWN new SQLite connection
        // (`open_database` is called internally each time, exactly like
        // `storage::operational_evidence`'s own established pattern) --
        // this is the same proof-of-restart-survival shape that module's
        // own test uses.
        let loaded = get_request(&db, "project-a", "reconciliation-request:1").unwrap().unwrap();
        assert_eq!(loaded.status, ReconciliationRequestStatus::Claimed);
        assert_eq!(loaded.claim_token.as_deref(), Some("token-a"));
    }
}
