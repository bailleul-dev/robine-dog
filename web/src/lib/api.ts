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
export type FacetValues = { path: string; values: { value: string; count: number }[]; min: number | null; max: number | null };
export type FacetDef = { id: number; dataset: string; path: string; name: string; group: string };

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

export type ServiceDetail = {
  name: string;
  type: string;
  operations: string[];
  summary: Stats;
  interval_ms: number;
  timeseries: { t: number; hits: number; errors: number; p50: number; p95: number; p99: number; max: number }[];
  resources: (Stats & { resource: string; operation: string })[];
  distribution: [number, number][];
  downstream: { service: string; calls: number; errors: number; avg: number }[];
};

export type Identity = { sub: string; name: string | null; email: string | null };

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

export type Scope = { query: string; from: number; to: number };

export const api = {
  me: () => call<Identity>("/me"),
  searchLogs: (s: Scope & { limit?: number; cursor?: string; after_id?: number }, signal?: AbortSignal) =>
    post<{ logs: Log[]; next_cursor: string | null; total: number | null }>("/logs/search", s, signal),
  logsHistogram: (s: Scope, signal?: AbortSignal) => post<Histogram>("/logs/histogram", s, signal),
  log: (id: number) => call<Log>(`/logs/${id}`),
  searchSpans: (s: Scope & { limit?: number; cursor?: string; sort?: string }, signal?: AbortSignal) =>
    post<{ spans: Span[]; next_cursor: string | null; total: number | null }>("/spans/search", s, signal),
  spansHistogram: (s: Scope, signal?: AbortSignal) => post<Histogram>("/spans/histogram", s, signal),
  trace: (traceId: string) => call<{ trace_id: string; spans: Span[] }>(`/traces/${encodeURIComponent(traceId)}`),
  facetValues: (dataset: "logs" | "spans", s: Scope, facets: string[], limit = 10, signal?: AbortSignal) =>
    post<{ facets: FacetValues[] }>("/facets/values", { dataset, ...s, facets, limit }, signal),
  facetDefs: (dataset: "logs" | "spans") => call<FacetDef[]>(`/facets?dataset=${dataset}`),
  addFacet: (f: Omit<FacetDef, "id">) => post<FacetDef>("/facets", f),
  deleteFacet: (id: number) => call<void>(`/facets/${id}`, { method: "DELETE" }),
  services: (from: number, to: number, env?: string) =>
    call<ServiceSummary[]>(`/apm/services?from=${from}&to=${to}${env ? `&env=${encodeURIComponent(env)}` : ""}`),
  service: (name: string, from: number, to: number, env?: string) =>
    call<ServiceDetail>(
      `/apm/services/${encodeURIComponent(name)}?from=${from}&to=${to}${env ? `&env=${encodeURIComponent(env)}` : ""}`,
    ),
  envs: () => call<string[]>("/apm/envs"),
};
