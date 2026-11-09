use crate::index::{Focus, Hit, Index};
use crate::platform;
use crate::search::{Filter, Grammar, Query};
use parking_lot::RwLock;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const SEARCH_PATH: &str = "/api/v1/search";
const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 1000;
const MAX_REQUEST_LINE: usize = 8 * 1024;
const TIMEOUT: Duration = Duration::from_secs(2);

pub(super) struct Server {
    port: u16,
    stopped: Arc<AtomicBool>,
}

impl Server {
    pub(super) fn start(port: u16, index: Arc<Index>, filters: Arc<RwLock<Vec<Filter>>>) -> Option<Self> {
        let listener = match TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
            Ok(listener) => listener,
            Err(error) => {
                log::warn!("The HTTP API couldn't listen on port {port}: {error}");
                return None;
            }
        };
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let spawned = std::thread::Builder::new().name("api".into()).spawn(move || {
            for stream in listener.incoming() {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                match stream {
                    Ok(stream) => {
                        if let Err(error) = answer(stream, port, &index, &filters) {
                            log::debug!("An HTTP API request failed: {error}");
                        }
                    }
                    Err(error) => log::debug!("An HTTP API connection failed: {error}"),
                }
            }
        });
        match spawned {
            Ok(_) => {
                log::info!("The HTTP API listens on 127.0.0.1:{port}");
                Some(Self { port, stopped })
            }
            Err(error) => {
                log::warn!("Couldn't start the HTTP API: {error}");
                None
            }
        }
    }

    pub(super) fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, self.port)), TIMEOUT);
    }
}

fn answer(stream: TcpStream, port: u16, index: &Index, filters: &RwLock<Vec<Filter>>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    let mut host = None;
    let mut request = None;
    loop {
        line.clear();
        if reader.by_ref().take(MAX_REQUEST_LINE as u64).read_line(&mut line)? == 0 {
            break;
        }
        let text = line.trim_end();
        if text.is_empty() {
            break;
        }
        match request {
            None => request = Some(text.to_owned()),
            Some(_) => {
                if let Some((name, value)) = text.split_once(':')
                    && name.trim().eq_ignore_ascii_case("host")
                {
                    host = Some(value.trim().to_owned());
                }
            }
        }
    }
    let (status, body) = match (request.as_deref().and_then(target), host.as_deref()) {
        (_, Some(host)) if !local(host, port) => ("403 Forbidden", json!({ "error": "unknown host" })),
        (_, None) => ("400 Bad Request", json!({ "error": "no host" })),
        (Some(target), Some(_)) => route(&target, index, filters),
        (None, Some(_)) => ("400 Bad Request", json!({ "error": "bad request" })),
    };
    respond(stream, status, &body)
}

fn route(target: &str, index: &Index, filters: &RwLock<Vec<Filter>>) -> (&'static str, Value) {
    let (path, parameters) = target.split_once('?').unwrap_or((target, ""));
    if path != SEARCH_PATH {
        return ("404 Not Found", json!({ "error": "not found" }));
    }
    let mut text = String::new();
    let mut limit = DEFAULT_LIMIT;
    for (name, value) in parameters.split('&').filter_map(|pair| pair.split_once('=')) {
        match name {
            "q" => text = decode(value),
            "limit" => limit = decode(value).parse::<usize>().map_or(DEFAULT_LIMIT, |limit| limit.clamp(1, MAX_LIMIT)),
            _ => {}
        }
    }
    let empty = ("200 OK", json!({ "total": 0, "items": [] }));
    if text.trim().is_empty() {
        return empty;
    }
    let grammar = Grammar::new(&filters.read(), platform::clock());
    let Some(query) = Query::parse(&text, &grammar) else { return empty };
    let results = index.search(&query, &Default::default(), &Focus::Anything, limit);
    let items: Vec<Value> = results.hits.iter().map(item).collect();
    ("200 OK", json!({ "total": results.total, "items": items }))
}

fn item(hit: &Hit) -> Value {
    json!({
        "path": hit.path,
        "name": hit.name(),
        "folder": hit.folder(),
        "directory": hit.dir,
        "size": hit.size,
        "modified": hit.modified,
        "snippet": hit.snippet.as_ref().map(|snippet| &snippet.text),
    })
}

fn target(request: &str) -> Option<String> {
    let mut parts = request.split_whitespace();
    let method = parts.next()?;
    let target = parts.next()?;
    (method == "GET" && parts.next()?.starts_with("HTTP/")).then(|| target.to_owned())
}

fn local(host: &str, port: u16) -> bool {
    let (name, given) = host.rsplit_once(':').unwrap_or((host, ""));
    let port_matches = given.is_empty() || given.parse::<u16>() == Ok(port);
    port_matches && ["127.0.0.1", "localhost"].iter().any(|known| name.eq_ignore_ascii_case(known))
}

fn respond(mut stream: TcpStream, status: &str, body: &Value) -> std::io::Result<()> {
    let body = body.to_string();
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    stream.flush()
}

fn decode(text: &str) -> String {
    let mut bytes = Vec::with_capacity(text.len());
    let mut input = text.bytes();
    while let Some(byte) = input.next() {
        match byte {
            b'+' => bytes.push(b' '),
            b'%' => {
                let digits: Vec<u8> = input.by_ref().take(2).collect();
                let decoded = std::str::from_utf8(&digits).ok().and_then(|hex| u8::from_str_radix(hex, 16).ok());
                match decoded {
                    Some(decoded) if digits.len() == 2 => bytes.push(decoded),
                    Some(_) | None => {
                        bytes.push(b'%');
                        bytes.extend(digits);
                    }
                }
            }
            byte => bytes.push(byte),
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_are_percent_decoded() {
        assert_eq!(decode("%D0%BE%D1%82%D1%87%D1%91%D1%82+2026"), "отчёт 2026");
        assert_eq!(decode("100%25"), "100%");
        assert_eq!(decode("50%"), "50%");
        assert_eq!(decode("%zz"), "%zz");
    }

    #[test]
    fn only_local_hosts_on_our_port_are_answered() {
        assert!(local("127.0.0.1:38431", 38431));
        assert!(local("LOCALHOST:38431", 38431));
        assert!(local("localhost", 38431));
        assert!(!local("evil.example:38431", 38431));
        assert!(!local("127.0.0.1:80", 38431));
    }

    #[test]
    fn only_get_requests_have_a_target() {
        assert_eq!(target("GET /api/v1/search?q=a HTTP/1.1").as_deref(), Some("/api/v1/search?q=a"));
        assert_eq!(target("POST /api/v1/search HTTP/1.1"), None);
        assert_eq!(target("GET"), None);
    }

    #[test]
    fn unknown_paths_are_not_found() {
        let index = Index::default();
        let filters = RwLock::new(Filter::defaults());
        assert_eq!(route("/api/v2", &index, &filters).0, "404 Not Found");
        let (status, body) = route("/api/v1/search?q=", &index, &filters);
        assert_eq!((status, body["total"].as_u64()), ("200 OK", Some(0)));
    }
}
