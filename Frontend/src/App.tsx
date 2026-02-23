import { useEffect, useState } from "react";
import {
  Chart as ChartJS,
  CategoryScale,
  LinearScale,
  BarElement,
  PointElement,
  LineElement,
  ArcElement, // <-- required for Pie chart
  Title,
  Tooltip,
  Legend,
} from "chart.js";
import { Line, Bar, Pie } from "react-chartjs-2";

// Register all required chart components
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
);

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

function App() {
  const [requests, setRequests] = useState<RequestMetrics[]>([]);
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

  // Prepare chart data
  const latencyData = {
    labels: requests.map((_, i) => i.toString()),
    datasets: [
      {
        label: "Latency (ms)",
        data: requests.map((r) => r.latency_ms),
        borderColor: "rgb(75, 192, 192)",
        backgroundColor: "rgba(75, 192, 192, 0.5)",
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
        label: "Requests per endpoint",
        data: Object.values(requestsPerEndpoint),
        backgroundColor: "rgba(255, 99, 132, 0.5)",
      },
    ],
  };

  const cacheData = {
    labels: ["Cache Hits", "Cache Misses"],
    datasets: [
      {
        label: "Cache",
        data: [summary.cache_hits, summary.cache_misses],
        backgroundColor: ["rgba(54, 162, 235, 0.5)", "rgba(255, 206, 86, 0.5)"],
      },
    ],
  };

  return (
    <div style={{ padding: "2rem", fontFamily: "Arial, sans-serif" }}>
      <h1>Rust Server Dashboard</h1>

      <div style={{ display: "flex", gap: "2rem", marginBottom: "1rem" }}>
        <div>Total Requests: {summary.total_requests}</div>
        <div>Total Bytes: {summary.total_bytes}</div>
        <div>Avg Latency: {summary.avg_latency_ms.toFixed(2)} ms</div>
        <div>Min Latency: {summary.min_latency_ms} ms</div>
        <div>Max Latency: {summary.max_latency_ms} ms</div>
        <div>Requests/sec: {(summary.rps * 30).toFixed(2)}</div>
        <div>Cache Hits: {summary.cache_hits}</div>
        <div>Cache Misses: {summary.cache_misses}</div>
        <div>Error Count: {summary.error_count}</div>
      </div>

      <h2>Charts</h2>
      <div style={{ display: "flex", gap: "2rem" }}>
        <div style={{ width: "400px" }}>
          <Line data={latencyData} redraw />
        </div>
        <div style={{ width: "400px" }}>
          <Bar data={endpointData} redraw />
        </div>aaa3
        <div style={{ width: "400px" }}>
          <Pie data={cacheData} redraw />
        </div>
      </div>

      <h2>Recent Requests (last 50)</h2>
      <table
        style={{
          borderCollapse: "collapse",
          width: "100%",
          marginTop: "0.5rem",
        }}
      >
        <thead>
          <tr>
            <th style={{ border: "1px solid #ccc", padding: "0.5rem" }}>
              Path
            </th>
            <th style={{ border: "1px solid #ccc", padding: "0.5rem" }}>
              Latency (ms)
            </th>
            <th style={{ border: "1px solid #ccc", padding: "0.5rem" }}>
              Bytes
            </th>
            <th style={{ border: "1px solid #ccc", padding: "0.5rem" }}>
              Status
            </th>
            <th style={{ border: "1px solid #ccc", padding: "0.5rem" }}>
              Method
            </th>
            <th style={{ border: "1px solid #ccc", padding: "0.5rem" }}>RPS</th>
            <th style={{ border: "1px solid #ccc", padding: "0.5rem" }}>
              CPU %
            </th>
            <th style={{ border: "1px solid #ccc", padding: "0.5rem" }}>
              Memory
            </th>
            <th style={{ border: "1px solid #ccc", padding: "0.5rem" }}>
              Active Conns
            </th>
            <th style={{ border: "1px solid #ccc", padding: "0.5rem" }}>
              Uptime (s)
            </th>
          </tr>
        </thead>
        <tbody>
          {requests.map((r, idx) => (
            <tr key={idx}>
              <td style={{ border: "1px solid #ccc", padding: "0.5rem" }}>
                {r.path}
              </td>
              <td style={{ border: "1px solid #ccc", padding: "0.5rem" }}>
                {r.latency_ms}
              </td>
              <td style={{ border: "1px solid #ccc", padding: "0.5rem" }}>
                {r.bytes}
              </td>
              <td style={{ border: "1px solid #ccc", padding: "0.5rem" }}>
                {r.status_code}
              </td>
              <td style={{ border: "1px solid #ccc", padding: "0.5rem" }}>
                {r.method}
              </td>
              <td style={{ border: "1px solid #ccc", padding: "0.5rem" }}>
                {r.request_rate.toFixed(2)}
              </td>
              <td style={{ border: "1px solid #ccc", padding: "0.5rem" }}>
                {r.cpu_usage_percent.toFixed(2)}
              </td>
              <td style={{ border: "1px solid #ccc", padding: "0.5rem" }}>
                {r.memory_usage_bytes}
              </td>
              <td style={{ border: "1px solid #ccc", padding: "0.5rem" }}>
                {r.active_connections}
              </td>
              <td style={{ border: "1px solid #ccc", padding: "0.5rem" }}>
                {r.uptime_seconds}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export default App;
