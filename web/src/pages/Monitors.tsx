import {
  keepPreviousData,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import {
  Link,
  useNavigate,
  useParams,
  useSearchParams,
} from "react-router-dom";
import { TimeChart, type Series } from "../components/Charts";
import { Modal } from "../components/Common";
import { MuteIcon, PlusIcon, TrashIcon } from "../components/Icons";
import { PageHeader } from "../components/Layout";
import { TimePicker } from "../components/TimePicker";
import {
  api,
  ApiError,
  type Monitor,
  type MonitorDraft,
  type MonitorEvent,
  type MonitorPreview,
  type Webhook,
} from "../lib/api";
import { ago, compact, fullDate, serviceColor } from "../lib/format";
import { useTimeRange } from "../lib/time";

export const MONITOR_TABS = [
  { to: "/monitors/manage", label: "Manage Monitors" },
  { to: "/monitors/triggered", label: "Triggered Monitors" },
  { to: "/monitors/webhooks", label: "Webhooks" },
];

const STATUSES = ["Alert", "Warn", "No Data", "OK"];
const TYPE_LABELS: Record<string, string> = {
  "metric alert": "Metric",
  "query alert": "Metric",
  "log alert": "Logs",
  "trace-analytics alert": "APM",
};

export function MonitorPill({
  status,
  small,
}: {
  status: string;
  small?: boolean;
}) {
  const cls = status.toLowerCase().replace(/\s+/g, "-");
  return (
    <span className={`monitor-pill monitor-${cls}${small ? " small" : ""}`}>
      {status === "No Data" ? "NO DATA" : status.toUpperCase()}
    </span>
  );
}

const isMuted = (m: Monitor) =>
  Object.keys(m.options.silenced ?? {}).length > 0;
const errorText = (e: unknown) =>
  e instanceof ApiError ? e.message : e ? String(e) : null;

function fmtValue(v: number | null | undefined): string {
  if (v === null || v === undefined || Number.isNaN(v)) return "—";
  if (Math.abs(v) >= 10000) return compact(v);
  return Number.isInteger(v)
    ? String(v)
    : v
        .toFixed(Math.abs(v) < 1 ? 4 : 2)
        .replace(/0+$/, "")
        .replace(/\.$/, "");
}

// --- Manage Monitors -------------------------------------------------------------------

function FacetBox({
  title,
  values,
  selected,
  onToggle,
}: {
  title: string;
  values: [string, number][];
  selected: Set<string>;
  onToggle: (v: string) => void;
}) {
  return (
    <div className="facet">
      <div className="facet-header" style={{ cursor: "default" }}>
        <span>{title}</span>
      </div>
      <div className="facet-body">
        {values.map(([v, n]) => (
          <label className="facet-value" key={v}>
            <input
              type="checkbox"
              checked={selected.has(v)}
              onChange={() => onToggle(v)}
            />
            <span className="facet-value-label">{v}</span>
            <span className="facet-value-count">{n}</span>
          </label>
        ))}
        {!values.length && <div className="facet-empty">None</div>}
      </div>
    </div>
  );
}

export function ManageMonitors() {
  const monitors = useQuery({
    queryKey: ["monitors"],
    queryFn: api.monitors,
    refetchInterval: 30_000,
  });
  const [params, setParams] = useSearchParams();
  const text = params.get("q") ?? "";
  const pick = (k: string) =>
    new Set((params.get(k) ?? "").split("|").filter(Boolean));
  const [status, type, muted, tags] = [
    pick("status"),
    pick("type"),
    pick("muted"),
    pick("tags"),
  ];
  const toggle = (k: string, v: string) =>
    setParams((prev) => {
      const p = new URLSearchParams(prev);
      const s = new Set((p.get(k) ?? "").split("|").filter(Boolean));
      if (s.has(v)) s.delete(v);
      else s.add(v);
      if (s.size) p.set(k, [...s].join("|"));
      else p.delete(k);
      return p;
    });
  const all = monitors.data ?? [];
  const typeOf = (m: Monitor) => TYPE_LABELS[m.type] ?? m.type;
  const mutedOf = (m: Monitor) => (isMuted(m) ? "Muted" : "Not muted");
  const shown = all.filter(
    (m) =>
      (!text ||
        `${m.name} ${m.query} ${m.tags.join(" ")}`
          .toLowerCase()
          .includes(text.toLowerCase())) &&
      (!status.size || status.has(m.overall_state)) &&
      (!type.size || type.has(typeOf(m))) &&
      (!muted.size || muted.has(mutedOf(m))) &&
      (!tags.size || m.tags.some((t) => tags.has(t))),
  );
  const count = (f: (m: Monitor) => string[] | string, order?: string[]) => {
    const c = new Map<string, number>();
    for (const m of all)
      for (const v of ([] as string[]).concat(f(m)))
        c.set(v, (c.get(v) ?? 0) + 1);
    const entries = [...c.entries()];
    return order
      ? order
          .filter((o) => c.has(o))
          .map((o) => [o, c.get(o)!] as [string, number])
      : entries.sort((a, b) => b[1] - a[1]);
  };
  const navigate = useNavigate();

  return (
    <>
      <PageHeader title="Monitors" tabs={MONITOR_TABS}>
        <Link className="btn primary" to="/monitors/create">
          <PlusIcon /> New Monitor
        </Link>
      </PageHeader>
      <div className="explorer">
        <aside className="facets" aria-label="Monitor facets">
          <div className="facets-search">
            <input
              className="input"
              placeholder="Search monitors"
              value={text}
              onChange={(e) =>
                setParams((prev) => {
                  const p = new URLSearchParams(prev);
                  if (e.target.value) p.set("q", e.target.value);
                  else p.delete("q");
                  return p;
                })
              }
            />
          </div>
          <FacetBox
            title="Status"
            values={count((m) => m.overall_state, STATUSES)}
            selected={status}
            onToggle={(v) => toggle("status", v)}
          />
          <FacetBox
            title="Type"
            values={count(typeOf)}
            selected={type}
            onToggle={(v) => toggle("type", v)}
          />
          <FacetBox
            title="Muted"
            values={count(mutedOf)}
            selected={muted}
            onToggle={(v) => toggle("muted", v)}
          />
          <FacetBox
            title="Tags"
            values={count((m) => m.tags)}
            selected={tags}
            onToggle={(v) => toggle("tags", v)}
          />
        </aside>
        <div className="content">
          <div className="toolbar">
            <span>
              <strong>{shown.length}</strong> of {all.length} monitors
            </span>
            {monitors.isFetching && <span className="spinner" />}
          </div>
          <div className="table-wrap">
            <table className="list monitors-table">
              <colgroup>
                <col style={{ width: 110 }} />
                <col />
                <col style={{ width: 110 }} />
                <col style={{ width: 70 }} />
                <col style={{ width: 150 }} />
              </colgroup>
              <thead>
                <tr>
                  <th>Status</th>
                  <th>Name</th>
                  <th>Type</th>
                  <th>Muted</th>
                  <th>Last Triggered</th>
                </tr>
              </thead>
              <tbody>
                {shown.map((m) => (
                  <tr key={m.id} onClick={() => navigate(`/monitors/${m.id}`)}>
                    <td>
                      <MonitorPill status={m.overall_state} />
                    </td>
                    <td>
                      <div className="monitor-name">
                        {m.priority && (
                          <span className="priority">P{m.priority}</span>
                        )}
                        {m.name}
                      </div>
                      {m.tags.length > 0 && (
                        <div className="monitor-tags">
                          {m.tags.map((t) => (
                            <span className="tag" key={t}>
                              {t}
                            </span>
                          ))}
                        </div>
                      )}
                    </td>
                    <td>{typeOf(m)}</td>
                    <td>{isMuted(m) && <MuteIcon className="muted-icon" />}</td>
                    <td className="muted">
                      {m.last_triggered_ts
                        ? ago(m.last_triggered_ts * 1000)
                        : "—"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            {!monitors.isLoading && !all.length && (
              <div className="empty-state">
                <h3>No monitors yet</h3>
                <Link to="/monitors/create">Create a monitor</Link> on a metric,
                on logs or on spans.
              </div>
            )}
          </div>
        </div>
      </div>
    </>
  );
}

export function TriggeredMonitors() {
  const monitors = useQuery({
    queryKey: ["monitors"],
    queryFn: api.monitors,
    refetchInterval: 30_000,
  });
  const navigate = useNavigate();
  const rows = (monitors.data ?? [])
    .flatMap((m) => Object.values(m.state.groups).map((g) => ({ m, g })))
    .filter(({ g }) => g.status !== "OK")
    .sort(
      (a, b) =>
        STATUSES.indexOf(a.g.status) - STATUSES.indexOf(b.g.status) ||
        b.g.since_ts - a.g.since_ts,
    );
  return (
    <>
      <PageHeader title="Monitors" tabs={MONITOR_TABS} />
      <div className="page-scroll page-padding">
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th style={{ width: 110 }}>Status</th>
                <th>Monitor</th>
                <th>Group</th>
                <th className="num">Value</th>
                <th style={{ width: 140 }}>Since</th>
              </tr>
            </thead>
            <tbody>
              {rows.map(({ m, g }) => (
                <tr
                  key={`${m.id}-${g.name}`}
                  onClick={() => navigate(`/monitors/${m.id}`)}
                >
                  <td>
                    <MonitorPill status={g.status} />
                  </td>
                  <td>{m.name}</td>
                  <td className="mono">
                    {g.name === "*" ? <span className="faint">—</span> : g.name}
                  </td>
                  <td className="num">{fmtValue(g.value)}</td>
                  <td className="muted">{ago(g.since_ts * 1000)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {monitors.data && !rows.length && (
            <div className="empty-state">
              <h3>All clear</h3>
              No monitor group is in Alert, Warn or No Data.
            </div>
          )}
        </div>
      </div>
    </>
  );
}

// --- Monitor status page -------------------------------------------------------------

function EventsTable({ events }: { events: MonitorEvent[] }) {
  return (
    <table className="list">
      <thead>
        <tr>
          <th style={{ width: 180 }}>Date</th>
          <th style={{ width: 110 }}>Status</th>
          <th>Event</th>
          <th className="num" style={{ width: 100 }}>
            Value
          </th>
          <th style={{ width: 200 }}>Notified</th>
        </tr>
      </thead>
      <tbody>
        {events.map((e) => (
          <tr key={e.id} title={e.message}>
            <td className="date">{fullDate(e.ts).replace(/\.\d+$/, "")}</td>
            <td>
              <MonitorPill status={e.to_status} small />
            </td>
            <td className="ellipsis">{e.title}</td>
            <td className="num">{fmtValue(e.value)}</td>
            <td className="muted ellipsis">{e.notified.join(", ") || "—"}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export function MonitorPage() {
  const { id } = useParams();
  const monitorId = Number(id);
  const qc = useQueryClient();
  const navigate = useNavigate();
  const time = useTimeRange("4h", 60_000);
  const monitor = useQuery({
    queryKey: ["monitor", monitorId],
    queryFn: () => api.monitor(monitorId),
    refetchInterval: 30_000,
  });
  const events = useQuery({
    queryKey: ["monitorEvents", monitorId],
    queryFn: () => api.monitorEvents(monitorId),
    refetchInterval: 30_000,
  });
  const history = useQuery({
    queryKey: ["monitorHistory", monitorId, time.from, time.to],
    queryFn: () => api.monitorHistory(monitorId, time.from, time.to),
    placeholderData: keepPreviousData,
  });
  const [error, setError] = useState<string | null>(null);
  const m = monitor.data;

  const chart = useMemo(() => {
    const series = history.data ?? [];
    const interval = 60_000;
    const start = Math.floor(time.from / interval) * interval;
    const times: number[] = [];
    for (let t = start; t <= time.to; t += interval) times.push(t);
    const lines: Series[] = series.slice(0, 20).map((s) => {
      const at = new Map<number, number>();
      for (const [t, v] of s.points)
        if (v !== null) at.set(Math.floor(t / interval) * interval, v);
      return {
        key: s.group,
        label: s.group === "*" ? (m?.name ?? "value") : s.group,
        color: s.group === "*" ? "#8b6fd0" : serviceColor(s.group),
        values: times.map((t) => at.get(t) ?? NaN),
      };
    });
    return { times, lines, interval };
  }, [history.data, time.from, time.to, m?.name]);

  if (monitor.error)
    return (
      <div className="page-padding error-box">{errorText(monitor.error)}</div>
    );
  if (!m) return <div className="page-padding faint">Loading…</div>;
  const th = m.options.thresholds;
  const markers = [
    th.critical !== undefined && {
      value: th.critical,
      color: "#e74c50",
      label: `Alert ${fmtValue(th.critical)}`,
    },
    th.warning !== undefined && {
      value: th.warning,
      color: "#f5a524",
      label: `Warn ${fmtValue(th.warning)}`,
    },
  ].filter(Boolean) as { value: number; color: string; label: string }[];
  const groups = Object.values(m.state.groups).sort(
    (a, b) =>
      STATUSES.indexOf(a.status) - STATUSES.indexOf(b.status) ||
      a.name.localeCompare(b.name),
  );
  const act = async (f: () => Promise<unknown>) => {
    setError(null);
    try {
      await f();
      await qc.invalidateQueries({ queryKey: ["monitor", monitorId] });
      await qc.invalidateQueries({ queryKey: ["monitors"] });
    } catch (e) {
      setError(errorText(e));
    }
  };

  return (
    <>
      <PageHeader title="Monitors" tabs={MONITOR_TABS}>
        <TimePicker time={time} allowLive={false} />
      </PageHeader>
      <div className="page-scroll page-padding">
        <div className="monitor-header">
          <MonitorPill status={m.overall_state} />
          <h2>
            {m.priority && <span className="priority">P{m.priority}</span>}
            {m.name}
          </h2>
          {isMuted(m) && (
            <span className="muted-badge">
              <MuteIcon /> Muted
            </span>
          )}
          <div className="spacer" />
          {isMuted(m) ? (
            <button
              className="btn"
              onClick={() => act(() => api.unmuteMonitor(m.id))}
            >
              Unmute
            </button>
          ) : (
            <button
              className="btn"
              onClick={() => act(() => api.muteMonitor(m.id))}
            >
              <MuteIcon /> Mute
            </button>
          )}
          <Link className="btn" to={`/monitors/create?clone=${m.id}`}>
            Clone
          </Link>
          <Link className="btn primary" to={`/monitors/${m.id}/edit`}>
            Edit
          </Link>
          <button
            className="btn"
            title="Delete"
            onClick={() => {
              if (window.confirm(`Delete the monitor "${m.name}"?`))
                act(async () => {
                  await api.deleteMonitor(m.id);
                  navigate("/monitors/manage");
                });
            }}
          >
            <TrashIcon />
          </button>
        </div>
        {error && <div className="error-box">{error}</div>}
        <div className="monitor-props">
          <div>
            <span className="faint">Type</span> {TYPE_LABELS[m.type] ?? m.type}
          </div>
          <div>
            <span className="faint">ID</span> {m.id}
          </div>
          <div>
            <span className="faint">Modified</span>{" "}
            {ago(Date.parse(m.modified))}
          </div>
          {m.tags.map((t) => (
            <span className="tag" key={t}>
              {t}
            </span>
          ))}
        </div>
        <div className="card">
          <div className="card-title">Query</div>
          <code className="monitor-query">{m.query}</code>
        </div>
        <div className="card">
          <div className="card-title">
            History <span className="muted">evaluated every minute</span>
          </div>
          {chart.lines.length ? (
            <TimeChart
              times={chart.times}
              interval={chart.interval}
              from={time.from}
              to={time.to}
              series={chart.lines}
              kind="lines"
              height={220}
              format={fmtValue}
              markers={markers}
              onZoom={time.setAbsolute}
            />
          ) : (
            <div className="empty-state">
              No evaluation in this time range yet.
            </div>
          )}
        </div>
        <div className="card" style={{ padding: 0 }}>
          <div className="card-title" style={{ padding: "12px 16px 0" }}>
            Status{" "}
            {m.multi && <span className="muted">{groups.length} groups</span>}
          </div>
          <table className="list">
            <thead>
              <tr>
                <th style={{ width: 110 }}>Status</th>
                <th>Group</th>
                <th className="num">Value</th>
                <th style={{ width: 140 }}>Since</th>
                <th style={{ width: 160 }}>Last Triggered</th>
              </tr>
            </thead>
            <tbody>
              {groups.map((g) => (
                <tr key={g.name}>
                  <td>
                    <MonitorPill status={g.status} small />
                  </td>
                  <td className="mono">
                    {g.name === "*" ? (
                      <span className="faint">(no groups)</span>
                    ) : (
                      g.name
                    )}
                  </td>
                  <td className="num">{fmtValue(g.value)}</td>
                  <td className="muted">{ago(g.since_ts * 1000)}</td>
                  <td className="muted">
                    {g.last_triggered_ts
                      ? ago(g.last_triggered_ts * 1000)
                      : "—"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {!groups.length && (
            <div className="empty-state faint">Not evaluated yet.</div>
          )}
        </div>
        <div className="card" style={{ padding: 0 }}>
          <div className="card-title" style={{ padding: "12px 16px 0" }}>
            Events
          </div>
          <EventsTable events={events.data ?? []} />
          {events.data && !events.data.length && (
            <div className="empty-state faint">No state change yet.</div>
          )}
        </div>
        {m.message && (
          <div className="card">
            <div className="card-title">Message</div>
            <pre className="monitor-message">{m.message}</pre>
          </div>
        )}
      </div>
    </>
  );
}

// --- editor ------------------------------------------------------------------------------

type Kind = "metric" | "logs" | "apm";
const WINDOWS = ["1m", "5m", "10m", "15m", "30m", "1h", "2h", "4h", "1d"];
const WINDOW_LABELS: Record<string, string> = {
  "1m": "1 minute",
  "5m": "5 minutes",
  "10m": "10 minutes",
  "15m": "15 minutes",
  "30m": "30 minutes",
  "1h": "1 hour",
  "2h": "2 hours",
  "4h": "4 hours",
  "1d": "1 day",
};
const COMPARATORS: [string, string][] = [
  [">", "above"],
  [">=", "above or equal to"],
  ["<", "below"],
  ["<=", "below or equal to"],
  ["==", "equal to"],
  ["!=", "not equal to"],
];
const EVENT_AGGS: [string, string][] = [
  ["count", "Count of events"],
  ["cardinality", "Unique count of"],
  ["avg", "Avg of"],
  ["sum", "Sum of"],
  ["min", "Min of"],
  ["max", "Max of"],
  ["median", "Median of"],
  ["pc90", "pc90 of"],
  ["pc95", "pc95 of"],
  ["pc99", "pc99 of"],
];

type Form = {
  kind: Kind;
  raw: string | null;
  metric: string;
  timeAgg: string;
  search: string;
  agg: string;
  measure: string;
  by: string;
  window: string;
  cmp: string;
  critical: string;
  warning: string;
  criticalRecovery: string;
  warningRecovery: string;
  notifyNoData: boolean;
  noDataTimeframe: string;
  name: string;
  message: string;
  tags: string;
  priority: string;
  renotify: string;
};

const blank = (kind: Kind): Form => ({
  kind,
  raw: null,
  metric: kind === "metric" ? "avg:system.cpu.user{*} by {host}" : "",
  timeAgg: "avg",
  search:
    kind === "logs"
      ? "status:error"
      : kind === "apm"
        ? "service:web-store"
        : "",
  agg: kind === "apm" ? "pc95" : "count",
  measure: kind === "apm" ? "@duration" : "",
  by: kind === "metric" ? "" : "service",
  window: "5m",
  cmp: ">",
  critical: "",
  warning: "",
  criticalRecovery: "",
  warningRecovery: "",
  notifyNoData: false,
  noDataTimeframe: "10",
  name: "",
  message: "{{#is_alert}}\n{{/is_alert}}\n{{#is_recovery}}\n{{/is_recovery}}",
  tags: "",
  priority: "",
  renotify: "0",
});

const quote = (s: string) =>
  `"${s.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
const unquote = (s: string) => s.replace(/\\"/g, '"').replace(/\\\\/g, "\\");

function buildQuery(f: Form): string {
  if (f.raw !== null) return f.raw;
  const tail = ` ${f.cmp} ${f.critical || "0"}`;
  if (f.kind === "metric")
    return `${f.timeAgg}(last_${f.window}):${f.metric.trim()}${tail}`;
  const fn = f.kind === "logs" ? "logs" : "trace-analytics";
  const rollup =
    f.agg === "count"
      ? `.rollup("count")`
      : `.rollup("${f.agg}", ${quote(f.measure)})`;
  const by = f.by.trim()
    ? `.by(${quote(
        f.by
          .split(",")
          .map((s) => s.trim())
          .filter(Boolean)
          .join(","),
      )})`
    : "";
  const index = f.kind === "logs" ? `.index("*")` : "";
  return `${fn}(${quote(f.search)})${index}${rollup}${by}.last("${f.window}")${tail}`;
}

/** The form fields of a query this editor can build; `raw` otherwise. */
function parseInto(q: string, f: Form): Form {
  const tail = q.match(/^([\s\S]*?)\s*(>=|<=|==|!=|>|<)\s*(-?[\d.eE+-]+)\s*$/);
  if (!tail) return { ...f, raw: q };
  const [, head, cmp, critical] = tail;
  const metric = head.match(/^(avg|min|max|sum)\(last_(\d+[smhdw])\):(.+)$/);
  if (metric && WINDOWS.includes(metric[2]))
    return {
      ...f,
      kind: "metric",
      raw: null,
      timeAgg: metric[1],
      window: metric[2],
      metric: metric[3],
      cmp,
      critical,
    };
  const ev = head.match(
    /^(logs|trace-analytics|spans)\("((?:[^"\\]|\\.)*)"\)(.*)$/,
  );
  if (!ev) return { ...f, raw: q };
  const out: Form = {
    ...f,
    kind: ev[1] === "logs" ? "logs" : "apm",
    raw: null,
    search: unquote(ev[2]),
    cmp,
    critical,
    by: "",
    agg: "count",
    measure: "",
  };
  for (const call of ev[3].matchAll(
    /\.(\w+)\(((?:\s*"(?:[^"\\]|\\.)*"\s*,?)*)\)/g,
  )) {
    const args = [...call[2].matchAll(/"((?:[^"\\]|\\.)*)"/g)].map((a) =>
      unquote(a[1]),
    );
    if (call[1] === "rollup") {
      out.agg = args[0] ?? "count";
      out.measure = args[1] ?? "";
    } else if (call[1] === "by") out.by = args.join(",");
    else if (call[1] === "last") out.window = args[0] ?? "5m";
  }
  if (!WINDOWS.includes(out.window)) return { ...f, raw: q };
  return out;
}

function fromMonitor(m: Monitor | MonitorDraft): Form {
  const kind: Kind =
    m.type === "log alert"
      ? "logs"
      : m.type === "trace-analytics alert"
        ? "apm"
        : "metric";
  const f = parseInto(m.query, blank(kind));
  const th = m.options.thresholds;
  const s = (v: number | undefined) =>
    v === undefined || v === null ? "" : String(v);
  return {
    ...f,
    critical: f.raw === null ? f.critical : s(th.critical),
    warning: s(th.warning),
    criticalRecovery: s(th.critical_recovery),
    warningRecovery: s(th.warning_recovery),
    notifyNoData: !!m.options.notify_no_data,
    noDataTimeframe: String(m.options.no_data_timeframe ?? 10),
    name: m.name,
    message: m.message,
    tags: m.tags.join(", "),
    priority: m.priority ? String(m.priority) : "",
    renotify: String(m.options.renotify_interval ?? 0),
  };
}

function toDraft(f: Form, base?: Monitor): MonitorDraft {
  const n = (s: string) => (s.trim() === "" ? undefined : Number(s));
  return {
    name: f.name.trim(),
    type:
      f.kind === "logs"
        ? "log alert"
        : f.kind === "apm"
          ? "trace-analytics alert"
          : "metric alert",
    query: buildQuery(f),
    message: f.message,
    tags: f.tags
      .split(",")
      .map((t) => t.trim())
      .filter(Boolean),
    priority: n(f.priority) ?? null,
    options: {
      ...(base?.options ?? {}),
      thresholds: {
        critical: n(f.critical),
        warning: n(f.warning),
        critical_recovery: n(f.criticalRecovery),
        warning_recovery: n(f.warningRecovery),
      },
      notify_no_data: f.notifyNoData,
      no_data_timeframe: f.notifyNoData ? n(f.noDataTimeframe) : undefined,
      renotify_interval: n(f.renotify) ?? 0,
    },
  };
}

function Step({
  n,
  title,
  children,
}: {
  n: number;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="monitor-step">
      <h3>
        <span className="step-number">{n}</span>
        {title}
      </h3>
      <div className="monitor-step-body">{children}</div>
    </section>
  );
}

function Preview({
  query,
  warning,
  enabled,
}: {
  query: string;
  warning: string;
  enabled: boolean;
}) {
  const [debounced, setDebounced] = useState({ query, warning });
  useEffect(() => {
    const t = setTimeout(() => setDebounced({ query, warning }), 600);
    return () => clearTimeout(t);
  }, [query, warning]);
  const res = useQuery({
    queryKey: ["monitorPreview", debounced],
    queryFn: ({ signal }) =>
      api.previewMonitor(
        debounced.query,
        {
          warning:
            debounced.warning.trim() === ""
              ? undefined
              : Number(debounced.warning),
        },
        signal,
      ),
    enabled,
    retry: false,
    placeholderData: keepPreviousData,
  });
  const data: MonitorPreview | undefined = res.data;
  return (
    <div className="monitor-preview">
      <div className="monitor-preview-title">
        Current evaluation {res.isFetching && <span className="spinner" />}
      </div>
      {res.error ? (
        <div className="error-box">{errorText(res.error)}</div>
      ) : data ? (
        data.groups.length ? (
          <table className="list">
            <tbody>
              {data.groups.slice(0, 50).map((g) => (
                <tr key={g.group}>
                  <td style={{ width: 96 }}>
                    <MonitorPill status={g.status} small />
                  </td>
                  <td className="mono ellipsis">
                    {g.group === "*" ? "(no groups)" : g.group}
                  </td>
                  <td className="num">{fmtValue(g.value)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <div className="faint">No data over the evaluation window.</div>
        )
      ) : null}
    </div>
  );
}

export function MonitorEditor() {
  const { id } = useParams();
  const editing = id ? Number(id) : null;
  const [params] = useSearchParams();
  const cloneId = Number(params.get("clone")) || null;
  const sourceId = editing ?? cloneId;
  const navigate = useNavigate();
  const qc = useQueryClient();
  const source = useQuery({
    queryKey: ["monitor", sourceId],
    queryFn: () => api.monitor(sourceId!),
    enabled: !!sourceId,
  });
  const webhooks = useQuery({ queryKey: ["webhooks"], queryFn: api.webhooks });
  const [form, setForm] = useState<Form | null>(
    sourceId ? null : blank((params.get("type") as Kind) || "metric"),
  );
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (source.data && !form) {
      const f = fromMonitor(source.data);
      setForm(cloneId ? { ...f, name: `${f.name} (clone)` } : f);
    }
  }, [source.data, form, cloneId]);
  if (!form) return <div className="page-padding faint">Loading…</div>;
  const set = (patch: Partial<Form>) => setForm({ ...form, ...patch });
  const query = buildQuery(form);
  const insert = (text: string) =>
    set({
      message: `${form.message}${form.message.endsWith("\n") || !form.message ? "" : " "}${text}`,
    });

  const save = async () => {
    setError(null);
    try {
      const draft = toDraft(form, editing ? source.data : undefined);
      const saved = editing
        ? await api.updateMonitor(editing, draft)
        : await api.createMonitor(draft);
      await qc.invalidateQueries({ queryKey: ["monitors"] });
      await qc.invalidateQueries({ queryKey: ["monitor", saved.id] });
      navigate(`/monitors/${saved.id}`);
    } catch (e) {
      setError(errorText(e));
    }
  };

  return (
    <>
      <PageHeader title="Monitors" tabs={MONITOR_TABS} />
      <div className="page-scroll page-padding monitor-editor">
        <h2>{editing ? `Edit ${form.name || "monitor"}` : "New Monitor"}</h2>
        {!editing && (
          <div className="monitor-kinds">
            {(
              [
                ["metric", "Metric", "Compare a metric with a threshold"],
                ["logs", "Logs", "Alert on log counts or measures"],
                ["apm", "APM", "Alert on span counts or latency"],
              ] as [Kind, string, string][]
            ).map(([k, label, help]) => (
              <button
                key={k}
                className={`monitor-kind${form.kind === k ? " selected" : ""}`}
                onClick={() =>
                  setForm({
                    ...blank(k),
                    name: form.name,
                    message: form.message,
                  })
                }
              >
                <strong>{label}</strong>
                <span className="muted">{help}</span>
              </button>
            ))}
          </div>
        )}
        <div className="monitor-editor-grid">
          <div>
            <Step
              n={1}
              title={
                form.kind === "metric"
                  ? "Define the metric"
                  : "Define the search query"
              }
            >
              {form.raw !== null ? (
                <>
                  <textarea
                    className="input mono"
                    rows={3}
                    value={form.raw}
                    onChange={(e) => set({ raw: e.target.value })}
                  />
                  <button
                    className="btn small"
                    onClick={() => setForm(parseInto(form.raw ?? "", form))}
                  >
                    Back to the editor
                  </button>
                </>
              ) : form.kind === "metric" ? (
                <>
                  <label className="form-field">
                    <span className="form-label">Metric query</span>
                    <input
                      className="input mono"
                      value={form.metric}
                      onChange={(e) => set({ metric: e.target.value })}
                      placeholder="avg:system.cpu.user{env:prod} by {host}"
                    />
                    <span className="form-hint">
                      As in the Metrics Explorer: aggregation, metric, filter,
                      `by {"{tag}"}` for one alert per group.
                    </span>
                  </label>
                  <div className="form-row inline">
                    Evaluate the
                    <select
                      className="input"
                      value={form.timeAgg}
                      onChange={(e) => set({ timeAgg: e.target.value })}
                    >
                      <option value="avg">average</option>
                      <option value="max">maximum</option>
                      <option value="min">minimum</option>
                      <option value="sum">sum</option>
                    </select>
                    of the query over the last
                    <select
                      className="input"
                      value={form.window}
                      onChange={(e) => set({ window: e.target.value })}
                    >
                      {WINDOWS.map((w) => (
                        <option key={w} value={w}>
                          {WINDOW_LABELS[w]}
                        </option>
                      ))}
                    </select>
                  </div>
                </>
              ) : (
                <>
                  <label className="form-field">
                    <span className="form-label">
                      {form.kind === "logs"
                        ? "Logs matching"
                        : "Spans matching"}
                    </span>
                    <input
                      className="input mono"
                      value={form.search}
                      onChange={(e) => set({ search: e.target.value })}
                      placeholder={
                        form.kind === "logs"
                          ? "service:web status:error"
                          : "service:web env:prod"
                      }
                    />
                  </label>
                  <div className="form-row inline">
                    <select
                      className="input"
                      value={form.agg}
                      onChange={(e) => set({ agg: e.target.value })}
                    >
                      {EVENT_AGGS.map(([v, l]) => (
                        <option key={v} value={v}>
                          {l}
                        </option>
                      ))}
                    </select>
                    {form.agg !== "count" && (
                      <input
                        className="input mono"
                        value={form.measure}
                        onChange={(e) => set({ measure: e.target.value })}
                        placeholder="@duration"
                        style={{ width: 180 }}
                      />
                    )}
                    by
                    <input
                      className="input mono"
                      value={form.by}
                      onChange={(e) => set({ by: e.target.value })}
                      placeholder="(everything)"
                      style={{ width: 200 }}
                    />
                    over the last
                    <select
                      className="input"
                      value={form.window}
                      onChange={(e) => set({ window: e.target.value })}
                    >
                      {WINDOWS.map((w) => (
                        <option key={w} value={w}>
                          {WINDOW_LABELS[w]}
                        </option>
                      ))}
                    </select>
                  </div>
                  <span className="form-hint">
                    Grouping by facets (`service,host`) sends one alert per
                    group.
                    {form.kind === "apm" && " Durations are in nanoseconds."}
                  </span>
                </>
              )}
              {form.raw === null && (
                <button
                  className="btn ghost small"
                  onClick={() => set({ raw: query })}
                >
                  Edit the query as text
                </button>
              )}
            </Step>
            <Step n={2} title="Set alert conditions">
              <div className="form-row inline">
                Trigger when the evaluated value is
                <select
                  className="input"
                  value={form.cmp}
                  onChange={(e) => set({ cmp: e.target.value })}
                  disabled={form.raw !== null}
                >
                  {COMPARATORS.map(([v, l]) => (
                    <option key={v} value={v}>
                      {l}
                    </option>
                  ))}
                </select>
                the threshold
              </div>
              <div className="threshold-grid">
                <label>
                  <span className="threshold-label alert">Alert threshold</span>
                  <input
                    className="input"
                    type="number"
                    value={form.critical}
                    onChange={(e) => set({ critical: e.target.value })}
                    disabled={form.raw !== null}
                  />
                </label>
                <label>
                  <span className="threshold-label warn">
                    Warning threshold
                  </span>
                  <input
                    className="input"
                    type="number"
                    value={form.warning}
                    onChange={(e) => set({ warning: e.target.value })}
                  />
                </label>
                <label>
                  <span className="threshold-label">
                    Alert recovery threshold
                  </span>
                  <input
                    className="input"
                    type="number"
                    value={form.criticalRecovery}
                    onChange={(e) => set({ criticalRecovery: e.target.value })}
                  />
                </label>
                <label>
                  <span className="threshold-label">
                    Warning recovery threshold
                  </span>
                  <input
                    className="input"
                    type="number"
                    value={form.warningRecovery}
                    onChange={(e) => set({ warningRecovery: e.target.value })}
                  />
                </label>
              </div>
              <label className="form-check">
                <input
                  type="checkbox"
                  checked={form.notifyNoData}
                  onChange={(e) => set({ notifyNoData: e.target.checked })}
                />
                Notify if data is missing for more than
                <input
                  className="input"
                  type="number"
                  value={form.noDataTimeframe}
                  onChange={(e) => set({ noDataTimeframe: e.target.value })}
                  disabled={!form.notifyNoData}
                  style={{ width: 70 }}
                />
                minutes
              </label>
            </Step>
            <Step n={3} title="Configure notifications">
              <label className="form-field">
                <span className="form-label">Monitor name</span>
                <input
                  className="input"
                  value={form.name}
                  onChange={(e) => set({ name: e.target.value })}
                  placeholder="High error rate on {{service.name}}"
                />
              </label>
              <label className="form-field">
                <span className="form-label">Message</span>
                <textarea
                  className="input mono"
                  rows={7}
                  value={form.message}
                  onChange={(e) => set({ message: e.target.value })}
                />
              </label>
              <div className="template-help">
                <span className="faint">Insert</span>
                {[
                  "{{#is_alert}}{{/is_alert}}",
                  "{{#is_warning}}{{/is_warning}}",
                  "{{#is_recovery}}{{/is_recovery}}",
                  "{{value}}",
                  "{{threshold}}",
                ].map((t) => (
                  <button
                    key={t}
                    className="btn small ghost mono"
                    onClick={() => insert(t)}
                  >
                    {t}
                  </button>
                ))}
                {(webhooks.data ?? []).map((w) => (
                  <button
                    key={w.name}
                    className="btn small mono"
                    onClick={() => insert(`@webhook-${w.name}`)}
                  >
                    @webhook-{w.name}
                  </button>
                ))}
                {webhooks.data && !webhooks.data.length && (
                  <Link className="faint" to="/monitors/webhooks">
                    Add a webhook to get notified
                  </Link>
                )}
              </div>
              <div className="form-row">
                <label className="form-field">
                  <span className="form-label">Tags</span>
                  <input
                    className="input"
                    value={form.tags}
                    onChange={(e) => set({ tags: e.target.value })}
                    placeholder="team:shop, env:prod"
                  />
                </label>
                <label className="form-field" style={{ flex: "0 0 120px" }}>
                  <span className="form-label">Priority</span>
                  <select
                    className="input"
                    value={form.priority}
                    onChange={(e) => set({ priority: e.target.value })}
                  >
                    <option value="">Not defined</option>
                    {[1, 2, 3, 4, 5].map((p) => (
                      <option key={p} value={p}>
                        P{p}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="form-field" style={{ flex: "0 0 220px" }}>
                  <span className="form-label">
                    Renotify if still not resolved
                  </span>
                  <select
                    className="input"
                    value={form.renotify}
                    onChange={(e) => set({ renotify: e.target.value })}
                  >
                    {[
                      ["0", "Never"],
                      ["10", "Every 10 minutes"],
                      ["30", "Every 30 minutes"],
                      ["60", "Every hour"],
                      ["240", "Every 4 hours"],
                      ["1440", "Every day"],
                    ].map(([v, l]) => (
                      <option key={v} value={v}>
                        {l}
                      </option>
                    ))}
                  </select>
                </label>
              </div>
            </Step>
          </div>
          <aside className="monitor-side">
            <div className="card">
              <div className="card-title">Query</div>
              <code className="monitor-query">{query}</code>
            </div>
            <Preview
              query={query}
              warning={form.warning}
              enabled={!!query.trim()}
            />
          </aside>
        </div>
        {error && <div className="error-box">{error}</div>}
        <div className="monitor-editor-footer">
          <button className="btn" onClick={() => navigate(-1)}>
            Cancel
          </button>
          <button
            className="btn primary"
            onClick={save}
            disabled={!form.name.trim()}
          >
            {editing ? "Save" : "Create"}
          </button>
        </div>
      </div>
    </>
  );
}

// --- webhooks --------------------------------------------------------------------------

const DEFAULT_PAYLOAD = `{
  "title": "$EVENT_TITLE",
  "body": "$EVENT_MSG",
  "transition": "$ALERT_TRANSITION",
  "scope": "$ALERT_SCOPE",
  "link": "$LINK"
}`;

function WebhookModal({
  initial,
  onClose,
}: {
  initial: Webhook | null;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const [w, setW] = useState<Webhook>(
    initial ?? {
      name: "",
      url: "",
      payload: DEFAULT_PAYLOAD,
      custom_headers: "",
      encode_as: "json",
    },
  );
  const [error, setError] = useState<string | null>(null);
  const set = (patch: Partial<Webhook>) => setW({ ...w, ...patch });
  return (
    <Modal
      title={initial ? `Edit ${initial.name}` : "New webhook"}
      onClose={onClose}
      width={620}
    >
      <div className="modal-body form">
        <label className="form-field">
          <span className="form-label">Name</span>
          <input
            className="input mono"
            value={w.name}
            onChange={(e) => set({ name: e.target.value })}
            placeholder="ops-alerts"
          />
          <span className="form-hint">
            Monitor messages reach it with @webhook-{w.name || "name"}.
          </span>
        </label>
        <label className="form-field">
          <span className="form-label">URL</span>
          <input
            className="input mono"
            value={w.url}
            onChange={(e) => set({ url: e.target.value })}
            placeholder="https://ntfy.sh/my-topic"
          />
        </label>
        <label className="form-field">
          <span className="form-label">Payload</span>
          <textarea
            className="input mono"
            rows={8}
            value={w.payload ?? ""}
            onChange={(e) => set({ payload: e.target.value })}
          />
          <span className="form-hint">
            Variables: $EVENT_TITLE $EVENT_MSG $ALERT_TRANSITION $ALERT_TYPE
            $ALERT_STATUS $ALERT_SCOPE $ALERT_ID $ALERT_QUERY $ALERT_PRIORITY
            $LINK $DATE $HOSTNAME $TAGS. Empty: Datadog's default payload.
          </span>
        </label>
        <label className="form-field">
          <span className="form-label">Custom headers (JSON)</span>
          <input
            className="input mono"
            value={w.custom_headers ?? ""}
            onChange={(e) => set({ custom_headers: e.target.value })}
            placeholder='{"Authorization": "Bearer …"}'
          />
        </label>
        <label className="form-field">
          <span className="form-label">Encode as</span>
          <select
            className="input"
            value={w.encode_as}
            onChange={(e) => set({ encode_as: e.target.value })}
          >
            <option value="json">JSON</option>
            <option value="form">Form</option>
          </select>
        </label>
        {error && <div className="error-box">{error}</div>}
      </div>
      <div className="modal-footer">
        <button className="btn" onClick={onClose}>
          Cancel
        </button>
        <button
          className="btn primary"
          onClick={async () => {
            try {
              if (initial) await api.updateWebhook(initial.name, w);
              else await api.createWebhook(w);
              await qc.invalidateQueries({ queryKey: ["webhooks"] });
              onClose();
            } catch (e) {
              setError(errorText(e));
            }
          }}
        >
          {initial ? "Save" : "Create"}
        </button>
      </div>
    </Modal>
  );
}

export function Webhooks() {
  const qc = useQueryClient();
  const hooks = useQuery({ queryKey: ["webhooks"], queryFn: api.webhooks });
  const [editing, setEditing] = useState<{ hook: Webhook | null } | null>(null);
  const [results, setResults] = useState<Record<string, string>>({});
  return (
    <>
      <PageHeader title="Monitors" tabs={MONITOR_TABS}>
        <button
          className="btn primary"
          onClick={() => setEditing({ hook: null })}
        >
          <PlusIcon /> New Webhook
        </button>
      </PageHeader>
      <div className="page-scroll page-padding">
        <p className="muted" style={{ maxWidth: 900, marginTop: 0 }}>
          Webhooks receive monitor notifications: add <code>@webhook-name</code>{" "}
          to a monitor's message. Same format as Datadog's Webhooks integration
          (and its API,
          <code> /api/v1/integration/webhooks/configuration/webhooks</code>).
        </p>
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th style={{ width: 220 }}>Name</th>
                <th>URL</th>
                <th style={{ width: 260 }} />
              </tr>
            </thead>
            <tbody>
              {(hooks.data ?? []).map((h) => (
                <tr key={h.name}>
                  <td className="mono">@webhook-{h.name}</td>
                  <td className="mono ellipsis">{h.url}</td>
                  <td>
                    <span
                      className="row-actions"
                      style={{ justifyContent: "flex-end" }}
                    >
                      {results[h.name] && (
                        <span className="muted">{results[h.name]}</span>
                      )}
                      <button
                        className="btn ghost small"
                        onClick={async () => {
                          const r = await api
                            .testWebhook(h.name)
                            .catch((e) => ({
                              ok: false,
                              error: errorText(e) ?? "",
                            }));
                          setResults({
                            ...results,
                            [h.name]: r.ok
                              ? `Sent (HTTP ${"status" in r ? r.status : ""})`
                              : `Failed: ${"error" in r ? r.error : ""}`,
                          });
                        }}
                      >
                        Test
                      </button>
                      <button
                        className="btn ghost small"
                        onClick={() => setEditing({ hook: h })}
                      >
                        Edit
                      </button>
                      <button
                        className="btn ghost small"
                        title="Delete"
                        onClick={async () => {
                          if (!window.confirm(`Delete the webhook ${h.name}?`))
                            return;
                          await api.deleteWebhook(h.name);
                          await qc.invalidateQueries({
                            queryKey: ["webhooks"],
                          });
                        }}
                      >
                        <TrashIcon />
                      </button>
                    </span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {hooks.data && !hooks.data.length && (
            <div className="empty-state">No webhook yet.</div>
          )}
        </div>
      </div>
      {editing && (
        <WebhookModal initial={editing.hook} onClose={() => setEditing(null)} />
      )}
    </>
  );
}
