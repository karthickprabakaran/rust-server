use std::{
    convert::Infallible,
    fs,
    net::{IpAddr, SocketAddr},
    num::NonZeroUsize,
    str::FromStr,
    sync::Arc,
    time::{Instant, Duration},
};

use anyhow::Result;
use dashmap::{DashMap, DashSet};
use governor::clock::Clock;
use governor::{Quota, RateLimiter};
use futures::{SinkExt, StreamExt};
use hyper::{
    server::conn::AddrStream,
    service::{make_service_fn, service_fn},
    Body, Request, Response, StatusCode, Server,
};
use lru::LruCache;
use std::num::NonZeroU32;
use once_cell::sync::OnceCell;
use serde::Serialize;
use sysinfo::System;
use tokio::{
    net::TcpListener,
    sync::{broadcast, Mutex},
    time::sleep,
};
use tokio_tungstenite::tungstenite::protocol::Message;

// Cache and metrics
static CACHE: OnceCell<Mutex<LruCache<String, Arc<Vec<u8>>>>> = OnceCell::new();
static METRICS: OnceCell<DashMap<&'static str, u64>> = OnceCell::new();
static REQUEST_METRICS: OnceCell<Mutex<Vec<RequestMetrics>>> = OnceCell::new();
static WS_SENDER: OnceCell<broadcast::Sender<RequestMetrics>> = OnceCell::new();
static START_TIME: OnceCell<Instant> = OnceCell::new();

// Track active connections for WebSocket
static WS_CLIENTS_COUNT: OnceCell<Mutex<usize>> = OnceCell::new();

// Per-IP request count (all requests, including blocked)
static REQUEST_COUNT_BY_IP: OnceCell<DashMap<IpAddr, u64>> = OnceCell::new();

#[derive(Serialize, Clone, Debug)]
struct RequestMetrics {
    path: String,
    latency_ms: u128,
    bytes: usize,
    status_code: u16,
    method: String,
    request_rate: f64,
    cpu_usage_percent: f32,
    memory_usage_bytes: u64,
    active_connections: usize,
    uptime_seconds: u64,
}

#[derive(Serialize, Default)]
struct Summary {
    total_requests: u64,
    total_bytes: usize,
    avg_latency_ms: f64,
    min_latency_ms: u128,
    max_latency_ms: u128,
    rps: f64,
    cache_hits: u64,
    cache_misses: u64,
    error_count: u64,
}

#[derive(Serialize)]
struct IpRequestCount {
    ip: String,
    count: u64,
}

#[derive(Serialize)]
struct IpStatsResponse {
    blocked_ips: Vec<String>,
    requests_by_ip: Vec<IpRequestCount>,
}

type KeyedLimiter = governor::DefaultKeyedRateLimiter<IpAddr>;

fn load_blacklist() -> DashSet<IpAddr> {
    let set = DashSet::new();
    if let Ok(ips) = std::env::var("BLACKLIST_IPS") {
        for s in ips.split(',') {
            let s = s.trim();
            if let Ok(ip) = IpAddr::from_str(s) {
                set.insert(ip);
            }
        }
    }
    if let Ok(path) = std::env::var("BLACKLIST_FILE") {
        if let Ok(contents) = fs::read_to_string(path) {
            for line in contents.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Ok(ip) = IpAddr::from_str(line) {
                    set.insert(ip);
                }
            }
        }
    }
    set
}

