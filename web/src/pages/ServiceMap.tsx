import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { PageHeader } from "../components/Layout";
import { TimePicker } from "../components/TimePicker";
import { api, type ServiceEdge, type ServiceSummary } from "../lib/api";
import { compact, duration, percent, rate, spanTypeIcon } from "../lib/format";
import { useTimeRange } from "../lib/time";
import { EnvSelect, useEnv } from "./Apm";
import { APM_TABS } from "./TraceExplorer";

const COL = 250;
const ROW = 118;
const R = 28;

function health(errorRate: number): string {
  if (errorRate >= 0.05) return "#e74c50";
  if (errorRate >= 0.01) return "#f5a524";
  return "#2ab673";
}

type Placed = {
  name: string;
  x: number;
  y: number;
  depth: number;
  summary?: ServiceSummary;
};

/** Layered layout: callers left of callees, rows ordered by their callers. */
function layout(names: string[], edges: ServiceEdge[]): Map<string, Placed> {
  const depth = new Map(names.map((n) => [n, 0]));
  for (let i = 0; i < names.length; i++) {
    let changed = false;
    for (const e of edges) {
      const d = (depth.get(e.source) ?? 0) + 1;
      if (
        e.source !== e.target &&
        d > (depth.get(e.target) ?? 0) &&
        d < names.length
      ) {
        depth.set(e.target, d);
        changed = true;
      }
    }
    if (!changed) break;
  }
  const columns: string[][] = [];
  for (const n of names) (columns[depth.get(n) ?? 0] ??= []).push(n);
  const row = new Map<string, number>();
  columns.forEach((col, c) => {
    const score = (n: string) => {
      const parents = edges
        .filter((e) => e.target === n && row.has(e.source))
        .map((e) => row.get(e.source)!);
      return parents.length
        ? parents.reduce((a, b) => a + b, 0) / parents.length
        : Infinity;
    };
    if (c > 0) col.sort((a, b) => score(a) - score(b) || a.localeCompare(b));
    else
      col.sort(
        (a, b) =>
          edges.filter((e) => e.source === b).length -
            edges.filter((e) => e.source === a).length || a.localeCompare(b),
      );
    col.forEach((n, i) => row.set(n, i - (col.length - 1) / 2));
  });
  const tallest = Math.max(1, ...columns.map((c) => c?.length ?? 0));
  const out = new Map<string, Placed>();
  for (const n of names) {
    const d = depth.get(n) ?? 0;
    out.set(n, {
      name: n,
      depth: d,
      x: 110 + d * COL,
      y: 70 + ((tallest - 1) / 2 + (row.get(n) ?? 0)) * ROW,
    });
  }
  return out;
}

