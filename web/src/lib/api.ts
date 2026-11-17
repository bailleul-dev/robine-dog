export type Log = {
  id: number;
  timestamp: number;
  status: string;
  service: string | null;
  host: string | null;
  source: string | null;
  message: string;
  tags: string[];
  attributes: Record<string, unknown>;
  trace_id: string | null;
  span_id: string | null;
};

export type Span = {
  id: number;
  trace_id: string;
  span_id: string;
  parent_id: string;
  service: string;
  name: string;
  resource: string;
  type: string | null;
  env: string | null;
  version: string | null;
  start: number; // ns
  duration: number; // ns
  error: boolean;
  top_level: boolean;
  meta: Record<string, string>;
  metrics: Record<string, number>;
};

export type Bucket = { t: number; counts: Record<string, number> };
export type Histogram = { interval_ms: number; buckets: Bucket[] };
export type FacetValues = {
  path: string;
  values: { value: string; count: number }[];
  min: number | null;
  max: number | null;
};
export type FacetDef = {
  id: number;
  dataset: string;
  path: string;
  name: string;
  group: string;
};

export type Stats = {
  hits: number;
  errors: number;
  error_rate: number;
  p50: number;
  p75: number;
  p90: number;
  p95: number;
  p99: number;
  max: number;
  avg: number;
  total_time: number;
};

export type ServiceSummary = Stats & {
  name: string;
  type: string;
  operation: string;
  requests_per_second: number;
  sparkline: number[];
};

export type ServiceEdge = {
  source: string;
  target: string;
  calls: number;
  errors: number;
  avg: number;
};

export type ServiceDetail = {
  name: string;
  type: string;
  operations: string[];
  summary: Stats;
  interval_ms: number;
  timeseries: {
    t: number;
    hits: number;
    errors: number;
    p50: number;
    p95: number;
    p99: number;
    max: number;
  }[];
  resources: (Stats & { resource: string; operation: string })[];
  distribution: [number, number][];
  downstream: { service: string; calls: number; errors: number; avg: number }[];
};

export type MetricMeta = {
  name: string;
  type: string;
  unit: string | null;
  description: string | null;
  last_seen: number;
};
export type MetricDetail = MetricMeta & {
  hosts: string[];
  tags: Record<string, string[]>;
  series: number;
};
export type SyntheticAssertion = { type: string; operator: string; property?: string | null; target: string | number };
export type SyntheticTest = {
  public_id?: string;
  name: string;
  status: string;
  config: {
    request: { method: string; url: string; headers?: Record<string, string>; body?: string | null; timeout?: number | null };
    assertions: SyntheticAssertion[];
  };
  options: { tick_every?: number | null; min_failure_duration?: number | null };
  monitor_status?: string;
  message: string;
  tags: string[];
  last_result?: { ts: number; passed: boolean; status_code: number | null; duration_ms: number; failure: string | null };
};
export type Slo = {
  id?: string;
  name: string;
  description?: string | null;
  type?: string;
  query: { numerator: string; denominator: string };
  thresholds: { timeframe: string; target: number; warning?: number | null }[];
  tags: string[];
};
export type RumPage = {
  service: string | null;
  name: string;
  views: number;
  loading_time: number | null;
  largest_contentful_paint: number | null;
  first_contentful_paint: number | null;
  interaction_to_next_paint: number | null;
  cumulative_layout_shift: number | null;
  errors: number;
};
export type RumSession = {
  id: string;
  start: number;
  end: number;
  service: string | null;
  user: string | null;
  views: number;
  actions: number;
  errors: number;
  first_view: string | null;
};
export type RumPerformance = { sessions: number; views: number; errors: number; actions: number; pages: RumPage[] };
export type Incident = {
  id: string;
  public_id: number;
  title: string;
  severity: string;
  state: string;
  created: number;
  modified: number;
  resolved: number | null;
  commander: string | null;
  summary: string;
  services: string[];
  public: boolean;
  timeline: { ts: number; kind: "declared" | "note" | "state" | "severity"; text: string; by: string | null }[];
};
export type StatusComponent = { name: string; kind: "synthetic" | "monitor"; ref: string; status?: string; uptime?: number | null };
export type StatusPage = { enabled: boolean; title: string; description: string; components: StatusComponent[] };
export type WatchdogStory = {
  id: string;
  kind: "error_logs" | "apm_errors" | "apm_latency";
  service: string;
  title: string;
  start: number;
  last_seen: number;
  end: number | null;
  value: number;
  baseline: number;
  unit: string;
};
export type NotebookCell = { id: string; type: "markdown" | "metric" | "logs"; text: string };
export type Notebook = { id?: string; name: string; cells: NotebookCell[] };
export type Downtime = {
  id: string;
  scope: string;
  monitor_id: number | null;
  monitor_tags: string[];
  start: number;
  end: number | null;
  message: string | null;
  status: "scheduled" | "active" | "ended" | "canceled";
};
export type HostInfo = {
  name: string;
  tags: string[];
  aliases: string[];
  agent_version?: string;
  os?: string;
};
export type MetricSeries = {
  query_index: number;
  group: Record<string, string>;
  points: [number, number][];
};
export type MetricQueryResult = {
  query: string;
  metric: string | null;
  type: string | null;
  unit: string | null;
  error: string | null;
};
export type MetricsResponse = {
  interval_ms: number;
  results: MetricQueryResult[];
  series: MetricSeries[];
};

