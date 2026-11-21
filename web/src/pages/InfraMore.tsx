import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { Fragment, useState } from "react";
import { Link } from "react-router-dom";
import { ServiceChip } from "../components/Common";
import { PageHeader } from "../components/Layout";
import { TimePicker } from "../components/TimePicker";
import { api } from "../lib/api";
import { ago, duration, thousands } from "../lib/format";
import { useTimeRange } from "../lib/time";

const bytes = (n: number) => {
  const u = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1024 && i < u.length - 1) {
    n /= 1024;
    i++;
  }
  return `${n.toFixed(i ? 1 : 0)} ${u[i]}`;
};

/** Resource Catalog: every resource robine-dog has seen in the last hour. */
const KINDS: [string, string, string][] = [
  [
    "Container",
    "avg:container.uptime{*} by {kube_container_name,short_image,host}",
    "/containers",
  ],
  [
    "Pod",
    "sum:kubernetes_state.pod.age{*} by {kube_namespace,pod_name}",
    "/orchestration/overview/pod",
  ],
  [
    "Deployment",
    "sum:kubernetes_state.deployment.replicas_desired{*} by {kube_namespace,kube_deployment}",
    "/orchestration/overview/deployment",
  ],
  [
    "Node",
    "sum:kubernetes_state.node.count{*} by {node}",
    "/orchestration/overview/node",
  ],
];

