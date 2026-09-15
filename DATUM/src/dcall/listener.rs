//! Minimal, deliberately hand-rolled HTTP/1.1 server for the D-Call
//! cross-D-Node delivery listener (ADR-0022, section 17: "use the smallest
//! existing Rust HTTP stack... do not introduce a large framework"). DATUM
//! has no HTTP SERVER dependency at all (only `reqwest` as a client), and
//! adding one purely to accept a single POST route on a loopback/lab-trust
//! listener would be disproportionate. This reads just enough HTTP/1.1 to
//! serve one route correctly: the request line + headers for
//! `Content-Length`, exactly that many body bytes (no chunked
//! transfer-encoding -- unneeded, since the only real client is
//! `reqwest`'s own `.json()`, which always sends `Content-Length`), and
//! writes back a JSON response. This is the SAME hand-rolled technique
//! `dcode::client`'s own test module already uses to mock DServer for
//! tests -- here it is the real production listener, not a mock.
//!
//! Trust boundary, stated honestly (never overclaimed elsewhere in this
//! module): bare HTTP, no TLS, no authentication beyond the D-Call delivery
//! grant's bearer token. Suitable ONLY for the trusted/lab network boundary
//! this whole repository already assumes -- this does not, and cannot,
//! claim transport confidentiality (`dcall-transport-trust-boundary-audit.txt`).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const DELIVER_PATH: &str = "/datum-dnode/v1/dcall/deliver";
/// Hard cap on an accepted request body -- rejected BEFORE allocating
/// unbounded memory for it (section 17: "reject oversized payload before
/// allocation where practical").
pub const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 64 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Serialize, Deserialize)]
pub struct DeliverRequest {
    pub grant_id: String,
    pub token: String,
    pub payload_b64: String,
    pub payload_sha256: String,
    pub payload_size: u64,
}

#[derive(Debug, Serialize, Default)]
pub struct DeliverResponse {
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dcode_evidence: Option<serde_json::Value>,
}

fn failed(code: &str, message: impl Into<String>) -> DeliverResponse {
    DeliverResponse { outcome: "failed".to_string(), error_code: Some(code.to_string()), message: Some(message.into()), dcode_evidence: None }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn write_response(stream: &mut TcpStream, status: u16, response: &DeliverResponse) {
    let body = serde_json::to_string(response).unwrap_or_else(|_| "{}".to_string());
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        410 => "Gone",
        413 => "Payload Too Large",
        422 => "Unprocessable Entity",
        _ => "Internal Server Error",
    };
    let response_text = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response_text.as_bytes());
}

/// Parses and handles exactly ONE HTTP/1.1 request off `stream`, calling
/// `handle` with the parsed body and the status/response it returns.
/// Malformed requests, unsupported paths, and oversized bodies are rejected
/// before `handle` is ever called.
pub fn serve_one_request<F>(mut stream: TcpStream, handle: F)
where
    F: FnOnce(DeliverRequest) -> (u16, DeliverResponse),
{
    // Windows may pass the listener's nonblocking mode to accepted sockets.
    // This handler uses bounded blocking reads, not readiness polling.
    if stream.set_nonblocking(false).is_err() {
        return;
    }
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let headers_end = loop {
        match stream.read(&mut chunk) {
            Ok(0) => {
                write_response(&mut stream, 400, &failed("malformed_request", "connection closed before headers were complete"));
                return;
            }
            Ok(n) => {
                buffer.extend_from_slice(&chunk[..n]);
                if let Some(pos) = find_subslice(&buffer, b"\r\n\r\n") {
                    break pos + 4;
                }
                if buffer.len() > MAX_HEADER_BYTES {
                    write_response(&mut stream, 400, &failed("malformed_request", "request headers too large"));
                    return;
                }
            }
            Err(_) => {
                write_response(&mut stream, 400, &failed("malformed_request", "error reading request"));
                return;
            }
        }
    };

    let header_text = String::from_utf8_lossy(&buffer[..headers_end]).to_string();
    let is_post_to_deliver_path = header_text.lines().next().map(|line| line.starts_with(&format!("POST {DELIVER_PATH} "))).unwrap_or(false);
    if !is_post_to_deliver_path {
        write_response(&mut stream, 404, &failed("not_found", "only POST to the D-Call deliver path is supported"));
        return;
    }
    let content_length = header_text.lines().find_map(|line| {
        let lower = line.to_ascii_lowercase();
        lower.strip_prefix("content-length:").and_then(|value| value.trim().parse::<usize>().ok())
    });
    let content_length = match content_length {
        Some(length) => length,
        None => {
            write_response(&mut stream, 400, &failed("malformed_request", "missing Content-Length"));
            return;
        }
    };
    if content_length > MAX_BODY_BYTES {
        write_response(&mut stream, 413, &failed("payload_too_large", format!("request body {content_length} bytes exceeds the {MAX_BODY_BYTES}-byte limit")));
        return;
    }

    let mut body = buffer[headers_end..].to_vec();
    while body.len() < content_length {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
    }
    if body.len() < content_length {
        write_response(&mut stream, 400, &failed("malformed_request", "request body shorter than declared Content-Length"));
        return;
    }
    body.truncate(content_length);

    let request: DeliverRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(error) => {
            write_response(&mut stream, 400, &failed("malformed_request", format!("invalid JSON body: {error}")));
            return;
        }
    };

    let (status, response) = handle(request);
    write_response(&mut stream, status, &response);
}

