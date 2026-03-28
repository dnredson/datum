use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use serde_json::{json, Value as JsonValue};

pub fn run_tcp_port(
    host: &str,
    port: u16,
    timeout_ms: u64,
    observed_at: &str,
) -> JsonValue {
    let addr = format!("{}:{}", host, port);

    let start = Instant::now();

    let addrs = match addr.to_socket_addrs() {
        Ok(a) => a,
        Err(e) => {
            return json!({
                "kind": "tcp_port",
                "status": "fail",
                "error": format!("resolve_error: {}", e),
                "checked_at": observed_at
            })
        }
    };

    let timeout = Duration::from_millis(timeout_ms);

    for a in addrs {
        match TcpStream::connect_timeout(&a, timeout) {
            Ok(_) => {
                let latency = start.elapsed().as_millis();

                return json!({
                    "kind": "tcp_port",
                    "status": "available",
                    "latency_ms": latency,
                    "checked_at": observed_at
                });
            }
            Err(_) => continue,
        }
    }

    json!({
        "kind": "tcp_port",
        "status": "unavailable",
        "error": "connection_failed",
        "checked_at": observed_at
    })
}