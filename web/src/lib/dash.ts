import {
  api,
  ApiError,
  type DashFormula,
  type DashQuery,
  type QueryRequest,
  type ScalarResponse,
  type TemplateVar,
  type TimeseriesResponse,
  type WidgetRequest,
} from "./api";
import { compact } from "./format";
import type { TimeRange } from "./time";

export const PALETTE = [
  "#8b6fd0",
  "#3d9be9",
  "#2ab673",
  "#f5a524",
  "#e74c50",
  "#17b8be",
  "#c95cc4",
  "#7f8c9d",
  "#a3c93a",
  "#ef7d3c",
];
export const errorText = (e: unknown) =>
  e instanceof ApiError ? e.message : e ? String(e) : null;
export function fmt(v: number | null | undefined, precision?: number): string {
  if (v === null || v === undefined || Number.isNaN(v)) return "—";
  if (precision !== undefined)
    return v.toLocaleString("en-US", {
      minimumFractionDigits: precision,
      maximumFractionDigits: precision,
    });
  if (Math.abs(v) >= 10000) return compact(v);
  return Number.isInteger(v)
    ? String(v)
    : v
        .toFixed(Math.abs(v) < 1 ? 3 : 2)
        .replace(/0+$/, "")
        .replace(/\.$/, "");
}

// --- template variables ------------------------------------------------------------

export type Vars = Record<string, string>;

/** `$service` → `service:web` (or nothing for `*`), `$service.value` → `web`. */
export function substitute(
  text: string,
  defs: TemplateVar[],
  vars: Vars,
  metric: boolean,
): string {
  let out = text;
  for (const d of [...defs].sort((a, b) => b.name.length - a.name.length)) {
    const value = vars[d.name] ?? d.default ?? "*";
    out = out.split(`$${d.name}.value`).join(value);
    const tag =
      value === "*"
        ? metric
          ? "*"
          : ""
        : d.prefix
          ? `${d.prefix}:${value}`
          : value;
    out = out.split(`$${d.name}`).join(tag);
  }
  if (metric)
    out = out
      .replace(/\{\s*\*\s*,\s*/g, "{")
      .replace(/,\s*\*\s*\}/g, "}")
      .replace(/,\s*\*\s*,/g, ",");
  return out.replace(/\s+/g, " ").trim();
}

/** The request's queries, with legacy `q` requests turned into formula queries. */
export function normalizeRequest(r: WidgetRequest): {
  queries: DashQuery[];
  formulas: DashFormula[];
} {
  if (r.queries?.length)
    return { queries: r.queries, formulas: r.formulas ?? [] };
  if (r.q) {
    const parts = r.q.split(/,(?![^{]*\})/).map((s) => s.trim());
    return {
      queries: parts.map((q, i) => ({
        data_source: "metrics",
        name: `query${i + 1}`,
        query: q,
      })),
      formulas: [],
    };
  }
  return { queries: [], formulas: [] };
}

export function buildRequest(
  r: WidgetRequest,
  defs: TemplateVar[],
  vars: Vars,
  from: number,
  to: number,
): QueryRequest {
  const { queries, formulas } = normalizeRequest(r);
  return {
    from,
    to,
    queries: queries.map((q) =>
      q.data_source === "metrics"
        ? { ...q, query: substitute(q.query ?? "", defs, vars, true) }
        : {
            ...q,
            search: {
              query: substitute(q.search?.query ?? "", defs, vars, false),
            },
          },
    ),
    formulas,
  };
}

/** Where a widget's queries run when not through the query API: the
 * public dashboard links, which only run the requests stored in them. */
export type Remote = {
  run: <T>(
    r: WidgetRequest,
    from: number,
    to: number,
    scalar: boolean,
    signal?: AbortSignal,
  ) => Promise<T>;
};

export type Ctx = {
  time: TimeRange;
  defs: TemplateVar[];
  vars: Vars;
  remote?: Remote;
};

export const runTimeseries = (
  r: WidgetRequest,
  ctx: Ctx,
  from: number,
  to: number,
  signal?: AbortSignal,
) =>
  ctx.remote
    ? ctx.remote.run<TimeseriesResponse>(r, from, to, false, signal)
    : api.queryTimeseries(
        buildRequest(r, ctx.defs, ctx.vars, from, to),
        signal,
      );

export const runScalar = (
  r: WidgetRequest,
  ctx: Ctx,
  from: number,
  to: number,
  signal?: AbortSignal,
) =>
  ctx.remote
    ? ctx.remote.run<ScalarResponse>(r, from, to, true, signal)
    : api.queryScalar(buildRequest(r, ctx.defs, ctx.vars, from, to), signal);

/** The widgets that read something else than the stored requests. */
export const NOT_PUBLIC = [
  "distribution",
  "scatterplot",
  "hostmap",
  "list_stream",
  "event_stream",
  "event_timeline",
  "slo",
  "manage_status",
  "alert_graph",
  "alert_value",
  "check_status",
  "trace_service",
  "funnel",
  "split_graph",
];