export function ServiceMap() {
  const time = useTimeRange("1h", 60_000);
  const [env] = useEnv();
  const [filter, setFilter] = useState("");
  const [hover, setHover] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const edges = useQuery({
    queryKey: ["servicemap", time.from, time.to, env],
    queryFn: () => api.serviceMap(time.from, time.to, env || undefined),
    placeholderData: keepPreviousData,
  });
  const services = useQuery({
    queryKey: ["services", time.from, time.to, env],
    queryFn: () => api.services(time.from, time.to, env || undefined),
    placeholderData: keepPreviousData,
  });

  const graph = useMemo(() => {
    const es = edges.data ?? [];
    const byName = new Map((services.data ?? []).map((s) => [s.name, s]));
    const names = [
      ...new Set([
        ...byName.keys(),
        ...es.flatMap((e) => [e.source, e.target]),
      ]),
    ]
      .filter(Boolean)
      .sort();
    const placed = layout(names, es);
    for (const p of placed.values()) p.summary = byName.get(p.name);
    const maxCalls = Math.max(1, ...es.map((e) => e.calls));
    const width = Math.max(...[...placed.values()].map((p) => p.x), 0) + 160;
    const height = Math.max(...[...placed.values()].map((p) => p.y), 0) + 90;
    return { es, placed, maxCalls, width, height };
  }, [edges.data, services.data]);

  const focus =
    hover ??
    selected ??
    (filter
      ? ([...graph.placed.keys()].find((n) => n.includes(filter)) ?? null)
      : null);
  const neighbours = useMemo(() => {
    if (!focus) return null;
    const s = new Set([focus]);
    for (const e of graph.es) {
      if (e.source === focus) s.add(e.target);
      if (e.target === focus) s.add(e.source);
    }
    return s;
  }, [focus, graph.es]);
  const minutes = Math.max(1, (time.to - time.from) / 60_000);
  const sel = selected ? graph.placed.get(selected) : null;

  return (
    <>
      <PageHeader title="APM" tabs={APM_TABS}>
        <TimePicker time={time} />
      </PageHeader>
      <div className="filters-row">
        <EnvSelect />
        <input
          className="input"
          placeholder="Find a service"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          style={{ width: 240 }}
        />
        <div className="spacer" />
        <span className="map-legend">
          <i style={{ background: "#2ab673" }} /> &lt;1% errors{" "}
          <i style={{ background: "#f5a524" }} /> 1–5%{" "}
          <i style={{ background: "#e74c50" }} /> ≥5%
        </span>
        <span className="muted">
          {graph.placed.size} services · {graph.es.length} dependencies
        </span>
      </div>
      <div className="service-map-wrap">
        <div className="service-map-scroll" onClick={() => setSelected(null)}>
          {graph.placed.size === 0 && !edges.isLoading && (
            <div className="empty-state">No traces in this time range.</div>
          )}
          <svg
            width={graph.width}
            height={graph.height}
            className="service-map"
          >
            <defs>
              {["#b8bcc8", "#f5a524", "#e74c50", "#632ca6"].map((c) => (
                <marker
                  key={c}
                  id={`arrow-${c.slice(1)}`}
                  viewBox="0 0 10 10"
                  refX="9"
                  refY="5"
                  markerWidth="9"
                  markerHeight="9"
                  markerUnits="userSpaceOnUse"
                  orient="auto-start-reverse"
                >
                  <path d="M 0 0 L 10 5 L 0 10 z" fill={c} />
                </marker>
              ))}
            </defs>
            {graph.es.map((e) => {
              const a = graph.placed.get(e.source);
              const b = graph.placed.get(e.target);
              if (!a || !b || a === b) return null;
              const er = e.calls ? e.errors / e.calls : 0;
              const active =
                focus && (e.source === focus || e.target === focus);
              const color = active
                ? "#632ca6"
                : er >= 0.05
                  ? "#e74c50"
                  : er >= 0.01
                    ? "#f5a524"
                    : "#b8bcc8";
              const w =
                1 +
                (3 * Math.log10(1 + e.calls)) / Math.log10(1 + graph.maxCalls);
              const forward = b.x > a.x;
              const [x1, y1, x2, y2] = forward
                ? [a.x + R, a.y, b.x - R - 4, b.y]
                : [a.x, a.y + R, b.x, b.y + R + 4];
              const mx = (x1 + x2) / 2;
              const d = forward
                ? `M ${x1} ${y1} C ${mx} ${y1}, ${mx} ${y2}, ${x2} ${y2}`
                : `M ${x1} ${y1} C ${x1} ${y1 + 60}, ${x2} ${y2 + 60}, ${x2} ${y2}`;
              return (
                <g
                  key={`${e.source}->${e.target}`}
                  opacity={neighbours && !active ? 0.15 : 1}
                >
                  <path
                    d={d}
                    fill="none"
                    stroke={color}
                    strokeWidth={w}
                    markerEnd={`url(#arrow-${color.slice(1)})`}
                  >
                    <title>
                      {`${e.source} → ${e.target}\n${compact(e.calls)} calls · ${percent(er)} errors · avg ${duration(e.avg)}`}
                    </title>
                  </path>
                  {active && (
                    <text
                      x={mx}
                      y={(y1 + y2) / 2 - 6}
                      className="edge-label"
                      textAnchor="middle"
                    >
                      {`${(e.calls / minutes).toFixed(e.calls / minutes >= 10 ? 0 : 1)}/min · ${duration(e.avg)}`}
                    </text>
                  )}
                </g>
              );
            })}
            {[...graph.placed.values()].map((p) => {
              const s = p.summary;
              const color = s ? health(s.error_rate) : "#b8bcc8";
              const dim = neighbours && !neighbours.has(p.name);
              return (
                <g
                  key={p.name}
                  transform={`translate(${p.x}, ${p.y})`}
                  className={`map-node${selected === p.name ? " selected" : ""}`}
                  opacity={dim ? 0.25 : 1}
                  onMouseEnter={() => setHover(p.name)}
                  onMouseLeave={() => setHover(null)}
                  onClick={(e) => {
                    e.stopPropagation();
                    setSelected(p.name);
                  }}
                >
                  <circle
                    r={R + 4}
                    fill="none"
                    stroke={color}
                    strokeWidth={4}
                    opacity={0.35}
                  />
                  <circle
                    r={R}
                    fill="var(--bg)"
                    stroke={color}
                    strokeWidth={2.5}
                  />
                  <text className="node-type" textAnchor="middle" dy="4">
                    {s ? spanTypeIcon(s.type) : "EXT"}
                  </text>
                  <text className="node-name" textAnchor="middle" y={R + 20}>
                    {p.name}
                  </text>
                  {s && (
                    <text className="node-stats" textAnchor="middle" y={R + 35}>
                      {`${rate(s.requests_per_second)} · ${duration(s.p95)}`}
                    </text>
                  )}
                </g>
              );
            })}
          </svg>
        </div>
        {sel && (
          <aside className="map-panel">
            <div className="map-panel-title">
              <strong>{sel.name}</strong>
              <span className="span-type">
                {sel.summary ? spanTypeIcon(sel.summary.type) : "EXT"}
              </span>
            </div>
            {sel.summary ? (
              <div className="map-stats">
                <div>
                  <span className="faint">Requests</span>
                  <strong>{rate(sel.summary.requests_per_second)}</strong>
                </div>
                <div>
                  <span className="faint">Errors</span>
                  <strong style={{ color: health(sel.summary.error_rate) }}>
                    {percent(sel.summary.error_rate)}
                  </strong>
                </div>
                <div>
                  <span className="faint">p50</span>
                  <strong>{duration(sel.summary.p50)}</strong>
                </div>
                <div>
                  <span className="faint">p95</span>
                  <strong>{duration(sel.summary.p95)}</strong>
                </div>
              </div>
            ) : (
              <div className="faint">
                No entry spans for this service in the range.
              </div>
            )}
            {(["source", "target"] as const).map((side) => {
              const list = graph.es.filter(
                (e) => (side === "source" ? e.target : e.source) === sel.name,
              );
              return (
                <div key={side} className="map-deps">
                  <div className="breakdown-title">
                    {side === "source" ? "Called by" : "Calls"}
                  </div>
                  {list.map((e) => (
                    <button
                      key={e[side]}
                      className="map-dep"
                      onClick={() => setSelected(e[side])}
                    >
                      <span>{e[side]}</span>
                      <span className="faint">
                        {compact(e.calls)} · {duration(e.avg)}
                      </span>
                    </button>
                  ))}
                  {!list.length && <div className="faint">None</div>}
                </div>
              );
            })}
            {sel.summary && (
              <Link
                className="btn primary"
                to={`/apm/services/${encodeURIComponent(sel.name)}${window.location.search}`}
              >
                View service page
              </Link>
            )}
          </aside>
        )}
      </div>
    </>
  );
}
