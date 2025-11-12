use std::{
    convert::Infallible,
    net::SocketAddr,
    num::NonZeroUsize,
    sync::Arc,
    time::Duration,
};

use anyhow::Result;
use dashmap::DashMap;
use hyper::{service::{make_service_fn, service_fn}, Body, Request, Response, Server};
use lru::LruCache;
use once_cell::sync::OnceCell;
use tokio::{sync::Mutex, time::sleep};

static CACHE: OnceCell<Mutex<LruCache<String, Arc<Vec<u8>>>>> = OnceCell::new();
static METRICS: OnceCell<DashMap<&'static str, u64>> = OnceCell::new();

async fn backend() {
    sleep(Duration::from_millis(2)).await;
}

async fn handle(req: Request<Body>) -> Result<Response<Body>, Infallible> {
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

    // Render injects a PORT env var (default 10000)
    let port = std::env::var("PORT").unwrap_or_else(|_| "10000".to_string());
    let addr: SocketAddr = format!("0.0.0.0:{port}").parse()?;
    println!("✅ Listening on http://{}", addr);

    let make_svc = make_service_fn(|_| async { Ok::<_, Infallible>(service_fn(handle)) });
    Server::bind(&addr).serve(make_svc).await?;

    Ok(())
}
