import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { Link, useNavigate, useParams, useSearchParams } from "react-router-dom";
import { Sparkline, TimeChart, useWidth } from "../components/Charts";
import { ServiceChip } from "../components/Common";
import { PageHeader } from "../components/Layout";
import { TimePicker } from "../components/TimePicker";
import { api, type ServiceSummary } from "../lib/api";
import { ago, compact, duration, percent, rate, serviceColor, spanTypeIcon } from "../lib/format";
import { useTimeRange } from "../lib/time";
import { APM_TABS } from "./TraceExplorer";

export function useEnv(): [string, (v: string) => void] {
  const [params, setParams] = useSearchParams();
  const env = params.get("env") ?? "";
  return [
    env,
    (v) =>
      setParams((prev) => {
        const p = new URLSearchParams(prev);
        if (v) p.set("env", v);
        else p.delete("env");
        return p;
      }),
  ];
}

export function EnvSelect() {
  const [env, setEnv] = useEnv();
  const envs = useQuery({ queryKey: ["envs"], queryFn: api.envs });
  return (
    <label>
      env:
      <select className="input" value={env} onChange={(e) => setEnv(e.target.value)}>
        <option value="">* (all)</option>
        {(envs.data ?? []).map((e) => (
          <option key={e} value={e}>
            {e}
          </option>
        ))}
      </select>
    </label>
  );
}

type SortKey = "name" | "hits" | "p95" | "error_rate" | "type";

