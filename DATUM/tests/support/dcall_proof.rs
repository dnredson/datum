use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Exchange {
    pub path: String,
    pub request: serde_json::Value,
    pub status: u16,
    pub response: serde_json::Value,
}

/// Transparent loopback test observer. Real DServer/listener handles every
/// request; this observer records messages without deriving authority.
pub struct RecordingProxy {
    pub url: String,
    pub exchanges: Arc<Mutex<Vec<Exchange>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl RecordingProxy {
    pub fn new(upstream: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let exchanges = Arc::new(Mutex::new(Vec::new()));
        let recorded = exchanges.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = std::thread::spawn(move || {
            let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(20)).build().unwrap();
            while !stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => forward(stream, &upstream, &client, &recorded),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(10)),
                    Err(e) => panic!("observer accept: {e}"),
                }
            }
        });
        Self { url, exchanges, stop, thread: Some(thread) }
    }
    pub fn records(&self) -> Vec<Exchange> { self.exchanges.lock().unwrap().clone() }
}

impl Drop for RecordingProxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Err(error) = self.thread.take().unwrap().join() {
            // Preserve the primary assertion/transport failure. A second
            // panic while unwinding would abort the Windows test process
            // and conceal both diagnostics.
            if std::thread::panicking() {
                eprintln!("HTTP proof observer also failed: {error:?}");
            } else {
                std::panic::resume_unwind(error);
            }
        }
    }
}

fn forward(mut stream: TcpStream, upstream: &str, client: &reqwest::blocking::Client, recorded: &Arc<Mutex<Vec<Exchange>>>) {
    stream.set_nonblocking(false).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let mut bytes = Vec::new();
    let mut chunk = [0; 4096];
    let end = loop {
        let n = stream.read(&mut chunk).unwrap();
        if n == 0 { return; }
        bytes.extend_from_slice(&chunk[..n]);
        if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") { break i + 4; }
        assert!(bytes.len() <= 65536);
    };
    let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
    let mut lines = headers.lines();
    let first = lines.next().unwrap();
    let mut words = first.split_whitespace();
    let method = words.next().unwrap().parse::<reqwest::Method>().unwrap();
    let path = words.next().unwrap().to_string();
    let mut request = client.request(method, format!("{upstream}{path}"));
    let mut length = 0;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            match name.to_ascii_lowercase().as_str() {
                "content-length" => length = value.trim().parse::<usize>().unwrap(),
                "host" | "connection" => {},
                _ => request = request.header(name, value.trim()),
            }
        }
    }
    assert!(length <= 16 * 1024 * 1024);
    while bytes.len() - end < length {
        let n = stream.read(&mut chunk).unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&chunk[..n]);
    }
    let body = &bytes[end..end + length];
    let response = request.body(body.to_vec()).send().unwrap();
    let status = response.status();
    let response_body = response.bytes().unwrap();
    recorded.lock().unwrap().push(Exchange {
        path, request: serde_json::from_slice(body).unwrap_or(serde_json::Value::Null),
        status: status.as_u16(), response: serde_json::from_slice(&response_body).unwrap_or(serde_json::Value::Null),
    });
    write!(stream, "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", status.as_u16(), status.canonical_reason().unwrap_or("Response"), response_body.len()).unwrap();
    stream.write_all(&response_body).unwrap();
}
