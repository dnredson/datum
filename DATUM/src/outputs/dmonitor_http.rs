//! Production DMonitor HTTP output (Phase 114B2, ADR-0024). Sends the exact
//! `CanonicalDMonitorObservationV1` body to the Phase-114B1 production
//! ingestion route, `POST /api/v1/dmonitor/observations/:project_id`. This
//! is the SAME wire format DServer's `api::dmonitor::post_dmonitor_observation`
//! deserializes -- no second DMonitor ingestion format is introduced here.
//!
//! This is read-only EVIDENCE publication: a failure here must never trigger
//! remediation or host mutation (section 23), and never falsely reports
//! success. [`DMonitorHttpOutput::publish_all`] defines and documents the
//! deterministic, BOUNDED partial-cycle behavior: observations are published
//! in sequence order, and publication stops at the first failure -- already
//! -accepted observations before that point stay accepted, the failing
//! observation is reported as failed, and every observation after it in
//! that batch is reported as not attempted. There is no retry loop; the
//! NEXT collection cycle allocates entirely new sequences (Phase 114B1.1
//! already establishes that a stream's sequence counter never reuses or
//! rewinds an allocated number for a different observation) rather than
//! re-attempting the ones this cycle left unpublished.
//!
//! # Why this module takes `serde_json::Value`, not `CanonicalDMonitorObservationV1`
//!
//! `src/outputs/` is compiled as part of BOTH the `datum` library crate
//! (`src/lib.rs`, which declares `pub mod agent;`) AND the separate legacy
//! `datum` binary (`src/main.rs`, which does NOT declare `agent` at all).
//! Referencing `crate::agent::canonical_dmonitor::CanonicalDMonitorObservationV1`
//! directly from a file under `src/outputs/` would fail to compile under
//! `main.rs`'s narrower module tree. This module therefore stays agent-
//! agnostic: callers (which run only as part of the library-linked
//! `smartsentinel-observe` binary, where `agent` genuinely exists) serialize
//! the canonical observation to a `serde_json::Value` themselves
//! (`serde_json::to_value(&observation)?`) and pass its `observation_id`
//! alongside for reporting -- the wire body sent over HTTP is byte-identical
//! either way.

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DMonitorPublishOutcome {
    Accepted,
    IdempotentReplay,
}

pub struct DMonitorHttpOutput {
    client: Client,
    /// The DServer base URL (e.g. `http://127.0.0.1:8080`), WITHOUT the
    /// `/api/v1/dmonitor/observations/...` suffix -- that suffix, including
    /// the project scope path segment, is built by [`Self::url`].
    base_url: String,
    project_id: String,
}

