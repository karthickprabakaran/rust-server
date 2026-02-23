import { useEffect, useState } from "react";
import { motion } from "framer-motion";
import {
  Chart as ChartJS,
  CategoryScale,
  LinearScale,
  BarElement,
  PointElement,
  LineElement,
  ArcElement,
  Title,
  Tooltip,
  Legend,
  Filler,
} from "chart.js";
import { Line, Bar, Pie } from "react-chartjs-2";
import "./App.css";

ChartJS.register(
  CategoryScale,
  LinearScale,
  PointElement,
  LineElement,
  BarElement,
  ArcElement,
  Title,
  Tooltip,
  Legend,
  Filler,
);

/* Theme-aligned chart defaults */
const chartColors = {
  primary: "rgba(99, 102, 241, 0.9)",
  primaryFill: "rgba(99, 102, 241, 0.15)",
  success: "rgba(34, 197, 94, 0.9)",
  warning: "rgba(245, 158, 11, 0.9)",
  pink: "rgba(236, 72, 153, 0.9)",
  cyan: "rgba(6, 182, 212, 0.9)",
};

const chartDefaults = {
  responsive: true,
  maintainAspectRatio: false,
  plugins: {
    legend: {
      labels: {
        color: "rgba(161, 161, 170, 0.9)",
        font: { family: "'Plus Jakarta Sans', sans-serif", size: 11 },
      },
    },
  },
  scales: {
    x: {
      grid: { color: "rgba(255,255,255,0.04)" },
      ticks: { color: "rgba(161, 161, 170, 0.7)", maxTicksLimit: 8 },
    },
    y: {
      grid: { color: "rgba(255,255,255,0.04)" },
      ticks: { color: "rgba(161, 161, 170, 0.7)" },
    },
  },
};

interface RequestMetrics {
  path: string;
  latency_ms: number;
  bytes: number;
  status_code: number;
  method: string;
  request_rate: number;
  cpu_usage_percent: number;
  memory_usage_bytes: number;
  active_connections: number;
  uptime_seconds: number;
}

interface Summary {
  total_requests: number;
  total_bytes: number;
  avg_latency_ms: number;
  min_latency_ms: number;
  max_latency_ms: number;
  rps: number;
  cache_hits: number;
  cache_misses: number;
  error_count: number;
}

interface IpStatsResponse {
  blocked_ips: string[];
  requests_by_ip: { ip: string; count: number }[];
}

const API_BASE = import.meta.env.VITE_API_URL || "http://localhost:10000";

function formatBytes(n: number): string {
  if (n >= 1e9) return (n / 1e9).toFixed(2) + " GB";
  if (n >= 1e6) return (n / 1e6).toFixed(2) + " MB";
  if (n >= 1e3) return (n / 1e3).toFixed(2) + " KB";
  return n + " B";
}

function latencyClass(ms: number): string {
  if (ms <= 10) return "status-ok";
  if (ms <= 50) return "status-warn";
  return "status-err";
}

const container = {
  hidden: { opacity: 0 },
  show: {
    opacity: 1,
    transition: { staggerChildren: 0.04, delayChildren: 0.1 },
  },
};

const item = {
  hidden: { opacity: 0, y: 8 },
  show: { opacity: 1, y: 0 },
};

