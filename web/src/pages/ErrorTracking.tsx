import {
  keepPreviousData,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { useMemo, useState } from "react";
import {
  Link,
  useNavigate,
  useParams,
  useSearchParams,
} from "react-router-dom";
import { Sparkline, TimeChart } from "../components/Charts";
import { ServiceChip } from "../components/Common";
import { SearchBar, type KeyHint } from "../components/SearchBar";
import { PageHeader } from "../components/Layout";
import { TimePicker } from "../components/TimePicker";
import { api, ApiError, type ErrorIssue } from "../lib/api";
import { ago, fullDate, thousands } from "../lib/format";
import { useTimeRange } from "../lib/time";

export const STATUS_LABELS: Record<string, string> = {
  FOR_REVIEW: "For Review",
  REVIEWED: "Reviewed",
  IGNORED: "Ignored",
  RESOLVED: "Resolved",
};
const KEYS: KeyHint[] = [
  { path: "service", name: "Service" },
  { path: "env", name: "Env" },
  { path: "version", name: "Version" },
  { path: "resource_name", name: "Resource" },
  { path: "@error.type", name: "Error Type" },
  { path: "@error.message", name: "Error Message" },
];
/** The time range part of the URL. */
const keepTime = (p: URLSearchParams) =>
  new URLSearchParams(
    [...p.entries()].filter(([k]) =>
      ["range", "from_ts", "to_ts", "live"].includes(k),
    ),
  );
const SOURCE_LABELS: Record<string, string> = { spans: "APM", logs: "Logs" };
const errorText = (e: unknown) =>
  e instanceof ApiError ? e.message : e ? String(e) : null;

function IssueStatus({
  status,
  regression,
}: {
  status: string;
  regression?: boolean;
}) {
  return (
    <span className="issue-status-wrap">
      <span className={`issue-status issue-${status.toLowerCase()}`}>
        {STATUS_LABELS[status] ?? status}
      </span>
      {regression && <span className="issue-regression">Regression</span>}
    </span>
  );
}

/** Splits `Type: message` off a frame list and marks the application's frames. */
function StackTrace({ stack }: { stack: string }) {
  const lines = stack.split("\n");
  return (
    <pre className="stack-trace">
      {lines.map((l, i) => {
        const frame = /^\s+(at |File |\/)|^\s+\S+\.go:\d+/.test(l);
        const vendor =
          /site-packages|org\.springframework|jakarta\.|\/usr\/lib|node_modules|runtime\//.test(
            l,
          );
        return (
          <div
            key={i}
            className={
              frame
                ? vendor
                  ? "frame vendor"
                  : "frame app"
                : i === 0 || !frame
                  ? "frame-head"
                  : ""
            }
          >
            {l || " "}
          </div>
        );
      })}
    </pre>
  );
}