export type Processor = {
  type: string;
  name: string;
  is_enabled: boolean;
  [k: string]: unknown;
};
export type Pipeline = {
  id?: string;
  type: "pipeline";
  name: string;
  is_enabled: boolean;
  filter: { query: string };
  processors: Processor[];
  is_read_only: boolean;
  description?: string;
};
export type AggregateBucket = {
  by: Record<string, string>;
  computes: Record<
    string,
    number | null | { time: string; value: number | null }[]
  >;
};
export type AggregateResponse = {
  data: { buckets: AggregateBucket[] };
  meta: { status: string; elapsed: number; interval_ms: number | null };
};
export type AggregateRequest = {
  compute: {
    aggregation: string;
    metric?: string;
    type?: string;
    interval?: string;
  }[];
  filter: { query: string; from: number | string; to: number | string };
  group_by: { facet: string; limit?: number }[];
};
export type LogPattern = {
  pattern: string;
  count: number;
  status: string;
  statuses: Record<string, number>;
  services: [string, number][];
  sample: string;
  sample_id: number;
  query: string;
};
export type DashQuery = {
  data_source: string;
  name: string;
  query?: string;
  aggregator?: string;
  search?: { query: string };
  compute?: { aggregation: string; metric?: string };
  group_by?: { facet: string; limit?: number }[];
};
export type DashFormula = {
  formula: string;
  alias?: string;
  limit?: { count?: number; order?: string };
};
export type WidgetRequest = {
  queries?: DashQuery[];
  formulas?: DashFormula[];
  response_format?: string;
  display_type?: string;
  /** Legacy metric request. */
  q?: string;
  [k: string]: unknown;
};
export type ConditionalFormat = {
  comparator: string;
  value: number;
  palette: string;
};
export type WidgetDef = {
  type: string;
  title?: string;
  requests?: WidgetRequest[];
  content?: string;
  background_color?: string;
  precision?: number;
  custom_unit?: string;
  conditional_formats?: ConditionalFormat[];
  widgets?: Widget[];
  show_legend?: boolean;
  [k: string]: unknown;
};
export type WidgetLayout = {
  x: number;
  y: number;
  width: number;
  height: number;
};
export type Widget = {
  id?: number;
  definition: WidgetDef;
  layout?: WidgetLayout;
};
export type TemplateVar = {
  name: string;
  prefix?: string | null;
  default?: string;
  available_values?: string[];
};
export type Dashboard = {
  id: string;
  title: string;
  description: string | null;
  layout_type: string;
  widgets: Widget[];
  template_variables: TemplateVar[];
  url: string;
  created_at: string;
  modified_at: string;
  [k: string]: unknown;
};
export type DashboardSummary = Pick<
  Dashboard,
  | "id"
  | "title"
  | "description"
  | "url"
  | "created_at"
  | "modified_at"
  | "layout_type"
> & { widget_count: number };
export type QueryRequest = {
  from: number;
  to: number;
  queries: DashQuery[];
  formulas?: DashFormula[];
};
export type TimeseriesResponse = {
  data: {
    attributes: {
      times: number[];
      interval: number;
      series: { query_index: number; name: string; group_tags: string[] }[];
      values: (number | null)[][];
    };
  };
};
export type ScalarColumn =
  | { type: "group"; name: string; values: string[][] }
  | {
      type: "number";
      name: string;
      query_index: number;
      values: (number | null)[];
    };
