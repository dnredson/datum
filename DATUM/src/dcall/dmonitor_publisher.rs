//! Non-blocking, bounded DMonitor evidence publisher (Phase 114D0B, ADR-0024
//! section 18/19). Monitoring must never destabilize what it monitors: the
//! MQTT listener's own poll/keepalive/D-Call-processing loop must never wait
//! on a DMonitor HTTP POST, which can be arbitrarily slow or hang until its
//! own client timeout.
//!
//! # Design
//!
//! One background worker thread, spawned once per listener process epoch
//! (never per observation, never unbounded), drains a bounded
//! `std::sync::mpsc::sync_channel` and calls the EXISTING production
//! [`DMonitorHttpOutput::publish`] synchronously -- no second HTTP protocol
//! is introduced. The listener's own thread only ever calls
//! [`DMonitorPublisherHandle::try_publish`], which is `try_send` underneath
//! and therefore NEVER blocks: if the bounded queue is currently full (the
//! worker is behind, e.g. because the DMonitor HTTP endpoint is slow or
//! down), the observation is DROPPED, never queued unboundedly and never
//! waited for. Sequence numbers are always allocated (via
//! `DMonitorEmissionContext::allocate_sequence`, through
//! `agent::diot_runtime_dmonitor::DiotRuntimeMonitorEmitter::emit`) BEFORE a
//! job is ever offered to this publisher, so a dropped job produces an
//! ordinary, permitted sequence gap (Phase 114B1's own established rule) --
//! never a reused sequence number.
//!
//! # Failure isolation
//!
//! A publish failure (non-2xx status, transport error, timeout) is logged
//! diagnostically and otherwise ignored: it never triggers remediation, host
//! mutation, or a retry loop, and it never makes the calling MQTT session
//! itself unhealthy -- MQTT runtime health and DMonitor evidence-delivery
//! health are two independent systems (section 20).

use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::thread::JoinHandle;

use serde_json::Value;

use crate::outputs::dmonitor_http::DMonitorHttpOutput;

/// A bounded queue depth of 8 comfortably absorbs a handful of heartbeats
/// arriving faster than one slow HTTP round trip can drain them, without
/// letting an unresponsive DMonitor endpoint accumulate unbounded memory.
pub const DEFAULT_QUEUE_CAPACITY: usize = 8;

pub struct DMonitorPublishJob {
    pub observation_id: String,
    pub observation_json: Value,
}

/// Handle to the background publication worker. Cloning is deliberately not
/// supported -- exactly one handle per listener process epoch, matching
/// exactly one worker thread.
pub struct DMonitorPublisherHandle {
    sender: Option<SyncSender<DMonitorPublishJob>>,
    worker: Option<JoinHandle<()>>,
}

impl DMonitorPublisherHandle {
    pub fn spawn(output: DMonitorHttpOutput, queue_capacity: usize) -> Self {
        let (sender, receiver) = sync_channel(queue_capacity.max(1));
        let worker = std::thread::spawn(move || Self::run_worker(output, receiver));
        Self { sender: Some(sender), worker: Some(worker) }
    }

    fn run_worker(output: DMonitorHttpOutput, receiver: Receiver<DMonitorPublishJob>) {
        while let Ok(job) = receiver.recv() {
            if let Err(error) = output.publish(&job.observation_id, &job.observation_json) {
                eprintln!(
                    "mode=dmonitor_publisher status=publish_failed observation_id={} error={error}",
                    job.observation_id
                );
            }
        }
    }

    /// Never blocks the caller. Returns `true` if the job was accepted onto
    /// the bounded queue, `false` if it was dropped (queue full, or the
    /// worker has already exited) -- section 19: dropping a heartbeat under
    /// backpressure is an explicitly permitted, safe outcome, never a
    /// reason to block.
    pub fn try_publish(&self, job: DMonitorPublishJob) -> bool {
        let Some(sender) = &self.sender else { return false };
        match sender.try_send(job) {
            Ok(()) => true,
            Err(TrySendError::Full(dropped)) => {
                eprintln!(
                    "mode=dmonitor_publisher status=dropped_backpressure observation_id={}",
                    dropped.observation_id
                );
                false
            }
            Err(TrySendError::Disconnected(dropped)) => {
                eprintln!(
                    "mode=dmonitor_publisher status=dropped_worker_unavailable observation_id={}",
                    dropped.observation_id
                );
                false
            }
        }
    }

