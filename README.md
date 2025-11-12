# ⚡ High-Concurrency HTTPS Server in Rust (Edge Emulator)

A high-performance **HTTPS (HTTP/1.1 + HTTP/2)** edge server built in **Rust**, inspired by how requests flow through Google’s infrastructure — from edge cache to backend response.  
This project demonstrates how modern async Rust can deliver **Google-scale performance** on a single **MacBook M3 Pro**.

---

## 🚀 Features

✅ HTTPS (TLS 1.3 via Rustls)  
✅ HTTP/1.1 and HTTP/2 support via ALPN negotiation  
✅ Handles up to **25,000 concurrent requests** using async concurrency (Tokio + Semaphore)  
✅ LRU in-memory cache for repeated requests  
✅ Multi-threaded Tokio runtime for full CPU utilization  
✅ Fully async request lifecycle (mimicking Google’s frontend servers)  
✅ Easily benchmarkable using `wrk` or `hey`

---

## 🧱 Dependencies

| Crate                                                       | Version | Purpose                                |
| ----------------------------------------------------------- | ------- | -------------------------------------- |
| [`tokio`](https://crates.io/crates/tokio)                   | 1.x     | Asynchronous runtime for concurrency   |
| [`hyper`](https://crates.io/crates/hyper)                   | 1.x     | HTTP server (HTTP/1.1 + HTTP/2)        |
| [`tokio-rustls`](https://crates.io/crates/tokio-rustls)     | 0.25    | TLS integration for Tokio              |
| [`rustls`](https://crates.io/crates/rustls)                 | 0.23    | TLS library (secure transport)         |
| [`rustls-pemfile`](https://crates.io/crates/rustls-pemfile) | 2.1     | Parse PEM files for TLS certs/keys     |
| [`lru`](https://crates.io/crates/lru)                       | 0.12    | Caching layer for responses            |
| [`dashmap`](https://crates.io/crates/dashmap)               | 5.5     | Thread-safe metrics store              |
| [`once_cell`](https://crates.io/crates/once_cell)           | 1.19    | Global cache and metric initialization |
| [`anyhow`](https://crates.io/crates/anyhow)                 | 1.0     | Simplified error handling              |

Add this to your `Cargo.toml`:

```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
hyper = { version = "1.3", features = ["server", "http2"] }
tokio-rustls = "0.25"
rustls = "0.23"
rustls-pemfile = "2.1"
dashmap = "5.5"
lru = "0.12"
once_cell = "1.19"
anyhow = "1.0"
```

🧠 Request Lifecycle

1 Tokio Listener - Accepts incoming TCP connections
2 Rustls TLS Handshake - Negotiates TLS (HTTPS) and ALPN (HTTP/1.1 or HTTP/2)
3 Hyper Connection - Upgrades socket to HTTP/1.1 or HTTP/2 stream
4 Service Handler - Invokes handle() for each request
5 Cache Lookup - Checks if path is cached (LruCache)
6 Backend Simulation - Simulates 2ms backend delay
7 Response Caching - Stores generated response in cache
8 Response Return - Sends back “Hello from Rust edge!”
9 Keep-alive - Connection stays open for further requests

📊 Benchmark Results

$ wrk -t12 -c400 -d30s --latency https://localhost:8443

Running 30s test @ https://localhost:8443
12 threads and 400 connections
Thread Stats Avg Stdev Max +/- Stdev
Latency 2.22ms 456.62us 47.52ms 97.78%
Req/Sec 14.78k 0.86k 18.90k 93.42%
Latency Distribution
50% 2.17ms
75% 2.25ms
90% 2.42ms
99% 2.94ms
5,295,495 requests in 30.02s, 494.92 MB read
Requests/sec: 176,408.32
Transfer/sec: 16.49 MB
