import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import { useSearchParams } from "react-router-dom";
import { api, ApiError, type AggregateBucket, type Scope } from "../lib/api";
import {
  compact,
  duration,
  serviceColor,
  statusColor,
  thousands,
} from "../lib/format";
import { TimeChart, type Series } from "./Charts";
import type { KeyHint } from "./SearchBar";

export type Viz = "list" | "patterns" | "timeseries" | "toplist" | "table";
type Dataset = "logs" | "spans";

const VIZ_LABELS: Record<Viz, string> = {
  list: "List",
  patterns: "Patterns",
  timeseries: "Timeseries",
  toplist: "Top List",
  table: "Table",
};

const AGGREGATIONS: { value: string; label: string }[] = [
  { value: "count", label: "Count of" },
  { value: "cardinality", label: "Unique count of" },
  { value: "avg", label: "Avg of" },
  { value: "sum", label: "Sum of" },
  { value: "min", label: "Min of" },
  { value: "max", label: "Max of" },
  { value: "median", label: "Median of" },
  { value: "pc75", label: "pc75 of" },
  { value: "pc90", label: "pc90 of" },
  { value: "pc95", label: "pc95 of" },
  { value: "pc99", label: "pc99 of" },
];

/** Visualization settings, kept in the URL like Datadog's explorers. */
export function useViz(dataset: Dataset) {
  const [params, setParams] = useSearchParams();
  const get = (k: string, d: string) => params.get(k) ?? d;
  const set = (patch: Record<string, string | null>) =>
    setParams((prev) => {
      const p = new URLSearchParams(prev);
      for (const [k, v] of Object.entries(patch)) {
        if (v === null || v === "") p.delete(k);
        else p.set(k, v);
      }
      return p;
    });
  return {
    viz: get("viz", "list") as Viz,
    agg: get("agg", "count"),
    measure: get("measure", dataset === "spans" ? "@duration" : ""),
    by: get("by", ""),
    by2: get("by2", ""),
    limit: Number(get("limit", "10")),
    kind: get("kind", "lines") as "lines" | "bars",
    set,
  };
}

export type VizState = ReturnType<typeof useViz>;

export function VizPicker({
  viz,
  options,
  onChange,
}: {
  viz: Viz;
  options: Viz[];
  onChange: (v: Viz) => void;
}) {
  return (
    <div className="viz-picker">
      <span className="faint">Visualize as</span>
      <div className="btn-group">
        {options.map((o) => (
          <button
            key={o}
            className={`btn small${viz === o ? " selected" : ""}`}
            onClick={() => onChange(o)}
          >
            {VIZ_LABELS[o]}
          </button>
        ))}
      </div>
    </div>
  );
}

const isDuration = (dataset: Dataset, measure: string) =>
  measure === "@duration" || (dataset === "spans" && measure === "duration");

function formatter(
  dataset: Dataset,
  agg: string,
  measure: string,
): (v: number) => string {
  if (agg === "count" || agg === "cardinality") return compact;
  if (isDuration(dataset, measure)) return (v) => duration(v);
  return compact;
}

