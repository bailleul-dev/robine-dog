/**
 * Datadog metric queries, split into the fields of the query editor and
 * assembled back: `sum:trace.http.request.hits{env:prod} by {service}.as_count()`.
 */

export const SPACE_AGGREGATORS = ["avg", "sum", "min", "max"] as const;
export const PERCENTILES = ["p50", "p75", "p90", "p95", "p99"] as const;

export type QueryParts = {
  agg: string;
  metric: string;
  /** Tag filter, Datadog style: `env:prod,!host:a`; empty means `*`. */
  filter: string;
  by: string[];
  fn: "" | "as_count" | "as_rate";
  /** `.rollup(...)` arguments, e.g. `max, 60`. */
  rollup: string;
};

export const emptyParts = (): QueryParts => ({ agg: "avg", metric: "", filter: "", by: [], fn: "", rollup: "" });

export function buildQuery(p: QueryParts): string {
  const filter = p.filter.trim() || "*";
  let q = `${p.agg}:${p.metric.trim()}{${filter}}`;
  const by = p.by.map((b) => b.trim()).filter(Boolean);
  if (by.length) q += ` by {${by.join(",")}}`;
  if (p.fn) q += `.${p.fn}()`;
  if (p.rollup.trim()) q += `.rollup(${p.rollup.trim()})`;
  return q;
}

const RE =
  /^\s*(?:(avg|sum|min|max|count|p50|p75|p90|p95|p99|median):)?([\w.-]+?)(?:\{([^}]*)\})?(?:\s*by\s*\{([^}]*)\})?((?:\s*\.\w+\([^)]*\))*)\s*$/;

/** The editor fields of a query, or null when it only fits the raw editor. */
export function parseQuery(q: string): QueryParts | null {
  const m = RE.exec(q);
  if (!m) return null;
  const parts: QueryParts = {
    agg: m[1] === "median" ? "p50" : (m[1] ?? "avg"),
    metric: m[2],
    filter: (m[3] ?? "").trim() === "*" ? "" : (m[3] ?? "").trim(),
    by: (m[4] ?? "")
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean),
    fn: "",
    rollup: "",
  };
  for (const f of (m[5] ?? "").matchAll(/\.(\w+)\(([^)]*)\)/g)) {
    if (f[1] === "as_count" || f[1] === "as_rate") parts.fn = f[1];
    else if (f[1] === "rollup") parts.rollup = f[2].trim();
    else return null;
  }
  return parts;
}

/** A readable value with its unit: seconds become durations, bytes get a prefix. */
export function formatValue(v: number, unit?: string | null): string {
  if (!Number.isFinite(v)) return "–";
  const abs = Math.abs(v);
  if (unit === "second" || unit === "s") {
    if (abs >= 1) return `${v.toFixed(abs >= 10 ? 1 : 2)} s`;
    if (abs >= 1e-3) return `${(v * 1e3).toFixed(abs >= 0.1 ? 0 : 1)} ms`;
    if (abs > 0) return `${(v * 1e6).toFixed(0)} µs`;
    return "0 s";
  }
  if (unit === "byte") {
    const units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let i = 0;
    let x = abs;
    while (x >= 1024 && i < units.length - 1) {
      x /= 1024;
      i++;
    }
    return `${(Math.sign(v) * x).toFixed(x >= 100 || i === 0 ? 0 : 1)} ${units[i]}`;
  }
  const suffix = unit && unit !== "byte" ? ` ${unit}` : "";
  if (abs >= 1e9) return `${(v / 1e9).toFixed(2)}B${suffix}`;
  if (abs >= 1e6) return `${(v / 1e6).toFixed(2)}M${suffix}`;
  if (abs >= 1e4) return `${(v / 1e3).toFixed(1)}K${suffix}`;
  if (abs >= 100 || Number.isInteger(v)) return `${Math.round(v * 10) / 10}${suffix}`;
  if (abs >= 1) return `${v.toFixed(2)}${suffix}`;
  return `${v.toPrecision(3)}${suffix}`;
}

/** Datadog's trace metrics and distributions are in seconds. */
export function defaultUnit(metric: string, type: string | null): string | null {
  if (type === "distribution" && metric.startsWith("trace.")) return "second";
  return null;
}
