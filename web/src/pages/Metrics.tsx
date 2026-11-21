import { keepPreviousData, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { TimeChart, type Series } from "../components/Charts";
import { SidePanel } from "../components/Common";
import { PlusIcon, TrashIcon } from "../components/Icons";
import { PageHeader } from "../components/Layout";
import { TimePicker } from "../components/TimePicker";
import { api, type MetricMeta, type MetricsResponse } from "../lib/api";
import { serviceColor } from "../lib/format";
import {
  buildQuery,
  defaultUnit,
  emptyParts,
  formatValue,
  parseQuery,
  PERCENTILES,
  SPACE_AGGREGATORS,
  type QueryParts,
} from "../lib/metrics";
import { useTimeRange } from "../lib/time";

const METRICS_TABS = [
  { to: "/metrics/explorer", label: "Explorer" },
  { to: "/metrics/summary", label: "Summary" },
  { to: "/metrics/volume", label: "Volume" },
];

const LETTERS = "abcdefghij";

function useMetricList() {
  return useQuery({ queryKey: ["metrics"], queryFn: api.metrics, staleTime: 30_000 });
}

/** One query of the explorer: the field editor, or the raw query text. */
function QueryRow({
  index,
  query,
  metrics,
  onChange,
  onRemove,
  error,
}: {
  index: number;
  query: string;
  metrics: MetricMeta[];
  onChange: (q: string) => void;
  onRemove?: () => void;
  error: string | null;
}) {
  const parsed = useMemo(() => (query ? parseQuery(query) : emptyParts()), [query]);
  const [raw, setRaw] = useState(parsed === null);
  const [text, setText] = useState(query);
  const [draft, setDraft] = useState<QueryParts>(parsed ?? emptyParts());

  useEffect(() => {
    setText(query);
    const p = query ? parseQuery(query) : emptyParts();
    if (p) setDraft(p);
    else setRaw(true);
  }, [query]);

  const meta = metrics.find((m) => m.name === draft.metric);
  const isDistribution = meta?.type === "distribution";
  const isCount = meta?.type === "count" || meta?.type === "rate";
  const commit = (p: QueryParts) => {
    setDraft(p);
    if (p.metric.trim()) onChange(buildQuery(p));
  };
  const tags = useQuery({
    queryKey: ["metricTags", draft.metric],
    queryFn: () => api.metric(draft.metric, Date.now() - 3_600_000, Date.now()),
    enabled: !!meta,
    staleTime: 60_000,
  });
  const tagKeys = Object.keys(tags.data?.tags ?? {});
  const tagPairs = Object.entries(tags.data?.tags ?? {}).flatMap(([k, vs]) => vs.slice(0, 20).map((v) => `${k}:${v}`));
  const listId = `metric-${index}`;

  return (
    <div className={`metric-query${error ? " has-error" : ""}`}>
      <span className="metric-query-letter">{LETTERS[index]}</span>
      {raw ? (
        <input
          className="input mono metric-query-raw"
          value={text}
          placeholder="avg:system.load.1{env:prod} by {host}"
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && onChange(text.trim())}
          onBlur={() => text.trim() !== query && onChange(text.trim())}
          aria-label={`Query ${LETTERS[index]}`}
        />
      ) : (
        <div className="metric-query-fields">
          <input
            className="input mono"
            list={listId}
            value={draft.metric}
            placeholder="metric name"
            style={{ width: 260 }}
            onChange={(e) => setDraft({ ...draft, metric: e.target.value })}
            onBlur={() => commit(draft)}
            onKeyDown={(e) => e.key === "Enter" && commit(draft)}
            aria-label="Metric"
          />
          <datalist id={listId}>
            {metrics.map((m) => (
              <option key={m.name} value={m.name}>
                {m.type}
              </option>
            ))}
          </datalist>
          <span className="faint">from</span>
          <input
            className="input mono"
            list={`${listId}-tags`}
            value={draft.filter}
            placeholder="everywhere"
            style={{ width: 220 }}
            onChange={(e) => setDraft({ ...draft, filter: e.target.value })}
            onBlur={() => commit(draft)}
            onKeyDown={(e) => e.key === "Enter" && commit(draft)}
            aria-label="Filter"
          />
          <datalist id={`${listId}-tags`}>
            {tagPairs.map((t) => (
              <option key={t} value={draft.filter ? `${draft.filter.replace(/[^,]*$/, "")}${t}` : t} />
            ))}
          </datalist>
          <select className="input" value={draft.agg} onChange={(e) => commit({ ...draft, agg: e.target.value })} aria-label="Space aggregation">
            {SPACE_AGGREGATORS.map((a) => (
              <option key={a} value={a}>
                {a} by
              </option>
            ))}
            {isDistribution &&
              PERCENTILES.map((a) => (
                <option key={a} value={a}>
                  {a} by
                </option>
              ))}
          </select>
          <input
            className="input mono"
            list={`${listId}-keys`}
            value={draft.by.join(",")}
            placeholder="(everything)"
            style={{ width: 170 }}
            onChange={(e) => setDraft({ ...draft, by: e.target.value.split(",") })}
            onBlur={() => commit(draft)}
            onKeyDown={(e) => e.key === "Enter" && commit(draft)}
            aria-label="Group by"
          />
          <datalist id={`${listId}-keys`}>
            {["host", ...tagKeys].map((k) => (
              <option key={k} value={k} />
            ))}
          </datalist>
          {isCount && (
            <select className="input" value={draft.fn} onChange={(e) => commit({ ...draft, fn: e.target.value as QueryParts["fn"] })} aria-label="Function">
              <option value="">default</option>
              <option value="as_count">.as_count()</option>
              <option value="as_rate">.as_rate()</option>
            </select>
          )}
        </div>
      )}
      <button
        className={`btn ghost small${raw ? " selected" : ""}`}
        title={raw ? "Back to the query editor" : "Edit the query as text"}
        onClick={() => {
          if (raw) {
            const p = parseQuery(text);
            if (p) {
              setDraft(p);
              setRaw(false);
            }
          } else {
            setText(query || buildQuery(draft));
            setRaw(true);
          }
        }}
      >
        {"</>"}
      </button>
      {onRemove && (
        <button className="btn ghost small" title="Remove this query" onClick={onRemove}>
          <TrashIcon />
        </button>
      )}
      {error && <div className="metric-query-error">{error}</div>}
    </div>
  );
}