function Controls({
  dataset,
  v,
  keys,
}: {
  dataset: Dataset;
  v: VizState;
  keys: KeyHint[];
}) {
  const facetList = `${dataset}-facet-keys`;
  const needsMeasure = v.agg !== "count";
  return (
    <div className="analytics-controls">
      <span className="faint">Show</span>
      <select
        className="input"
        value={v.agg}
        onChange={(e) => v.set({ agg: e.target.value })}
      >
        {AGGREGATIONS.map((a) => (
          <option key={a.value} value={a.value}>
            {a.label}
          </option>
        ))}
      </select>
      {needsMeasure ? (
        <input
          className="input mono"
          list={facetList}
          placeholder={v.agg === "cardinality" ? "@usr.id" : "@duration"}
          defaultValue={v.measure}
          key={`m-${v.measure}`}
          onBlur={(e) => v.set({ measure: e.target.value.trim() })}
          onKeyDown={(e) =>
            e.key === "Enter" &&
            v.set({ measure: (e.target as HTMLInputElement).value.trim() })
          }
          style={{ width: 200 }}
        />
      ) : (
        <span className="analytics-fixed">all {dataset}</span>
      )}
      <span className="faint">by</span>
      <input
        className="input mono"
        list={facetList}
        placeholder="Everything"
        defaultValue={v.by}
        key={`b-${v.by}`}
        onBlur={(e) => v.set({ by: e.target.value.trim() })}
        onKeyDown={(e) =>
          e.key === "Enter" &&
          v.set({ by: (e.target as HTMLInputElement).value.trim() })
        }
        style={{ width: 180 }}
      />
      {v.viz === "table" && v.by && (
        <>
          <span className="faint">then by</span>
          <input
            className="input mono"
            list={facetList}
            placeholder="—"
            defaultValue={v.by2}
            key={`b2-${v.by2}`}
            onBlur={(e) => v.set({ by2: e.target.value.trim() })}
            onKeyDown={(e) =>
              e.key === "Enter" &&
              v.set({ by2: (e.target as HTMLInputElement).value.trim() })
            }
            style={{ width: 160 }}
          />
        </>
      )}
      {v.by && (
        <>
          <span className="faint">limit to top</span>
          <select
            className="input"
            value={v.limit}
            onChange={(e) => v.set({ limit: e.target.value })}
          >
            {[5, 10, 25, 50, 100].map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
        </>
      )}
      {v.viz === "timeseries" && (
        <div className="btn-group" style={{ marginLeft: "auto" }}>
          {(["lines", "bars"] as const).map((k) => (
            <button
              key={k}
              className={`btn small${v.kind === k ? " selected" : ""}`}
              onClick={() => v.set({ kind: k })}
            >
              {k === "lines" ? "Lines" : "Bars"}
            </button>
          ))}
        </div>
      )}
      <datalist id={facetList}>
        {keys.map((k) => (
          <option key={k.path} value={k.path}>
            {k.name}
          </option>
        ))}
      </datalist>
    </div>
  );
}

const label = (b: AggregateBucket, facets: string[]) =>
  facets.map((f) => String(b.by[f] ?? "")).join(" · ") || "All";

/** Timeseries, top list and table views of logs or spans. */
export function AnalyticsView({
  dataset,
  scope,
  keys,
  v,
  onZoom,
  onFilter,
}: {
  dataset: Dataset;
  scope: Scope;
  keys: KeyHint[];
  v: VizState;
  onZoom: (from: number, to: number) => void;
  onFilter: (facet: string, value: string) => void;
}) {
  const facets = [v.by, v.viz === "table" ? v.by2 : ""].filter(Boolean);
  const metric = v.agg === "count" ? undefined : v.measure || undefined;
  const ready = v.agg === "count" || !!metric;
  const timeseries = v.viz === "timeseries";
  const req = {
    compute: [
      { aggregation: v.agg, metric, type: timeseries ? "timeseries" : "total" },
    ],
    filter: { query: scope.query, from: scope.from, to: scope.to },
    group_by: facets.map((f) => ({ facet: f, limit: v.limit })),
  };
  const res = useQuery({
    queryKey: ["aggregate", dataset, req],
    queryFn: ({ signal }) => api.aggregate(dataset, req, signal),
    enabled: ready,
    placeholderData: keepPreviousData,
  });
  const fmt = formatter(dataset, v.agg, v.measure);
  const error =
    res.error instanceof ApiError
      ? res.error.message
      : res.error
        ? String(res.error)
        : null;
  const buckets = res.data?.data.buckets ?? [];
  const interval = res.data?.meta.interval_ms ?? 60_000;

  const chart = useMemo(() => {
    if (!timeseries) return null;
    const start = Math.floor(scope.from / interval) * interval;
    const times: number[] = [];
    for (let t = start; t <= scope.to; t += interval) times.push(t);
    const fill = v.agg === "count" ? 0 : NaN;
    const series: Series[] = buckets.map((b) => {
      const byTime = new Map<number, number>();
      for (const p of (b.computes.c0 as {
        time: string;
        value: number | null;
      }[]) ?? [])
        byTime.set(Date.parse(p.time), p.value ?? NaN);
      const name = label(b, facets);
      return {
        key: name,
        label: name,
        color: facets.length ? serviceColor(name) : "#8b6fd0",
        values: times.map((t) => byTime.get(t) ?? fill),
      };
    });
    return { times, series };
  }, [
    timeseries,
    buckets,
    interval,
    scope.from,
    scope.to,
    v.agg,
    facets.join(),
  ]);

  const totals = buckets.map((b) => ({
    b,
    value: (b.computes.c0 as number | null) ?? NaN,
  }));
  const max = Math.max(
    1,
    ...totals.map((t) => (Number.isFinite(t.value) ? t.value : 0)),
  );

  return (
    <div className="analytics">
      <Controls dataset={dataset} v={v} keys={keys} />
      {error && <div className="error-box">{error}</div>}
      {!ready && (
        <div className="empty-state">Pick a measure to aggregate.</div>
      )}
      {ready && res.isLoading && (
        <div className="empty-state faint">Loading…</div>
      )}
      {ready && res.data && !buckets.length && (
        <div className="empty-state">
          <h3>No data</h3>
          Nothing matches this query{facets.length ? " with these facets" : ""}.
        </div>
      )}
      {chart && buckets.length > 0 && (
        <div className="card analytics-chart">
          <TimeChart
            times={chart.times}
            interval={interval}
            from={scope.from}
            to={scope.to}
            series={chart.series}
            kind={v.kind}
            height={300}
            format={fmt}
            onZoom={onZoom}
          />
          {facets.length > 0 && (
            <div className="analytics-legend">
              {chart.series.map((s, i) => (
                <button
                  key={s.key}
                  className="legend-item"
                  onClick={() =>
                    onFilter(facets[0], String(buckets[i].by[facets[0]]))
                  }
                  title="Filter on this value"
                >
                  <span
                    className="legend-swatch"
                    style={{ background: s.color }}
                  />
                  {s.label}
                </button>
              ))}
            </div>
          )}
        </div>
      )}
      {v.viz === "toplist" && buckets.length > 0 && (
        <div className="card toplist">
          {totals.map(({ b, value }, i) => (
            <button
              key={i}
              className="toplist-row"
              onClick={() =>
                facets[0] && onFilter(facets[0], String(b.by[facets[0]]))
              }
              disabled={!facets.length}
            >
              <span className="toplist-label">{label(b, facets)}</span>
              <span className="toplist-bar">
                <span
                  style={{
                    width: `${(Math.max(0, Number.isFinite(value) ? value : 0) / max) * 100}%`,
                  }}
                />
              </span>
              <span className="toplist-value">
                {Number.isFinite(value) ? fmt(value) : "—"}
              </span>
            </button>
          ))}
        </div>
      )}
      {v.viz === "table" && buckets.length > 0 && (
        <div className="card" style={{ padding: 0 }}>
          <table className="list analytics-table">
            <thead>
              <tr>
                {facets.length ? (
                  facets.map((f) => <th key={f}>{f}</th>)
                ) : (
                  <th>All</th>
                )}
                <th className="num">
                  {AGGREGATIONS.find((a) => a.value === v.agg)?.label.replace(
                    / of$/,
                    "",
                  )}{" "}
                  {metric ?? dataset}
                </th>
              </tr>
            </thead>
            <tbody>
              {totals.map(({ b, value }, i) => (
                <tr key={i}>
                  {facets.length ? (
                    facets.map((f) => (
                      <td key={f}>
                        <button
                          className="link-button"
                          onClick={() => onFilter(f, String(b.by[f]))}
                        >
                          {String(b.by[f])}
                        </button>
                      </td>
                    ))
                  ) : (
                    <td>All</td>
                  )}
                  <td className="num">
                    {Number.isFinite(value)
                      ? v.agg === "count"
                        ? thousands(value)
                        : fmt(value)
                      : "—"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

/** Log Patterns: the latest matching logs grouped by message shape. */
export function PatternsView({
  scope,
  onOpen,
}: {
  scope: Scope;
  onOpen: (query: string) => void;
}) {
  const res = useQuery({
    queryKey: ["patterns", scope],
    queryFn: ({ signal }) => api.logPatterns(scope, signal),
    placeholderData: keepPreviousData,
  });
  const patterns = res.data?.patterns ?? [];
  const max = Math.max(1, ...patterns.map((p) => p.count));
  return (
    <div className="analytics">
      <div className="toolbar">
        {res.data ? (
          <span>
            <strong>{thousands(patterns.length)}</strong> patterns in the latest{" "}
            <strong>{thousands(res.data.sampled)}</strong> logs
          </span>
        ) : (
          <span className="faint">Grouping…</span>
        )}
        {res.isFetching && <span className="spinner" />}
      </div>
      <div className="table-wrap">
        <table className="list patterns-table">
          <colgroup>
            <col style={{ width: 6 }} />
            <col style={{ width: 150 }} />
            <col style={{ width: 160 }} />
            <col />
          </colgroup>
          <thead>
            <tr>
              <th />
              <th>Count</th>
              <th>Service</th>
              <th>Pattern</th>
            </tr>
          </thead>
          <tbody>
            {patterns.map((p, i) => (
              <tr
                key={i}
                onClick={() => {
                  const q =
                    p.query ||
                    (p.services[0] ? `service:${p.services[0][0]}` : "");
                  onOpen([scope.query, q].filter(Boolean).join(" "));
                }}
                title={p.sample}
              >
                <td className="status-cell">
                  <span
                    className="status-bar"
                    style={{ background: statusColor(p.status) }}
                  />
                </td>
                <td>
                  <div className="pattern-count">
                    <span>{thousands(p.count)}</span>
                    <span className="pattern-bar">
                      <span
                        style={{
                          width: `${(p.count / max) * 100}%`,
                          background: statusColor(p.status),
                        }}
                      />
                    </span>
                  </div>
                </td>
                <td className="ellipsis">
                  {p.services.map(([s]) => s).join(", ")}
                </td>
                <td className="content-cell mono">
                  {p.pattern.split(" ").map((t, j) => (
                    <span key={j}>
                      {j ? " " : ""}
                      {t === "*" ? (
                        <span className="pattern-wildcard">*</span>
                      ) : (
                        t
                      )}
                    </span>
                  ))}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {res.data && !patterns.length && (
          <div className="empty-state">
            <h3>No logs match this query</h3>
          </div>
        )}
      </div>
    </div>
  );
}
