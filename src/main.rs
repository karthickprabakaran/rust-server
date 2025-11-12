use std::{
    convert::Infallible,
    fs::File,
    io::BufReader,
    net::SocketAddr,
    num::NonZeroUsize,
    sync::Arc,
    time::Duration,
};

use anyhow::Result;
use dashmap::DashMap;
use hyper::{service::service_fn, Body, Request, Response};
use hyper::server::conn::Http;
use lru::LruCache;
use once_cell::sync::OnceCell;
use tokio::{
    net::TcpListener,
    sync::{Mutex, Semaphore},
    time::sleep,
};
use tokio_rustls::{TlsAcceptor, rustls};
use rustls::{pki_types::{CertificateDer, PrivateKeyDer}, ServerConfig};
use rustls_pemfile::{certs, pkcs8_private_keys};

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

    let body = Arc::new(b"Hello from Rust edge!\n".to_vec());
    let mut lock = CACHE.get().unwrap().lock().await;
    lock.put(path.clone(), body.clone());
    Ok(Response::new(Body::from(body.as_ref().clone())))
}

fn load_tls() -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>) {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use rustls_pemfile::{certs, pkcs8_private_keys, rsa_private_keys};
    use std::io::BufReader;
    use std::fs::File;

    // --- Load certificate(s) ---
    let mut cert_reader = BufReader::new(File::open("cert.pem").expect("missing cert.pem"));
    let certs = certs(&mut cert_reader)
        .into_iter()
        .flatten()
        .collect::<Vec<CertificateDer<'static>>>();

    // --- Load private key(s) ---
    let mut key_reader = BufReader::new(File::open("key.pem").expect("missing key.pem"));

    // Try PKCS#8 first
    let mut keys: Vec<PrivateKeyDer<'static>> = pkcs8_private_keys(&mut key_reader)
        .into_iter()
        .flatten()
        .map(PrivateKeyDer::from)
        .collect();

    if keys.is_empty() {
        let mut key_reader = BufReader::new(File::open("key.pem").unwrap());
        keys = rsa_private_keys(&mut key_reader)
            .into_iter()
            .flatten()
            .map(PrivateKeyDer::from)
            .collect();
    }

    if keys.is_empty() {
        panic!("❌ No valid private keys found in key.pem — check PEM format.");
    }

    (certs, keys.remove(0))
}

#[tokio::main(flavor = "multi_thread", worker_threads = 12)]
async fn main() -> Result<()> {
    CACHE.set(Mutex::new(LruCache::new(NonZeroUsize::new(1024).unwrap()))).unwrap();
    METRICS.set(DashMap::new()).unwrap();

    let (certs, key) = load_tls();
    let mut cfg = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;

    // Advertise HTTP/2 and HTTP/1.1 for ALPN negotiation
    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    let acceptor = TlsAcceptor::from(Arc::new(cfg));
    let addr: SocketAddr = "0.0.0.0:8443".parse()?;
    let listener = TcpListener::bind(addr).await?;
    println!("✅ Listening on https://{}", addr);

    let sem = Arc::new(Semaphore::new(25000));

    loop {
        let (stream, _) = listener.accept().await?;
        let acceptor = acceptor.clone();
        let sem = sem.clone();

        tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.unwrap();

            if let Ok(tls_stream) = acceptor.accept(stream).await {
                let service = service_fn(handle);

let mut http = Http::new();
http.http2_max_concurrent_streams(1000);

if let Err(e) = http.serve_connection(tls_stream, service).await {
    eprintln!("HTTP error: {e}");
}
            }
        });
    }
}
