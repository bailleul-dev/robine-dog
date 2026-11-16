import { useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { useWidth } from "../components/Charts";
import { HttpCode, ServiceChip, SidePanel, StatusPill } from "../components/Common";
import { JsonTree } from "../components/JsonTree";
import { api, type Span } from "../lib/api";
import { duration, fullDate, logDate, percent, serviceColor, statusColor } from "../lib/format";

type Node = Span & { depth: number; children: Node[]; self: number };

/** Builds the span tree, depths and self (exclusive) times. */
function buildTree(spans: Span[]) {
  const byId = new Map<string, Node>();
  for (const s of spans) byId.set(s.span_id, { ...s, depth: 0, children: [], self: s.duration });
  const roots: Node[] = [];
  for (const n of byId.values()) {
    const parent = n.parent_id !== "0" ? byId.get(n.parent_id) : undefined;
    if (parent && parent !== n) parent.children.push(n);
    else roots.push(n);
  }
  const order: Node[] = [];
  const walk = (n: Node, depth: number) => {
    n.depth = depth;
    n.children.sort((a, b) => a.start - b.start);
    // Self time: the part of the span not covered by its children.
    const intervals = n.children.map((c) => [Math.max(c.start, n.start), Math.min(c.start + c.duration, n.start + n.duration)]).filter(([a, b]) => b > a);
    intervals.sort((a, b) => a[0] - b[0]);
    let covered = 0;
    let curA = -1;
    let curB = -1;
    for (const [a, b] of intervals) {
      if (a > curB) {
        if (curB > curA) covered += curB - curA;
        curA = a;
        curB = b;
      } else curB = Math.max(curB, b);
    }
    if (curB > curA) covered += curB - curA;
    n.self = Math.max(0, n.duration - covered);
    order.push(n);
    for (const c of n.children) walk(c, depth + 1);
  };
  roots.sort((a, b) => a.start - b.start);
  for (const r of roots) walk(r, 0);
  const start = Math.min(...spans.map((s) => s.start));
  const end = Math.max(...spans.map((s) => s.start + s.duration));
  return { roots, order, start, end, maxDepth: Math.max(0, ...order.map((n) => n.depth)) };
}

type Tree = ReturnType<typeof buildTree>;

export function TracePanel({
  traceId,
  spanId,
  onSelectSpan,
  onClose,
}: {
  traceId: string;
  spanId: string | null;
  onSelectSpan: (id: string) => void;
  onClose: () => void;
}) {
  const trace = useQuery({ queryKey: ["trace", traceId], queryFn: () => api.trace(traceId), retry: false });
  const tree = useMemo(() => (trace.data ? buildTree(trace.data.spans) : null), [trace.data]);
  const [view, setView] = useState<"flame" | "waterfall" | "list">("flame");
  const root = tree?.roots[0];
  const selected = tree?.order.find((s) => s.span_id === spanId) ?? root;
  const errors = tree?.order.filter((s) => s.error).length ?? 0;

  const services = useMemo(() => {
    if (!tree) return [];
    const m = new Map<string, number>();
    for (const n of tree.order) m.set(n.service, (m.get(n.service) ?? 0) + n.self);
    const total = [...m.values()].reduce((a, b) => a + b, 0) || 1;
    return [...m.entries()].sort((a, b) => b[1] - a[1]).map(([name, t]) => ({ name, share: t / total }));
  }, [tree]);

  return (
    <SidePanel
      onClose={onClose}
      storageKey="trace"
      initialWidth={0.72}
      header={
        root ? (
          <>
            <ServiceChip name={root.service} />
            <strong style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{root.resource}</strong>
            {errors > 0 && <StatusPill status="error" />}
          </>
        ) : (
          <span className="faint">Trace {traceId}</span>
        )
      }
    >
      {trace.isLoading && <div className="loading">Loading trace…</div>}
      {trace.isError && (
        <div className="empty-state">
          <h3>Trace not found</h3>
          Trace <span className="mono">{traceId}</span> has no stored spans (it may be past retention).
        </div>
      )}
      {tree && root && selected && (
        <>
          <div className="panel-section">
            <div className="trace-summary">
              <span className="big">{duration(tree.end - tree.start)}</span>
              <span className="muted">{fullDate(tree.start / 1e6)}</span>
              <span>
                <span className="muted">Spans</span> <strong>{tree.order.length}</strong>
              </span>
              <span>
                <span className="muted">Errors</span> <strong style={{ color: errors ? "var(--red)" : undefined }}>{errors}</strong>
              </span>
              <span className="mono muted">trace_id:{traceId}</span>
            </div>
            <div className="legend" style={{ marginTop: 10 }}>
              {services.map((s) => (
                <span className="legend-item" key={s.name}>
                  <i style={{ background: serviceColor(s.name) }} />
                  {s.name} <span className="faint">{percent(s.share)}</span>
                </span>
              ))}
            </div>
          </div>
          <div className="tabs">
            <button className={view === "flame" ? "active" : ""} onClick={() => setView("flame")}>
              Flame Graph
            </button>
            <button className={view === "waterfall" ? "active" : ""} onClick={() => setView("waterfall")}>
              Waterfall
            </button>
            <button className={view === "list" ? "active" : ""} onClick={() => setView("list")}>
              Span List
            </button>
          </div>
          <div style={{ padding: "8px 16px 12px", borderBottom: "1px solid var(--border)" }}>
            {view === "flame" && <FlameGraph tree={tree} selected={selected.span_id} onSelect={onSelectSpan} />}
            {view === "waterfall" && <Waterfall tree={tree} selected={selected.span_id} onSelect={onSelectSpan} />}
            {view === "list" && <SpanList tree={tree} onSelect={onSelectSpan} />}
          </div>
          <SpanDetails span={selected} tree={tree} traceId={traceId} />
        </>
      )}
    </SidePanel>
  );
}

function FlameGraph({ tree, selected, onSelect }: { tree: Tree; selected: string; onSelect: (id: string) => void }) {
  const [ref, width] = useWidth<HTMLDivElement>();
  const [zoom, setZoom] = useState<[number, number] | null>(null);
  const [hover, setHover] = useState<{ n: Node; x: number; y: number } | null>(null);
  const [lo, hi] = zoom ?? [tree.start, tree.end];
  const span = Math.max(1, hi - lo);
  const x = (t: number) => ((t - lo) / span) * width;
  const rowH = 24;
  const height = (tree.maxDepth + 1) * rowH + 4;
  const ticks = 6;

  return (
    <div className="flamegraph" ref={ref}>
      <div className="flame-axis">
        {Array.from({ length: ticks + 1 }, (_, i) => (
          <span key={i} style={{ left: `${(i / ticks) * 100}%`, transform: i === 0 ? "none" : i === ticks ? "translateX(-100%)" : undefined }}>
            {duration(lo - tree.start + (span * i) / ticks)}
          </span>
        ))}
      </div>
      <div className="flame-rows" style={{ height: Math.min(height, 420) }} onMouseLeave={() => setHover(null)}>
        <div style={{ position: "relative", height }}>
          {tree.order.map((n) => {
            const left = x(n.start);
            const w = Math.max(1, x(n.start + n.duration) - left);
            if (left > width || left + w < 0) return null;
            const label = w > 40;
            return (
              <div
                key={n.span_id}
                className={`flame-span${n.span_id === selected ? " selected" : ""}${n.error ? " error" : ""}`}
                style={{ left, width: w, top: n.depth * rowH + 2, background: serviceColor(n.service) }}
                onClick={() => onSelect(n.span_id)}
                onDoubleClick={() => setZoom([n.start, n.start + n.duration])}
                onMouseMove={(e) => {
                  const r = (e.currentTarget.parentElement as HTMLElement).getBoundingClientRect();
                  setHover({ n, x: e.clientX - r.left, y: e.clientY - r.top });
                }}
              >
                {label && (
                  <>
                    <b>{n.name}</b> {n.resource !== n.name ? n.resource : ""}
                  </>
                )}
              </div>
            );
          })}
          {hover && (
            <div className="chart-tooltip" style={{ left: Math.min(hover.x + 12, width - 280), top: hover.y + 14, maxWidth: 360, whiteSpace: "normal" }}>
              <div className="chart-tooltip-title">
                <ServiceChip name={hover.n.service} />
              </div>
              <div className="mono" style={{ marginBottom: 4 }}>
                {hover.n.name}
              </div>
              <div style={{ marginBottom: 4, wordBreak: "break-all" }}>{hover.n.resource}</div>
              <div className="chart-tooltip-row">
                <span>Duration</span>
                <span>{duration(hover.n.duration)}</span>
              </div>
              <div className="chart-tooltip-row">
                <span>Exec time</span>
                <span>{duration(hover.n.self)}</span>
              </div>
              <div className="chart-tooltip-row">
                <span>% of trace</span>
                <span>{percent(hover.n.duration / Math.max(1, tree.end - tree.start))}</span>
              </div>
            </div>
          )}
        </div>
      </div>
      <div style={{ display: "flex", gap: 8, marginTop: 6, alignItems: "center" }}>
        <span className="faint" style={{ fontSize: 11.5 }}>
          Click a span to inspect it · double-click to zoom
        </span>
        {zoom && (
          <button className="btn small" onClick={() => setZoom(null)}>
            Reset zoom
          </button>
        )}
      </div>
    </div>
  );
}

function Waterfall({ tree, selected, onSelect }: { tree: Tree; selected: string; onSelect: (id: string) => void }) {
  const total = Math.max(1, tree.end - tree.start);
  return (
    <div style={{ maxHeight: 440, overflow: "auto" }}>
      {tree.order.map((n) => {
        const left = ((n.start - tree.start) / total) * 100;
        const w = Math.max(0.2, (n.duration / total) * 100);
        return (
          <div key={n.span_id} className={`waterfall-row${n.span_id === selected ? " selected" : ""}`} onClick={() => onSelect(n.span_id)}>
            <div className="waterfall-name" style={{ paddingLeft: n.depth * 14 }}>
              <i style={{ width: 8, height: 8, borderRadius: 2, background: serviceColor(n.service), flex: "none" }} />
              <span style={{ fontWeight: 600 }}>{n.service}</span>
              <span className="mono">{n.name}</span>
              <span className="resource">{n.resource}</span>
            </div>
            <div className="waterfall-track">
              <div className="waterfall-bar" style={{ left: `${left}%`, width: `${w}%`, background: n.error ? statusColor("error") : serviceColor(n.service) }} />
              <span className="waterfall-label" style={left + w > 80 ? { right: `${100 - left + 0.5}%` } : { left: `calc(${left + w}% + 4px)` }}>
                {duration(n.duration)}
              </span>
            </div>
          </div>
        );
      })}
    </div>
  );
}

function SpanList({ tree, onSelect }: { tree: Tree; onSelect: (id: string) => void }) {
  const groups = useMemo(() => {
    const m = new Map<string, { service: string; resource: string; name: string; count: number; total: number; self: number; first: string; errors: number }>();
    for (const n of tree.order) {
      const k = `${n.service}\u0000${n.resource}`;
      const g = m.get(k) ?? { service: n.service, resource: n.resource, name: n.name, count: 0, total: 0, self: 0, first: n.span_id, errors: 0 };
      g.count++;
      g.total += n.duration;
      g.self += n.self;
      g.errors += n.error ? 1 : 0;
      m.set(k, g);
    }
    return [...m.values()].sort((a, b) => b.self - a.self);
  }, [tree]);
  const all = groups.reduce((a, g) => a + g.self, 0) || 1;
  return (
    <table className="list">
      <thead>
        <tr>
          <th style={{ width: 160 }}>Service</th>
          <th>Resource</th>
          <th className="num" style={{ width: 70 }}>
            Spans
          </th>
          <th className="num" style={{ width: 100 }}>
            Avg Duration
          </th>
          <th className="num" style={{ width: 110 }}>
            Exec Time
          </th>
        </tr>
      </thead>
      <tbody>
        {groups.map((g) => (
          <tr key={`${g.service}/${g.resource}`} onClick={() => onSelect(g.first)}>
            <td>
              <ServiceChip name={g.service} />
            </td>
            <td title={g.resource}>
              {g.resource} {g.errors > 0 && <span style={{ color: "var(--red)" }}>· {g.errors} error{g.errors > 1 ? "s" : ""}</span>}
            </td>
            <td className="num">{g.count}</td>
            <td className="num mono">{duration(g.total / g.count)}</td>
            <td className="num">{percent(g.self / all)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function SpanDetails({ span, tree, traceId }: { span: Node; tree: Tree; traceId: string }) {
  const [tab, setTab] = useState<"info" | "logs" | "error">("info");
  const logs = useQuery({
    queryKey: ["traceLogs", traceId, tree.start],
    queryFn: () => api.searchLogs({ query: `trace_id:${traceId}`, from: Math.floor(tree.start / 1e6) - 3_600_000, to: Math.ceil(tree.end / 1e6) + 3_600_000, limit: 200 }),
  });
  const spanLogs = logs.data?.logs ?? [];
  const meta = Object.fromEntries(Object.entries(span.meta).filter(([k]) => !k.startsWith("error.")));
  const total = Math.max(1, tree.end - tree.start);

  return (
    <div>
      <div className="panel-section">
        <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 10, flexWrap: "wrap" }}>
          <ServiceChip name={span.service} />
          <span className="mono">{span.name}</span>
          {span.error && <StatusPill status="error" />}
          {span.meta["http.status_code"] && <HttpCode code={span.meta["http.status_code"]} />}
          <div className="spacer" />
          <Link to={`/apm/services/${encodeURIComponent(span.service)}`} className="btn small">
            Service page
          </Link>
        </div>
        <div style={{ fontWeight: 600, marginBottom: 10, wordBreak: "break-all" }}>{span.resource}</div>
        <dl className="kv-grid">
          <dt>Duration</dt>
          <dd>
            {duration(span.duration)} <span className="faint">({percent(span.duration / total)} of trace)</span>
          </dd>
          <dt>Exec time</dt>
          <dd>{duration(span.self)}</dd>
          <dt>Start</dt>
          <dd>
            {logDate(span.start / 1e6)} <span className="faint">(+{duration(span.start - tree.start)})</span>
          </dd>
          <dt>Env / Version</dt>
          <dd>
            {span.env ?? "none"} / {span.version ?? "—"}
          </dd>
          <dt>Span ID</dt>
          <dd className="mono">{span.span_id}</dd>
          <dt>Parent ID</dt>
          <dd className="mono">{span.parent_id}</dd>
          <dt>Type</dt>
          <dd>{span.type ?? "custom"}</dd>
        </dl>
      </div>
      <div className="tabs">
        <button className={tab === "info" ? "active" : ""} onClick={() => setTab("info")}>
          Info
        </button>
        <button className={tab === "logs" ? "active" : ""} onClick={() => setTab("logs")}>
          Logs <span className="count-badge">{spanLogs.length}</span>
        </button>
        {span.error && (
          <button className={tab === "error" ? "active" : ""} onClick={() => setTab("error")}>
            Errors
          </button>
        )}
      </div>
      <div className="panel-section" style={{ borderBottom: 0 }}>
        {tab === "info" && (
          <>
            <h4>Tags</h4>
            <JsonTree data={meta} />
            <h4 style={{ marginTop: 16 }}>Metrics</h4>
            <JsonTree data={span.metrics} />
          </>
        )}
        {tab === "logs" && (
          <>
            {spanLogs.length === 0 && <div className="faint">No logs carry this trace's id. Enable log injection (DD_LOGS_INJECTION=true) to correlate them.</div>}
            {spanLogs.length > 0 && (
              <table className="list">
                <colgroup>
                  <col style={{ width: 6 }} />
                  <col style={{ width: 172 }} />
                  <col style={{ width: 150 }} />
                  <col />
                </colgroup>
                <tbody>
                  {[...spanLogs].reverse().map((l) => (
                    <tr key={l.id} className={l.span_id === span.span_id ? "selected" : undefined}>
                      <td className="status-cell">
                        <span className="status-bar" style={{ background: statusColor(l.status) }} />
                      </td>
                      <td className="date">{logDate(l.timestamp)}</td>
                      <td>
                        <ServiceChip name={l.service} />
                      </td>
                      <td className="content-cell">
                        <Link to={`/logs?query=${encodeURIComponent(`trace_id:${traceId}`)}&from_ts=${l.timestamp - 900_000}&to_ts=${l.timestamp + 900_000}&live=false&event=${l.id}`}>{l.message}</Link>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </>
        )}
        {tab === "error" && (
          <div className="error-box">
            <div style={{ fontWeight: 600, color: "#a3232a", marginBottom: 4 }}>{span.meta["error.type"] ?? "Error"}</div>
            <div style={{ marginBottom: 8 }}>{span.meta["error.message"] ?? span.meta["error.msg"]}</div>
            {span.meta["error.stack"] && <div className="mono">{span.meta["error.stack"]}</div>}
          </div>
        )}
      </div>
    </div>
  );
}
