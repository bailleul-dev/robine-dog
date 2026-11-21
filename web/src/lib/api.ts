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
  deployments: (Stats & {
    version: string;
    first_seen: number;
    last_seen: number;
  })[];
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
export type SyntheticAssertion = {
  type: string;
  operator: string;
  property?: string | null;
  // A number, a text, or `{jsonPath, operator, targetValue}`.
  target: unknown;
};
export type SyntheticRequest = {
  method?: string;
  url?: string;
  headers?: Record<string, string>;
  body?: string | null;
  timeout?: number | null;
  host?: string;
  port?: number | string | null;
  servername?: string;
  dnsServer?: string;
  dnsServerPort?: number | string;
  message?: string;
  service?: string;
  callType?: string;
  numberOfPackets?: number;
  [key: string]: unknown;
};
export type SyntheticExtract = {
  name: string;
  type: string;
  field?: string;
  parser: { type: string; value?: string };
  secure?: boolean;
};
export type SyntheticApiStep = {
  name: string;
  subtype: string;
  request: SyntheticRequest;
  assertions: SyntheticAssertion[];
  extractedValues?: SyntheticExtract[];
  allowFailure?: boolean;
  isCritical?: boolean;
  value?: number;
};
export type SyntheticBrowserStep = {
  name?: string;
  type: string;
  params: Record<string, any>;
  allowFailure?: boolean;
  isCritical?: boolean;
  timeout?: number;
};
export type SyntheticConfigVariable = {
  name: string;
  type: string;
  pattern?: string;
  id?: string;
  example?: string;
  secure?: boolean;
};
export type SyntheticStepResult = {
  name: string;
  type: string;
  passed: boolean;
  duration_ms: number;
  failure?: string;
  allow_failure: boolean;
  status_code?: number;
  url?: string;
  details?: Record<string, unknown>;
  extracted?: Record<string, unknown>;
  screenshot?: boolean;
};
export type SyntheticResult = {
  result_id: string;
  ts: number;
  passed: boolean;
  status_code: number | null;
  duration_ms: number;
  failure: string | null;
  steps?: SyntheticStepResult[];
  details?: Record<string, unknown>;
  batch_id?: string;
  retries?: number;
};
export type SyntheticTest = {
  public_id?: string;
  name: string;
  type?: string;
  subtype?: string;
  status: string;
  config: {
    request: SyntheticRequest;
    assertions: SyntheticAssertion[];
    steps?: SyntheticApiStep[];
    configVariables?: SyntheticConfigVariable[];
    variables?: SyntheticConfigVariable[];
  };
  options: {
    tick_every?: number | null;
    min_failure_duration?: number | null;
    follow_redirects?: boolean;
    accept_self_signed?: boolean;
    device_ids?: string[];
    retry?: { count: number; interval?: number };
    [key: string]: unknown;
  };
  steps?: SyntheticBrowserStep[];
  locations?: string[];
  monitor_status?: string;
  message: string;
  tags: string[];
  last_result?: SyntheticResult;
};
export type SyntheticVariable = {
  id?: string;
  name: string;
  description: string;
  tags: string[];
  value: { value?: string; secure: boolean };
};
export type SyntheticBatchRun = {
  result_id?: string;
  test_public_id: string;
  test_name: string;
  test_type: string;
  execution_rule: string;
  status: string;
  duration: number;
  timed_out: boolean;
  device?: { id: string };
};
export type SyntheticBatch = {
  batch_id: string;
  created: number;
  status: string;
  metadata?: Record<string, unknown>;
  results: SyntheticBatchRun[];
};
export type SloThreshold = {
  timeframe: string;
  target: number;
  warning?: number | null;
};
export type TimeSlice = {
  query: {
    formulas: { formula: string }[];
    queries: { data_source: string; name: string; query: string }[];
  };
  comparator: string;
  threshold: number;
  query_interval_seconds?: number;
};
export type Slo = {
  id?: string;
  name: string;
  description?: string | null;
  type?: "metric" | "monitor" | "time_slice" | string;
  query?: { numerator: string; denominator: string } | null;
  monitor_ids?: number[];
  groups?: string[];
  sli_specification?: { time_slice: TimeSlice } | null;
  thresholds: SloThreshold[];
  tags: string[];
  created_at?: number;
  modified_at?: number;
};
export type SloStatus = {
  timeframe: string;
  target: number;
  warning?: number | null;
  sli: number | null;
  good: number;
  total: number;
  error_budget_remaining: number | null;
  state: "ok" | "warning" | "breached" | "no_data";
  unit: "events" | "seconds";
  error?: string;
};
export type SloHistory = {
  data: {
    from_ts: number;
    to_ts: number;
    type: string;
    overall: {
      sli_value: number | null;
      error_budget_remaining: { custom: number | null };
      corrections: { start: number; end: number }[];
      history?: [number, number][];
    };
    series: {
      interval: number;
      times: number[];
      numerator: { sum: number; values: number[] };
      denominator: { sum: number; values: number[] };
      unit: string;
    };
  };
};
export type SloCorrection = {
  id: string;
  type: "correction";
  attributes: {
    slo_id: string;
    start: number;
    end?: number | null;
    duration?: number | null;
    rrule?: string | null;
    category: string;
    description?: string | null;
    timezone?: string | null;
  };
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
  frustrations: number;
  has_replay: boolean;
};
export type RumReplay = {
  session_id: string;
  views: { view_id: string; start: number; end: number; records: number; segments: number }[];
  records: { type: number; timestamp: number; data?: unknown; format?: number }[];
};
export type RumFrustrationTally = {
  count: number;
  by_type: Record<string, number>;
  sessions: number;
  view?: string;
  target?: string;
};
export type RumFrustrations = {
  total: RumFrustrationTally;
  sessions: number;
  pages: RumFrustrationTally[];
  targets: RumFrustrationTally[];
  latest: {
    ts: number;
    types: string[];
    target: string;
    view: string;
    session_id: string | null;
    service: string | null;
    user: string | null;
  }[];
};
export type RumRetention = {
  period: string;
  cohorts: { start: number; users: number; returned: number[]; rates: number[] }[];
};
export type RumRetentionReq = {
  start_query: string;
  return_query?: string;
  period: "day" | "week";
  by?: string;
  from: number;
  to: number;
};
export type RumPerformance = {
  sessions: number;
  views: number;
  errors: number;
  actions: number;
  pages: RumPage[];
};
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
  timeline: {
    ts: number;
    kind: "declared" | "note" | "state" | "severity";
    text: string;
    by: string | null;
  }[];
};
export type StatusComponent = {
  name: string;
  kind: "synthetic" | "monitor";
  ref: string;
  status?: string;
  uptime?: number | null;
};
export type StatusPage = {
  enabled: boolean;
  title: string;
  description: string;
  components: StatusComponent[];
};
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
export type NotebookCell = {
  id: string;
  type: "markdown" | "metric" | "logs";
  text: string;
};
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
  /** `[t, lower, upper]`: the band of `anomalies()` / `forecast()`. */
  bounds?: [number, number, number][];
  outlier?: boolean;
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
export type TemplatePreset = {
  name: string;
  template_variables: { name: string; value?: string; values?: string[] }[];
};
export type DashboardListInfo = {
  id: number;
  name: string;
  dashboard_count: number;
  dashboard_ids: string[];
  modified: string;
};
export type DashboardShare = {
  token: string;
  dashboard_id: string;
  public_url: string;
  created: string;
  share_type: string;
  global_time: { live_span: string };
  global_time_selectable_enabled: boolean;
  selectable_template_vars: {
    name: string;
    prefix?: string | null;
    default_value?: string;
    visible_tags?: string[];
  }[];
};
export type PublicDashboard = Pick<
  Dashboard,
  "title" | "description" | "layout_type" | "widgets" | "template_variables"