type Line = {
  key: string;
  label: string;
  color: string;
  values: number[];
  unit: string | null;
  query: number;
  band?: { lower: number[]; upper: number[] };
  dim?: boolean;
};

export function lines(data: MetricsResponse, from: number, to: number): { times: number[]; lines: Line[] } {
  const interval = data.interval_ms;
  const times: number[] = [];
  // A forecast runs past the end of the range.
  const end = Math.max(to, ...data.series.flatMap((s) => s.points.map((p) => p[0])));
  for (let t = from - (from % interval); t <= end; t += interval) times.push(t);
  const index = new Map(times.map((t, i) => [t, i]));
  const out = data.series.map((s) => {
    const values = times.map(() => NaN);
    for (const [t, v] of s.points) {
      const i = index.get(t);
      if (i !== undefined) values[i] = v;
    }
    let band: Line["band"];
    if (s.bounds) {
      band = { lower: times.map(() => NaN), upper: times.map(() => NaN) };
      for (const [t, lo, hi] of s.bounds) {
        const i = index.get(t);
        if (i !== undefined) {
          band.lower[i] = lo;
          band.upper[i] = hi;
        }
      }
    }
    const result = data.results[s.query_index];
    const group = Object.entries(s.group)
      .map(([k, v]) => `${k}:${v}`)
      .join(", ");
    const label = `${LETTERS[s.query_index]}: ${group || result.metric}`;
    return {
      key: `${s.query_index}|${group}`,
      label,
      color: serviceColor(label),
      values,
      unit: result.unit ?? defaultUnit(result.metric ?? "", result.type),
      query: s.query_index,
      band,
      dim: s.outlier === false,
    };
  });
  return { times, lines: out };
}

const stat = (vs: number[], f: (xs: number[]) => number) => {
  const xs = vs.filter((v) => !Number.isNaN(v));
  return xs.length ? f(xs) : NaN;
};