impl DMonitorHttpOutput {
    pub fn new(dserver_base_url: String, project_id: String, timeout_s: u64) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_s))
            .build()
            .context("build blocking reqwest DMonitor client")?;
        Ok(Self {
            client,
            base_url: dserver_base_url.trim().trim_end_matches('/').to_string(),
            project_id,
        })
    }

    /// `{base_url}/api/v1/dmonitor/observations/{project_id}` -- the exact
    /// Phase-114B1 production route, project scope in the path (this
    /// project's established convention, matching `/api/v1/diot/
    /// runtime-bindings/:project_id/...` and `/api/v1/dcall/calls/
    /// :project_id/...`; DServer has no header-based project scope
    /// mechanism, so this deliberately does NOT use an `X-Datum-Project`
    /// header the way the legacy snapshot/telemetry outputs do).
    pub fn url(&self) -> String {
        format!(
            "{}/api/v1/dmonitor/observations/{}",
            self.base_url, self.project_id
        )
    }

    /// Publishes exactly ONE canonical observation, already serialized to a
    /// `serde_json::Value` (see the module doc for why). `observation_id` is
    /// used only for error messages/reporting, never sent as a separate
    /// field -- it must already be present inside `observation_json` itself.
    /// A 2xx response is success (either `accepted` or `idempotent_replay`,
    /// distinguished by the response body's `outcome` field); any non-2xx
    /// status, transport error, or timeout is surfaced as an explicit `Err`
    /// -- never silently treated as success.
    pub fn publish(
        &self,
        observation_id: &str,
        observation_json: &Value,
    ) -> Result<DMonitorPublishOutcome> {
        let payload = serde_json::to_string(observation_json)
            .context("serialize canonical DMonitor observation")?;

        let response = self
            .client
            .post(self.url())
            .header("Content-Type", "application/json")
            .body(payload)
            .send()
            .with_context(|| format!("send DMonitor observation '{observation_id}' to dserver"))?;

        let status = response.status();
        let body = response.text().unwrap_or_default();

        if !status.is_success() {
            return Err(anyhow!(
                "DMonitor publish failed for observation_id '{observation_id}': status={status} body={body}"
            ));
        }

        let parsed: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        let outcome = parsed.get("outcome").and_then(Value::as_str).unwrap_or("accepted");
        Ok(match outcome {
            "idempotent_replay" => DMonitorPublishOutcome::IdempotentReplay,
            _ => DMonitorPublishOutcome::Accepted,
        })
    }

    /// Publishes `observations` (`(observation_id, canonical JSON)` pairs,
    /// in order), stopping at the first failure (section 23's documented,
    /// bounded partial-cycle behavior -- see the module doc). Never panics,
    /// never retries internally.
    pub fn publish_all(&self, observations: &[(String, Value)]) -> DMonitorPublicationReport {
        let mut report = DMonitorPublicationReport::default();
        let mut iter = observations.iter();

        for (observation_id, observation_json) in iter.by_ref() {
            match self.publish(observation_id, observation_json) {
                Ok(DMonitorPublishOutcome::Accepted) => report.accepted.push(observation_id.clone()),
                Ok(DMonitorPublishOutcome::IdempotentReplay) => {
                    report.idempotent_replay.push(observation_id.clone())
                }
                Err(error) => {
                    report.failed = Some((observation_id.clone(), error.to_string()));
                    break;
                }
            }
        }

        report.not_attempted = iter.map(|(observation_id, _)| observation_id.clone()).collect();
        report
    }
}

/// Deterministic, bounded outcome of one [`DMonitorHttpOutput::publish_all`]
/// batch. `accepted.len() + idempotent_replay.len() + failed.is_some() as
/// usize + not_attempted.len() == observations.len()` always holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DMonitorPublicationReport {
    pub accepted: Vec<String>,
    pub idempotent_replay: Vec<String>,
    /// `(observation_id, error message)` of the FIRST failure in this batch,
    /// if any.
    pub failed: Option<(String, String)>,
    pub not_attempted: Vec<String>,
}