export type ScalarResponse = {
  data: { attributes: { columns: ScalarColumn[] } };
};
export type ErrorIssue = {
  id: string;
  source: "spans" | "logs";
  service: string;
  type: string;
  message: string;
  pattern: string;
  frame: string;
  count: number;
  first_seen: number;
  last_seen: number;
  status: string;
  regression: boolean;
  series: number[];
};
export type ErrorOccurrence = {
  ts: number;
  id: string | null;
  trace_id: string | null;
  span_id: string | null;
  host: string | null;
  message: string | null;
  stack: string | null;
  resource?: string | null;
  env?: string | null;
  version?: string | null;
};
export type ErrorIssueDetail = Omit<ErrorIssue, "message" | "series"> & {
  interval: number;
  series: [number, number][];
  occurrences: ErrorOccurrence[];
  breakdown: Record<string, [string, number][]>;
  query: string;
};
export type SavedView = {
  id: string;
  name: string;
  page: string;
  params: string;
};
export type GeneratedMetricAttributes = {
  compute: {
    aggregation_type: string;
    path?: string;
    include_percentiles?: boolean;
  };
  filter: { query: string };
  group_by: { path: string; tag_name?: string }[];
};
export type GeneratedMetric = {
  id: string;
  type: string;
  attributes: GeneratedMetricAttributes;
};
export type MonitorOptions = {
  thresholds: {
    critical?: number;
    warning?: number;
    critical_recovery?: number;
    warning_recovery?: number;
  };
  notify_no_data?: boolean;
  no_data_timeframe?: number;
  renotify_interval?: number;
  evaluation_delay?: number;
  include_tags?: boolean;
  silenced?: Record<string, number | null>;
  [k: string]: unknown;
};
export type MonitorGroup = {
  name: string;
  status: string;
  value: number | null;
  last_triggered_ts: number | null;
  last_notified_ts: number | null;
  last_resolved_ts: number | null;
  last_data_ts: number | null;
  since_ts: number;
};
export type Monitor = {
  id: number;
  name: string;
  type: string;
  query: string;
  message: string;
  tags: string[];
  priority: number | null;
  options: MonitorOptions;
  overall_state: string;
  overall_state_modified: string | null;
  created: string;
  modified: string;
  multi: boolean;
  last_triggered_ts: number | null;
  state: { groups: Record<string, MonitorGroup> };
};
export type MonitorDraft = Pick<
  Monitor,
  "name" | "type" | "query" | "message" | "tags" | "priority" | "options"
>;
export type MonitorEvent = {
  id: number;
  monitor_id: number;
  group: string;
  ts: number;
  from_status: string;
  to_status: string;
  value: number | null;
  title: string;
  message: string;
  notified: string[];
};
export type MonitorPreview = {
  type: string;
  window_ms: number;
  comparator: string;
  threshold: number;
  groups: { group: string; value: number | null; status: string }[];
};
export type Webhook = {
  name: string;
  url: string;
  payload: string | null;
  custom_headers: string | null;
  encode_as: string;
};
export type GrokSample = {
  sample: string;
  rule: string | null;
  attributes: Record<string, unknown>;
};

export type Identity = {
  sub: string;
  name: string | null;
  email: string | null;
};

export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
  }
}

async function call<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`/api/ui${path}`, {
    ...init,
    headers: { "content-type": "application/json", ...(init?.headers ?? {}) },
    credentials: "same-origin",
  });
  if (res.status === 401) {
    window.dispatchEvent(new CustomEvent("robine-dog:unauthenticated"));
  }
  if (!res.ok) {
    let msg = res.statusText;
    try {
      msg = (await res.json()).error ?? msg;
    } catch {
      /* not JSON */
    }
    throw new ApiError(res.status, msg);
  }
  if (res.status === 204) return undefined as T;
  return res.json();
}

const post = <T>(path: string, body: unknown, signal?: AbortSignal) =>
  call<T>(path, { method: "POST", body: JSON.stringify(body), signal });
const put = <T>(path: string, body: unknown) =>
  call<T>(path, { method: "PUT", body: JSON.stringify(body) });
const del = <T>(path: string) => call<T>(path, { method: "DELETE" });

export type Scope = { query: string; from: number; to: number };

