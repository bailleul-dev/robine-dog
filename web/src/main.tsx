import {
  IncidentList,
  IncidentPage,
  StatusPageSettings,
} from "./pages/Incidents";
import { Watchdog, WatchdogInsights } from "./pages/Watchdog";
import { NotebookList, NotebookPage } from "./pages/Notebooks";
import { Integrations } from "./pages/Integrations";
import { EventsExplorer, RumExplorer } from "./pages/Events";
import { RumAnalytics, RumPerformancePage, RumSessions } from "./pages/Rum";
import {
  RumFrustrations,
  RumFunnels,
  RumReplayPage,
  RumRetention,
} from "./pages/RumAnalysis";
import { SloPage, Slos } from "./pages/Slos";
import {
  SyntheticCi,
  Synthetics,
  SyntheticVariables,
} from "./pages/Synthetics";
import { Downtimes } from "./pages/Downtimes";
import { Containers, HostMap, Hosts, Kubernetes } from "./pages/Infrastructure";
import {
  QueryClient,
  QueryClientProvider,
  useQuery,
} from "@tanstack/react-query";
import { StrictMode, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";
import { Layout } from "./components/Layout";
import { api, ApiError } from "./lib/api";
import { ServiceCatalog, ServicePage } from "./pages/Apm";
import { LiveTail } from "./pages/LiveTail";
import { Login } from "./pages/Login";
import { LogsExplorer } from "./pages/LogsExplorer";
import {
  MetricsExplorer,
  MetricsSummary,
  MetricsVolume,
} from "./pages/Metrics";
import { DashboardList, DashboardPage } from "./pages/Dashboards";
import { ErrorTracking, IssuePage } from "./pages/ErrorTracking";
import {
  ManageMonitors,
  MonitorEditor,
  MonitorPage,
  TriggeredMonitors,
  Webhooks,
} from "./pages/Monitors";
import { GenerateMetrics } from "./pages/GenerateMetrics";
import { CiPipelinesPage, CiTestsPage } from "./pages/CiVisibility";
import { LlmOverview, LlmTraces } from "./pages/LlmObs";
import { AuditTrail, DetectionRules, SecuritySignals } from "./pages/Security";
import { Cases, OnCall } from "./pages/CasesOnCall";
import { Apps, Workflows } from "./pages/Workflows";
import {
  MonitorQuality,
  MonitorTemplates,
  Reports,
  Sheets,
} from "./pages/Reporting";
import { DataStreams, Databases, Processes } from "./pages/InfraAdvanced";
import { Profiler } from "./pages/Profiler";
import {
  IngestionControl,
  ResourcePage,
  RetentionFilters,
  Scorecards,
  Teams,
} from "./pages/Catalog";
import {
  LogArchives,
  LogIndexes,
  ReferenceTables,
  SensitiveDataScanner,
} from "./pages/LogsConfig";
import { Pipelines } from "./pages/Pipelines";
import { ServiceMap } from "./pages/ServiceMap";
import { TraceExplorer } from "./pages/TraceExplorer";
import { SharedDashboard } from "./pages/SharedDashboard";
import "./styles.css";
import { Coverage, Deployments, Dora } from "./pages/Delivery";
import {
  AppProtection,
  CloudSecurity,
  CodeSecurity,
  WorkloadProtection,
} from "./pages/AppSec";
import {
  CloudNetwork,
  DataJobs,
  Fleet,
  NetworkDevices,
  ResourceCatalog,
} from "./pages/InfraMore";
import { LlmAgents, LlmExperiments } from "./pages/LlmMore";
import { LogForwarding, MetricTags } from "./pages/Forwarding";
import { AgentSetup, ApiKeys, Help, OpenTelemetrySetup } from "./pages/Setup";

const client = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 5_000,
      refetchOnWindowFocus: false,
      retry: (count, err) =>
        !(err instanceof ApiError && err.status < 500) && count < 2,
    },
  },
});