impl DMonitorPublicationReport {
    pub fn is_fully_successful(&self) -> bool {
        self.failed.is_none() && self.not_attempted.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration as StdDuration;

    /// A minimal, local (no external mocking crate), raw-socket mock DServer,
    /// mirroring the established `HttpObserver` pattern already used by
    /// `dcall::delivery::tests::endpoint_authority`. Accepts requests one at
    /// a time on a background thread, records each request line + body, and
    /// replies with a caller-configured status/body -- or, for the timeout
    /// test, never replies at all.
    struct MockDServer {
        url: String,
        requests: Arc<Mutex<Vec<(String, String)>>>, // (request_line, body)
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    enum MockResponse {
        Status(u16, String),
        NeverReply,
    }

    impl MockDServer {
        fn start(response: MockResponse) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            listener.set_nonblocking(true).unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let captured = requests.clone();
            let stopping = stop.clone();

            let thread = std::thread::spawn(move || {
                while !stopping.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream.set_nonblocking(false).unwrap();
                            stream
                                .set_read_timeout(Some(StdDuration::from_secs(5)))
                                .unwrap();
                            let mut bytes = Vec::new();
                            let mut buffer = [0u8; 4096];
                            let mut request_line = String::new();
                            loop {
                                let count = match stream.read(&mut buffer) {
                                    Ok(0) => break,
                                    Ok(count) => count,
                                    Err(_) => break,
                                };
                                bytes.extend_from_slice(&buffer[..count]);
                                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                                    let headers = String::from_utf8_lossy(&bytes[..end]).to_string();
                                    request_line = headers.lines().next().unwrap_or("").to_string();
                                    let length = headers
                                        .lines()
                                        .find_map(|line| {
                                            let (name, value) = line.split_once(':')?;
                                            name.eq_ignore_ascii_case("content-length")
                                                .then(|| value.trim().parse::<usize>().unwrap_or(0))
                                        })
                                        .unwrap_or(0);
                                    if bytes.len() >= end + 4 + length {
                                        break;
                                    }
                                }
                            }
                            let body_start = bytes
                                .windows(4)
                                .position(|part| part == b"\r\n\r\n")
                                .map(|position| position + 4)
                                .unwrap_or(bytes.len());
                            let body = String::from_utf8_lossy(&bytes[body_start..]).to_string();
                            captured.lock().unwrap().push((request_line, body));

                            match &response {
                                MockResponse::Status(status, response_body) => {
                                    let reason = match status {
                                        200 => "OK",
                                        202 => "Accepted",
                                        409 => "Conflict",
                                        422 => "Unprocessable Entity",
                                        500 => "Internal Server Error",
                                        _ => "Status",
                                    };
                                    let _ = write!(
                                        stream,
                                        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                                        response_body.len()
                                    );
                                }
                                MockResponse::NeverReply => {
                                    // Long enough to outlast the test's short
                                    // client timeout (so the client genuinely
                                    // times out rather than racing a fast
                                    // reply), short enough that `Drop`'s
                                    // `thread.join()` doesn't stall the suite.
                                    std::thread::sleep(StdDuration::from_secs(2));
                                }
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(StdDuration::from_millis(2));
                        }
                        Err(_) => break,
                    }
                }
            });