fn create_rate_limiter() -> KeyedLimiter {
    let per_minute: u32 = std::env::var("RATE_LIMIT_PER_MINUTE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60)
        .max(1);
    let quota = Quota::per_minute(NonZeroU32::new(per_minute).unwrap());
    RateLimiter::keyed(quota)
}

async fn backend() {
    sleep(Duration::from_millis(2)).await;
}

fn response_403() -> Response<Body> {
    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .header("X-Blocked-Reason", "blacklist")
        .body(Body::from("Forbidden"))
        .unwrap()
}

fn response_429(retry_after_secs: u64) -> Response<Body> {
    Response::builder()
        .status(StatusCode::TOO_MANY_REQUESTS)
        .header("Retry-After", retry_after_secs.to_string())
        .body(Body::from("Too Many Requests"))
        .unwrap()
}

async fn broadcast_ws(metric: RequestMetrics) {
    if let Some(tx) = WS_SENDER.get() {
        let _ = tx.send(metric);
    }
}

fn compute_summary() -> Summary {
    let metrics = REQUEST_METRICS.get().unwrap();
    let lock = metrics.blocking_lock();
    let total_requests = lock.len() as u64;
    let total_bytes: usize = lock.iter().map(|m| m.bytes).sum();
    let avg_latency_ms = if total_requests > 0 {
        lock.iter().map(|m| m.latency_ms).sum::<u128>() as f64 / total_requests as f64
    } else {
        0.0
    };
    let min_latency_ms = lock.iter().map(|m| m.latency_ms).min().unwrap_or(0);
    let max_latency_ms = lock.iter().map(|m| m.latency_ms).max().unwrap_or(0);
    Summary {
        total_requests,
        total_bytes,
        avg_latency_ms,
        min_latency_ms,
        max_latency_ms,
        rps: 0.0,           // compute RPS later if needed
        cache_hits: METRICS.get().unwrap().get("cache_hits").map(|v| *v).unwrap_or(0),
        cache_misses: METRICS.get().unwrap().get("cache_misses").map(|v| *v).unwrap_or(0),
        error_count: METRICS.get().unwrap().get("errors").map(|v| *v).unwrap_or(0),
    }
}

fn cors_headers() -> [(&'static str, &'static str); 2] {
    [
        ("Access-Control-Allow-Origin", "*"),
        ("Access-Control-Allow-Methods", "GET, OPTIONS"),
    ]
}

async fn handle(
    req: Request<Body>,
    blacklist: Arc<DashSet<IpAddr>>,
    rate_limiter: Arc<KeyedLimiter>,
) -> Result<Response<Body>, Infallible> {
    let path = req.uri().path().to_string();

    // CORS preflight
    if req.method().as_str() == "OPTIONS" {
        let mut res = Response::builder().status(StatusCode::NO_CONTENT);
        for (k, v) in cors_headers() {
            res = res.header(k, v);
        }
        return Ok(res.body(Body::empty()).unwrap());
    }

    let client_ip = req
        .extensions()
        .get::<SocketAddr>()
        .map(SocketAddr::ip)
        .ok_or_else(|| {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("Missing client address"))
                .unwrap()
        });
    let client_ip = match client_ip {
        Ok(ip) => ip,
        Err(res) => return Ok(res),
    };

    // Count request per IP (all requests)
    REQUEST_COUNT_BY_IP
        .get()
        .unwrap()
        .entry(client_ip)
        .and_modify(|c| *c += 1)
        .or_insert(1);

    // Return IP stats API (blocked list + requests by IP)
    if path == "/api/ip-stats" {
        let blocked_ips: Vec<String> = blacklist.iter().map(|ip| ip.to_string()).collect();
        let mut requests_by_ip: Vec<IpRequestCount> = REQUEST_COUNT_BY_IP
            .get()
            .unwrap()
            .iter()
            .map(|r| IpRequestCount {
                ip: r.key().to_string(),
                count: *r.value(),
            })
            .collect();
        requests_by_ip.sort_by(|a, b| b.count.cmp(&a.count));
        let body = serde_json::to_string(&IpStatsResponse {
            blocked_ips,
            requests_by_ip,
        })
        .unwrap();
        let mut b = Response::builder().status(StatusCode::OK).header("Content-Type", "application/json");
        for (k, v) in cors_headers() {
            b = b.header(k, v);
        }
        return Ok(b.body(Body::from(body)).unwrap());
    }

    if blacklist.contains(&client_ip) {
        METRICS
            .get()
            .unwrap()
            .entry("blacklist_hits")
            .and_modify(|v| *v += 1)
            .or_insert(1);
        return Ok(response_403());
    }

    if let Err(not_until) = rate_limiter.check_key(&client_ip) {
        METRICS
            .get()
            .unwrap()
            .entry("rate_limit_hits")
            .and_modify(|v| *v += 1)
            .or_insert(1);
        let wait_secs = not_until
            .wait_time_from(rate_limiter.clock().now())
            .as_secs()
            .max(1);
        return Ok(response_429(wait_secs));
    }

    let method = req.method().to_string();

    // Return summary if requested
    if path == "/summary" {
        let summary = compute_summary();
        let body = serde_json::to_string(&summary).unwrap();
        return Ok(Response::new(Body::from(body)));
    }

    let start = Instant::now();
    METRICS
        .get()
        .unwrap()
        .entry("requests")
        .and_modify(|v| *v += 1)
        .or_insert(1);

    let cache = CACHE.get().unwrap();
    let mut lock = cache.lock().await;

    let mut cache_hit = false;
    let body = if let Some(v) = lock.get(&path) {
        cache_hit = true;
        v.clone()
    } else {
        drop(lock);
        backend().await;
        let body = Arc::new(b"Hello from Rust on Render!\n".to_vec());
        let mut lock = CACHE.get().unwrap().lock().await;
        lock.put(path.clone(), body.clone());
        body
    };

    // Update cache hit/miss counters
    if cache_hit {
        METRICS.get().unwrap().entry("cache_hits").and_modify(|v| *v += 1).or_insert(1);
    } else {
        METRICS.get().unwrap().entry("cache_misses").and_modify(|v| *v += 1).or_insert(1);
    }

    let latency = start.elapsed().as_millis();
    let bytes = body.len();

    // System info
let mut sys = System::new_all(); // new_all still exists
sys.refresh_all();
let cpus = sys.cpus();
let cpu_usage = if !cpus.is_empty() {
    cpus.iter().map(|c| c.cpu_usage()).sum::<f32>() / cpus.len() as f32
} else {
    0.0
};
let memory_usage = sys.used_memory();

    let uptime = START_TIME.get().unwrap().elapsed().as_secs();
    let active_connections = WS_CLIENTS_COUNT.get().unwrap().lock().await.clone();

    // Compute request rate (approx)
    let request_rate = {
        let total = METRICS.get().unwrap().get("requests").map(|v| *v).unwrap_or(0);
        total as f64 / uptime.max(1) as f64
    };

    let metrics_entry = RequestMetrics {
        path,
        latency_ms: latency,
        bytes,
        status_code: 200,
        method,
        request_rate,
        cpu_usage_percent: cpu_usage,
        memory_usage_bytes: memory_usage,
        active_connections,
        uptime_seconds: uptime,
    };

    REQUEST_METRICS.get().unwrap().lock().await.push(metrics_entry.clone());

    // Broadcast to WebSocket clients
    broadcast_ws(metrics_entry).await;

    Ok(Response::new(Body::from(body.as_ref().clone())))
}