export function ServiceCatalog() {
  const time = useTimeRange("1h", 30_000);
  const [env] = useEnv();
  const [filter, setFilter] = useState("");
  const [sort, setSort] = useState<{ key: SortKey; desc: boolean }>({ key: "hits", desc: true });
  const navigate = useNavigate();
  const services = useQuery({
    queryKey: ["services", time.from, time.to, env],
    queryFn: () => api.services(time.from, time.to, env || undefined),
    placeholderData: keepPreviousData,
  });

  const rows = [...(services.data ?? [])]
    .filter((s) => !filter || s.name.includes(filter))
    .sort((a, b) => {
      const va = a[sort.key] as number | string;
      const vb = b[sort.key] as number | string;
      const c = typeof va === "string" ? va.localeCompare(vb as string) : (va as number) - (vb as number);
      return sort.desc ? -c : c;
    });

  const th = (key: SortKey, label: string, num = false) => (
    <th className={`sortable${num ? " num" : ""}`} onClick={() => setSort((s) => ({ key, desc: s.key === key ? !s.desc : key !== "name" }))}>
      {label} {sort.key === key ? (sort.desc ? "↓" : "↑") : ""}
    </th>
  );

  const qs = (s: ServiceSummary) => {
    const p = new URLSearchParams(window.location.search);
    return `/apm/services/${encodeURIComponent(s.name)}?${p}`;
  };

  return (
    <>
      <PageHeader title="APM" tabs={APM_TABS}>
        <TimePicker time={time} />
      </PageHeader>
      <div className="filters-row">
        <EnvSelect />
        <input className="input" placeholder="Filter services" value={filter} onChange={(e) => setFilter(e.target.value)} style={{ width: 240 }} />
        <div className="spacer" />
        <span className="muted">{rows.length} services</span>
      </div>
      <div className="page-scroll">
        <table className="list">
          <colgroup>
            <col />
            <col style={{ width: 90 }} />
            <col style={{ width: 230 }} />
            <col style={{ width: 120 }} />
            <col style={{ width: 120 }} />
            <col style={{ width: 160 }} />
          </colgroup>
          <thead>
            <tr>
              {th("name", "Service")}
              {th("type", "Type")}
              {th("hits", "Requests")}
              {th("p95", "P95 Latency", true)}
              <th className="num">Avg Latency</th>
              {th("error_rate", "Error Rate", true)}
            </tr>
          </thead>
          <tbody>
            {rows.map((s) => (
              <tr key={s.name} onClick={() => navigate(qs(s))} style={{ height: 44 }}>
                <td style={{ height: 44 }}>
                  <ServiceChip name={s.name} />
                  <div className="faint mono" style={{ fontSize: 11, paddingLeft: 15 }}>
                    {s.operation}
                  </div>
                </td>
                <td>
                  <span className="type-badge">{spanTypeIcon(s.type)}</span>
                </td>
                <td>
                  <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                    <Sparkline values={s.sparkline} color={serviceColor(s.name)} />
                    <span>
                      {compact(s.hits)} <span className="faint">· {rate(s.requests_per_second)}</span>
                    </span>
                  </div>
                </td>
                <td className="num mono">{duration(s.p95)}</td>
                <td className="num mono">{duration(s.avg)}</td>
                <td className="num">
                  {s.error_rate > 0 && <span className="errbar" style={{ width: Math.max(3, Math.min(60, s.error_rate * 300)) }} />}
                  <span style={{ color: s.error_rate > 0.05 ? "var(--red)" : undefined }}>{percent(s.error_rate)}</span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {!services.isLoading && rows.length === 0 && (
          <div className="empty-state">
            <h3>No services reported traces in this time range</h3>
            Instrument an app with ddtrace and point it at <span className="mono">DD_TRACE_AGENT_URL=http://&lt;robine-dog&gt;:8126</span>.
          </div>
        )}
      </div>
    </>
  );
}

export function ServicePage() {
  const { name = "" } = useParams();
  const time = useTimeRange("1h", 30_000);
  const [env] = useEnv();
  const navigate = useNavigate();
  const detail = useQuery({
    queryKey: ["service", name, time.from, time.to, env],
    queryFn: () => api.service(name, time.from, time.to, env || undefined),
    placeholderData: keepPreviousData,
  });
  const d = detail.data;
  const times = d?.timeseries.map((p) => p.t) ?? [];
  const seconds = Math.max(1, (time.to - time.from) / 1000);

  const tracesLink = (extra: string) => {
    const q = [`service:${name}`, env ? `env:${env}` : "", extra].filter(Boolean).join(" ");
    const p = new URLSearchParams();
    p.set("query", q);
    if (time.relative && !time.paused) p.set("range", time.relative);
    else {
      p.set("from_ts", String(time.from));
      p.set("to_ts", String(time.to));
      p.set("live", "false");
    }
    return `/apm/traces?${p}`;
  };

  return (
    <>
      <PageHeader
        title={
          <>
            <Link to={`/apm/services${window.location.search}`} className="muted" style={{ fontWeight: 400 }}>
              Services
            </Link>
            <span className="faint">/</span>
            <span className="service-chip">
              <i style={{ background: serviceColor(name), width: 12, height: 12 }} />
              {name}
            </span>
            {d && <span className="type-badge">{spanTypeIcon(d.type)}</span>}
          </>
        }
      >
        <TimePicker time={time} />
      </PageHeader>
      <div className="filters-row">
        <EnvSelect />
        {d && d.operations.length > 0 && (
          <span className="muted">
            Operation: <span className="mono">{d.operations[0]}</span>
          </span>
        )}
        <div className="spacer" />
        <Link className="btn" to={tracesLink("")}>
          View Traces
        </Link>
        <Link className="btn" to={`/logs?query=${encodeURIComponent(`service:${name}`)}`}>
          View Logs
        </Link>
      </div>
      <div className="page-scroll">
        {d && (
          <>
            <div className="stat-row">
              <div className="stat">
                <span className="label">Requests</span>
                <span className="value">{compact(d.summary.hits)}</span>
                <span className="faint">{rate(d.summary.hits / seconds)}</span>
              </div>
              <div className="stat">
                <span className="label">Errors</span>
                <span className="value" style={{ color: d.summary.errors ? "var(--red)" : undefined }}>
                  {compact(d.summary.errors)}
                </span>
                <span className="faint">{percent(d.summary.error_rate)} error rate</span>
              </div>
              <div className="stat">
                <span className="label">P50</span>
                <span className="value">{duration(d.summary.p50)}</span>
              </div>
              <div className="stat">
                <span className="label">P95</span>
                <span className="value">{duration(d.summary.p95)}</span>
              </div>
              <div className="stat">
                <span className="label">P99</span>
                <span className="value">{duration(d.summary.p99)}</span>
              </div>
              <div className="stat">
                <span className="label">Max</span>
                <span className="value">{duration(d.summary.max)}</span>
              </div>
            </div>
            <div className="page-padding">
              <div className="cards">
                <div className="card">
                  <div className="card-title">
                    Requests <span className="muted">hits, by status</span>
                  </div>
                  <TimeChart
                    times={times}
                    interval={d.interval_ms}
                    from={time.from}
                    to={time.to}
                    height={150}
                    series={[
                      { key: "ok", label: "OK", color: "#8b6fd0", values: d.timeseries.map((p) => p.hits - p.errors) },
                      { key: "errors", label: "Errors", color: "#e74c50", values: d.timeseries.map((p) => p.errors) },
                    ]}
                    onZoom={time.setAbsolute}
                  />
                </div>
                <div className="card">
                  <div className="card-title">
                    Errors <span className="muted">count</span>
                  </div>
                  <TimeChart
                    times={times}
                    interval={d.interval_ms}
                    from={time.from}
                    to={time.to}
                    height={150}
                    series={[{ key: "errors", label: "Errors", color: "#e74c50", values: d.timeseries.map((p) => p.errors) }]}
                    onZoom={time.setAbsolute}
                  />
                </div>
                <div className="card">
                  <div className="card-title">
                    Latency <span className="muted">p50 · p95 · p99 · max</span>
                  </div>
                  <TimeChart
                    kind="lines"
                    times={times}
                    interval={d.interval_ms}
                    from={time.from}
                    to={time.to}
                    height={150}
                    format={duration}
                    series={[
                      { key: "p50", color: "#5d8ae6", values: d.timeseries.map((p) => p.p50) },
                      { key: "p95", color: "#9b6fd6", values: d.timeseries.map((p) => p.p95) },
                      { key: "p99", color: "#e3a63c", values: d.timeseries.map((p) => p.p99) },
                      { key: "max", color: "#d65c9b", values: d.timeseries.map((p) => p.max) },
                    ]}
                    onZoom={time.setAbsolute}
                  />
                </div>
              </div>

              <div className="cards" style={{ marginTop: 12 }}>
                <div className="card">
                  <div className="card-title">
                    Latency Distribution <span className="muted">all requests, log scale</span>
                  </div>
                  <Distribution data={d.distribution} p50={d.summary.p50} p95={d.summary.p95} p99={d.summary.p99} />
                </div>
                <div className="card">
                  <div className="card-title">
                    Downstream Services <span className="muted">calls from {name}</span>
                  </div>
                  {d.downstream.length === 0 && <div className="faint">No calls into other instrumented services.</div>}
                  {d.downstream.length > 0 && (
                    <table className="list">
                      <thead>
                        <tr>
                          <th>Service</th>
                          <th className="num">Calls</th>
                          <th className="num">Errors</th>
                          <th className="num">Avg</th>
                        </tr>
                      </thead>
                      <tbody>
                        {d.downstream.map((s) => (
                          <tr key={s.service} onClick={() => navigate(`/apm/services/${encodeURIComponent(s.service)}${window.location.search}`)}>
                            <td>
                              <ServiceChip name={s.service} />
                            </td>
                            <td className="num">{compact(s.calls)}</td>
                            <td className="num" style={{ color: s.errors ? "var(--red)" : undefined }}>
                              {compact(s.errors)}
                            </td>
                            <td className="num mono">{duration(s.avg)}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  )}
                </div>
              </div>

              {d.deployments.length > 0 && (
                <>
                  <div className="section-title">Deployments</div>
                  <table className="list" style={{ border: "1px solid var(--border)", borderRadius: 6 }}>
                    <thead>
                      <tr>
                        <th>Version</th>
                        <th style={{ width: 170 }}>First seen</th>
                        <th style={{ width: 170 }}>Last seen</th>
                        <th className="num" style={{ width: 110 }}>
                          Requests
                        </th>
                        <th className="num" style={{ width: 130 }}>
                          Error rate
                        </th>
                        <th className="num" style={{ width: 100 }}>
                          P50
                        </th>
                        <th className="num" style={{ width: 100 }}>
                          P95
                        </th>
                      </tr>
                    </thead>
                    <tbody>
                      {d.deployments.map((v, i) => {
                        // Against the version before it: is this deployment worse?
                        const prev = d.deployments[i + 1];
                        const worse = (now: number, before: number, floor: number) => !!prev && now > floor && now > before * 1.5;
                        return (
                          <tr key={v.version} onClick={() => navigate(tracesLink(`version:"${v.version.replace(/"/g, '\\"')}"`))}>
                            <td className="mono">
                              {v.version}
                            </td>
                            <td className="muted" title={new Date(v.first_seen).toLocaleString()}>
                              {ago(v.first_seen)}
                            </td>
                            <td className="muted" title={new Date(v.last_seen).toLocaleString()}>
                              {ago(v.last_seen)}
                            </td>
                            <td className="num">{compact(v.hits)}</td>
                            <td className="num" style={{ color: worse(v.error_rate, prev?.error_rate ?? 0, 0.01) ? "var(--red)" : undefined }}>
                              {percent(v.error_rate)}
                            </td>
                            <td className="num mono">{duration(v.p50)}</td>
                            <td className="num mono" style={{ color: worse(v.p95, prev?.p95 ?? 0, 0) ? "var(--red)" : undefined }}>
                              {duration(v.p95)}
                            </td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                </>
              )}

              <div className="section-title">Resources</div>
              <table className="list" style={{ border: "1px solid var(--border)", borderRadius: 6 }}>
                <colgroup>
                  <col />
                  <col style={{ width: 110 }} />
                  <col style={{ width: 110 }} />
                  <col style={{ width: 110 }} />
                  <col style={{ width: 100 }} />
                  <col style={{ width: 100 }} />
                  <col style={{ width: 100 }} />
                  <col style={{ width: 100 }} />
                </colgroup>
                <thead>
                  <tr>
                    <th>Resource</th>
                    <th className="num">Requests</th>
                    <th className="num">Req/s</th>
                    <th className="num">Total Time</th>
                    <th className="num">P50</th>
                    <th className="num">P95</th>
                    <th className="num">P99</th>
                    <th className="num">Errors</th>
                  </tr>
                </thead>
                <tbody>
                  {d.resources.map((r) => (
                    <tr key={`${r.operation}/${r.resource}`} onClick={() => navigate(tracesLink(`resource_name:"${r.resource.replace(/"/g, '\\"')}"`))}>
                      <td title={r.resource}>
                        {r.resource} <span className="faint mono">{r.operation}</span>
                      </td>
                      <td className="num">{compact(r.hits)}</td>
                      <td className="num">{(r.hits / seconds).toFixed(2)}</td>
                      <td className="num">{percent(r.total_time / Math.max(1, d.summary.total_time))}</td>
                      <td className="num mono">{duration(r.p50)}</td>
                      <td className="num mono">{duration(r.p95)}</td>
                      <td className="num mono">{duration(r.p99)}</td>
                      <td className="num" style={{ color: r.errors ? "var(--red)" : undefined }}>
                        {compact(r.errors)} <span className="faint">({percent(r.error_rate)})</span>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </>
        )}
        {detail.isLoading && <div className="loading">Loading…</div>}
      </div>
    </>
  );
}

function Distribution({ data, p50, p95, p99 }: { data: [number, number][]; p50: number; p95: number; p99: number }) {
  const [ref, width] = useWidth<HTMLDivElement>();
  if (!data.length) return <div ref={ref} className="faint">No requests.</div>;
  const height = 150;
  const max = Math.max(1, ...data.map((d) => d[1]));
  const lo = Math.log10(data[0][0]) - (Math.log10(data[1]?.[0] ?? data[0][0] * 10) - Math.log10(data[0][0]));
  const hi = Math.log10(data[data.length - 1][0]);
  const x = (ns: number) => ((Math.log10(Math.max(1, ns)) - lo) / (hi - lo)) * width;
  const bw = width / data.length;
  return (
    <div ref={ref} className="chart">
      <svg width={width} height={height + 18}>
        {data.map(([, c], i) => {
          const h = (c / max) * height;
          return <rect key={i} x={i * bw + 0.5} y={height - h} width={Math.max(1, bw - 1)} height={h} fill="#8b6fd0" />;
        })}
        {[
          ["p50", p50, "#5d8ae6"],
          ["p95", p95, "#9b6fd6"],
          ["p99", p99, "#e3a63c"],
        ].map(([label, v, color], i) => (
          <g key={label as string}>
            <line x1={x(v as number)} x2={x(v as number)} y1={0} y2={height} stroke={color as string} strokeDasharray="3 3" />
            <text x={x(v as number) + 3} y={11 + i * 13} fontSize={10.5} fill={color as string}>
              {label as string} {duration(v as number)}
            </text>
          </g>
        ))}
        <g className="chart-axis">
          {[0, 0.25, 0.5, 0.75, 1].map((f) => (
            <text key={f} x={f * width} y={height + 13} textAnchor={f === 0 ? "start" : f === 1 ? "end" : "middle"}>
              {duration(Math.pow(10, lo + (hi - lo) * f))}
            </text>
          ))}
        </g>
      </svg>
    </div>
  );
}