export function MetricsExplorer() {
  const time = useTimeRange("1h", 30_000);
  const [params, setParams] = useSearchParams();
  const urlQueries = params.getAll("q");
  const queries = urlQueries.length ? urlQueries : [""];
  const [kind, setKind] = useState<"lines" | "bars">("lines");
  const metrics = useMetricList();

  const setQueries = (qs: string[]) =>
    setParams((prev) => {
      const p = new URLSearchParams(prev);
      p.delete("q");
      for (const q of qs) p.append("q", q);
      return p;
    });

  const active = queries.filter(Boolean);
  const result = useQuery({
    queryKey: ["metricsQuery", active, time.from, time.to],
    queryFn: ({ signal }) => api.queryMetrics(active, time.from, time.to, signal),
    enabled: active.length > 0,
    placeholderData: keepPreviousData,
  });
  const errorOf = (q: string) => result.data?.results.find((r) => r.query === q)?.error ?? null;
  const chart = result.data ? lines(result.data, time.from, time.to) : null;
  const unit = chart?.lines[0]?.unit ?? null;

  return (
    <>
      <PageHeader title="Metrics" tabs={METRICS_TABS}>
        <TimePicker time={time} />
      </PageHeader>
      <div className="page-scroll page-padding">
        <div className="card">
          <div className="metric-queries">
            {queries.map((q, i) => (
              <QueryRow
                key={i}
                index={i}
                query={q}
                metrics={metrics.data ?? []}
                error={q ? errorOf(q) : null}
                onChange={(v) => setQueries(queries.map((x, j) => (j === i ? v : x)))}
                onRemove={queries.length > 1 ? () => setQueries(queries.filter((_, j) => j !== i)) : undefined}
              />
            ))}
          </div>
          <div className="toolbar" style={{ marginTop: 8 }}>
            {queries.length < LETTERS.length && (
              <button className="btn small" onClick={() => setQueries([...queries, ""])}>
                <PlusIcon /> Add Query
              </button>
            )}
            <div className="spacer" />
            <div className="btn-group">
              {(["lines", "bars"] as const).map((k) => (
                <button key={k} className={`btn small${kind === k ? " selected" : ""}`} onClick={() => setKind(k)}>
                  {k === "lines" ? "Lines" : "Bars"}
                </button>
              ))}
            </div>
          </div>
        </div>

        <div className="card" style={{ marginTop: 12 }}>
          {!active.length && (
            <div className="empty-state">
              Pick a metric to graph. Metrics arrive through DogStatsD (UDP 8125), <code>POST /api/v2/series</code> and the trace
              metrics (<code>trace.*</code>) derived from your spans.
            </div>
          )}
          {active.length > 0 && chart && (
            <>
              <TimeChart
                times={chart.times}
                interval={result.data!.interval_ms}
                from={time.from}
                to={Math.max(time.to, (chart.times[chart.times.length - 1] ?? 0) + result.data!.interval_ms)}
                series={chart.lines.map((l): Series => ({ key: l.key, label: l.label, color: l.color, values: l.values, band: l.band, dim: l.dim }))}
                kind={kind}
                height={280}
                format={(v) => formatValue(v, unit)}
                onZoom={(a, b) => time.setAbsolute(Math.round(a), Math.round(b))}
              />
              <table className="list metric-legend">
                <colgroup>
                  <col />
                  <col style={{ width: 110 }} />
                  <col style={{ width: 110 }} />
                  <col style={{ width: 110 }} />
                  <col style={{ width: 110 }} />
                </colgroup>
                <thead>
                  <tr>
                    <th>Series</th>
                    <th className="num">Avg</th>
                    <th className="num">Min</th>
                    <th className="num">Max</th>
                    <th className="num">Value</th>
                  </tr>
                </thead>
                <tbody>
                  {chart.lines.map((l) => (
                    <tr key={l.key}>
                      <td>
                        <i className="legend-swatch" style={{ background: l.color }} />
                        <span className="mono">{l.label}</span>
                      </td>
                      <td className="num">{formatValue(stat(l.values, (x) => x.reduce((a, b) => a + b, 0) / x.length), l.unit)}</td>
                      <td className="num">{formatValue(stat(l.values, (x) => Math.min(...x)), l.unit)}</td>
                      <td className="num">{formatValue(stat(l.values, (x) => Math.max(...x)), l.unit)}</td>
                      <td className="num">{formatValue(stat(l.values, (x) => x[x.length - 1]), l.unit)}</td>
                    </tr>
                  ))}
                  {!chart.lines.length && (
                    <tr>
                      <td colSpan={5} className="muted">
                        No data in this time range.
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </>
          )}
          {result.isError && <div className="error-box">{String(result.error)}</div>}
        </div>
      </div>
    </>
  );
}

function MetricPanel({ meta, onClose }: { meta: MetricMeta; onClose: () => void }) {
  const qc = useQueryClient();
  const now = Date.now();
  const detail = useQuery({
    queryKey: ["metricDetail", meta.name],
    queryFn: () => api.metric(meta.name, now - 3_600_000, now),
  });
  const [unit, setUnit] = useState(meta.unit ?? "");
  const [description, setDescription] = useState(meta.description ?? "");
  const [saved, setSaved] = useState(false);
  useEffect(() => {
    setUnit(meta.unit ?? "");
    setDescription(meta.description ?? "");
  }, [meta]);
  const save = async () => {
    await api.updateMetric(meta.name, { unit: unit || null, description: description || null });
    await qc.invalidateQueries({ queryKey: ["metrics"] });
    setSaved(true);
    setTimeout(() => setSaved(false), 1500);
  };
  const agg = meta.type === "count" ? "sum" : meta.type === "distribution" ? "p95" : "avg";
  const graph = `/metrics/explorer?q=${encodeURIComponent(`${agg}:${meta.name}{*}${meta.type === "count" ? ".as_count()" : ""}`)}`;

  return (
    <SidePanel
      onClose={onClose}
      storageKey="metric"
      initialWidth={0.4}
      header={
        <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <span className="mono" style={{ fontWeight: 600 }}>
            {meta.name}
          </span>
          <span className="type-badge">{meta.type}</span>
          <div className="spacer" />
          <Link className="btn small primary" to={graph}>
            Graph in Explorer
          </Link>
        </div>
      }
    >
      <div className="panel-section">
        <div className="section-title">Metadata</div>
        <div className="kv-grid">
          <span className="muted">Type</span>
          <span>{meta.type}</span>
          <span className="muted">Unit</span>
          <input className="input" value={unit} placeholder={defaultUnit(meta.name, meta.type) ?? "e.g. second, byte, request"} onChange={(e) => setUnit(e.target.value)} />
          <span className="muted">Description</span>
          <input className="input" value={description} onChange={(e) => setDescription(e.target.value)} />
        </div>
        <div style={{ marginTop: 8 }}>
          <button className="btn small primary" onClick={save}>
            {saved ? "Saved" : "Save"}
          </button>
        </div>
      </div>
      <div className="panel-section">
        <div className="section-title">Tags (past hour)</div>
        {detail.isLoading && <div className="muted">Loading…</div>}
        {detail.data && (
          <>
            <div className="muted" style={{ marginBottom: 6 }}>
              {detail.data.series} series · {detail.data.hosts.length} hosts
            </div>
            {detail.data.hosts.length > 0 && (
              <div className="metric-tag-row">
                <span className="mono">host</span>
                <span>
                  {detail.data.hosts.slice(0, 12).map((h) => (
                    <span key={h} className="tag">
                      {h}
                    </span>
                  ))}
                </span>
              </div>
            )}
            {Object.entries(detail.data.tags).map(([k, vs]) => (
              <div className="metric-tag-row" key={k}>
                <span className="mono">{k}</span>
                <span>
                  {vs.slice(0, 12).map((v) => (
                    <span key={v} className="tag">
                      {v}
                    </span>
                  ))}
                  {vs.length > 12 && <span className="muted"> +{vs.length - 12}</span>}
                </span>
              </div>
            ))}
            {!detail.data.series && <div className="muted">No points in the past hour.</div>}
          </>
        )}
      </div>
    </SidePanel>
  );
}

export function MetricsSummary() {
  const metrics = useMetricList();
  const [filter, setFilter] = useState("");
  const [type, setType] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const rows = (metrics.data ?? []).filter((m) => (!filter || m.name.includes(filter)) && (!type || m.type === type));
  const types = [...new Set((metrics.data ?? []).map((m) => m.type))].sort();
  const current = metrics.data?.find((m) => m.name === selected) ?? null;

  return (
    <>
      <PageHeader title="Metrics" tabs={METRICS_TABS} />
      <div className="filters-row">
        <input className="input" placeholder="Search metrics" value={filter} onChange={(e) => setFilter(e.target.value)} style={{ width: 280 }} />
        <select className="input" value={type} onChange={(e) => setType(e.target.value)} aria-label="Metric type">
          <option value="">All types</option>
          {types.map((t) => (
            <option key={t} value={t}>
              {t}
            </option>
          ))}
        </select>
        <div className="spacer" />
        <span className="muted">{rows.length} metrics</span>
      </div>
      <div className="page-scroll">
        <table className="list">
          <colgroup>
            <col />
            <col style={{ width: 130 }} />
            <col style={{ width: 120 }} />
            <col style={{ width: 300 }} />
            <col style={{ width: 170 }} />
          </colgroup>
          <thead>
            <tr>
              <th>Metric</th>
              <th>Type</th>
              <th>Unit</th>
              <th>Description</th>
              <th>Last reported</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((m) => (
              <tr key={m.name} className={m.name === selected ? "selected" : undefined} onClick={() => setSelected(m.name)}>
                <td className="mono">{m.name}</td>
                <td>
                  <span className="type-badge">{m.type}</span>
                </td>
                <td>{m.unit ?? <span className="faint">{defaultUnit(m.name, m.type) ?? "—"}</span>}</td>
                <td className="muted">{m.description ?? ""}</td>
                <td className="muted">{new Date(m.last_seen).toLocaleString()}</td>
              </tr>
            ))}
            {!rows.length && !metrics.isLoading && (
              <tr>
                <td colSpan={5} className="muted">
                  No metrics yet: send some through DogStatsD (UDP 8125) or <code>POST /api/v2/series</code>.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      {current && <MetricPanel meta={current} onClose={() => setSelected(null)} />}
    </>
  );
}

const VOLUME_WINDOWS: [string, number][] = [
  ["Past hour", 3_600_000],
  ["Past 4 hours", 14_400_000],
  ["Past day", 86_400_000],
];

/** Which metrics fill the storage: points and series per metric, and per prefix. */
export function MetricsVolume() {
  const [windowMs, setWindowMs] = useState(3_600_000);
  const [filter, setFilter] = useState("");
  const [byPrefix, setByPrefix] = useState(false);
  const q = useQuery({
    queryKey: ["metricsVolume", windowMs],
    queryFn: () => {
      const to = Date.now();
      return api.metricsVolume(to - windowMs, to);
    },
    placeholderData: keepPreviousData,
  });
  const all = q.data ?? [];
  const total = all.reduce((a, m) => a + m.points, 0);
  const rows = useMemo(() => {
    let list = all;
    if (byPrefix) {
      const groups = new Map<string, { metric: string; points: number; series: number }>();
      for (const m of all) {
        const prefix = `${m.metric.split(".")[0]}.*`;
        const g = groups.get(prefix) ?? { metric: prefix, points: 0, series: 0 };
        g.points += m.points;
        g.series += m.series;
        groups.set(prefix, g);
      }
      list = [...groups.values()].sort((a, b) => b.points - a.points);
    }
    const f = filter.trim().toLowerCase();
    return f ? list.filter((m) => m.metric.toLowerCase().includes(f)) : list;
  }, [all, byPrefix, filter]);
  const perDay = (n: number) => formatValue((n * 86_400_000) / windowMs);
  return (
    <>
      <PageHeader title="Metrics" tabs={METRICS_TABS} />
      <div className="filters-row">
        <input className="input" placeholder="Search metrics" value={filter} onChange={(e) => setFilter(e.target.value)} style={{ width: 280 }} />
        <select className="input" value={windowMs} onChange={(e) => setWindowMs(Number(e.target.value))} aria-label="Window">
          {VOLUME_WINDOWS.map(([label, ms]) => (
            <option key={ms} value={ms}>
              {label}
            </option>
          ))}
        </select>
        <div className="btn-group">
          <button className={`btn small${byPrefix ? "" : " selected"}`} onClick={() => setByPrefix(false)}>
            By metric
          </button>
          <button className={`btn small${byPrefix ? " selected" : ""}`} onClick={() => setByPrefix(true)}>
            By prefix
          </button>
        </div>
        <div className="spacer" />
        <span className="muted">
          {formatValue(total)} points stored, about {perDay(total)} a day
        </span>
      </div>
      <div className="page-scroll">
        <table className="list">
          <thead>
            <tr>
              <th>Metric</th>
              <th className="num" style={{ width: 130 }}>
                Points
              </th>
              <th className="num" style={{ width: 130 }}>
                Per day
              </th>
              <th className="num" style={{ width: 110 }}>
                Series
              </th>
              <th style={{ width: "26%" }}>Share</th>
            </tr>
          </thead>
          <tbody>
            {rows.slice(0, 500).map((m) => (
              <tr key={m.metric}>
                <td className="mono ellipsis">
                  {byPrefix ? m.metric : <Link to={`/metrics/explorer?q=${encodeURIComponent(`avg:${m.metric}{*}`)}`}>{m.metric}</Link>}
                </td>
                <td className="num">{formatValue(m.points)}</td>
                <td className="num">{perDay(m.points)}</td>
                <td className="num">{formatValue(m.series)}</td>
                <td>
                  <div className="volume-bar">
                    <div style={{ width: `${total ? (m.points / total) * 100 : 0}%` }} />
                    <span>{total ? ((m.points / total) * 100).toFixed(1) : 0} %</span>
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {q.data && rows.length === 0 && <div className="empty-state">No metric stored over this window.</div>}
      </div>
    </>
  );
}