            Self {
                url,
                requests,
                stop,
                thread: Some(thread),
            }
        }

        fn requests(&self) -> Vec<(String, String)> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl Drop for MockDServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn sample_observation_json(observation_id: &str) -> Value {
        serde_json::json!({
            "schema": "datum.dmonitor-observation/1",
            "observation_id": observation_id,
            "node_id": "fog-01",
            "monitor_instance_id": "dmonitor-instance:fog-01:test",
            "sequence": 1,
            "observed_at_utc": "2026-09-25T00:00:00Z",
            "collector": "smartsentinel_host_telemetry",
            "subject": {"kind": "dnode", "node_id": "fog-01"},
            "health": "unknown",
            "facts": {}
        })
    }

    #[test]
    fn publish_uses_the_correct_production_route_and_project_scope() {
        let server = MockDServer::start(MockResponse::Status(
            200,
            r#"{"outcome":"accepted"}"#.to_string(),
        ));
        let output = DMonitorHttpOutput::new(server.url.clone(), "proj:lab".to_string(), 5).unwrap();
        assert_eq!(output.url(), format!("{}/api/v1/dmonitor/observations/proj:lab", server.url));

        output.publish("obs-1", &sample_observation_json("obs-1")).unwrap();
        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0].0.starts_with("POST /api/v1/dmonitor/observations/proj:lab"),
            "unexpected request line: {}",
            requests[0].0
        );
    }

    #[test]
    fn publish_sends_the_exact_canonical_observation_body() {
        let server = MockDServer::start(MockResponse::Status(
            200,
            r#"{"outcome":"accepted"}"#.to_string(),
        ));
        let output = DMonitorHttpOutput::new(server.url.clone(), "proj:lab".to_string(), 5).unwrap();
        let observation = sample_observation_json("obs-1");
        output.publish("obs-1", &observation).unwrap();

        let requests = server.requests();
        let sent: Value = serde_json::from_str(&requests[0].1).unwrap();
        assert_eq!(sent, observation);
    }

    #[test]
    fn publish_returns_accepted_outcome() {
        let server = MockDServer::start(MockResponse::Status(
            200,
            r#"{"outcome":"accepted"}"#.to_string(),
        ));
        let output = DMonitorHttpOutput::new(server.url.clone(), "proj:lab".to_string(), 5).unwrap();
        let outcome = output.publish("obs-1", &sample_observation_json("obs-1")).unwrap();
        assert_eq!(outcome, DMonitorPublishOutcome::Accepted);
    }

    #[test]
    fn publish_returns_idempotent_replay_outcome() {
        let server = MockDServer::start(MockResponse::Status(
            200,
            r#"{"outcome":"idempotent_replay"}"#.to_string(),
        ));
        let output = DMonitorHttpOutput::new(server.url.clone(), "proj:lab".to_string(), 5).unwrap();
        let outcome = output.publish("obs-1", &sample_observation_json("obs-1")).unwrap();
        assert_eq!(outcome, DMonitorPublishOutcome::IdempotentReplay);
    }

    #[test]
    fn non_2xx_status_surfaces_as_explicit_error() {
        let server = MockDServer::start(MockResponse::Status(
            409,
            r#"{"error":"dmonitor_sequence_conflict"}"#.to_string(),
        ));
        let output = DMonitorHttpOutput::new(server.url.clone(), "proj:lab".to_string(), 5).unwrap();
        let error = output.publish("obs-1", &sample_observation_json("obs-1")).unwrap_err();
        assert!(error.to_string().contains("409"));
        assert!(error.to_string().contains("dmonitor_sequence_conflict"));
    }

    #[test]
    fn timeout_surfaces_as_explicit_error() {
        let server = MockDServer::start(MockResponse::NeverReply);
        let output = DMonitorHttpOutput::new(server.url.clone(), "proj:lab".to_string(), 1).unwrap();
        let error = output.publish("obs-1", &sample_observation_json("obs-1")).unwrap_err();
        assert!(!error.to_string().is_empty());
        assert!(error.to_string().contains("obs-1"));
    }

    #[test]
    fn publish_all_stops_at_first_failure_and_reports_bounded_outcome() {
        // Sequenced responses: request 1 -> accepted, request 2 -> 500,
        // (request 3 must never be sent).
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let seen = Arc::new(Mutex::new(0usize));
        let stop = Arc::new(AtomicBool::new(false));
        let seen_clone = seen.clone();
        let stopping = stop.clone();
        let thread = std::thread::spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream.set_read_timeout(Some(StdDuration::from_secs(5))).unwrap();
                        let mut bytes = Vec::new();
                        let mut buffer = [0u8; 4096];
                        loop {
                            let count = match stream.read(&mut buffer) {
                                Ok(0) => break,
                                Ok(count) => count,
                                Err(_) => break,
                            };
                            bytes.extend_from_slice(&buffer[..count]);
                            if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                                let headers = String::from_utf8_lossy(&bytes[..end]).to_string();
                                let length = headers
                                    .lines()
                                    .find_map(|line| {
                                        let (name, value) = line.split_once(':')?;
                                        name.eq_ignore_ascii_case("content-length")
                                            .then(|| value.trim().parse::<usize>().unwrap_or(0))
                                    })
                                    .unwrap_or(0);
                                if bytes.len() >= end + 4 + length {
                                    break;
                                }
                            }
                        }
                        let mut count = seen_clone.lock().unwrap();
                        *count += 1;
                        let (status, reason, body) = if *count == 1 {
                            (200, "OK", r#"{"outcome":"accepted"}"#)
                        } else {
                            (500, "Internal Server Error", r#"{"error":"internal"}"#)
                        };
                        let _ = write!(
                            stream,
                            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(StdDuration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });

        let output = DMonitorHttpOutput::new(url, "proj:lab".to_string(), 5).unwrap();
        let observations = vec![
            ("obs-1".to_string(), sample_observation_json("obs-1")),
            ("obs-2".to_string(), sample_observation_json("obs-2")),
            ("obs-3".to_string(), sample_observation_json("obs-3")),
        ];
        let report = output.publish_all(&observations);

        stop.store(true, Ordering::SeqCst);
        thread.join().unwrap();

        assert_eq!(report.accepted, vec!["obs-1".to_string()]);
        assert_eq!(report.failed.as_ref().map(|(id, _)| id.clone()), Some("obs-2".to_string()));
        assert_eq!(report.not_attempted, vec!["obs-3".to_string()]);
        assert!(!report.is_fully_successful());
        assert_eq!(*seen.lock().unwrap(), 2, "obs-3 must never have been sent");
    }
}
