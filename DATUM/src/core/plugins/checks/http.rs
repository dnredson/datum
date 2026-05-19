use serde_json::{json, Value as JsonValue};
use std::time::Instant;

pub fn run_http_health(url: &str, timeout_ms: u64, observed_at: &str) -> JsonValue {
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_millis(timeout_ms))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return json!({
                "kind": "http_health",
                "status": "fail",
                "error": format!("client_build_error: {}", e),
                "checked_at": observed_at
            });
        }
    };

    let start = Instant::now();
    match client.get(url).send() {
        Ok(resp) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            if resp.status().is_success() {
                json!({
                    "kind": "http_health",
                    "status": "available",
                    "latency_ms": latency_ms,
                    "checked_at": observed_at
                })
            } else {
                json!({
                    "kind": "http_health",
                    "status": "unavailable",
                    "http_status": resp.status().as_u16(),
                    "latency_ms": latency_ms,
                    "checked_at": observed_at
                })
            }
        }
        Err(e) => {
            json!({
                "kind": "http_health",
                "status": "unavailable",
                "error": e.to_string(),
                "checked_at": observed_at
            })
        }
    }
}
