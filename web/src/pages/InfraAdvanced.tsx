import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useSearchParams } from "react-router-dom";
import { TimeChart } from "../components/Charts";
import { ServiceChip, SidePanel } from "../components/Common";
import { PageHeader } from "../components/Layout";
import { TimePicker } from "../components/TimePicker";
import { api } from "../lib/api";
import { compact, duration, fullDate } from "../lib/format";
import { useTimeRange } from "../lib/time";

const mib = (b: number) =>
  b < 1 << 30
    ? `${(b / (1 << 20)).toFixed(1)} MiB`
    : `${(b / (1 << 30)).toFixed(2)} GiB`;
const secs = (v: number | null) => (v === null ? "—" : duration(v * 1e9));
const ms = (v: number) => duration(v * 1e6);

export function Processes() {
  const [params, setParams] = useSearchParams();
  const host = params.get("host") ?? "";
  const [query, setQuery] = useState(params.get("query") ?? "");
  const q = useQuery({
    queryKey: ["processes", host, params.get("query")],
    queryFn: () =>
      api.processes(host || undefined, params.get("query") || undefined),
    refetchInterval: 10000,
    placeholderData: keepPreviousData,
  });
  const set = (k: string, v: string) => {
    const p = new URLSearchParams(params);
    if (v) p.set(k, v);
    else p.delete(k);
    setParams(p, { replace: true });
  };
  const d = q.data;
  return (
    <>
      <PageHeader title="Live Processes" />
      <div className="filters-row">
        <select
          className="input"
          value={host}
          onChange={(e) => set("host", e.target.value)}
        >
          <option value="">All hosts</option>
          {d?.hosts.map((h) => (
            <option key={h.host}>{h.host}</option>
          ))}
        </select>
        <form
          style={{ flex: 1, display: "flex" }}
          onSubmit={(e) => {
            e.preventDefault();
            set("query", query);
          }}
        >
          <input
            className="input"
            style={{ flex: 1 }}
            placeholder="Filter by command, user or PID"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </form>
        <span className="muted">{d ? `${d.total} processes` : ""}</span>
      </div>
      <div className="page-scroll">
        {d && !d.hosts.length && (
          <div className="page-padding">
            <div className="card muted">
              No process reported in the last 2 minutes. Enable the process
              agent (
              <code>DD_PROCESS_CONFIG_PROCESS_COLLECTION_ENABLED=true</code>)
              with <code>DD_PROCESS_CONFIG_PROCESS_DD_URL</code> pointing at
              robine-dog.
            </div>
          </div>
        )}
        <table className="list">
          <thead>
            <tr>
              <th>Host</th>
              <th className="num">PID</th>
              <th>User</th>
              <th>Command</th>
              <th className="num">CPU</th>
              <th className="num">RSS</th>
              <th className="num">Threads</th>
              <th className="num">FDs</th>
            </tr>
          </thead>
          <tbody>
            {d?.processes.map((p) => (
              <tr key={`${p.host}/${p.pid}`}>
                <td className="muted">{p.host}</td>
                <td className="num mono">{p.pid}</td>
                <td>{p.user}</td>
                <td
                  className="mono llm-cell"
                  style={{ maxWidth: 600 }}
                  title={p.command}
                >
                  <b>{p.name}</b> {p.command}
                </td>
                <td className="num">{p.cpu_pct.toFixed(1)}%</td>
                <td className="num">{mib(p.rss)}</td>
                <td className="num">{p.threads}</td>
                <td className="num">{p.fds || "—"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </>
  );
}

export function DataStreams() {
  const time = useTimeRange("1h", 30_000);
  const q = useQuery({
    queryKey: ["dsm", time.from, time.to],
    queryFn: () => api.dataStreams(time.from, time.to),
    placeholderData: keepPreviousData,
  });
  const d = q.data;
  return (
    <>
      <PageHeader title="Data Streams Monitoring">
        <TimePicker time={time} />
      </PageHeader>
      <div className="page-scroll page-padding">
        {d && !d.edges.length && (
          <div className="card muted">
            No pathway in this period. Enable Data Streams in the tracers (
            <code>DD_DATA_STREAMS_ENABLED=true</code>): they send{" "}
            <code>/v0.1/pipeline_stats</code> to the trace port.
          </div>
        )}
        {d && d.edges.length > 0 && (
          <>
            <div className="card" style={{ padding: 0 }}>
              <table className="list">
                <thead>
                  <tr>
                    <th>Service</th>
                    <th>Direction</th>
                    <th>Queue</th>
                    <th>Consumer group</th>
                    <th className="num">Throughput</th>
                    <th className="num">Edge latency p50 / p99</th>
                    <th className="num">End-to-end p50 / p99</th>
                  </tr>
                </thead>
                <tbody>
                  {d.edges.map((e, i) => (
                    <tr key={i}>
                      <td>
                        <ServiceChip name={e.service} />
                      </td>
                      <td>
                        {e.direction === "out"
                          ? "produces to →"
                          : e.direction === "in"
                            ? "← consumes from"
                            : "—"}
                      </td>
                      <td className="mono">
                        {e.topic ?? "—"} <span className="faint">{e.type}</span>
                      </td>
                      <td>{e.group ?? "—"}</td>
                      <td className="num">{e.throughput.toFixed(2)}/s</td>
                      <td className="num mono">
                        {secs(e.edge_p50)} / {secs(e.edge_p99)}
                      </td>
                      <td className="num mono">
                        {secs(e.pathway_p50)} / {secs(e.pathway_p99)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <div className="section-title">Consumer lag (Kafka offsets)</div>
            <div className="card" style={{ padding: 0 }}>
              <table className="list">
                <thead>
                  <tr>
                    <th>Consumer group</th>
                    <th>Topic</th>
                    <th className="num">Lag (messages)</th>
                  </tr>
                </thead>
                <tbody>
                  {d.lag.map((l) => (
                    <tr key={l.consumer_group + l.topic}>
                      <td>{l.consumer_group}</td>
                      <td className="mono">{l.topic}</td>
                      <td
                        className="num"
                        style={{
                          color: l.lag > 1000 ? "var(--red)" : undefined,
                        }}
                      >
                        {compact(l.lag)}
                      </td>
                    </tr>
                  ))}
                  {!d.lag.length && (
                    <tr>
                      <td colSpan={3} className="faint">
                        No Kafka offsets reported.
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
          </>
        )}
      </div>
    </>
  );
}

function QueryPanel({
  sig,
  text,
  from,
  to,
  onClose,
}: {
  sig: string;
  text: string;
  from: number;
  to: number;
  onClose: () => void;
}) {
  const q = useQuery({
    queryKey: ["dbm-query", sig, from, to],
    queryFn: () => api.databaseQuery(sig, from, to),
  });
  const d = q.data;
  return (
    <SidePanel
      onClose={onClose}
      storageKey="dbm-query"
      header={<span className="mono">{sig}</span>}
    >
      <div className="panel-section">
        <pre
          className="mono"
          style={{ whiteSpace: "pre-wrap", margin: 0, fontSize: 12 }}
        >
          {text}
        </pre>
      </div>
      {d && (
        <>
          <div className="panel-section">
            <h4>Calls and average latency</h4>
            <TimeChart
              times={d.series.map((p) => p.t)}
              interval={d.interval_ms}
              from={from}
              to={to}
              height={120}
              series={[
                {
                  key: "calls",
                  label: "Calls",
                  color: "#8b6fd0",
                  values: d.series.map((p) => p.calls),
                },
              ]}
            />
            <TimeChart
              kind="lines"
              times={d.series.map((p) => p.t)}
              interval={d.interval_ms}
              from={from}
              to={to}
              height={120}
              format={ms}
              series={[
                {
                  key: "avg",
                  label: "Avg latency",
                  color: "#e3a63c",
                  values: d.series.map((p) =>
                    p.calls ? p.total_time_ms / p.calls : 0,
                  ),
                },
              ]}
            />
          </div>
          <div className="panel-section">
            <h4>Samples and explain plans</h4>
            {d.samples.map((s, i) => (
              <details key={i} className="llm-message">
                <summary>
                  {fullDate(s.ts)} · {s.host} {s.db && `· ${s.db}`}{" "}
                  {s.duration_ns !== null && `· ${duration(s.duration_ns)}`}
                </summary>
                {s.statement && (
                  <pre
                    className="mono"
                    style={{ whiteSpace: "pre-wrap", fontSize: 12 }}
                  >
                    {s.statement}
                  </pre>
                )}
                {s.plan !== null && (
                  <pre
                    className="mono"
                    style={{ fontSize: 11, maxHeight: 300, overflow: "auto" }}
                  >
                    {JSON.stringify(s.plan, null, 2)}
                  </pre>
                )}
              </details>
            ))}
            {!d.samples.length && (
              <div className="faint">No sample in this period.</div>
            )}
          </div>
        </>
      )}
    </SidePanel>
  );
}

export function Databases() {
  const time = useTimeRange("1h", 30_000);
  const [params, setParams] = useSearchParams();
  const host = params.get("host") ?? "";
  const [open, setOpen] = useState<{ sig: string; text: string } | null>(null);
  const q = useQuery({
    queryKey: ["dbm", time.from, time.to, host],
    queryFn: () => api.databases(time.from, time.to, host || undefined),
    placeholderData: keepPreviousData,
  });
  const d = q.data;
  return (
    <>
      <PageHeader title="Database Monitoring">
        <TimePicker time={time} />
      </PageHeader>
      <div className="filters-row">
        <select
          className="input"
          value={host}
          onChange={(e) => {
            const p = new URLSearchParams(params);
            if (e.target.value) p.set("host", e.target.value);
            else p.delete("host");
            setParams(p, { replace: true });
          }}
        >
          <option value="">All database hosts</option>
          {d?.hosts.map((h) => (
            <option key={h.host} value={h.host}>
              {h.host} ({h.dbms})
            </option>
          ))}
        </select>
      </div>
      <div className="page-scroll page-padding">
        {d && !d.queries.length && !d.activity.length && (
          <div className="card muted">
            No database telemetry. Set <code>dbm: true</code> on the Agent's
            postgres or mysql check and point{" "}
            <code>database_monitoring.metrics.dd_url</code>,{" "}
            <code>samples.dd_url</code> and <code>activity.dd_url</code> at
            robine-dog.
          </div>
        )}
        {d && d.queries.length > 0 && (
          <div className="card" style={{ padding: 0 }}>
            <table className="list">
              <thead>
                <tr>
                  <th>Normalized query</th>
                  <th>Database</th>
                  <th className="num">Calls</th>
                  <th className="num">Avg latency</th>
                  <th className="num">Total time</th>
                  <th className="num">Rows</th>
                </tr>
              </thead>
              <tbody>
                {d.queries.map((x) => (
                  <tr
                    key={x.signature}
                    style={{ cursor: "pointer" }}
                    onClick={() => setOpen({ sig: x.signature, text: x.query })}
                  >
                    <td
                      className="mono llm-cell"
                      style={{ maxWidth: 560 }}
                      title={x.query}
                    >
                      {x.query}
                    </td>
                    <td>
                      {x.db ?? "—"} <span className="faint">{x.dbms}</span>
                    </td>
                    <td className="num">{compact(x.calls)}</td>
                    <td className="num mono">{ms(x.avg_ms)}</td>
                    <td className="num mono">{ms(x.total_time_ms)}</td>
                    <td className="num">{compact(x.rows)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
        {d && d.activity.length > 0 && (
          <>
            <div className="section-title">Active sessions</div>
            <div className="card" style={{ padding: 0 }}>
              <table className="list">
                <thead>
                  <tr>
                    <th>Host</th>
                    <th>User</th>
                    <th>State</th>
                    <th>Wait</th>
                    <th>Query</th>
                  </tr>
                </thead>
                <tbody>
                  {d.activity.map((a, i) => (
                    <tr key={i}>
                      <td className="muted">{String(a.host)}</td>
                      <td>{String(a.usename ?? a.processlist_user ?? "")}</td>
                      <td>{String(a.state ?? a.processlist_state ?? "")}</td>
                      <td>{String(a.wait_event ?? a.wait_event_type ?? "")}</td>
                      <td className="mono llm-cell" style={{ maxWidth: 500 }}>
                        {String(a.query ?? a.sql_text ?? "")}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </>
        )}
      </div>
      {open && (
        <QueryPanel
          sig={open.sig}
          text={open.text}
          from={time.from}
          to={time.to}
          onClose={() => setOpen(null)}
        />
      )}
    </>
  );
}