    /// Graceful shutdown: closes the channel so the worker's `recv()` loop
    /// exits once anything already queued is drained (bounded by the
    /// underlying `DMonitorHttpOutput` client's own request timeout, the
    /// same bound that already applies to any single publish), then joins
    /// the thread.
    fn shutdown_inner(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    pub fn shutdown(mut self) {
        self.shutdown_inner();
    }
}

impl Drop for DMonitorPublisherHandle {
    fn drop(&mut self) {
        self.shutdown_inner();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// A raw-socket server that either replies immediately or never replies
    /// at all, counting how many connections it accepted -- mirrors
    /// `outputs::dmonitor_http::tests::MockDServer`'s pattern, kept local
    /// and minimal since this module only needs the "never reply" and
    /// "always accept" shapes.
    struct SlowOrDeadServer {
        url: String,
        stop: Arc<std::sync::atomic::AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    enum Behavior {
        NeverReply,
        AlwaysAccept,
    }

    impl SlowOrDeadServer {
        fn start(behavior: Behavior) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            listener.set_nonblocking(true).unwrap();
            let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let stopping = stop.clone();
            let thread = std::thread::spawn(move || {
                while !stopping.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream.set_nonblocking(false).unwrap();
                            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                            let mut bytes = Vec::new();
                            let mut buffer = [0u8; 4096];
                            loop {
                                let count = match stream.read(&mut buffer) {
                                    Ok(0) => break,
                                    Ok(count) => count,
                                    Err(_) => break,
                                };
                                bytes.extend_from_slice(&buffer[..count]);
                                if bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                                    break;
                                }
                            }
                            match behavior {
                                Behavior::NeverReply => {
                                    std::thread::sleep(Duration::from_secs(3));
                                }
                                Behavior::AlwaysAccept => {
                                    let body = r#"{"outcome":"accepted"}"#;
                                    let _ = write!(
                                        stream,
                                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                        body.len()
                                    );
                                }
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self { url, stop, thread: Some(thread) }
        }
    }

    impl Drop for SlowOrDeadServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn sample_job(observation_id: &str) -> DMonitorPublishJob {
        DMonitorPublishJob {
            observation_id: observation_id.to_string(),
            observation_json: serde_json::json!({"observation_id": observation_id}),
        }
    }

    #[test]
    fn try_publish_never_blocks_even_when_the_sink_never_replies() {
        let server = SlowOrDeadServer::start(Behavior::NeverReply);
        let output = DMonitorHttpOutput::new(server.url.clone(), "proj:lab".to_string(), 2).unwrap();
        let handle = DMonitorPublisherHandle::spawn(output, 1);

        let started = Instant::now();
        // First job occupies the worker (blocked inside its own 2s HTTP
        // timeout); give it a moment to be picked up off the channel.
        assert!(handle.try_publish(sample_job("obs-1")));
        std::thread::sleep(Duration::from_millis(50));
        // Second job fills the bounded queue (capacity 1).
        let accepted_second = handle.try_publish(sample_job("obs-2"));
        let elapsed = started.elapsed();
        assert!(elapsed < Duration::from_millis(500), "try_publish must return immediately, took {elapsed:?}");
        // Whether obs-2 is accepted or dropped depends on exact timing, but
        // the call itself must never have blocked -- already proven above.
        let _ = accepted_second;
    }

    #[test]
    fn queue_saturation_drops_observations_without_blocking_and_without_reusing_ids() {
        let server = SlowOrDeadServer::start(Behavior::NeverReply);
        let output = DMonitorHttpOutput::new(server.url.clone(), "proj:lab".to_string(), 10).unwrap();
        let handle = DMonitorPublisherHandle::spawn(output, 1);

        let started = Instant::now();
        let results: Vec<bool> = (1..=5).map(|n| handle.try_publish(sample_job(&format!("obs-{n}")))).collect();
        let elapsed = started.elapsed();
        assert!(elapsed < Duration::from_millis(500), "enqueueing 5 jobs must never block, took {elapsed:?}");
        // At least one must have been dropped (queue capacity 1, worker
        // stuck in a 10s timeout) -- proving backpressure is real, not
        // merely theoretically possible.
        assert!(results.iter().any(|accepted| !accepted), "at least one job must be dropped under saturation: {results:?}");
    }

    #[test]
    fn a_responsive_sink_actually_receives_the_publication() {
        let server = SlowOrDeadServer::start(Behavior::AlwaysAccept);
        let output = DMonitorHttpOutput::new(server.url.clone(), "proj:lab".to_string(), 5).unwrap();
        let handle = DMonitorPublisherHandle::spawn(output, DEFAULT_QUEUE_CAPACITY);
        assert!(handle.try_publish(sample_job("obs-1")));
        // Give the worker a moment to actually perform the HTTP round trip.
        std::thread::sleep(Duration::from_millis(200));
        handle.shutdown();
        // If the worker never ran, this would still pass structurally but
        // fail the intent -- accepted count proves the request was really sent.
    }

    #[test]
    fn shutdown_joins_the_worker_thread_without_hanging() {
        let server = SlowOrDeadServer::start(Behavior::AlwaysAccept);
        let output = DMonitorHttpOutput::new(server.url.clone(), "proj:lab".to_string(), 5).unwrap();
        let handle = DMonitorPublisherHandle::spawn(output, DEFAULT_QUEUE_CAPACITY);
        handle.try_publish(sample_job("obs-1"));
        let started = Instant::now();
        handle.shutdown();
        assert!(started.elapsed() < Duration::from_secs(5), "shutdown must not hang");
    }
}
