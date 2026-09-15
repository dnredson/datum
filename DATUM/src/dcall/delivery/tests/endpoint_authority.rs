//! Actual endpoint-fetch and delivery HTTP path, including a live observer.
use super::*;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};

struct HttpObserver {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl HttpObserver {
    fn new(body: String) -> Self {
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
                        stream.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
                        let mut bytes = Vec::new();
                        let mut buffer = [0; 4096];
                        loop {
                            let count = stream.read(&mut buffer).unwrap();
                            if count == 0 { break; }
                            bytes.extend_from_slice(&buffer[..count]);
                            if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                                let headers = String::from_utf8_lossy(&bytes[..end]);
                                let length = headers.lines().find_map(|line| {
                                    let (name, value) = line.split_once(':')?;
                                    name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().unwrap())
                                }).unwrap_or(0);
                                if bytes.len() >= end + 4 + length { break; }
                            }
                        }
                        captured.lock().unwrap().push(String::from_utf8(bytes).unwrap());
                        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(std::time::Duration::from_millis(2)),
                    Err(error) => panic!("observer accept: {error}"),
                }
            }
        });
        Self { url, requests, stop, thread: Some(thread) }
    }

    fn finish(&mut self) -> Vec<String> {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for HttpObserver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

fn endpoint(url: &str) -> serde_json::Value {
    let mut value = serde_json::json!({"node_id":"fog-01", "transport":"http", "endpoint_id":"E1", "revision":1, "base_url":url});
    value["descriptor_digest"] = serde_json::json!(crate::agent::jcs::canonical_digest_sha256_hex(&value).unwrap());
    value
}

fn bound_grant(value: &serde_json::Value) -> DCallDeliveryGrantV1 {
    let mut grant = sample_grant();
    grant.route_kind = "remote".into();
    grant.source.node_id = "source-node".into();
    grant.endpoint_id = Some(value["endpoint_id"].as_str().unwrap().into());
    grant.endpoint_revision = value["revision"].as_u64();
    grant.endpoint_descriptor_digest = Some(value["descriptor_digest"].as_str().unwrap().into());
    grant
}

#[test]
fn changed_current_endpoint_receives_zero_requests_and_grant_is_not_redeemed() {
    let mut observer = HttpObserver::new("{}".into());
    let original = endpoint("http://127.0.0.1:1/original");
    let grant = bound_grant(&original);
    let mut current = endpoint(&observer.url);
    current["revision"] = serde_json::json!(2);
    current.as_object_mut().unwrap().remove("descriptor_digest");
    current["descriptor_digest"] = serde_json::json!(crate::agent::jcs::canonical_digest_sha256_hex(&current).unwrap());
    let mut control = HttpObserver::new(current.to_string());
    let error = deliver_remote(&control.url, &grant, "secret-bearer", b"secret-payload", &sha256_hex(b"secret-payload")).unwrap_err();
    assert_eq!(error.code, DCallDeliveryErrorCode::DCallEndpointAuthorityMismatch);
    assert!(error.redeemed_at.is_none());
    assert!(observer.finish().is_empty(), "changed endpoint received delivery/token/payload");
    let requests = control.finish();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("GET /api/v1/dcall/endpoints/fog-01/http "));
    assert!(!requests[0].contains("secret-bearer") && !requests[0].contains("secret-payload"));
    // All control-plane traffic is observed: no redeem request was made.
    let evidence = evidence::build_grant_bound_failure_evidence("attempt".into(), &grant, error.redeemed_at, error.code, error.diagnostic.as_deref(), 0);
    let persistent = serde_json::to_string(&evidence).unwrap();
    for secret in [&observer.url, "secret-bearer", "secret-payload"] { assert!(!persistent.contains(secret)); }
}

#[test]
fn every_endpoint_binding_or_self_digest_mismatch_fails_before_send() {
    for field in ["node_id", "transport", "endpoint_id", "revision", "descriptor_digest", "base_url"] {
        let mut observer = HttpObserver::new("{}".into());
        let original = endpoint(&observer.url);
        let grant = bound_grant(&original);
        let mut changed = original;
        changed[field] = match field {
            "revision" => serde_json::json!(2),
            "base_url" => serde_json::json!(format!("{}/tampered", observer.url)),
            _ => serde_json::json!("substituted"),
        };
        let mut control = HttpObserver::new(changed.to_string());
        let error = deliver_remote(&control.url, &grant, "bearer", b"payload", &grant.payload_sha256).unwrap_err();
        assert_eq!(error.code, DCallDeliveryErrorCode::DCallEndpointAuthorityMismatch, "{field}");
        assert!(observer.finish().is_empty(), "{field}");
        assert_eq!(control.finish().len(), 1, "{field}: unexpected redemption");
    }
}

#[test]
fn malformed_http_success_responses_fail_closed_on_real_delivery_path() {
    for body in ["not JSON", "null", "[]", "{}", r#"{"outcome":null}"#, r#"{"outcome":5}"#, r#"{"outcome":"unknown"}"#, r#"{"outcome":"success"}"#, r#"{"outcome":"success","dcode_evidence":{}}"#] {
        let mut observer = HttpObserver::new(body.into());
        let value = endpoint(&observer.url);
        let grant = bound_grant(&value);
        let mut control = HttpObserver::new(value.to_string());
        let error = deliver_remote(&control.url, &grant, "bearer", b"payload", &grant.payload_sha256).unwrap_err();
        assert_eq!(error.code, DCallDeliveryErrorCode::DCallRemoteResponseMalformed, "{body}");
        assert_eq!(observer.finish().len(), 1);
        assert_eq!(control.finish().len(), 1);
    }
}

#[test]
fn structured_destination_failure_remains_execution_failure() {
    let mut observer = HttpObserver::new(r#"{"outcome":"failed"}"#.into());
    let value = endpoint(&observer.url);
    let grant = bound_grant(&value);
    let mut control = HttpObserver::new(value.to_string());
    let error = deliver_remote(&control.url, &grant, "bearer", b"payload", &grant.payload_sha256).unwrap_err();
    assert_eq!(error.code, DCallDeliveryErrorCode::DCallDestinationExecutionFailed);
    assert_eq!(observer.finish().len(), 1);
    assert_eq!(control.finish().len(), 1);
}