export function ResourceCatalog() {
  const [filter, setFilter] = useState("");
  const [to] = useState(() => Date.now());
  const hosts = useQuery({ queryKey: ["hosts"], queryFn: api.hosts });
  const devices = useQuery({
    queryKey: ["network-devices"],
    queryFn: api.networkDevices,
  });
  const m = useQuery({
    queryKey: ["resource-catalog", to],
    queryFn: () =>
      api.queryMetrics(
        KINDS.map((k) => k[1]),
        to - 3_600_000,
        to,
        undefined,
        5000,
      ),
  });
  type Row = {
    kind: string;
    name: string;
    tags: string[];
    seen: number | null;
    link: string;
  };
  const rows: Row[] = [
    ...(hosts.data ?? []).map((h) => ({
      kind: "Host",
      name: h.name,
      tags: h.tags,
      seen: null,
      link: `/infrastructure?host=${encodeURIComponent(h.name)}`,
    })),
    ...(m.data?.series ?? []).map((s) => {
      const [kind, , link] = KINDS[s.query_index];
      const g = Object.entries(s.group);
      return {
        kind,
        name:
          g
            .map(([, v]) => v)
            .filter((v) => v && v !== "N/A")
            .slice(-1)[0] ?? "",
        tags: g.map(([k, v]) => `${k}:${v}`),
        seen: s.points.length ? s.points[s.points.length - 1][0] : null,
        link,
      };
    }),
    ...(devices.data?.data ?? []).map((d) => ({
      kind: "Network device",
      name: String(d.name ?? d.ip_address),
      tags: (d.tags as string[] | undefined) ?? [],
      seen: (d.last_seen as number) ?? null,
      link: "/devices",
    })),
  ];
  const f = filter.toLowerCase();
  const shown = rows.filter(
    (r) =>
      !f || `${r.kind} ${r.name} ${r.tags.join(" ")}`.toLowerCase().includes(f),
  );
  const counts = new Map<string, number>();
  for (const r of rows) counts.set(r.kind, (counts.get(r.kind) ?? 0) + 1);
  return (
    <>
      <PageHeader title="Resource Catalog" />
      <div className="filters-row">
        <input
          className="input"
          style={{ flex: 1 }}
          placeholder="Filter by name or tag"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
        <span className="muted">
          {[...counts].map(([k, n]) => `${n} ${k.toLowerCase()}s`).join(" · ")}
        </span>
      </div>
      <div className="page-scroll page-padding">
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th style={{ width: 140 }}>Type</th>
                <th>Name</th>
                <th>Tags</th>
                <th style={{ width: 120 }}>Last seen</th>
              </tr>
            </thead>
            <tbody>
              {shown.slice(0, 1000).map((r) => (
                <tr key={`${r.kind} ${r.tags.join(",")} ${r.name}`}>
                  <td className="muted">{r.kind}</td>
                  <td>
                    <Link to={r.link}>{r.name}</Link>
                  </td>
                  <td>
                    {r.tags.slice(0, 6).map((t) => (
                      <span className="tag" key={t}>
                        {t}
                      </span>
                    ))}
                  </td>
                  <td className="muted">{r.seen ? ago(r.seen) : "live"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    </>
  );
}

/** Cloud Network Monitoring: flows between hosts, from the system-probe. */
export function CloudNetwork() {
  const time = useTimeRange("1h", 60_000);
  const [filter, setFilter] = useState("");
  const q = useQuery({
    queryKey: ["npm", time.from, time.to],
    queryFn: () => api.npm(time.from, time.to),
    placeholderData: keepPreviousData,
  });
  const f = filter.toLowerCase();
  const rows = (q.data?.data ?? []).filter(
    (r) => !f || `${r.source} ${r.dest} ${r.port}`.toLowerCase().includes(f),
  );
  return (
    <>
      <PageHeader title="Cloud Network">
        <TimePicker time={time} />
      </PageHeader>
      <div className="filters-row">
        <input
          className="input"
          placeholder="Filter by host, address or port"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
        <div className="spacer" />
        <span className="muted">
          Agent with <code>network_config.enabled: true</code> (system-probe);
          flows kept 3 days
        </span>
      </div>
      <div className="page-scroll page-padding">
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Source</th>
                <th>Destination</th>
                <th>Port</th>
                <th className="num">Sent</th>
                <th className="num">Received</th>
                <th className="num">Retransmits</th>
                <th className="num">RTT</th>
                <th className="num">Connections</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <tr
                  key={`${r.source} ${r.dest} ${r.port} ${r.protocol} ${r.direction}`}
                >
                  <td>{r.direction === "incoming" ? r.dest : r.source}</td>
                  <td>{r.direction === "incoming" ? r.source : r.dest}</td>
                  <td className="mono">
                    {r.protocol}/{r.port}
                  </td>
                  <td className="num">{bytes(r.bytes_sent)}</td>
                  <td className="num">{bytes(r.bytes_received)}</td>
                  <td className="num">
                    {r.retransmits ? thousands(r.retransmits) : ""}
                  </td>
                  <td className="num">
                    {r.rtt_us ? `${(r.rtt_us / 1000).toFixed(1)} ms` : ""}
                  </td>
                  <td className="num">{thousands(r.connections)}</td>
                </tr>
              ))}
              {q.data && !rows.length && (
                <tr>
                  <td colSpan={8} className="muted">
                    No flow in this range.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </div>
    </>
  );
}

/** Network Devices: SNMP devices and interfaces, from the Agent's NDM metadata. */
export function NetworkDevices() {
  const q = useQuery({
    queryKey: ["network-devices"],
    queryFn: api.networkDevices,
  });
  const [open, setOpen] = useState<string | null>(null);
  const status = (v: unknown) =>
    v === 1 || v === "up" ? "ok" : v === 2 || v === "down" ? "error" : "";
  return (
    <>
      <PageHeader title="Network Devices" />
      <div className="filters-row">
        <span className="muted">
          The Agent's <code>snmp</code> integration (autodiscovery or
          instances); metrics are <code>snmp.*</code>
        </span>
      </div>
      <div className="page-scroll page-padding">
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Device</th>
                <th>IP</th>
                <th>Vendor / model</th>
                <th>OS</th>
                <th>Location</th>
                <th className="num">Interfaces</th>
                <th>Status</th>
                <th>Last seen</th>
              </tr>
            </thead>
            <tbody>
              {q.data?.data.map((d) => {
                const id = String(d.id);
                const ifs = (d.interfaces as Record<string, unknown>[]) ?? [];
                return (
                  <Fragment key={id}>
                    <tr onClick={() => setOpen(open === id ? null : id)}>
                      <td>{String(d.name ?? id)}</td>
                      <td className="mono">{String(d.ip_address ?? "")}</td>
                      <td>{[d.vendor, d.model].filter(Boolean).join(" ")}</td>
                      <td>
                        {[d.os_name, d.os_version].filter(Boolean).join(" ")}
                      </td>
                      <td className="muted">{String(d.location ?? "")}</td>
                      <td className="num">{ifs.length}</td>
                      <td>
                        <span
                          className={`status-pill ${d.status === 1 ? "ok" : "error"}`}
                        >
                          {d.status === 1 ? "reachable" : "unreachable"}
                        </span>
                      </td>
                      <td className="muted">
                        {d.last_seen ? ago(d.last_seen as number) : ""}
                      </td>
                    </tr>
                    {open === id &&
                      ifs.map((i) => (
                        <tr key={`${id}:${i.index}`} className="faint">
                          <td style={{ paddingLeft: 32 }}>
                            {String(i.name ?? i.index)}
                          </td>
                          <td className="mono">
                            {String(i.mac_address ?? "")}
                          </td>
                          <td colSpan={3}>
                            {String(i.alias ?? i.description ?? "")}
                          </td>
                          <td />
                          <td>
                            <span
                              className={`status-pill ${status(i.oper_status)}`}
                            >
                              admin {String(i.admin_status ?? "?")} · oper{" "}
                              {String(i.oper_status ?? "?")}
                            </span>
                          </td>
                          <td />
                        </tr>
                      ))}
                  </Fragment>
                );
              })}
              {q.data && !q.data.data.length && (
                <tr>
                  <td colSpan={8} className="muted">
                    No device yet.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </div>
    </>
  );
}

/** Fleet Automation (read only): every Agent, its version and checks. */
export function Fleet() {
  const q = useQuery({ queryKey: ["fleet"], queryFn: api.fleet });
  const [open, setOpen] = useState<string | null>(null);
  const versions = new Map<string, number>();
  for (const a of q.data?.data ?? [])
    versions.set(
      String(a.version ?? "?"),
      (versions.get(String(a.version ?? "?")) ?? 0) + 1,
    );
  return (
    <>
      <PageHeader title="Fleet Automation" />
      <div className="filters-row">
        <span className="muted">
          {[...versions].map(([v, n]) => `${n} on ${v}`).join(" · ")}. Read
          only: remote upgrades and configuration need Remote Configuration,
          which robine-dog does not provide.
        </span>
      </div>
      <div className="page-scroll page-padding">
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Host</th>
                <th>Version</th>
                <th>OS</th>
                <th>Install method</th>
                <th>Checks</th>
                <th>Last inventory</th>
              </tr>
            </thead>
            <tbody>
              {q.data?.data.map((a) => {
                const h = String(a.hostname);
                const m = (a.agent_metadata ?? {}) as Record<string, unknown>;
                const checks = (a.checks as string[]) ?? [];
                return (
                  <Fragment key={h}>
                    <tr onClick={() => setOpen(open === h ? null : h)}>
                      <td>{h}</td>
                      <td className="mono">{String(a.version ?? "")}</td>
                      <td>{String(a.os ?? "")}</td>
                      <td className="muted">
                        {String(m.install_method_tool ?? "")}
                      </td>
                      <td>
                        {checks.slice(0, 8).map((c) => (
                          <span className="tag" key={c}>
                            {c}
                          </span>
                        ))}
                        {checks.length > 8 && (
                          <span className="faint"> +{checks.length - 8}</span>
                        )}
                      </td>
                      <td className="muted">
                        {a.last_seen
                          ? ago(a.last_seen as number)
                          : "host metadata only"}
                      </td>
                    </tr>
                    {open === h && (
                      <tr key={`${h}-detail`}>
                        <td colSpan={6}>
                          <pre
                            className="mono"
                            style={{
                              maxHeight: 400,
                              overflow: "auto",
                              fontSize: 12,
                            }}
                          >
                            {String(
                              m.full_configuration ??
                                JSON.stringify(m, null, 2),
                            )}
                          </pre>
                        </td>
                      </tr>
                    )}
                  </Fragment>
                );
              })}
            </tbody>
          </table>
        </div>
      </div>
    </>
  );
}

/** Data Jobs Monitoring: Spark applications, jobs and stages from the Java tracer. */
export function DataJobs() {
  const time = useTimeRange("1d", 60_000);
  const q = useQuery({
    queryKey: ["data-jobs", time.from, time.to],
    queryFn: () => api.dataJobs(time.from, time.to),
    placeholderData: keepPreviousData,
  });
  const d = q.data;
  return (
    <>
      <PageHeader title="Data Jobs Monitoring">
        <TimePicker time={time} />
      </PageHeader>
      <div className="filters-row">
        <span className="muted">
          Spark with the Java tracer (
          <code>-javaagent:dd-java-agent.jar -Ddd.data.jobs.enabled=true</code>)
        </span>
      </div>
      <div className="page-scroll page-padding">
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Application</th>
                <th>Level</th>
                <th className="num">Runs</th>
                <th className="num">Failed</th>
                <th className="num">Avg duration</th>
                <th className="num">Max duration</th>
                <th>Last run</th>
              </tr>
            </thead>
            <tbody>
              {d?.summary.map((s) => (
                <tr key={`${s.app} ${s.kind}`}>
                  <td>
                    <ServiceChip name={s.app} />
                  </td>
                  <td className="muted">{s.kind.replace("spark.", "")}</td>
                  <td className="num">{thousands(s.count)}</td>
                  <td className="num">{s.errors || ""}</td>
                  <td className="num">{duration(s.avg_ns)}</td>
                  <td className="num">{duration(s.max_ns)}</td>
                  <td className="muted">{ago(s.last_seen)}</td>
                </tr>
              ))}
              {d && !d.summary.length && (
                <tr>
                  <td colSpan={7} className="muted">
                    No Spark span in this range.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
        {!!d?.runs.length && (
          <>
            <div className="section-title">Application runs</div>
            <div className="card" style={{ padding: 0 }}>
              <table className="list">
                <tbody>
                  {d.runs.map((r) => (
                    <tr key={`${r.trace_id} ${r.span_id}`}>
                      <td>
                        <ServiceChip name={r.app} />
                      </td>
                      <td>{r.resource}</td>
                      <td className="num">{duration(r.duration_ns)}</td>
                      <td>
                        {r.error && (
                          <span className="status-pill error">failed</span>
                        )}
                      </td>
                      <td className="muted">{ago(r.ts)}</td>
                      <td>
                        <Link
                          to={`/apm/traces?${new URLSearchParams({ query: "", trace: r.trace_id, span: r.span_id })}`}
                        >
                          Trace
                        </Link>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </>
        )}
      </div>
    </>
  );
}