function App() {
  const [requests, setRequests] = useState<RequestMetrics[]>([]);
  const [ipStats, setIpStats] = useState<IpStatsResponse | null>(null);
  const [ipStatsError, setIpStatsError] = useState<string | null>(null);
  const [summary, setSummary] = useState<Summary>({
    total_requests: 0,
    total_bytes: 0,
    avg_latency_ms: 0,
    min_latency_ms: 0,
    max_latency_ms: 0,
    rps: 0,
    cache_hits: 0,
    cache_misses: 0,
    error_count: 0,
  });

  useEffect(() => {
    const ws = new WebSocket("ws://localhost:9000");

    ws.onmessage = (event: MessageEvent) => {
      const data: RequestMetrics = JSON.parse(event.data);

      setRequests((prev) => {
        const updated = [data, ...prev];
        return updated.slice(0, 50);
      });

      setSummary((prev) => {
        const totalRequests = prev.total_requests + 1;
        const totalBytes = prev.total_bytes + data.bytes;
        const avgLatency =
          (prev.avg_latency_ms * prev.total_requests + data.latency_ms) /
          totalRequests;
        const minLatency = Math.min(
          prev.min_latency_ms || data.latency_ms,
          data.latency_ms,
        );
        const maxLatency = Math.max(
          prev.max_latency_ms || data.latency_ms,
          data.latency_ms,
        );

        return {
          ...prev,
          total_requests: totalRequests,
          total_bytes: totalBytes,
          avg_latency_ms: avgLatency,
          min_latency_ms: minLatency,
          max_latency_ms: maxLatency,
          rps: data.request_rate,
          cache_hits: prev.cache_hits,
          cache_misses: prev.cache_misses,
          error_count: prev.error_count,
        };
      });
    };

    ws.onclose = () => console.log("WebSocket closed");
    return () => ws.close();
  }, []);

  // Poll IP stats (blocked list + requests by IP)
  useEffect(() => {
    const fetchIpStats = async () => {
      try {
        const res = await fetch(`${API_BASE}/api/ip-stats`);
        if (res.ok) {
          const data: IpStatsResponse = await res.json();
          setIpStats(data);
          setIpStatsError(null);
        } else {
          setIpStats({ blocked_ips: [], requests_by_ip: [] });
          setIpStatsError(`Backend returned ${res.status}`);
        }
      } catch (e) {
        setIpStats({ blocked_ips: [], requests_by_ip: [] });
        setIpStatsError("Backend unreachable (check URL and CORS)");
      }
    };
    fetchIpStats();
    const interval = setInterval(fetchIpStats, 5000);
    return () => clearInterval(interval);
  }, []);

  const latencyData = {
    labels: requests.map((_, i) => i.toString()),
    datasets: [
      {
        label: "Latency (ms)",
        data: requests.map((r) => r.latency_ms),
        borderColor: chartColors.primary,
        backgroundColor: chartColors.primaryFill,
        fill: true,
        tension: 0.35,
        pointRadius: 0,
        pointHoverRadius: 4,
      },
    ],
  };

  const requestsPerEndpoint: Record<string, number> = {};
  requests.forEach((r) => {
    requestsPerEndpoint[r.path] = (requestsPerEndpoint[r.path] || 0) + 1;
  });

  const endpointData = {
    labels: Object.keys(requestsPerEndpoint),
    datasets: [
      {
        label: "Requests",
        data: Object.values(requestsPerEndpoint),
        backgroundColor: [
          chartColors.primary,
          chartColors.success,
          chartColors.warning,
          chartColors.pink,
          chartColors.cyan,
        ],
      },
    ],
  };

  const cacheData = {
    labels: ["Cache Hits", "Cache Misses"],
    datasets: [
      {
        data: [summary.cache_hits, summary.cache_misses],
        backgroundColor: [chartColors.success, chartColors.warning],
        borderWidth: 0,
      },
    ],
  };

  const kpis = [
    { label: "Total Requests", value: summary.total_requests.toLocaleString() },
    { label: "Total Bytes", value: formatBytes(summary.total_bytes) },
    { label: "Avg Latency", value: `${summary.avg_latency_ms.toFixed(2)} ms` },
    { label: "Min Latency", value: `${summary.min_latency_ms} ms` },
    { label: "Max Latency", value: `${summary.max_latency_ms} ms` },
    { label: "Req/s", value: (summary.rps * 30).toFixed(2) },
    { label: "Cache Hits", value: summary.cache_hits.toLocaleString() },
    { label: "Cache Misses", value: summary.cache_misses.toLocaleString() },
    { label: "Errors", value: summary.error_count.toLocaleString() },
  ];

  return (
    <motion.div
      className="dashboard"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: 0.4 }}
    >
      <div className="dashboard-header">
        <h1 className="dashboard-title">
          Rust Server
          <span className="live-badge">
            <span className="live-dot" />
            Live
          </span>
        </h1>
      </div>

      <motion.div
        className="kpi-grid"
        variants={container}
        initial="hidden"
        animate="show"
      >
        {kpis.map((k) => (
          <motion.div
            key={k.label}
            className="kpi-card"
            variants={item}
            transition={{ type: "spring", stiffness: 400, damping: 25 }}
          >
            <div className="kpi-label">{k.label}</div>
            <div className="kpi-value mono">{k.value}</div>
          </motion.div>
        ))}
      </motion.div>

      <section className="section">
        <h2 className="section-title">Charts</h2>
        <div className="charts-grid">
          <motion.div
            className="chart-card"
            initial={{ opacity: 0, y: 16 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ delay: 0.15, duration: 0.35 }}
          >
            <h3>Latency (ms)</h3>
            <div style={{ height: 220 }}>
              <Line
                data={latencyData}
                options={{ ...chartDefaults }}
                redraw
              />
            </div>
          </motion.div>
          <motion.div
            className="chart-card"
            initial={{ opacity: 0, y: 16 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ delay: 0.2, duration: 0.35 }}
          >
            <h3>Requests per endpoint</h3>
            <div style={{ height: 220 }}>
              <Bar
                data={endpointData}
                options={{
                  ...chartDefaults,
                  plugins: {
                    ...chartDefaults.plugins,
                    legend: { display: false },
                  },
                }}
                redraw
              />
            </div>
          </motion.div>
          <motion.div
            className="chart-card"
            initial={{ opacity: 0, y: 16 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ delay: 0.25, duration: 0.35 }}
          >
            <h3>Cache</h3>
            <div style={{ height: 220 }}>
              <Pie
                data={cacheData}
                options={{
                  responsive: true,
                  maintainAspectRatio: false,
                  plugins: {
                    legend: {
                      position: "bottom",
                      labels: {
                        color: "rgba(161, 161, 170, 0.9)",
                        font: { family: "'Plus Jakarta Sans', sans-serif", size: 11 },
                      },
                    },
                  },
                }}
                redraw
              />
            </div>
          </motion.div>
        </div>
      </section>

      <section className="section">
        <h2 className="section-title">Blocked IPs & requests by IP</h2>
        {ipStatsError && (
          <p className="ip-stats-error" style={{ margin: 0, fontSize: "0.8125rem", color: "var(--warning)" }}>
            {ipStatsError}
          </p>
        )}
        <div className="ip-stats-grid">
          <motion.div
            className="table-wrap table-wrap-small"
            initial={{ opacity: 0, y: 16 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ delay: 0.28, duration: 0.35 }}
          >
            <h3 className="table-subtitle">Blocked IPs</h3>
            <table>
              <thead>
                <tr>
                  <th>IP address</th>
                </tr>
              </thead>
              <tbody>
                {!ipStats ? (
                  <tr>
                    <td className="empty-state">Loading…</td>
                  </tr>
                ) : ipStats.blocked_ips.length === 0 ? (
                  <tr>
                    <td className="empty-state">No blocked IPs</td>
                  </tr>
                ) : (
                  ipStats.blocked_ips.map((ip) => (
                    <tr key={ip}>
                      <td className="mono status-err">{ip}</td>
                    </tr>
                  ))
                )}
              </tbody>
            </table>
          </motion.div>
          <motion.div
            className="table-wrap table-wrap-flex"
            initial={{ opacity: 0, y: 16 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ delay: 0.32, duration: 0.35 }}
          >
            <h3 className="table-subtitle">Requests by IP</h3>
            <table>
              <thead>
                <tr>
                  <th>IP address</th>
                  <th>Requests</th>
                </tr>
              </thead>
              <tbody>
                {!ipStats ? (
                  <tr>
                    <td colSpan={2} className="empty-state">
                      Loading…
                    </td>
                  </tr>
                ) : ipStats.requests_by_ip.length === 0 ? (
                  <tr>
                    <td colSpan={2} className="empty-state">
                      No requests yet
                    </td>
                  </tr>
                ) : (
                  ipStats.requests_by_ip.map((row) => (
                    <tr key={row.ip}>
                      <td className="mono" style={{ color: "var(--text-primary)" }}>
                        {row.ip}
                      </td>
                      <td className="num">{row.count.toLocaleString()}</td>
                    </tr>
                  ))
                )}
              </tbody>
            </table>
          </motion.div>
        </div>
      </section>

      <section className="section">
        <h2 className="section-title">Recent requests (last 50)</h2>
        <motion.div
          className="table-wrap"
          initial={{ opacity: 0, y: 16 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ delay: 0.3, duration: 0.35 }}
        >
          <table>
            <thead>
              <tr>
                <th>Path</th>
                <th>Latency</th>
                <th>Bytes</th>
                <th>Status</th>
                <th>Method</th>
                <th>RPS</th>
                <th>CPU %</th>
                <th>Memory</th>
                <th>Conns</th>
                <th>Uptime</th>
              </tr>
            </thead>
            <tbody>
              {requests.length === 0 ? (
                <tr>
                  <td colSpan={10} className="empty-state">
                    Waiting for live metrics…
                  </td>
                </tr>
              ) : (
                requests.map((r, idx) => (
                  <motion.tr
                    key={`${r.path}-${r.latency_ms}-${idx}`}
                    initial={{ opacity: 0, x: -8 }}
                    animate={{ opacity: 1, x: 0 }}
                    transition={{ duration: 0.25 }}
                  >
                    <td className="mono" style={{ color: "var(--text-primary)" }}>
                      {r.path}
                    </td>
                    <td className={`num ${latencyClass(r.latency_ms)}`}>
                      {r.latency_ms} ms
                    </td>
                    <td className="num">{formatBytes(r.bytes)}</td>
                    <td className="num">{r.status_code}</td>
                    <td className="mono">{r.method}</td>
                    <td className="num">{r.request_rate.toFixed(2)}</td>
                    <td className="num">{r.cpu_usage_percent.toFixed(2)}</td>
                    <td className="num">{formatBytes(r.memory_usage_bytes)}</td>
                    <td className="num">{r.active_connections}</td>
                    <td className="num">{r.uptime_seconds}s</td>
                  </motion.tr>
                ))
              )}
            </tbody>
          </table>
        </motion.div>
      </section>
    </motion.div>
  );
}

export default App;
