use std::{
    convert::Infallible,
    net::SocketAddr,
    num::NonZeroUsize,
    sync::Arc,
    time::{Instant, Duration},
};

use anyhow::Result;
use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use hyper::{
    service::{make_service_fn, service_fn},
    Body, Request, Response, Server,
};
use lru::LruCache;
use once_cell::sync::OnceCell;
use serde::Serialize;
use sysinfo::{System, RefreshKind};
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

async fn backend() {
    sleep(Duration::from_millis(2)).await;
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

async fn handle(req: Request<Body>) -> Result<Response<Body>, Infallible> {
    let path = req.uri().path().to_string();
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

    // Start HTTP server
    let port = std::env::var("PORT").unwrap_or_else(|_| "10000".to_string());
    let addr: SocketAddr = format!("0.0.0.0:{port}").parse()?;
    println!("✅ HTTP listening on http://{}", addr);

    let make_svc = make_service_fn(|_| async { Ok::<_, Infallible>(service_fn(handle)) });
    Server::bind(&addr).serve(make_svc).await?;

    Ok(())
}