function App() {
  const me = useQuery({
    queryKey: ["me"],
    queryFn: api.me,
    retry: false,
    staleTime: Infinity,
  });

  useEffect(() => {
    const onUnauth = () => client.invalidateQueries({ queryKey: ["me"] });
    window.addEventListener("robine-dog:unauthenticated", onUnauth);
    return () =>
      window.removeEventListener("robine-dog:unauthenticated", onUnauth);
  }, []);

  // robine-dog watches its own UI: with a client token configured, the
  // browser SDK reports the sessions of this page to this very server.
  const signedIn = !!me.data;
  useEffect(() => {
    if (!signedIn) return;
    let cancelled = false;
    void api
      .rumConfig()
      .then(async (c) => {
        if (!c.client_token || cancelled) return;
        const { datadogRum } = await import("@datadog/browser-rum");
        datadogRum.init({
          applicationId: "robine-dog",
          clientToken: c.client_token,
          proxy: `${window.location.origin}/rum/proxy`,
          service: "robine-dog-web",
          env: c.env ?? undefined,
          version: c.version,
          sessionSampleRate: 100,
          sessionReplaySampleRate: c.replay_sample_rate ?? 0,
          defaultPrivacyLevel: "mask-user-input",
          trackUserInteractions: true,
          trackResources: true,
          trackLongTasks: true,
        });
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [signedIn]);

  if (me.isLoading) return <div className="loading">Loading…</div>;
  if (!me.data || window.location.pathname === "/login") {
    if (me.data && window.location.pathname === "/login")
      return <Navigate to="/logs" replace />;
    return <Login />;
  }
  return (
    <Routes>
      <Route element={<Layout me={me.data} />}>
        <Route path="/logs" element={<LogsExplorer />} />
        <Route path="/logs/livetail" element={<LiveTail />} />
        <Route path="/logs/pipelines" element={<Pipelines />} />
        <Route
          path="/logs/generate-metrics"
          element={<GenerateMetrics source="logs" key="logs" />}
        />
        <Route path="/logs/indexes" element={<LogIndexes />} />
        <Route path="/logs/archives" element={<LogArchives />} />
        <Route
          path="/logs/sensitive-data-scanner"
          element={<SensitiveDataScanner />}
        />
        <Route path="/logs/reference-tables" element={<ReferenceTables />} />
        <Route
          path="/apm/generate-metrics"
          element={<GenerateMetrics source="spans" key="spans" />}
        />
        <Route
          path="/dashboard"
          element={<Navigate to="/dashboard/lists" replace />}
        />
        <Route path="/dashboard/lists" element={<DashboardList />} />
        <Route
          path="/dashboard/:id/:slug?"
          element={<DashboardPage key="dash" />}
        />
        <Route path="/error-tracking" element={<ErrorTracking />} />
        <Route path="/error-tracking/issue/:id" element={<IssuePage />} />
        <Route
          path="/monitors"
          element={<Navigate to="/monitors/manage" replace />}
        />
        <Route path="/monitors/manage" element={<ManageMonitors />} />
        <Route path="/monitors/triggered" element={<TriggeredMonitors />} />
        <Route path="/monitors/downtimes" element={<Downtimes />} />
        <Route path="/monitors/webhooks" element={<Webhooks />} />
        <Route
          path="/monitors/create"
          element={<MonitorEditor key="create" />}
        />
        <Route path="/monitors/:id" element={<MonitorPage />} />
        <Route
          path="/monitors/:id/edit"
          element={<MonitorEditor key="edit" />}
        />
        <Route path="/apm/services" element={<ServiceCatalog />} />
        <Route path="/apm/services/:name" element={<ServicePage />} />
        <Route path="/apm/services/:name/resource" element={<ResourcePage />} />
        <Route path="/apm/scorecards" element={<Scorecards />} />
        <Route path="/apm/retention-filters" element={<RetentionFilters />} />
        <Route path="/apm/ingestion-control" element={<IngestionControl />} />
        <Route path="/teams" element={<Teams />} />
        <Route path="/profiling" element={<Profiler />} />
        <Route path="/llm/overview" element={<LlmOverview />} />
        <Route path="/llm/traces" element={<LlmTraces />} />
        <Route path="/llm/experiments" element={<LlmExperiments />} />
        <Route path="/llm/agents" element={<LlmAgents />} />
        <Route path="/ci/pipelines" element={<CiPipelinesPage />} />
        <Route path="/ci/deployments" element={<Deployments />} />
        <Route path="/ci/dora" element={<Dora />} />
        <Route path="/ci/coverage" element={<Coverage />} />
        <Route path="/ci/tests" element={<CiTestsPage />} />
        <Route path="/security/signals" element={<SecuritySignals />} />
        <Route path="/security/appsec" element={<AppProtection />} />
        <Route path="/security/code" element={<CodeSecurity />} />
        <Route path="/security/workload" element={<WorkloadProtection />} />
        <Route path="/security/cloud" element={<CloudSecurity />} />
        <Route path="/security/rules" element={<DetectionRules />} />
        <Route path="/audit" element={<AuditTrail />} />
        <Route path="/cases" element={<Cases />} />
        <Route path="/on-call" element={<OnCall />} />
        <Route path="/workflows" element={<Workflows />} />
        <Route path="/watchdog/insights" element={<WatchdogInsights />} />
        <Route path="/monitors/templates" element={<MonitorTemplates />} />
        <Route path="/monitors/quality" element={<MonitorQuality />} />
        <Route path="/dashboard/reports" element={<Reports />} />
        <Route path="/sheets" element={<Sheets />} />
        <Route path="/apps" element={<Apps />} />
        <Route path="/processes" element={<Processes />} />
        <Route path="/data-streams" element={<DataStreams />} />
        <Route path="/databases" element={<Databases />} />
        <Route path="/apm/traces" element={<TraceExplorer />} />
        <Route path="/apm/map" element={<ServiceMap />} />
        <Route path="/synthetics/tests" element={<Synthetics />} />
        <Route path="/synthetics/variables" element={<SyntheticVariables />} />
        <Route path="/synthetics/ci" element={<SyntheticCi />} />
        <Route path="/slo/manage" element={<Slos />} />
        <Route path="/slo/:id" element={<SloPage />} />
        <Route path="/event/explorer" element={<EventsExplorer />} />
        <Route path="/integrations" element={<Integrations />} />
        <Route
          path="/rum"
          element={<Navigate to="/rum/performance-monitoring" replace />}
        />
        <Route
          path="/rum/performance-monitoring"
          element={<RumPerformancePage />}
        />
        <Route path="/rum/explorer" element={<RumExplorer />} />
        <Route path="/rum/sessions" element={<RumSessions />} />
        <Route path="/rum/analytics" element={<RumAnalytics />} />
        <Route path="/rum/replay/:id" element={<RumReplayPage />} />
        <Route path="/rum/frustrations" element={<RumFrustrations />} />
        <Route path="/rum/funnels" element={<RumFunnels />} />
        <Route path="/rum/retention" element={<RumRetention />} />
        <Route
          path="/rum/generate-metrics"
          element={<GenerateMetrics source="rum" key="rum" />}
        />
        <Route path="/watchdog" element={<Watchdog />} />
        <Route path="/incidents" element={<IncidentList />} />
        <Route path="/incidents/:id" element={<IncidentPage />} />
        <Route path="/status-pages" element={<StatusPageSettings />} />
        <Route path="/notebook/list" element={<NotebookList />} />
        <Route path="/notebook/:id" element={<NotebookPage />} />
        <Route path="/infrastructure" element={<Hosts />} />
        <Route path="/resources" element={<ResourceCatalog />} />
        <Route path="/network" element={<CloudNetwork />} />
        <Route path="/devices" element={<NetworkDevices />} />
        <Route path="/fleet" element={<Fleet />} />
        <Route path="/data-jobs" element={<DataJobs />} />
        <Route path="/infrastructure/map" element={<HostMap />} />
        <Route path="/containers" element={<Containers />} />
        <Route
          path="/orchestration"
          element={<Navigate to="/orchestration/overview/pod" replace />}
        />
        {[
          "pod",
          "deployment",
          "statefulset",
          "daemonset",
          "node",
          "namespace",
        ].map((k) => (
          <Route
            key={k}
            path={`/orchestration/overview/${k}`}
            element={<Kubernetes kind={k} />}
          />
        ))}
        <Route
          path="/metrics"
          element={<Navigate to="/metrics/explorer" replace />}
        />
        <Route path="/metrics/explorer" element={<MetricsExplorer />} />
        <Route path="/metrics/summary" element={<MetricsSummary />} />
        <Route path="/metrics/volume" element={<MetricsVolume />} />
        <Route path="/metrics/tags" element={<MetricTags />} />
        <Route path="/organization-settings/api-keys" element={<ApiKeys />} />
        <Route path="/organization-settings/agent" element={<AgentSetup />} />
        <Route
          path="/organization-settings/opentelemetry"
          element={<OpenTelemetrySetup />}
        />
        <Route path="/help" element={<Help />} />
        <Route path="/logs/forwarding" element={<LogForwarding />} />
        <Route path="*" element={<Navigate to="/logs" replace />} />
      </Route>
    </Routes>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <QueryClientProvider client={client}>
      <BrowserRouter>
        <Routes>
          {/* Public dashboard links: no session. */}
          <Route path="/p/:token" element={<SharedDashboard />} />
          <Route path="*" element={<App />} />
        </Routes>
      </BrowserRouter>
    </QueryClientProvider>
  </StrictMode>,
);
