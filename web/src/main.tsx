import { Integrations } from "./pages/Integrations";
import { EventsExplorer, RumExplorer } from "./pages/Events";
import { RumPerformancePage } from "./pages/Rum";
import { Slos } from "./pages/Slos";
import { Synthetics } from "./pages/Synthetics";
import { Downtimes } from "./pages/Downtimes";
import { Containers, Hosts, Kubernetes } from "./pages/Infrastructure";
import { QueryClient, QueryClientProvider, useQuery } from "@tanstack/react-query";
import { StrictMode, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";
import { Layout } from "./components/Layout";
import { api, ApiError } from "./lib/api";
import { ServiceCatalog, ServicePage } from "./pages/Apm";
import { LiveTail } from "./pages/LiveTail";
import { Login } from "./pages/Login";
import { LogsExplorer } from "./pages/LogsExplorer";
import { MetricsExplorer, MetricsSummary, MetricsVolume } from "./pages/Metrics";
import { DashboardList, DashboardPage } from "./pages/Dashboards";
import { ErrorTracking, IssuePage } from "./pages/ErrorTracking";
import { ManageMonitors, MonitorEditor, MonitorPage, TriggeredMonitors, Webhooks } from "./pages/Monitors";
import { GenerateMetrics } from "./pages/GenerateMetrics";
import { Pipelines } from "./pages/Pipelines";
import { ServiceMap } from "./pages/ServiceMap";
import { TraceExplorer } from "./pages/TraceExplorer";
import "./styles.css";

const client = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 5_000,
      refetchOnWindowFocus: false,
      retry: (count, err) => !(err instanceof ApiError && err.status < 500) && count < 2,
    },
  },
});

function App() {
  const me = useQuery({ queryKey: ["me"], queryFn: api.me, retry: false, staleTime: Infinity });

  useEffect(() => {
    const onUnauth = () => client.invalidateQueries({ queryKey: ["me"] });
    window.addEventListener("robine-dog:unauthenticated", onUnauth);
    return () => window.removeEventListener("robine-dog:unauthenticated", onUnauth);
  }, []);

  if (me.isLoading) return <div className="loading">Loading…</div>;
  if (!me.data || window.location.pathname === "/login") {
    if (me.data && window.location.pathname === "/login") return <Navigate to="/logs" replace />;
    return <Login />;
  }
  return (
    <Routes>
      <Route element={<Layout me={me.data} />}>
        <Route path="/logs" element={<LogsExplorer />} />
        <Route path="/logs/livetail" element={<LiveTail />} />
        <Route path="/logs/pipelines" element={<Pipelines />} />
        <Route path="/logs/generate-metrics" element={<GenerateMetrics source="logs" key="logs" />} />
        <Route path="/apm/generate-metrics" element={<GenerateMetrics source="spans" key="spans" />} />
        <Route path="/dashboard" element={<Navigate to="/dashboard/lists" replace />} />
        <Route path="/dashboard/lists" element={<DashboardList />} />
        <Route path="/dashboard/:id/:slug?" element={<DashboardPage key="dash" />} />
        <Route path="/error-tracking" element={<ErrorTracking />} />
        <Route path="/error-tracking/issue/:id" element={<IssuePage />} />
        <Route path="/monitors" element={<Navigate to="/monitors/manage" replace />} />
        <Route path="/monitors/manage" element={<ManageMonitors />} />
        <Route path="/monitors/triggered" element={<TriggeredMonitors />} />
        <Route path="/monitors/downtimes" element={<Downtimes />} />
        <Route path="/monitors/webhooks" element={<Webhooks />} />
        <Route path="/monitors/create" element={<MonitorEditor key="create" />} />
        <Route path="/monitors/:id" element={<MonitorPage />} />
        <Route path="/monitors/:id/edit" element={<MonitorEditor key="edit" />} />
        <Route path="/apm/services" element={<ServiceCatalog />} />
        <Route path="/apm/services/:name" element={<ServicePage />} />
        <Route path="/apm/traces" element={<TraceExplorer />} />
        <Route path="/apm/map" element={<ServiceMap />} />
        <Route path="/synthetics/tests" element={<Synthetics />} />
        <Route path="/slo/manage" element={<Slos />} />
        <Route path="/event/explorer" element={<EventsExplorer />} />
        <Route path="/integrations" element={<Integrations />} />
        <Route path="/rum" element={<Navigate to="/rum/performance-monitoring" replace />} />
        <Route path="/rum/performance-monitoring" element={<RumPerformancePage />} />
        <Route path="/rum/explorer" element={<RumExplorer />} />
        <Route path="/infrastructure" element={<Hosts />} />
        <Route path="/containers" element={<Containers />} />
        <Route path="/orchestration" element={<Navigate to="/orchestration/overview/pod" replace />} />
        {["pod", "deployment", "statefulset", "daemonset", "node", "namespace"].map((k) => (
          <Route key={k} path={`/orchestration/overview/${k}`} element={<Kubernetes kind={k} />} />
        ))}
        <Route path="/metrics" element={<Navigate to="/metrics/explorer" replace />} />
        <Route path="/metrics/explorer" element={<MetricsExplorer />} />
        <Route path="/metrics/summary" element={<MetricsSummary />} />
        <Route path="/metrics/volume" element={<MetricsVolume />} />
        <Route path="*" element={<Navigate to="/logs" replace />} />
      </Route>
    </Routes>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <QueryClientProvider client={client}>
      <BrowserRouter>
        <App />
      </BrowserRouter>
    </QueryClientProvider>
  </StrictMode>,
);