export const api = {
  me: () => call<Identity>("/me"),
  searchLogs: (
    s: Scope & { limit?: number; cursor?: string; after_id?: number },
    signal?: AbortSignal,
  ) =>
    post<{ logs: Log[]; next_cursor: string | null; total: number | null }>(
      "/logs/search",
      s,
      signal,
    ),
  logsHistogram: (s: Scope, signal?: AbortSignal) =>
    post<Histogram>("/logs/histogram", s, signal),
  log: (id: number) => call<Log>(`/logs/${id}`),
  searchSpans: (
    s: Scope & { limit?: number; cursor?: string; sort?: string },
    signal?: AbortSignal,
  ) =>
    post<{ spans: Span[]; next_cursor: string | null; total: number | null }>(
      "/spans/search",
      s,
      signal,
    ),
  spansHistogram: (s: Scope, signal?: AbortSignal) =>
    post<Histogram>("/spans/histogram", s, signal),
  trace: (traceId: string) =>
    call<{ trace_id: string; spans: Span[] }>(
      `/traces/${encodeURIComponent(traceId)}`,
    ),
  facetValues: (
    dataset: "logs" | "spans" | "rum",
    s: Scope,
    facets: string[],
    limit = 10,
    signal?: AbortSignal,
  ) =>
    post<{ facets: FacetValues[] }>(
      "/facets/values",
      { dataset, ...s, facets, limit },
      signal,
    ),
  facetDefs: (dataset: "logs" | "spans") =>
    call<FacetDef[]>(`/facets?dataset=${dataset}`),
  addFacet: (f: Omit<FacetDef, "id">) => post<FacetDef>("/facets", f),
  deleteFacet: (id: number) =>
    call<void>(`/facets/${id}`, { method: "DELETE" }),
  services: (from: number, to: number, env?: string) =>
    call<ServiceSummary[]>(
      `/apm/services?from=${from}&to=${to}${env ? `&env=${encodeURIComponent(env)}` : ""}`,
    ),
  service: (name: string, from: number, to: number, env?: string) =>
    call<ServiceDetail>(
      `/apm/services/${encodeURIComponent(name)}?from=${from}&to=${to}${env ? `&env=${encodeURIComponent(env)}` : ""}`,
    ),
  envs: () => call<string[]>("/apm/envs"),
  serviceMap: (from: number, to: number, env?: string) =>
    call<ServiceEdge[]>(
      `/apm/map?from=${from}&to=${to}${env ? `&env=${encodeURIComponent(env)}` : ""}`,
    ),
  metrics: () => call<MetricMeta[]>("/metrics"),
  metric: (name: string, from: number, to: number) =>
    call<MetricDetail>(
      `/metrics/${encodeURIComponent(name)}?from=${from}&to=${to}`,
    ),
  updateMetric: (
    name: string,
    body: { unit: string | null; description: string | null },
  ) =>
    call<MetricMeta>(`/metrics/${encodeURIComponent(name)}`, {
      method: "PATCH",
      body: JSON.stringify(body),
    }),
  queryMetrics: (
    queries: string[],
    from: number,
    to: number,
    signal?: AbortSignal,
    maxSeries?: number,
  ) => post<MetricsResponse>("/metrics/query", { queries, from, to, max_series: maxSeries }, signal),
  hosts: () => call<HostInfo[]>("/hosts"),
  metricsVolume: (from: number, to: number) =>
    call<{ metric: string; points: number; series: number }[]>(`/metrics-volume?from=${from}&to=${to}`),
  synthetics: () => call<{ tests: SyntheticTest[] }>("/synthetics/tests"),
  createSynthetic: (t: SyntheticTest) => post<SyntheticTest>("/synthetics/tests", t),
  updateSynthetic: (id: string, t: SyntheticTest) => put<SyntheticTest>(`/synthetics/tests/${id}`, t),
  deleteSynthetic: (id: string) => del<unknown>(`/synthetics/tests/${id}`),
  triggerSynthetic: (id: string) => post<unknown>(`/synthetics/tests/${id}/trigger`, {}),
  slos: () => call<{ data: Slo[] }>("/slos"),
  createSlo: (s: Slo) => post<unknown>("/slos", s),
  updateSlo: (id: string, s: Slo) => put<unknown>(`/slos/${id}`, s),
  deleteSlo: (id: string) => del<unknown>(`/slos/${id}`),
  searchEvents: (s: Scope & { limit?: number; cursor?: string }, signal?: AbortSignal) =>
    post<{ logs: Log[]; next_cursor: string | null; total: number | null }>("/events/search", s, signal),
  eventsHistogram: (s: Scope, signal?: AbortSignal) => post<Histogram>("/events/histogram", s, signal),
  searchRum: (s: Scope & { limit?: number; cursor?: string }, signal?: AbortSignal) =>
    post<{ logs: Log[]; next_cursor: string | null; total: number | null }>("/rum/search", s, signal),
  rumHistogram: (s: Scope, signal?: AbortSignal) => post<Histogram>("/rum/histogram", s, signal),
  rumSessions: (s: Scope, signal?: AbortSignal) => post<{ sessions: RumSession[] }>("/rum/sessions", s, signal),
  rumPerformance: (s: Scope, signal?: AbortSignal) => post<RumPerformance>("/rum/performance", s, signal),
  incidents: () => call<{ incidents: Incident[] }>("/incidents"),
  incident: (id: string) => call<Incident>(`/incidents/${id}`),
  declareIncident: (i: { title: string; severity: string; summary: string; services: string[] }) => post<Incident>("/incidents", i),
  updateIncident: (id: string, p: Partial<Incident>) => put<Incident>(`/incidents/${id}`, p),
  noteIncident: (id: string, text: string) => post<Incident>(`/incidents/${id}/timeline`, { text }),
  deleteIncident: (id: string) => del<unknown>(`/incidents/${id}`),
  statusPage: () => call<StatusPage>("/status-page"),
  saveStatusPage: (p: StatusPage) => put<StatusPage>("/status-page", p),
  watchdog: () => call<{ stories: WatchdogStory[] }>("/watchdog/stories"),
  runWatchdog: () => post<{ stories: WatchdogStory[] }>("/watchdog/stories", {}),
  rumConfig: () => call<{ client_token: string | null; version: string; env: string | null }>("/rum-config"),
  notebooks: () => call<{ data: { id: string; name: string; cells: number; modified_at: number }[] }>("/notebooks"),
  notebook: (id: string) => call<Notebook>(`/notebooks/${id}`),
  createNotebook: (n: Notebook) => post<Notebook>("/notebooks", n),
  updateNotebook: (id: string, n: Notebook) => put<Notebook>(`/notebooks/${id}`, n),
  deleteNotebook: (id: string) => del<unknown>(`/notebooks/${id}`),
  postEvent: (e: Record<string, unknown>) => post<unknown>("/events", e),
  downtimes: () => call<Downtime[]>("/downtimes"),
  createDowntime: (d: Partial<Downtime>) => post<Downtime>("/downtimes", d),
  cancelDowntime: (id: string) => del<unknown>(`/downtimes/${id}`),
  aggregate: (
    dataset: "logs" | "spans" | "rum",
    req: AggregateRequest,
    signal?: AbortSignal,
  ) => post<AggregateResponse>(`/${dataset}/aggregate`, req, signal),
  logPatterns: (s: Scope, signal?: AbortSignal) =>
    post<{ sampled: number; patterns: LogPattern[] }>(
      "/logs/patterns",
      s,
      signal,
    ),
  dashboards: () => call<{ dashboards: DashboardSummary[] }>("/dashboards"),
  dashboard: (id: string) => call<Dashboard>(`/dashboards/${id}`),
  createDashboard: (d: Partial<Dashboard>) => post<Dashboard>("/dashboards", d),
  updateDashboard: (id: string, d: Partial<Dashboard>) =>
    put<Dashboard>(`/dashboards/${id}`, d),
  deleteDashboard: (id: string) => del<unknown>(`/dashboards/${id}`),
  cloneDashboard: (id: string) =>
    post<Dashboard>(`/dashboards/${id}/clone`, {}),
  queryTimeseries: (r: QueryRequest, signal?: AbortSignal) =>
    post<TimeseriesResponse>("/query/timeseries", r, signal),
  queryScalar: (r: QueryRequest, signal?: AbortSignal) =>
    post<ScalarResponse>("/query/scalar", r, signal),
  errorIssues: (
    r: { from: number; to: number; query: string },
    signal?: AbortSignal,
  ) =>
    post<{ interval: number; start: number; issues: ErrorIssue[] }>(
      "/errors/issues",
      r,
      signal,
    ),
  errorIssue: (id: string, from: number, to: number) =>
    call<ErrorIssueDetail>(`/errors/issues/${id}?from=${from}&to=${to}`),
  setIssueStatus: (id: string, status: string) =>
    put<{ id: string; status: string }>(`/errors/issues/${id}`, { status }),
  views: (page: string) => call<SavedView[]>(`/views?page=${page}`),
  createView: (v: Omit<SavedView, "id">) => post<SavedView>("/views", v),
  deleteView: (id: string) => del<unknown>(`/views/${id}`),
  generatedMetrics: (source: "logs" | "spans") =>
    call<{ data: GeneratedMetric[] }>(
      `/${source === "logs" ? "logs" : "apm"}/config/metrics`,
    ).then((r) => r.data),
  createGeneratedMetric: (
    source: "logs" | "spans",
    id: string,
    attributes: GeneratedMetricAttributes,
  ) =>
    post<{ data: GeneratedMetric }>(
      `/${source === "logs" ? "logs" : "apm"}/config/metrics`,
      {
        data: { id, type: `${source}_metrics`, attributes },
      },
    ),
  updateGeneratedMetric: (
    source: "logs" | "spans",
    id: string,
    attributes: GeneratedMetricAttributes,
  ) =>
    call<{ data: GeneratedMetric }>(
      `/${source === "logs" ? "logs" : "apm"}/config/metrics/${encodeURIComponent(id)}`,
      {
        method: "PATCH",
        body: JSON.stringify({
          data: { type: `${source}_metrics`, attributes },
        }),
      },
    ),
  deleteGeneratedMetric: (source: "logs" | "spans", id: string) =>
    del<unknown>(
      `/${source === "logs" ? "logs" : "apm"}/config/metrics/${encodeURIComponent(id)}`,
    ),
  monitors: () => call<Monitor[]>("/monitors"),
  monitor: (id: number) => call<Monitor>(`/monitors/${id}`),
  createMonitor: (m: MonitorDraft) => post<Monitor>("/monitors", m),
  updateMonitor: (id: number, m: Partial<MonitorDraft>) =>
    put<Monitor>(`/monitors/${id}`, m),
  deleteMonitor: (id: number) => del<unknown>(`/monitors/${id}`),
  muteMonitor: (id: number, scope = "*") =>
    post<Monitor>(`/monitors/${id}/mute`, { scope }),
  unmuteMonitor: (id: number) =>
    post<Monitor>(`/monitors/${id}/unmute`, { all_scopes: true }),
  monitorEvents: (id?: number, limit = 100) =>
    call<MonitorEvent[]>(
      id
        ? `/monitors/${id}/events?limit=${limit}`
        : `/monitors/events?limit=${limit}`,
    ),
  monitorHistory: (id: number, from: number, to: number) =>
    call<{ group: string; points: [number, number | null, string][] }[]>(
      `/monitors/${id}/history?from=${from}&to=${to}`,
    ),
  previewMonitor: (
    query: string,
    thresholds: MonitorOptions["thresholds"],
    signal?: AbortSignal,
  ) => post<MonitorPreview>("/monitors/preview", { query, thresholds }, signal),
  webhooks: () => call<Webhook[]>("/webhooks"),
  createWebhook: (w: Webhook) => post<Webhook>("/webhooks", w),
  updateWebhook: (name: string, w: Webhook) =>
    put<Webhook>(`/webhooks/${encodeURIComponent(name)}`, w),
  deleteWebhook: (name: string) =>
    del<unknown>(`/webhooks/${encodeURIComponent(name)}`),
  testWebhook: (name: string) =>
    post<{ ok: boolean; status?: number; error?: string }>(
      `/webhooks/${encodeURIComponent(name)}/test`,
      {},
    ),
  pipelines: () => call<Pipeline[]>("/pipelines"),
  createPipeline: (p: Pipeline) => post<Pipeline>("/pipelines", p),
  updatePipeline: (id: string, p: Pipeline) =>
    put<Pipeline>(`/pipelines/${id}`, p),
  deletePipeline: (id: string) => del<unknown>(`/pipelines/${id}`),
  clonePipeline: (id: string) => post<Pipeline>(`/pipelines/${id}/clone`, {}),
  orderPipelines: (pipeline_ids: string[]) =>
    put<{ pipeline_ids: string[] }>("/pipelines/order", { pipeline_ids }),
  grokTest: (
    grok: { match_rules: string; support_rules: string },
    samples: string[],
    signal?: AbortSignal,
  ) =>
    post<{ error: string | null; samples: GrokSample[] }>(
      "/pipelines/grok-test",
      { grok, samples },
      signal,
    ),
};
