use std::{
    convert::Infallible,
    fs,
    net::{IpAddr, SocketAddr},
    num::NonZeroUsize,
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use anyhow::Result;
use dashmap::{DashMap, DashSet};
use governor::{Quota, RateLimiter};
use hyper::{
    server::conn::AddrStream,
    service::{make_service_fn, service_fn},
    Body, Request, Response, StatusCode, Server,
};
use lru::LruCache;
use std::num::NonZeroU32;
use once_cell::sync::OnceCell;
use tokio::{sync::Mutex, time::sleep};

static CACHE: OnceCell<Mutex<LruCache<String, Arc<Vec<u8>>>>> = OnceCell::new();
static METRICS: OnceCell<DashMap<&'static str, u64>> = OnceCell::new();

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

async fn handle(
    req: Request<Body>,
    blacklist: Arc<DashSet<IpAddr>>,
    rate_limiter: Arc<KeyedLimiter>,
) -> Result<Response<Body>, Infallible> {
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

    METRICS.get().unwrap().entry("requests").and_modify(|v| *v += 1).or_insert(1);

    let path = req.uri().path().to_string();
    let cache = CACHE.get().unwrap();
    let mut lock = cache.lock().await;

    if let Some(v) = lock.get(&path) {
        return Ok(Response::new(Body::from(v.as_ref().clone())));
    }

    drop(lock);
    backend().await;

    let body = Arc::new(b"Hello from Rust on Render!\n".to_vec());
    let mut lock = CACHE.get().unwrap().lock().await;
    lock.put(path.clone(), body.clone());
    Ok(Response::new(Body::from(body.as_ref().clone())))
}

#[tokio::main(flavor = "multi_thread", worker_threads = 12)]
async fn main() -> Result<()> {
    CACHE.set(Mutex::new(LruCache::new(NonZeroUsize::new(1024).unwrap()))).unwrap();
    METRICS.set(DashMap::new()).unwrap();

    let blacklist = Arc::new(load_blacklist());
    let rate_limiter = Arc::new(create_rate_limiter());

    // Render injects a PORT env var (default 10000)
    let port = std::env::var("PORT").unwrap_or_else(|_| "10000".to_string());
    let addr: SocketAddr = format!("0.0.0.0:{port}").parse()?;
    println!("✅ Listening on http://{}", addr);

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