/// Runs the accept loop until `stop` is set, dispatching every accepted
/// connection to its own thread. `listener` should already be bound (e.g.
/// via `TcpListener::bind`) before calling this.
pub fn run<F>(listener: TcpListener, handle: F, stop: Arc<AtomicBool>)
where
    F: Fn(DeliverRequest) -> (u16, DeliverResponse) + Send + Sync + Clone + 'static,
{
    listener.set_nonblocking(true).ok();
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).ok();
                let handle = handle.clone();
                std::thread::spawn(move || serve_one_request(stream, handle));
            }
            Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send_raw_request(addr: std::net::SocketAddr, raw: &[u8]) -> String {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.write_all(raw).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut response = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => response.extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
        }
        String::from_utf8_lossy(&response).to_string()
    }

    #[test]
    fn accepted_nonblocking_socket_waits_for_delayed_request_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (ready, accepted) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream.set_nonblocking(true).unwrap();
            ready.send(()).unwrap();
            serve_one_request(stream, |_| (200, DeliverResponse { outcome: "success".into(), error_code: None, message: None, dcode_evidence: None }));
        });
        let mut client = TcpStream::connect(address).unwrap();
        accepted.recv().unwrap();
        std::thread::sleep(Duration::from_millis(50));
        let body = serde_json::json!({"grant_id":"g1","token":"tok","payload_b64":"aGk=","payload_sha256":"a".repeat(64),"payload_size":2}).to_string();
        write!(client, "POST {DELIVER_PATH} HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        client.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert!(response.contains("200 OK") && response.contains("success"), "{response}");
        thread.join().unwrap();
    }

    #[test]
    fn valid_deliver_request_reaches_the_handler() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                serve_one_request(stream, |request| {
                    assert_eq!(request.grant_id, "g1");
                    (200, DeliverResponse { outcome: "success".to_string(), error_code: None, message: None, dcode_evidence: None })
                });
            }
        });
        let body = serde_json::json!({"grant_id":"g1","token":"tok","payload_b64":"aGk=","payload_sha256":"a".repeat(64),"payload_size":2}).to_string();
        let raw = format!("POST {DELIVER_PATH} HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len());
        let response = send_raw_request(addr, raw.as_bytes());
        assert!(response.contains("200 OK"), "response: {response}");
        assert!(response.contains("\"outcome\":\"success\""));
    }

    #[test]
    fn oversized_content_length_is_rejected_before_reading_body() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                serve_one_request(stream, |_| panic!("handler must never be called for an oversized request"));
            }
        });
        let raw = format!("POST {DELIVER_PATH} HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n", MAX_BODY_BYTES + 1);
        let response = send_raw_request(addr, raw.as_bytes());
        assert!(response.contains("413"), "response: {response}");
    }

    #[test]
    fn malformed_json_body_is_rejected() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                serve_one_request(stream, |_| panic!("handler must never be called for malformed JSON"));
            }
        });
        let body = "{not valid json";
        let raw = format!("POST {DELIVER_PATH} HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{body}", body.len());
        let response = send_raw_request(addr, raw.as_bytes());
        assert!(response.contains("400"), "response: {response}");
    }

    #[test]
    fn wrong_path_is_rejected_with_404() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                serve_one_request(stream, |_| panic!("handler must never be called for the wrong path"));
            }
        });
        let raw = "POST /some/other/path HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\n\r\n{}";
        let response = send_raw_request(addr, raw.as_bytes());
        assert!(response.contains("404"), "response: {response}");
    }
}