#[tokio::main(flavor = "multi_thread", worker_threads = 12)]
async fn main() -> Result<()> {
    START_TIME.set(Instant::now()).unwrap();
    CACHE.set(Mutex::new(LruCache::new(NonZeroUsize::new(1024).unwrap())))
        .unwrap();
    METRICS.set(DashMap::new()).unwrap();
    REQUEST_METRICS.set(Mutex::new(Vec::new())).unwrap();
    WS_CLIENTS_COUNT.set(Mutex::new(0)).unwrap();
    REQUEST_COUNT_BY_IP.set(DashMap::new()).unwrap();

    let blacklist = Arc::new(load_blacklist());
    let rate_limiter = Arc::new(create_rate_limiter());

    // Broadcast channel for WebSocket
    let (tx, _rx) = broadcast::channel(100);
    WS_SENDER.set(tx.clone()).unwrap();

    // Start WebSocket server
    tokio::spawn(async move {
        let listener = TcpListener::bind("0.0.0.0:9000").await.unwrap();
        println!(" WebSocket listening on ws://0.0.0.0:9000");
        while let Ok((stream, _)) = listener.accept().await {
            let ws_stream = tokio_tungstenite::accept_async(stream).await.unwrap();
            let mut rx = tx.subscribe();

            // Track active connections
            {
                let mut count = WS_CLIENTS_COUNT.get().unwrap().lock().await;
                *count += 1;
            }

            tokio::spawn(async move {
                let (mut ws_tx, _) = ws_stream.split();
                while let Ok(msg) = rx.recv().await {
                    let text = serde_json::to_string(&msg).unwrap();
                    if ws_tx.send(Message::Text(text)).await.is_err() {
                        break;
                    }
                }

                // Connection closed
                let mut count = WS_CLIENTS_COUNT.get().unwrap().lock().await;
                *count = count.saturating_sub(1);
            });
        }
    });

    // Render injects a PORT env var (default 10000)
    let port = std::env::var("PORT").unwrap_or_else(|_| "10000".to_string());
    let addr: SocketAddr = format!("0.0.0.0:{port}").parse()?;
    println!("✅ HTTP listening on http://{}", addr);

    let make_svc = make_service_fn(move |conn: &AddrStream| {
        let remote_addr = conn.remote_addr();
        let blacklist = Arc::clone(&blacklist);
        let rate_limiter = Arc::clone(&rate_limiter);
        async move {
            Ok::<_, Infallible>(service_fn(move |mut req: Request<Body>| {
                req.extensions_mut().insert(remote_addr);
                handle(req, Arc::clone(&blacklist), Arc::clone(&rate_limiter))
            }))
        }
    });
    Server::bind(&addr).serve(make_svc).await?;

    Ok(())
}
