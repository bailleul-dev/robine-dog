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
  views: {
    view_id: string;
    start: number;
    end: number;
    records: number;
    segments: number;
  }[];
  records: {
    type: number;
    timestamp: number;
    data?: unknown;
    format?: number;
  }[];
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
  cohorts: {
    start: number;
    users: number;
    returned: number[];
    rates: number[];
  }[];
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

const profileQs = (f: Record<string, string | number | boolean | undefined>) =>
  new URLSearchParams(
    Object.entries(f)
      .filter(([, v]) => v !== undefined && v !== "")
      .map(([k, v]) => [k, String(v)]),
  ).toString();
const post = <T>(path: string, body: unknown, signal?: AbortSignal) =>
  call<T>(path, { method: "POST", body: JSON.stringify(body), signal });
const put = <T>(path: string, body: unknown) =>
  call<T>(path, { method: "PUT", body: JSON.stringify(body) });
const del = <T>(path: string) => call<T>(path, { method: "DELETE" });

export type Scope = { query: string; from: number; to: number };
export type LiveProcess = {
  host: string;
  pid: number;
  ppid: number;
  user: string;
  command: string;
  name: string;
  cpu_pct: number;
  rss: number;
  vms: number;
  threads: number;
  fds: number;
  started: number;
  container_id: string;
};
export type DbQuery = {
  signature: string;
  query: string;
  dbms: string;
  db: string | null;
  calls: number;
  total_time_ms: number;
  avg_ms: number;
  rows: number;
  hosts: number;
};
export type CaseItem = {
  id: string;
  attributes: {
    key: string;
    title: string;
    description: string;
    status: string;
    priority: string;
    project: string;
    assignee: string | null;
    origin: { type?: string; id?: string; url?: string } | null;
    created_at: number;
    created_by: string;
    modified_at: number;
    timeline: { ts: number; author: string; type: string; text: string }[];
  };
};
export type OnCallPage = {
  id: string;
  team: string;
  title: string;
  message: string;
  source: string;
  status: string;
  step: number;
  created_at: number;
  acked_by?: string;
  timeline: { ts: number; text: string }[];
};
export type OnCallSchedule = {
  id: string;
  name: string;
  members: string[];
  length_hours: number;
  start: number;
  on_call: string | null;
};
export type EscalationPolicy = {
  id: string;
  name: string;
  team: string;
  repeat?: number;
  steps: { targets: string[]; delay_minutes: number }[];
};
export type SecuritySignal = {
  id: string;
  attributes: {
    rule_id: string;
    title: string;
    severity: string;
    status: string;
    group: string;
    group_tags: Record<string, string>;
    query: string;
    message: string;
    value: number;
    first_seen: number;
    timestamp: number;
    assignee: string | null;
    archive_reason: string | null;
  };
};
export type SecurityRule = {
  id: string;
  name: string;
  isEnabled: boolean;
  isDefault?: boolean;
  message: string;
  tags?: string[];
  queries: {
    name?: string;
    query: string;
    groupByFields: string[];
    aggregation: string;
    metrics?: string[];
  }[];
  cases: { name?: string; status: string; condition: string }[];
  options: {
    evaluationWindow: number;
    keepAlive?: number;
    maxSignalDuration?: number;
  };
};
export type AuditEvent = {
  id: string;
  attributes: {
    timestamp: number;
    message: string;
    attributes: {
      evt: { source: string };
      http: {
        method: string;
        url_details: { path: string };
        status_code: number;
      };
      usr: { id: string; name: string | null };
    };
  };
};
export type CiFilter = {
  from: number;
  to: number;
  service?: string;
  status?: string;
  branch?: string;
  query?: string;
};
export type CiTests = {
  runs: {
    id: number;
    service: string;
    name: string;
    suite: string | null;
    status: string;
    duration: number;
    start: number;
    error: string | null;
    branch: string | null;
    commit: string | null;
    framework: string | null;
  }[];
  services: {
    service: string;
    tests: number;
    failed: number;
    skipped: number;
    sessions: number;
    avg: number;
  }[];
  flaky: {
    service: string;
    suite: string | null;
    name: string;
    commits: number;
    last: number;
    error: string | null;
  }[];
  slowest: {
    service: string;
    suite: string | null;
    name: string;
    runs: number;
    avg: number;
    max: number;
  }[];
};
export type CiPipelines = {
  executions: {
    id: number;
    provider: string;
    pipeline_id: string;
    name: string;
    status: string;
    start: number;
    end: number;
    branch: string | null;
    commit: string | null;
    repository: string | null;
    url: string | null;
  }[];
  pipelines: {
    name: string;
    provider: string;
    executions: number;
    successes: number;
    p50: number;
    p95: number;
    last_status: string;
    repository: string | null;
  }[];
};
export type LlmSpan = {
  trace_id: string;
  span_id: string;
  parent_id: string | null;
  ml_app: string;
  name: string;
  kind: string;
  start_ns: number;
  duration: number;
  error: boolean;
  model_name: string | null;
  model_provider: string | null;
  input_tokens: number | null;
  output_tokens: number | null;
  total_tokens: number | null;
  cost: number | null;
  session_id: string | null;
  service: string | null;
  env: string | null;
  tags: string[];
  input: string;
  output: string;
  body: Record<string, unknown>;
};
export type LlmEval = {
  span_id?: string;
  label: string;
  type: string;
  categorical: string | null;
  score: number | null;
};
export type LlmTraceRow = LlmSpan & {
  spans: number;
  llm_calls: number;
  tokens: number;
  cost: number;
  errors: number;
  evals: LlmEval[];
};
export type LlmFilter = {
  ml_app?: string;
  query?: string;
  kind?: string;
  errors?: boolean;
  from: number;
  to: number;
};
export type LlmOverview = {
  apps: { ml_app: string; traces: number }[];
  traces: number;
  errors: number;
  llm_calls: number;
  input_tokens: number;
  output_tokens: number;
  cost: number;
  p50: number;
  p95: number;
  interval_ms: number;
  series: {
    t: number;
    traces: number;
    errors: number;
    tokens: number;
    cost: number;
  }[];
  models: {
    provider: string;
    model: string;
    calls: number;
    errors: number;
    input_tokens: number;
    output_tokens: number;
    cost: number;
    p95: number;
  }[];
  evaluations: {
    label: string;
    type: string;
    count: number;
    avg_score: number | null;
    top_value: string | null;
    flagged: number;
  }[];
};
export type ProfileFilter = {
  service?: string;
  env?: string;
  version?: string;
  host?: string;
  from: number;
  to: number;
};
export type ProfileRow = {
  id: number;
  start: number;
  end: number;
  service: string;
  env: string | null;
  version: string | null;
  host: string | null;
  language: string | null;
  runtime_id: string | null;
  tags: string[];
  types: string[];
  files: string[];
  size: number;
};
export type ProfiledService = {
  service: string;
  profiles: number;
  types: string[];
  languages: string[];
  versions: string[];
};
export type FlameNode = {
  name: string;
  value: number;
  self: number;
  children: FlameNode[];
};
export type FlameGraph = {
  type: string;
  unit: string;
  profiles: number;
  types: Record<string, string>;
  tree: FlameNode;
  top: { name: string; self: number; total: number }[];
  endpoints: { endpoint: string; value: number }[];
};
export type RetentionFilter = {
  id: string;
  type: string;
  attributes: {
    name: string;
    filter: { query: string };
    rate: number;
    enabled: boolean;
    filter_type: string;
    execution_order: number;
    modified_at: number;
  };
};
export type IngestionService = {
  service: string;
  last_hour: {
    spans: number;
    kept_spans: number;
    traces: number;
    kept_traces: number;
    bytes: number;
  };
  spans_per_second: number;
  mechanisms: Record<string, number>;
  tracers: Record<string, number>;
  last_seen: number;
};
export type ApmPoint = {
  t: number;
  hits: number;
  errors: number;
  p50: number;
  p95: number;
  p99: number;
  max: number;
};
export type TraceSample = {
  trace_id: string;
  span_id: string;
  start: number;
  duration: number;
  error: boolean;
  status_code: string | null;
};
export type ResourceDetail = {
  service: string;
  resource: string;
  operation: string;
  summary: {
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
  interval_ms: number;
  timeseries: ApmPoint[];
  distribution: [number, number][];
  spans: {
    service: string;
    name: string;
    traces: number;
    spans_per_trace: number;
    avg: number;
    p95: number;
    time_share: number;
  }[];
  slowest: TraceSample[];
  errors: TraceSample[];
  status_codes: [string, number][];
};
export type ServiceDefinition = {
  id: string;
  attributes: {
    schema: {
      "schema-version": string;
      "dd-service": string;
      team?: string;
      tier?: string | number;
      lifecycle?: string;
      description?: string;
      application?: string;
      languages?: string[];
      contacts?: { name?: string; type: string; contact: string }[];
      links?: { name: string; type: string; url: string; provider?: string }[];
      repos?: { name: string; url: string }[];
      docs?: { name: string; url: string }[];
    };
    meta: { "last-modified-time": string };
  };
};
export type TeamView = {
  id: string;
  name: string;
  handle: string;
  description: string | null;
  members: { user: string; role: string | null; created_at: number }[];
  links: { id: string; label: string; url: string; position: number }[];
  services: string[];
  updated_at: number;
};
export type ScorecardRule = {
  id: string;
  attributes: {
    name: string;
    scorecard_name: string;
    description: string;
    enabled: boolean;
    custom: boolean;
    owner: string | null;
  };
};
export type Scorecards = {
  rules: ScorecardRule[];
  services: {
    service: string;
    team: string | null;
    has_definition: boolean;
    score: number | null;
    scorecards: Record<string, number | null>;
    outcomes: Record<string, { state: string; remarks: string }>;
  }[];
};

export type LogIndex = {
  name: string;
  filter: { query: string };
  exclusion_filters: {
    name: string;
    is_enabled: boolean;
    filter: { query: string; sample_rate?: number };
  }[];
  daily_limit?: number | null;
  daily_limit_reset?: { reset_time: string; reset_utc_offset: string } | null;
  daily_limit_warning_threshold_percentage?: number | null;
  num_retention_days?: number | null;
  is_rate_limited?: boolean;
  usage?: {
    window_start: number;
    indexed: number;
    excluded: number;
    over_quota: number;
  };
};
export type ArchiveAttributes = {
  name: string;
  query: string;
  destination: {
    type: string;
    bucket?: string;
    path: string;
    region?: string;
    endpoint?: string;
  };
  include_tags: boolean;
  rehydration_tags: string[];
  rehydration_max_scan_size_in_gb?: number | null;
  state?: string;
  status?: {
    state: string;
    archived: number;
    files: number;
    bytes: number;
    last_flush: number | null;
    last_error: string | null;
  };
};
export type LogArchive = {
  id: string;
  type: string;
  attributes: ArchiveAttributes;
};
export type HistoricalView = {
  id: string;
  name: string;
  archive_id: string;
  query: string;
  from: number;
  to: number;
  max_logs: number | null;
  status: string;
  files: number;
  scanned_bytes: number;
  scanned_logs: number;
  rehydrated: number;
  error: string | null;
  created_at: number;
  finished_at: number | null;
};
export type SdsReplacement = {
  type: string;
  replacement_string?: string;
  number_of_chars?: number;
};
export type SdsRuleAttributes = {
  name: string;
  description: string;
  pattern: string;
  namespaces: string[];
  excluded_namespaces: string[];
  tags: string[];
  text_replacement: SdsReplacement;
  is_enabled: boolean;
  priority?: number;
};
export type SdsJson = {
  id: string;
  type: string;
  attributes: Record<string, unknown>;
  relationships?: Record<
    string,
    { data: { id: string; type: string } | { id: string; type: string }[] }
  >;
};
export type SdsConfig = {
  data: SdsJson;
  included: SdsJson[];
  meta: { version: number };
};
export type SdsStandardPattern = {
  id: string;
  attributes: {
    name: string;
    pattern: string;
    description: string;
    tags: string[];
  };
};
export type ReferenceTable = {
  id: string;
  attributes: {
    table_name: string;
    description: string;
    row_count: number;
    schema: {
      primary_keys: string[];
      fields: { name: string; type: string }[];
    };
    updated_at: string;
  };
};

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
    call<{ data: GeneratedMetric[] }>(generatedPath(source)).then(
      (r) => r.data,
    ),
  createGeneratedMetric: (
    source: GeneratedSource,
    id: string,
    attributes: GeneratedMetricAttributes,
  ) =>
    post<{ data: GeneratedMetric }>(generatedPath(source), {
      data: { id, type: `${source}_metrics`, attributes },
    }),
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
  logIndexes: () =>
    call<{ indexes: LogIndex[] }>("/logs/config/indexes").then(
      (r) => r.indexes,
    ),
  createLogIndex: (i: LogIndex) => post<LogIndex>("/logs/config/indexes", i),
  updateLogIndex: (name: string, i: LogIndex) =>
    put<LogIndex>(`/logs/config/indexes/${encodeURIComponent(name)}`, i),
  deleteLogIndex: (name: string) =>
    del<unknown>(`/logs/config/indexes/${encodeURIComponent(name)}`),
  setLogIndexOrder: (names: string[]) =>
    put<unknown>("/logs/config/index-order", { index_names: names }),
  logArchives: () =>
    call<{ data: LogArchive[] }>("/logs/config/archives").then((r) => r.data),
  createLogArchive: (a: ArchiveAttributes) =>
    post<{ data: LogArchive }>("/logs/config/archives", {
      data: { type: "archives", attributes: a },
    }),
  updateLogArchive: (id: string, a: ArchiveAttributes) =>
    put<{ data: LogArchive }>(`/logs/config/archives/${id}`, {
      data: { type: "archives", attributes: a },
    }),
  deleteLogArchive: (id: string) => del<unknown>(`/logs/config/archives/${id}`),
  setLogArchiveOrder: (ids: string[]) =>
    put<unknown>("/logs/config/archive-order", {
      data: { type: "archive_order", attributes: { archive_ids: ids } },
    }),
  historicalViews: () => call<HistoricalView[]>("/logs/rehydrations"),
  rehydrate: (v: {
    name: string;
    archive_id: string;
    query: string;
    from: number;
    to: number;
    max_logs?: number;
  }) => post<HistoricalView>("/logs/rehydrations", v),
  deleteHistoricalView: (id: string) =>
    del<unknown>(`/logs/rehydrations/${id}`),
  sdsConfig: () => call<SdsConfig>("/sensitive-data-scanner/config"),
  sdsStandardPatterns: () =>
    call<{ data: SdsStandardPattern[] }>(
      "/sensitive-data-scanner/config/standard-patterns",
    ).then((r) => r.data),
  sdsReorderGroups: (ids: string[]) =>
    call<unknown>("/sensitive-data-scanner/config", {
      method: "PATCH",
      body: JSON.stringify({
        data: {
          id: "robine-dog",
          type: "sensitive_data_scanner_configuration",
          relationships: {
            groups: {
              data: ids.map((id) => ({
                id,
                type: "sensitive_data_scanner_group",
              })),
            },
          },
        },
        meta: {},
      }),
    }),
  sdsSaveGroup: (
    id: string | null,
    attributes: Record<string, unknown>,
    ruleOrder?: string[],
  ) =>
    id
      ? call<unknown>(`/sensitive-data-scanner/config/groups/${id}`, {
          method: "PATCH",
          body: JSON.stringify({
            data: {
              id,
              type: "sensitive_data_scanner_group",
              attributes,
              ...(ruleOrder
                ? {
                    relationships: {
                      rules: {
                        data: ruleOrder.map((r) => ({
                          id: r,
                          type: "sensitive_data_scanner_rule",
                        })),
                      },
                    },
                  }
                : {}),
            },
            meta: {},
          }),
        })
      : post<unknown>("/sensitive-data-scanner/config/groups", {
          data: {
            type: "sensitive_data_scanner_group",
            attributes,
            relationships: {
              configuration: {
                data: {
                  type: "sensitive_data_scanner_configuration",
                  id: "robine-dog",
                },
              },
              rules: { data: [] },
            },
          },
          meta: {},
        }),
  sdsDeleteGroup: (id: string) =>
    del<unknown>(`/sensitive-data-scanner/config/groups/${id}`),
  sdsSaveRule: (
    id: string | null,
    groupId: string,
    attributes: Record<string, unknown>,
    standard: string | null,
  ) => {
    const relationships: Record<string, unknown> = {
      group: { data: { type: "sensitive_data_scanner_group", id: groupId } },
    };
    if (standard)
      relationships.standard_pattern = {
        data: { type: "sensitive_data_scanner_standard_pattern", id: standard },
      };
    const body = JSON.stringify({
      data: { type: "sensitive_data_scanner_rule", attributes, relationships },
      meta: {},
    });
    return id
      ? call<unknown>(`/sensitive-data-scanner/config/rules/${id}`, {
          method: "PATCH",
          body,
        })
      : call<unknown>("/sensitive-data-scanner/config/rules", {
          method: "POST",
          body,
        });
  },
  sdsDeleteRule: (id: string) =>
    del<unknown>(`/sensitive-data-scanner/config/rules/${id}`),
  sdsTest: (t: {
    pattern: string;
    standard_pattern_id: string | null;
    text_replacement: SdsReplacement;
    text: string;
  }) =>
    post<{
      matches: { start: number; end: number; text: string }[];
      result: string;
    }>("/sensitive-data-scanner/test", t),
  referenceTables: () =>
    call<{ data: ReferenceTable[] }>("/reference-tables/tables").then(
      (r) => r.data,
    ),
  createReferenceTable: (t: {
    table_name: string;
    description: string;
    primary_key: string;
    csv: string;
  }) => post<{ data: ReferenceTable }>("/reference-tables/tables", t),
  replaceReferenceTable: (id: string, csv: string, description: string) =>
    call<{ data: ReferenceTable }>(`/reference-tables/tables/${id}`, {
      method: "PATCH",
      body: JSON.stringify({ csv, description }),
    }),
  deleteReferenceTable: (id: string) =>
    del<unknown>(`/reference-tables/tables/${id}`),
  referenceTableRows: (id: string, search: string) =>
    call<{
      data: { id: string; attributes: { values: Record<string, unknown> } }[];
      meta: { total: number };
    }>(
      `/reference-tables/tables/${id}/rows?limit=100&search=${encodeURIComponent(search)}`,
    ),
  retentionFilters: () =>
    call<{ data: RetentionFilter[] }>("/apm/config/retention-filters").then(
      (r) => r.data,
    ),
  saveRetentionFilter: (
    id: string | null,
    a: {
      name: string;
      filter: { query: string };
      rate: number;
      enabled: boolean;
    },
  ) => {
    const body = {
      data: {
        type: "apm_retention_filter",
        attributes: { ...a, filter_type: "spans-sampling-processor" },
      },
    };
    return id
      ? put<unknown>(`/apm/config/retention-filters/${id}`, body)
      : post<unknown>("/apm/config/retention-filters", body);
  },
  deleteRetentionFilter: (id: string) =>
    del<unknown>(`/apm/config/retention-filters/${id}`),
  reorderRetentionFilters: (ids: string[]) =>
    put<unknown>("/apm/config/retention-filters-execution-order", {
      data: ids.map((id) => ({ id, type: "apm_retention_filter" })),
    }),
  ciTests: (f: CiFilter) => call<CiTests>(`/ci/tests?${profileQs(f)}`),
  ciPipelines: (f: CiFilter) =>
    call<CiPipelines>(`/ci/pipelines?${profileQs(f)}`),
  ciJobs: (provider: string, id: string) =>
    call<
      {
        level: string;
        name: string;
        stage: string | null;
        status: string;
        start: number;
        end: number;
        url: string | null;
      }[]
    >(
      `/ci/pipelines/${encodeURIComponent(provider)}/${encodeURIComponent(id)}/jobs`,
    ),
  securitySignals: (f: {
    from: number;
    to: number;
    status?: string;
    severity?: string;
    query?: string;
  }) =>
    call<{ data: SecuritySignal[] }>(`/security/signals?${profileQs(f)}`).then(
      (r) => r.data,
    ),
  triageSignal: (
    id: string,
    what: "state" | "assignee",
    attributes: Record<string, unknown>,
  ) =>
    call<unknown>(`/security/signals/${id}/${what}`, {
      method: "PATCH",
      body: JSON.stringify({ data: { attributes } }),
    }),
  securityRules: () =>
    call<{ data: SecurityRule[] }>("/security/rules").then((r) => r.data),
  saveSecurityRule: (id: string | null, rule: Partial<SecurityRule>) =>
    id
      ? put<SecurityRule>(`/security/rules/${id}`, rule)
      : post<SecurityRule>("/security/rules", rule),
  deleteSecurityRule: (id: string) => del<unknown>(`/security/rules/${id}`),
  auditEvents: (f: { from: number; to: number; query?: string }) =>
    call<{ data: AuditEvent[] }>(`/audit/events?${profileQs(f)}`).then(
      (r) => r.data,
    ),
  cases: (status?: string, query?: string) =>
    call<{ data: CaseItem[] }>(`/cases?${profileQs({ status, query })}`).then(
      (r) => r.data,
    ),
  createCase: (attributes: Record<string, unknown>) =>
    post<{ data: CaseItem }>("/cases", {
      data: { type: "case", attributes },
    }).then((r) => r.data),
  caseAction: (id: string, what: string, attributes: Record<string, unknown>) =>
    post<{ data: CaseItem }>(`/cases/${id}/${what}`, { data: { attributes } }),
  onCallPages: () =>
    call<{ data: OnCallPage[] }>("/on-call/pages").then((r) => r.data),
  pageTeam: (team: string, title: string, message: string) =>
    post<unknown>("/on-call/pages", { team, title, message }),
  pageAction: (id: string, action: "acknowledge" | "resolve") =>
    post<unknown>(`/on-call/pages/${id}/${action}`, {}),
  onCallList: <T>(what: "schedules" | "escalation-policies") =>
    call<{ data: T[] }>(`/on-call/${what}`).then((r) => r.data),
  onCallSave: (
    what: "schedules" | "escalation-policies",
    id: string | null,
    body: unknown,
  ) =>
    id
      ? put<unknown>(`/on-call/${what}/${id}`, body)
      : post<unknown>(`/on-call/${what}`, body),
  onCallDelete: (what: "schedules" | "escalation-policies", id: string) =>
    del<unknown>(`/on-call/${what}/${id}`),
  processes: (host?: string, query?: string) =>
    call<{
      hosts: { host: string; updated: number }[];
      total: number;
      processes: LiveProcess[];
    }>(`/processes?${profileQs({ host, query })}`),
  dataStreams: (from: number, to: number) =>
    call<{
      edges: {
        service: string;
        direction: string | null;
        topic: string | null;
        type: string | null;
        group: string | null;
        throughput: number;
        edge_p50: number | null;
        edge_p99: number | null;
        pathway_p50: number | null;
        pathway_p99: number | null;
      }[];
      lag: { consumer_group: string; topic: string; lag: number }[];
    }>(`/data-streams?from=${from}&to=${to}`),
  databases: (from: number, to: number, host?: string) =>
    call<{
      queries: DbQuery[];
      hosts: {
        host: string;
        dbms: string;
        calls: number;
        total_time_ms: number;
      }[];
      activity: Record<string, unknown>[];
    }>(`/databases?${profileQs({ from, to, host })}`),
  databaseQuery: (sig: string, from: number, to: number) =>
    call<{
      interval_ms: number;
      series: {
        t: number;
        calls: number;
        total_time_ms: number;
        rows: number;
      }[];
      samples: {
        ts: number;
        host: string;
        db: string | null;
        statement: string | null;
        duration_ns: number | null;
        plan: unknown;
      }[];
    }>(`/databases/queries/${encodeURIComponent(sig)}?from=${from}&to=${to}`),
  llmTraces: (f: LlmFilter) =>
    call<LlmTraceRow[]>(`/llm-obs/traces?${profileQs(f)}`),
  llmTrace: (id: string) =>
    call<{ spans: LlmSpan[]; evals: (LlmEval & { ts: number })[] }>(
      `/llm-obs/traces/${id}`,
    ),
  llmOverview: (f: LlmFilter) =>
    call<LlmOverview>(`/llm-obs/overview?${profileQs(f)}`),
  profiledServices: (f: ProfileFilter) =>
    call<ProfiledService[]>(`/profiling/services?${profileQs(f)}`),
  profiles: (f: ProfileFilter) =>
    call<ProfileRow[]>(`/profiling/profiles?${profileQs(f)}`),
  flamegraph: (
    f: ProfileFilter & {
      type?: string;
      endpoint?: string;
      profile_id?: number;
    },
  ) => post<FlameGraph>("/profiling/flamegraph", f),
  apmIngestion: () =>
    call<{ services: IngestionService[] }>("/apm/ingestion").then(
      (r) => r.services,
    ),
  apmResource: (
    service: string,
    resource: string,
    from: number,
    to: number,
    env?: string,
  ) =>
    call<ResourceDetail>(
      `/apm/resource?service=${encodeURIComponent(service)}&resource=${encodeURIComponent(resource)}&from=${from}&to=${to}${env ? `&env=${encodeURIComponent(env)}` : ""}`,
    ),
  serviceDefinitions: () =>
    call<{ data: ServiceDefinition[] }>("/services/definitions").then(
      (r) => r.data,
    ),
  definitionYaml: (service: string) =>
    call<{ yaml: string; exists: boolean }>(
      `/apm/definition-template/${encodeURIComponent(service)}`,
    ),
  saveDefinition: (yaml: string) =>
    call<unknown>("/services/definitions", {
      method: "POST",
      body: yaml,
      headers: { "content-type": "application/yaml" },
    }),
  deleteDefinition: (service: string) =>
    del<unknown>(`/services/definitions/${encodeURIComponent(service)}`),
  teams: () => call<TeamView[]>("/teams"),
  saveTeam: (
    id: string | null,
    a: { name: string; handle: string; description: string },
  ) =>
    id
      ? call<unknown>(`/team/${id}`, {
          method: "PATCH",
          body: JSON.stringify({ data: { type: "team", attributes: a } }),
        })
      : post<unknown>("/team", { data: { type: "team", attributes: a } }),
  deleteTeam: (id: string) => del<unknown>(`/team/${id}`),
  addTeamMember: (id: string, user: string, admin: boolean) =>
    post<unknown>(`/team/${id}/memberships`, {
      data: {
        type: "team_memberships",
        attributes: { role: admin ? "admin" : null },
        relationships: { user: { data: { id: user, type: "users" } } },
      },
    }),
  removeTeamMember: (id: string, user: string) =>
    del<unknown>(`/team/${id}/memberships/${encodeURIComponent(user)}`),
  addTeamLink: (id: string, label: string, url: string) =>
    post<unknown>(`/team/${id}/links`, {
      data: { type: "team_links", attributes: { label, url } },
    }),
  removeTeamLink: (id: string, link: string) =>
    del<unknown>(`/team/${id}/links/${link}`),
  scorecards: () => call<Scorecards>("/scorecards"),
  createScorecardRule: (a: {
    name: string;
    scorecard_name: string;
    description: string;
  }) =>
    post<unknown>("/scorecard/rules", {
      data: { type: "rule", attributes: { ...a, enabled: true } },
    }),
  deleteScorecardRule: (id: string) => del<unknown>(`/scorecard/rules/${id}`),
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