export function ErrorTracking() {
  const time = useTimeRange("1d", 60_000);
  const [params, setParams] = useSearchParams();
  const navigate = useNavigate();
  const query = params.get("query") ?? "";
  const statusFilter = params.get("status") ?? "FOR_REVIEW";
  const source = params.get("source") ?? "";
  const service = params.get("service") ?? "";
  const team = params.get("team") ?? "";
  const res = useQuery({
    queryKey: ["issues", query, time.from, time.to],
    queryFn: ({ signal }) =>
      api.errorIssues({ from: time.from, to: time.to, query }, signal),
    placeholderData: keepPreviousData,
  });
  const set = (k: string, v: string | null) =>
    setParams((prev) => {
      const p = new URLSearchParams(prev);
      if (v === null) p.delete(k);
      else p.set(k, v);
      return p;
    });
  const all = res.data?.issues ?? [];
  const counts = useMemo(() => {
    const c = (f: (i: ErrorIssue) => string) => {
      const m = new Map<string, number>();
      for (const i of all) m.set(f(i), (m.get(f(i)) ?? 0) + 1);
      return m;
    };
    return {
      status: c((i) => i.status),
      source: c((i) => i.source),
      service: c((i) => i.service),
      team: c((i) => i.team ?? ""),
    };
  }, [all]);
  const shown = all.filter(
    (i) =>
      (statusFilter === "ALL" || i.status === statusFilter) &&
      (!source || i.source === source) &&
      (!service || i.service === service) &&
      (!team || (i.team ?? "") === team),
  );

  return (
    <>
      <PageHeader
        title="Error Tracking"
        tabs={[{ to: "/error-tracking", label: "Issues" }]}
      >
        <TimePicker time={time} allowLive={false} />
      </PageHeader>
      <div className="searchbar-row">
        <SearchBar
          value={query}
          onSubmit={(q) => set("query", q || null)}
          dataset="spans"
          scope={time}
          keys={KEYS}
          error={res.error ? errorText(res.error) : null}
          placeholder="Filter errors: service:shop-api env:prod"
        />
      </div>
      <div className="explorer">
        <aside className="facets" aria-label="Issue facets">
          <div className="facet">
            <div className="facet-header" style={{ cursor: "default" }}>
              <span>Status</span>
            </div>
            <div className="facet-body">
              {[...Object.keys(STATUS_LABELS), "ALL"].map((s) => (
                <label className="facet-value" key={s}>
                  <input
                    type="radio"
                    name="status"
                    checked={statusFilter === s}
                    onChange={() => set("status", s)}
                  />
                  <span className="facet-value-label">
                    {s === "ALL" ? "All" : STATUS_LABELS[s]}
                  </span>
                  <span className="facet-value-count">
                    {s === "ALL" ? all.length : (counts.status.get(s) ?? 0)}
                  </span>
                </label>
              ))}
            </div>
          </div>
          <div className="facet">
            <div className="facet-header" style={{ cursor: "default" }}>
              <span>Source</span>
            </div>
            <div className="facet-body">
              {[...counts.source.entries()].map(([s, n]) => (
                <label className="facet-value" key={s}>
                  <input
                    type="checkbox"
                    checked={source === s}
                    onChange={() => set("source", source === s ? null : s)}
                  />
                  <span className="facet-value-label">
                    {SOURCE_LABELS[s] ?? s}
                  </span>
                  <span className="facet-value-count">{n}</span>
                </label>
              ))}
            </div>
          </div>
          <div className="facet">
            <div className="facet-header" style={{ cursor: "default" }}>
              <span>Team</span>
            </div>
            <div className="facet-body">
              {[...counts.team.entries()]
                .sort((a, b) => b[1] - a[1])
                .map(([t, n]) => (
                  <label className="facet-value" key={t}>
                    <input
                      type="checkbox"
                      checked={team === t}
                      onChange={() => set("team", team === t ? null : t)}
                    />
                    <span className="facet-value-label">
                      {t || "(no owner in the Software Catalog)"}
                    </span>
                    <span className="facet-value-count">{n}</span>
                  </label>
                ))}
            </div>
          </div>
          <div className="facet">
            <div className="facet-header" style={{ cursor: "default" }}>
              <span>Service</span>
            </div>
            <div className="facet-body">
              {[...counts.service.entries()]
                .sort((a, b) => b[1] - a[1])
                .map(([s, n]) => (
                  <label className="facet-value" key={s}>
                    <input
                      type="checkbox"
                      checked={service === s}
                      onChange={() => set("service", service === s ? null : s)}
                    />
                    <span className="facet-value-label">{s || "(none)"}</span>
                    <span className="facet-value-count">{n}</span>
                  </label>
                ))}
            </div>
          </div>
        </aside>
        <div className="content">
          <div className="toolbar">
            <span>
              <strong>{shown.length}</strong> issues
            </span>
            {res.isFetching && <span className="spinner" />}
          </div>
          <div className="table-wrap">
            <table className="list issues-table">
              <thead>
                <tr>
                  <th>Issue</th>
                  <th style={{ width: 140 }}>Volume</th>
                  <th className="num" style={{ width: 80 }}>
                    Count
                  </th>
                  <th style={{ width: 150 }}>Seen</th>
                  <th style={{ width: 130 }}>Status</th>
                </tr>
              </thead>
              <tbody>
                {shown.map((i) => (
                  <tr
                    key={i.id}
                    onClick={() =>
                      navigate(
                        `/error-tracking/issue/${i.id}?${keepTime(params)}`,
                      )
                    }
                  >
                    <td>
                      <div className="issue-title">
                        <span className="issue-type">{i.type}</span>
                        <span className="issue-message">{i.message}</span>
                      </div>
                      <div className="issue-meta">
                        <span className="issue-source">
                          {SOURCE_LABELS[i.source]}
                        </span>
                        <ServiceChip name={i.service || null} />
                        {i.team && <span className="tag">team:{i.team}</span>}
                        {i.frame && (
                          <span className="mono faint ellipsis">{i.frame}</span>
                        )}
                      </div>
                    </td>
                    <td>
                      <Sparkline
                        values={i.series}
                        color="#e74c50"
                        width={120}
                        height={26}
                      />
                    </td>
                    <td className="num">{thousands(i.count)}</td>
                    <td className="muted">
                      <div>Last seen {ago(i.last_seen)}</div>
                      <div className="faint">
                        First seen {ago(i.first_seen)}
                      </div>
                    </td>
                    <td>
                      <IssueStatus
                        status={i.status}
                        regression={i.regression}
                      />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            {res.data && !shown.length && (
              <div className="empty-state">
                <h3>No issue</h3>
                Errors are spans with <code>error.type</code> and error logs
                with <code>error.kind</code> or <code>error.stack</code>.
              </div>
            )}
          </div>
        </div>
      </div>
    </>
  );
}

export function IssuePage() {
  const { id } = useParams();
  const qc = useQueryClient();
  const time = useTimeRange("1d", 60_000);
  const res = useQuery({
    queryKey: ["issue", id, time.from, time.to],
    queryFn: () => api.errorIssue(id!, time.from, time.to),
    placeholderData: keepPreviousData,
  });
  const [selected, setSelected] = useState(0);
  const issue = res.data;
  const chart = useMemo(() => {
    if (!issue) return null;
    const at = new Map(issue.series.map(([t, c]) => [t, c]));
    const start = Math.floor(time.from / issue.interval) * issue.interval;
    const times: number[] = [];
    for (let t = start; t <= time.to; t += issue.interval) times.push(t);
    return { times, values: times.map((t) => at.get(t) ?? 0) };
  }, [issue, time.from, time.to]);
  if (res.error)
    return <div className="page-padding error-box">{errorText(res.error)}</div>;
  if (!issue || !chart)
    return <div className="page-padding faint">Loading…</div>;
  const occ =
    issue.occurrences[Math.min(selected, issue.occurrences.length - 1)];
  const explorer =
    issue.source === "logs"
      ? `/logs?${new URLSearchParams({ query: issue.query, from_ts: String(time.from), to_ts: String(time.to) })}`
      : `/apm/traces?${new URLSearchParams({ query: issue.query, spans: "all", from_ts: String(time.from), to_ts: String(time.to) })}`;
  return (
    <>
      <PageHeader
        title="Error Tracking"
        tabs={[{ to: "/error-tracking", label: "Issues" }]}
      >
        <TimePicker time={time} allowLive={false} />
      </PageHeader>
      <div className="page-scroll page-padding">
        <Link to="/error-tracking" className="faint back-link">
          ← All issues
        </Link>
        <div className="issue-header">
          <div style={{ minWidth: 0 }}>
            <h2 className="issue-type">{issue.type}</h2>
            <div className="issue-message big">
              {occ?.message ?? issue.pattern}
            </div>
            <div className="issue-meta">
              <span className="issue-source">
                {SOURCE_LABELS[issue.source]}
              </span>
              <ServiceChip name={issue.service || null} />
              {issue.frame && <span className="mono faint">{issue.frame}</span>}
            </div>
          </div>
          <div className="spacer" />
          <label className="issue-status-select">
            {issue.regression && (
              <span className="issue-regression">Regression</span>
            )}
            <select
              className={`input issue-status-input issue-${(issue.regression ? "FOR_REVIEW" : issue.status).toLowerCase()}`}
              value={issue.regression ? "FOR_REVIEW" : issue.status}
              onChange={async (e) => {
                await api.setIssueStatus(issue.id, e.target.value);
                await qc.invalidateQueries({ queryKey: ["issue", id] });
                await qc.invalidateQueries({ queryKey: ["issues"] });
              }}
            >
              {Object.entries(STATUS_LABELS).map(([k, v]) => (
                <option key={k} value={k}>
                  {v}
                </option>
              ))}
            </select>
          </label>
        </div>
        <div className="issue-stats">
          <div>
            <span className="faint">Occurrences</span>
            <strong>{thousands(issue.count)}</strong>
          </div>
          <div>
            <span className="faint">First seen</span>
            <strong title={fullDate(issue.first_seen)}>
              {ago(issue.first_seen)}
            </strong>
          </div>
          <div>
            <span className="faint">Last seen</span>
            <strong title={issue.last_seen ? fullDate(issue.last_seen) : ""}>
              {issue.last_seen ? ago(issue.last_seen) : "—"}
            </strong>
          </div>
          {Object.entries(issue.breakdown)
            .filter(([k]) => k === "version" || k === "env")
            .map(([k, vals]) => (
              <div key={k}>
                <span className="faint">
                  {k === "version" ? "Versions" : "Envs"}
                </span>
                <strong>{vals.map(([v]) => v).join(", ") || "—"}</strong>
              </div>
            ))}
          <div className="spacer" />
          <Link className="btn" to={explorer}>
            View in{" "}
            {issue.source === "logs" ? "Log Explorer" : "Trace Explorer"}
          </Link>
        </div>
        <div className="card">
          <TimeChart
            times={chart.times}
            interval={issue.interval}
            from={time.from}
            to={time.to}
            series={[
              {
                key: "errors",
                label: "Errors",
                color: "#e74c50",
                values: chart.values,
              },
            ]}
            height={110}
            onZoom={time.setAbsolute}
          />
        </div>
        <div className="issue-grid">
          <div className="card" style={{ minWidth: 0 }}>
            <div className="card-title">
              Stack trace
              {occ && <span className="muted">{fullDate(occ.ts)}</span>}
              <div className="spacer" />
              {occ?.trace_id && issue.source === "spans" && (
                <Link
                  className="btn small"
                  to={`/apm/traces?${new URLSearchParams({ query: "", spans: "all", trace: occ.trace_id, span: occ.span_id ?? "" })}`}
                >
                  View trace
                </Link>
              )}
              {occ?.id && issue.source === "logs" && (
                <Link
                  className="btn small"
                  to={`/logs?${new URLSearchParams({ query: issue.query, event: occ.id, from_ts: String(time.from), to_ts: String(time.to) })}`}
                >
                  View log
                </Link>
              )}
            </div>
            {occ?.snapshot_ids?.length ? (
              <div style={{ marginBottom: 8 }}>
                Exception Replay:{" "}
                <Link
                  to={`/logs?${new URLSearchParams({ query: `source:dd_debugger (${occ.snapshot_ids.map((x) => `@debugger.snapshot.id:${x}`).join(" OR ")})`, from_ts: String(occ.ts - 60_000), to_ts: String(occ.ts + 60_000), live: "false" })}`}
                >
                  {occ.snapshot_ids.length} snapshot
                  {occ.snapshot_ids.length > 1 ? "s" : ""} with the local
                  variables
                </Link>
              </div>
            ) : null}
            {occ?.stack ? (
              <StackTrace stack={occ.stack} />
            ) : (
              <div className="faint">No stack trace on this error.</div>
            )}
          </div>
          <div className="card">
            <div className="card-title">Breakdown</div>
            {Object.entries(issue.breakdown).map(([k, vals]) => (
              <div className="breakdown" key={k}>
                <div className="breakdown-title">{k}</div>
                {vals.map(([v, n]) => (
                  <div className="breakdown-row" key={v}>
                    <span className="mono ellipsis" title={v}>
                      {v}
                    </span>
                    <span className="breakdown-bar">
                      <i
                        style={{
                          width: `${(n / Math.max(1, issue.occurrences.length)) * 100}%`,
                        }}
                      />
                    </span>
                    <span className="num">
                      {Math.round(
                        (n / Math.max(1, issue.occurrences.length)) * 100,
                      )}
                      %
                    </span>
                  </div>
                ))}
              </div>
            ))}
            {!Object.keys(issue.breakdown).length && (
              <div className="faint">Nothing to break down.</div>
            )}
          </div>
        </div>
        <div className="card" style={{ padding: 0 }}>
          <div className="card-title" style={{ padding: "12px 16px 0" }}>
            Latest occurrences{" "}
            <span className="muted">{issue.occurrences.length} shown</span>
          </div>
          <table className="list">
            <thead>
              <tr>
                <th style={{ width: 190 }}>Date</th>
                {issue.source === "spans" && <th>Resource</th>}
                <th>Message</th>
                {issue.source === "spans" && <th style={{ width: 90 }}>Env</th>}
                {issue.source === "spans" && (
                  <th style={{ width: 90 }}>Version</th>
                )}
                <th style={{ width: 160 }}>Host</th>
              </tr>
            </thead>
            <tbody>
              {issue.occurrences.slice(0, 50).map((o, i) => (
                <tr
                  key={`${o.id}-${i}`}
                  className={i === selected ? "selected" : ""}
                  onClick={() => setSelected(i)}
                >
                  <td className="date">{fullDate(o.ts)}</td>
                  {issue.source === "spans" && (
                    <td className="mono ellipsis">{o.resource}</td>
                  )}
                  <td className="ellipsis">{o.message}</td>
                  {issue.source === "spans" && <td>{o.env}</td>}
                  {issue.source === "spans" && <td>{o.version}</td>}
                  <td className="mono ellipsis">{o.host ?? "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    </>
  );
}