> & {
  global_time: { live_span: string };
  global_time_selectable_enabled: boolean;
  selectable_template_vars: DashboardShare["selectable_template_vars"];
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
      series: {
        query_index: number;
        name: string;
        group_tags: string[];
        /** robine-dog's addition: the band of `anomalies()` / `forecast()`. */
        bounds?: { lower: (number | null)[]; upper: (number | null)[] };
        outlier?: boolean;
      }[];
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
export type Distribution = {
  buckets: { lower: number; upper: number; count: number }[];
  count: number;
  min?: number;
  max?: number;
  avg?: number;
  p50?: number;
  p75?: number;
  p90?: number;
  p95?: number;
  p99?: number;
};
export type Funnel = {
  steps: {
    label: string;
    search: string;
    sessions: number;
    conversion: number;
    dropoff: number;
  }[];
};
export type MonitorSearch = {
  monitors: {
    id: number;
    name: string;
    status: string;
    type: string;
    tags: string[];
    last_triggered_ts: number | null;
  }[];
  counts: Record<string, { name: string | boolean; count: number }[]>;
  metadata: { total_count: number };
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
  event_type?: string;
  uniqueness?: { when: "match" | "end" };
};
export type GeneratedSource = "logs" | "spans" | "rum";
const generatedPath = (s: GeneratedSource) =>
  `/${s === "spans" ? "apm" : s}/config/metrics`;
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
    ok?: number;
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
export type CheckSummary = {
  check: string;
  host: string | null;
  status: string;
  message: string;
  last: number;
  tags: string[];
};
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
  ) =>
    post<MetricsResponse>(
      "/metrics/query",
      { queries, from, to, max_series: maxSeries },
      signal,
    ),
  hosts: () => call<HostInfo[]>("/hosts"),
  metricsVolume: (from: number, to: number) =>
    call<{ metric: string; points: number; series: number }[]>(
      `/metrics-volume?from=${from}&to=${to}`,
    ),
  synthetics: () => call<{ tests: SyntheticTest[] }>("/synthetics/tests"),
  createSynthetic: (t: SyntheticTest) =>
    post<SyntheticTest>("/synthetics/tests", t),
  updateSynthetic: (id: string, t: SyntheticTest) =>
    put<SyntheticTest>(`/synthetics/tests/${id}`, t),
  deleteSynthetic: (id: string) => del<unknown>(`/synthetics/tests/${id}`),
  triggerSynthetic: (id: string) =>
    post<SyntheticResult>(`/synthetics/tests/${id}/trigger`, {}),
  setSyntheticStatus: (id: string, status: string) =>
    put<boolean>(`/synthetics/tests/${id}/status`, { new_status: status }),
  syntheticResults: (id: string, from: number, to: number) =>
    call<{ results: SyntheticResult[] }>(
      `/synthetics/tests/${id}/results?from_ts=${from}&to_ts=${to}&limit=200`,
    ),
  syntheticScreenshot: (resultId: string, step: number) =>
    `/api/ui/synthetics/results/${resultId}/screenshots/${step}`,
  syntheticVariables: () =>
    call<{ variables: SyntheticVariable[] }>("/synthetics/variables"),
  createSyntheticVariable: (v: SyntheticVariable) =>
    post<SyntheticVariable>("/synthetics/variables", v),
  updateSyntheticVariable: (id: string, v: SyntheticVariable) =>
    put<SyntheticVariable>(`/synthetics/variables/${id}`, v),
  deleteSyntheticVariable: (id: string) =>
    del<unknown>(`/synthetics/variables/${id}`),
  syntheticBatches: () =>
    call<{ batches: SyntheticBatch[] }>("/synthetics/ci/batches"),
  slos: () => call<{ data: Slo[] }>("/slos"),
  createSlo: (s: Slo) => post<unknown>("/slos", s),
  updateSlo: (id: string, s: Slo) => put<unknown>(`/slos/${id}`, s),
  deleteSlo: (id: string, force = false) =>
    del<unknown>(`/slos/${id}${force ? "?force=true" : ""}`),
  slo: (id: string) =>
    call<{ data: Slo & { configured_alert_ids: number[] } }>(`/slos/${id}`),
  sloStatus: (id: string) => call<SloStatus[]>(`/slos/${id}/status`),
  sloHistory: (id: string, from: number, to: number, target?: number) =>
    call<SloHistory>(
      `/slos/${id}/history?from=${from}&to=${to}${target ? `&target=${target}` : ""}`,
    ),
  sloCorrections: (id: string) =>
    call<{ data: SloCorrection[] }>(`/slos/${id}/corrections`),
  createSloCorrection: (a: SloCorrection["attributes"]) =>
    post<unknown>("/slo-corrections", {
      data: { type: "correction", attributes: a },
    }),
  deleteSloCorrection: (id: string) => del<unknown>(`/slo-corrections/${id}`),
  searchEvents: (
    s: Scope & { limit?: number; cursor?: string },
    signal?: AbortSignal,
  ) =>
    post<{ logs: Log[]; next_cursor: string | null; total: number | null }>(
      "/events/search",
      s,
      signal,
    ),
  eventsHistogram: (s: Scope, signal?: AbortSignal) =>
    post<Histogram>("/events/histogram", s, signal),
  searchRum: (
    s: Scope & { limit?: number; cursor?: string },
    signal?: AbortSignal,
  ) =>
    post<{ logs: Log[]; next_cursor: string | null; total: number | null }>(
      "/rum/search",
      s,
      signal,
    ),
  rumHistogram: (s: Scope, signal?: AbortSignal) =>
    post<Histogram>("/rum/histogram", s, signal),
  rumSessions: (s: Scope, signal?: AbortSignal) =>
    post<{ sessions: RumSession[] }>("/rum/sessions", s, signal),
  rumReplay: (sessionId: string, viewId?: string, signal?: AbortSignal) =>
    call<RumReplay>(
      `/rum/replay/${encodeURIComponent(sessionId)}${viewId ? `?view_id=${encodeURIComponent(viewId)}` : ""}`,
      { signal },
    ),
  rumFrustrations: (s: Scope, signal?: AbortSignal) =>
    post<RumFrustrations>("/rum/frustrations", s, signal),
  rumRetention: (r: RumRetentionReq, signal?: AbortSignal) =>
    post<RumRetention>("/rum/retention", r, signal),
  rumPerformance: (s: Scope, signal?: AbortSignal) =>
    post<RumPerformance>("/rum/performance", s, signal),
  incidents: () => call<{ incidents: Incident[] }>("/incidents"),
  incident: (id: string) => call<Incident>(`/incidents/${id}`),
  declareIncident: (i: {
    title: string;
    severity: string;
    summary: string;
    services: string[];
  }) => post<Incident>("/incidents", i),
  updateIncident: (id: string, p: Partial<Incident>) =>
    put<Incident>(`/incidents/${id}`, p),
  noteIncident: (id: string, text: string) =>
    post<Incident>(`/incidents/${id}/timeline`, { text }),
  deleteIncident: (id: string) => del<unknown>(`/incidents/${id}`),
  statusPage: () => call<StatusPage>("/status-page"),
  saveStatusPage: (p: StatusPage) => put<StatusPage>("/status-page", p),
  watchdog: () => call<{ stories: WatchdogStory[] }>("/watchdog/stories"),
  runWatchdog: () =>
    post<{ stories: WatchdogStory[] }>("/watchdog/stories", {}),
  rumConfig: () =>
    call<{
      client_token: string | null;
      version: string;
      env: string | null;
      replay_sample_rate?: number;
    }>("/rum-config"),
  notebooks: () =>
    call<{
      data: { id: string; name: string; cells: number; modified_at: number }[];
    }>("/notebooks"),
  notebook: (id: string) => call<Notebook>(`/notebooks/${id}`),
  createNotebook: (n: Notebook) => post<Notebook>("/notebooks", n),
  updateNotebook: (id: string, n: Notebook) =>
    put<Notebook>(`/notebooks/${id}`, n),
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
  dashboardLists: () => call<DashboardListInfo[]>("/dashboard-lists"),
  createDashboardList: (name: string) =>
    post<DashboardListInfo>("/dashboard-lists", { name }),
  renameDashboardList: (id: number, name: string) =>
    put<DashboardListInfo>(`/dashboard-lists/${id}`, { name }),
  deleteDashboardList: (id: number) => del<unknown>(`/dashboard-lists/${id}`),
  addToDashboardList: (id: number, dashboards: string[]) =>
    post<unknown>(`/dashboard-lists/${id}/dashboards`, {
      dashboards: dashboards.map((d) => ({ id: d })),
    }),
  removeFromDashboardList: (id: number, dashboards: string[]) =>
    call<unknown>(`/dashboard-lists/${id}/dashboards`, {
      method: "DELETE",
      body: JSON.stringify({ dashboards: dashboards.map((d) => ({ id: d })) }),
    }),
  dashboardFavorites: () => call<string[]>("/dashboard-favorites"),
  setDashboardFavorite: (id: string, on: boolean) =>
    call<string[]>(`/dashboards/${id}/favorite`, {
      method: on ? "PUT" : "DELETE",
    }),
  dashboardShares: (id: string) =>
    call<DashboardShare[]>(`/dashboards/${id}/shares`),
  shareDashboard: (body: Partial<DashboardShare>) =>
    post<DashboardShare>("/dashboard-shares", body),
  updateDashboardShare: (token: string, body: Partial<DashboardShare>) =>
    put<DashboardShare>(`/dashboard-shares/${token}`, body),
  revokeDashboardShare: (token: string) =>
    del<unknown>(`/dashboard-shares/${token}`),
  queryTimeseries: (r: QueryRequest, signal?: AbortSignal) =>
    post<TimeseriesResponse>("/query/timeseries", r, signal),
  queryScalar: (r: QueryRequest, signal?: AbortSignal) =>
    post<ScalarResponse>("/query/scalar", r, signal),
  queryDistribution: (
    r: {
      from: number;
      to: number;
      data_source: string;
      query?: string;
      search?: string;
      metric?: string;
      buckets?: number;
    },
    signal?: AbortSignal,
  ) => post<Distribution>("/query/distribution", r, signal),
  rumFunnel: (
    r: {
      from: number;
      to: number;
      query?: string;
      steps: { search: string; label?: string }[];
    },
    signal?: AbortSignal,
  ) => post<Funnel>("/rum/funnel", r, signal),
  monitorSearch: (query: string) =>
    call<MonitorSearch>(
      `/monitors/search?query=${encodeURIComponent(query)}&per_page=100`,
    ),
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
  generatedMetrics: (source: GeneratedSource) =>
    call<{ data: GeneratedMetric[] }>(generatedPath(source)).then((r) => r.data),
  createGeneratedMetric: (
    source: GeneratedSource,
    id: string,
    attributes: GeneratedMetricAttributes,
  ) =>
    post<{ data: GeneratedMetric }>(
      generatedPath(source),
      {
        data: { id, type: `${source}_metrics`, attributes },
      },
    ),
  updateGeneratedMetric: (
    source: GeneratedSource,
    id: string,
    attributes: GeneratedMetricAttributes,
  ) =>
    call<{ data: GeneratedMetric }>(
      `${generatedPath(source)}/${encodeURIComponent(id)}`,
      {
        method: "PATCH",
        body: JSON.stringify({
          data: { type: `${source}_metrics`, attributes },
        }),
      },
    ),
  deleteGeneratedMetric: (source: GeneratedSource, id: string) =>
    del<unknown>(`${generatedPath(source)}/${encodeURIComponent(id)}`),
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
  checks: () => call<CheckSummary[]>("/checks"),
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

/** The read-only side of a public dashboard link, without a session. */
async function publicCall<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`/api/public/dashboard/${path}`, {
    ...init,
    headers: { "content-type": "application/json" },
    credentials: "omit",
  });
  if (!res.ok) {
    let msg = res.statusText;
    try {
      msg = (await res.json()).error ?? msg;
    } catch {
      /* not JSON */
    }
    throw new ApiError(res.status, msg);
  }
  return res.json();
}

export const publicApi = {
  dashboard: (token: string) =>
    publicCall<PublicDashboard>(encodeURIComponent(token)),
  query: <T>(
    token: string,
    body: {
      widget_id: number;
      request: number;
      from: number;
      to: number;
      scalar: boolean;
      vars: Record<string, string>;
    },
    signal?: AbortSignal,
  ) =>
    publicCall<T>(`${encodeURIComponent(token)}/query`, {
      method: "POST",
      body: JSON.stringify(body),
      signal,
    }),
};
