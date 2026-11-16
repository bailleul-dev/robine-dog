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
import { MetricsExplorer, MetricsSummary } from "./pages/Metrics";
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
        <Route path="/apm/services" element={<ServiceCatalog />} />
        <Route path="/apm/services/:name" element={<ServicePage />} />
        <Route path="/apm/traces" element={<TraceExplorer />} />
        <Route path="/metrics" element={<Navigate to="/metrics/explorer" replace />} />
        <Route path="/metrics/explorer" element={<MetricsExplorer />} />
        <Route path="/metrics/summary" element={<MetricsSummary />} />
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
