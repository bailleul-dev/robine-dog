import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { TimeChart, type Series } from "../components/Charts";
import { SidePanel } from "../components/Common";
import { PageHeader } from "../components/Layout";
import { api, type HostInfo } from "../lib/api";
import { formatValue } from "../lib/metrics";
import { lines } from "./Metrics";

/**
 * The infrastructure lists (hosts, containers, Kubernetes resources). Each
 * is a table with a row per group of tags, its columns being the latest
 * value of a metric query grouped by those tags: what the Agent reports is
 * all there is to know, as in Datadog.
 */
type Column = {
  label: string;
  /** A metric query without its `by`: `avg:system.load.1{*}`. */
  query: string;
  unit?: string;
  /** From the metric's value to the displayed one (idle → used…). */
  map?: (v: number) => number;
  /** Values that deserve attention. */
  bad?: (v: number) => boolean;
};
type Chart = { title: string; query: (filter: string) => string; unit?: string; map?: (v: number) => number };
type View = {
  id: string;
  /** Tags identifying a row; the last one is its name. */
  by: string[];
  /** A query whose extra tags describe the row (image, phase, node…). */
  info?: { query: string; tags: string[]; bad?: Record<string, (v: string) => boolean> };
  columns: Column[];
  charts: Chart[];
  /** The log query of a row. */
  logs?: (row: Record<string, string>) => string;
};

const pct = (v: number) => v * 100;
const cores = (v: number) => v / 1e9;
const LABELS: Record<string, string> = {
  host: "Host",
  kube_namespace: "Namespace",
  pod_name: "Pod",
  kube_container_name: "Container",
  short_image: "Image",
  image_tag: "Tag",
  pod_phase: "Phase",
  node: "Node",
  kube_deployment: "Deployment",
  kube_stateful_set: "StatefulSet",
  kube_daemon_set: "DaemonSet",
  kubelet_version: "Kubelet",
  os_image: "OS",
};

const HOSTS: View = {
  id: "hosts",
  by: ["host"],
  columns: [
    { label: "CPU", query: "avg:system.cpu.idle{*}", unit: "%", map: (v) => 100 - v, bad: (v) => v > 90 },
    { label: "Load 1", query: "avg:system.load.1{*}" },
    { label: "Memory", query: "avg:system.mem.pct_usable{*}", unit: "%", map: (v) => (1 - v) * 100, bad: (v) => v > 90 },
    { label: "Disk (fullest)", query: "max:system.disk.in_use{*}", unit: "%", map: pct, bad: (v) => v > 85 },
    { label: "I/O wait", query: "avg:system.cpu.iowait{*}", unit: "%" },
    { label: "Uptime", query: "avg:system.uptime{*}", unit: "second" },
  ],
  charts: [
    { title: "CPU usage", query: (f) => `avg:system.cpu.idle{${f}}`, unit: "%", map: (v) => 100 - v },
    { title: "Load", query: (f) => `avg:system.load.1{${f}}` },
    { title: "Memory used", query: (f) => `avg:system.mem.used{${f}}`, unit: "byte" },
    { title: "Disk in use", query: (f) => `max:system.disk.in_use{${f}} by {device_name}`, unit: "%", map: pct },
    { title: "Network received", query: (f) => `sum:system.net.bytes_rcvd{${f}}`, unit: "byte" },
    { title: "Network sent", query: (f) => `sum:system.net.bytes_sent{${f}}`, unit: "byte" },
  ],
  logs: (r) => `host:${r.host}`,
};

const CONTAINERS: View = {
  id: "containers",
  by: ["kube_namespace", "pod_name", "kube_container_name"],
  info: { query: "avg:container.uptime{*}", tags: ["short_image", "image_tag"] },
  columns: [
    { label: "CPU", query: "avg:container.cpu.usage{*}", unit: "core", map: cores },
    { label: "Memory", query: "avg:container.memory.usage{*}", unit: "byte" },
    { label: "Memory limit", query: "avg:container.memory.limit{*}", unit: "byte" },
    { label: "Restarts", query: "max:kubernetes.containers.restarts{*}", bad: (v) => v > 5 },
    { label: "Threads", query: "avg:container.pid.thread_count{*}" },
    { label: "Uptime", query: "avg:container.uptime{*}", unit: "second" },
  ],
  charts: [
    { title: "CPU", query: (f) => `avg:container.cpu.usage{${f}}`, unit: "core", map: cores },
    { title: "CPU throttled", query: (f) => `avg:container.cpu.throttled{${f}}`, unit: "core", map: cores },
    { title: "Memory", query: (f) => `avg:container.memory.usage{${f}}`, unit: "byte" },
    { title: "Network received", query: (f) => `sum:container.net.rcvd{${f}}`, unit: "byte" },
    { title: "Network sent", query: (f) => `sum:container.net.sent{${f}}`, unit: "byte" },
    { title: "Disk written", query: (f) => `sum:container.io.write{${f}}`, unit: "byte" },
  ],
  logs: (r) => `pod_name:${r.pod_name} container_name:${r.kube_container_name}`,
};

const podCharts: Chart[] = [
  { title: "CPU", query: (f) => `sum:kubernetes.cpu.usage.total{${f}}`, unit: "core", map: cores },
  { title: "Memory", query: (f) => `sum:kubernetes.memory.usage{${f}}`, unit: "byte" },
  { title: "Network received", query: (f) => `sum:kubernetes.network.rx_bytes{${f}}`, unit: "byte" },
  { title: "Network sent", query: (f) => `sum:kubernetes.network.tx_bytes{${f}}`, unit: "byte" },
  { title: "Restarts", query: (f) => `sum:kubernetes.containers.restarts{${f}}` },
];

const KUBERNETES: View[] = [
  {
    id: "pods",
    by: ["kube_namespace", "pod_name"],
    info: {
      query: "sum:kubernetes_state.pod.status_phase{*}",
      tags: ["pod_phase", "node"],
      bad: { pod_phase: (v) => !["Running", "Succeeded"].includes(v) },
    },
    columns: [
      { label: "Ready", query: "sum:kubernetes_state.pod.ready{condition:true}" },
      { label: "Restarts", query: "sum:kubernetes_state.container.restarts{*}", bad: (v) => v > 5 },
      { label: "CPU", query: "sum:kubernetes.cpu.usage.total{*}", unit: "core", map: cores },
      { label: "Memory", query: "sum:kubernetes.memory.usage{*}", unit: "byte" },
      { label: "Age", query: "avg:kubernetes_state.pod.age{*}", unit: "second" },
    ],
    charts: podCharts,
    logs: (r) => `pod_name:${r.pod_name}`,
  },
  {
    id: "deployments",
    by: ["kube_namespace", "kube_deployment"],
    columns: [
      { label: "Desired", query: "sum:kubernetes_state.deployment.replicas_desired{*}" },
      { label: "Available", query: "sum:kubernetes_state.deployment.replicas_available{*}" },
      { label: "Unavailable", query: "sum:kubernetes_state.deployment.replicas_unavailable{*}", bad: (v) => v > 0 },
      { label: "Updated", query: "sum:kubernetes_state.deployment.replicas_updated{*}" },
      { label: "CPU", query: "sum:kubernetes.cpu.usage.total{*}", unit: "core", map: cores },
      { label: "Memory", query: "sum:kubernetes.memory.usage{*}", unit: "byte" },
    ],
    charts: [
      { title: "Replicas available", query: (f) => `sum:kubernetes_state.deployment.replicas_available{${f}}` },
      ...podCharts,
    ],
    logs: (r) => `kube_deployment:${r.kube_deployment} kube_namespace:${r.kube_namespace}`,
  },
  {
    id: "statefulsets",
    by: ["kube_namespace", "kube_stateful_set"],
    columns: [
      { label: "Desired", query: "sum:kubernetes_state.statefulset.replicas_desired{*}" },
      { label: "Ready", query: "sum:kubernetes_state.statefulset.replicas_ready{*}" },
      { label: "CPU", query: "sum:kubernetes.cpu.usage.total{*}", unit: "core", map: cores },
      { label: "Memory", query: "sum:kubernetes.memory.usage{*}", unit: "byte" },
    ],
    charts: podCharts,
    logs: (r) => `kube_stateful_set:${r.kube_stateful_set} kube_namespace:${r.kube_namespace}`,
  },
  {
    id: "daemonsets",
    by: ["kube_namespace", "kube_daemon_set"],
    columns: [
      { label: "Desired", query: "sum:kubernetes_state.daemonset.desired{*}" },
      { label: "Ready", query: "sum:kubernetes_state.daemonset.ready{*}" },
      { label: "Unavailable", query: "sum:kubernetes_state.daemonset.daemons_unavailable{*}", bad: (v) => v > 0 },
    ],
    charts: podCharts,
    logs: (r) => `kube_daemon_set:${r.kube_daemon_set} kube_namespace:${r.kube_namespace}`,
  },
  {
    id: "nodes",
    by: ["node"],
    info: { query: "sum:kubernetes_state.node.count{*}", tags: ["kubelet_version", "os_image"] },
    columns: [
      { label: "Ready", query: "sum:kubernetes_state.node.by_condition{condition:ready,status:true}", bad: (v) => v < 1 },
      { label: "Pods", query: "sum:kubernetes_state.pod.count{*}" },
      { label: "CPU allocatable", query: "sum:kubernetes_state.node.cpu_allocatable{*}", unit: "core" },
      { label: "Memory allocatable", query: "sum:kubernetes_state.node.memory_allocatable{*}", unit: "byte" },
      { label: "Age", query: "avg:kubernetes_state.node.age{*}", unit: "second" },
    ],
    charts: [
      { title: "Pods", query: (f) => `sum:kubernetes_state.pod.count{${f}}` },
      { title: "Disk usage", query: (f) => `avg:kubernetes.node.filesystem.usage_pct{${f.replace("node:", "kube_node:")}}`, unit: "%", map: pct },
    ],
  },
  {
    id: "namespaces",
    by: ["kube_namespace"],
    columns: [
      { label: "Pods", query: "sum:kubernetes_state.pod.count{*}" },
      { label: "Deployments", query: "sum:kubernetes_state.deployment.count{*}" },
      { label: "Restarts", query: "sum:kubernetes_state.container.restarts{*}" },
      { label: "CPU", query: "sum:kubernetes.cpu.usage.total{*}", unit: "core", map: cores },
      { label: "Memory", query: "sum:kubernetes.memory.usage{*}", unit: "byte" },
      { label: "Volumes used", query: "sum:kubernetes.kubelet.volume.stats.used_bytes{*}", unit: "byte" },
    ],
    charts: podCharts,
    logs: (r) => `kube_namespace:${r.kube_namespace}`,
  },
];

function show(v: number | undefined, unit?: string): string {
  if (v === undefined || !Number.isFinite(v)) return "–";
  if (unit === "%") return `${v.toFixed(v >= 10 ? 0 : 1)} %`;
  if (unit === "core") return v >= 1 ? `${v.toFixed(2)} cores` : `${Math.round(v * 1000)} m`;
  if (unit === "second") {
    if (v >= 172_800) return `${Math.floor(v / 86_400)} d`;
    if (v >= 7200) return `${Math.floor(v / 3600)} h`;
    if (v >= 120) return `${Math.floor(v / 60)} min`;
    return `${Math.round(v)} s`;
  }
  return formatValue(v, unit);
}

type Row = { key: string; group: Record<string, string>; values: (number | undefined)[]; info: Record<string, string> };

/** The view's rows over the last minutes: one per group, the latest value of each column. */
function useRows(view: View) {
  return useQuery({
    queryKey: ["infra", view.id],
    refetchInterval: 30_000,
    placeholderData: keepPreviousData,
    queryFn: async ({ signal }) => {
      const to = Date.now();
      const by = ` by {${view.by.join(",")}}`;
      const queries = view.columns.map((c) => c.query + by);
      if (view.info) queries.push(`${view.info.query} by {${[...view.by, ...view.info.tags].join(",")}}`);
      const data = await api.queryMetrics(queries, to - 600_000, to, signal, 5000);
      const rows = new Map<string, Row>();
      const strength = new Map<string, number>();
      for (const s of data.series) {
        const last = s.points[s.points.length - 1]?.[1];
        const key = view.by.map((t) => s.group[t]).join("\u0000");
        // A group lacking the identifying tags is not a resource of this kind.
        if (last === undefined || view.by.some((t) => !s.group[t] || s.group[t] === "N/A")) continue;
        let row = rows.get(key);
        if (!row) {
          row = { key, group: Object.fromEntries(view.by.map((t) => [t, s.group[t]])), values: [], info: {} };
          rows.set(key, row);
        }
        if (s.query_index < view.columns.length) {
          const c = view.columns[s.query_index];
          row.values[s.query_index] = c.map ? c.map(last) : last;
        } else if (last >= (strength.get(key) ?? -Infinity)) {
          // Several phases or images over the window: the current one wins.
          strength.set(key, last);
          for (const t of view.info!.tags) row.info[t] = s.group[t] === "N/A" ? "" : s.group[t];
        }
      }
      // What only has a row because of a column that is not its own (a pod's CPU in the deployments list).
      const errors = data.results.map((r) => r.error).filter(Boolean);
      return { rows: [...rows.values()], error: errors.length === data.results.length ? errors[0] : null };
    },
  });
}

function DetailChart({ chart, filter }: { chart: Chart; filter: string }) {
  const q = useQuery({
    queryKey: ["infra-chart", chart.title, filter],
    refetchInterval: 60_000,
    queryFn: async ({ signal }) => {
      const to = Date.now();
      const from = to - 3_600_000;
      return { from, to, data: await api.queryMetrics([chart.query(filter)], from, to, signal) };
    },
  });
  const built = useMemo(() => (q.data ? lines(q.data.data, q.data.from, q.data.to) : null), [q.data]);
  return (
    <div className="card infra-chart">
      <div className="card-title">{chart.title}</div>
      {built && built.lines.length > 0 ? (
        <TimeChart
          times={built.times}
          interval={q.data!.data.interval_ms}
          from={q.data!.from}
          to={q.data!.to}
          kind="lines"
          height={120}
          series={built.lines.map((l): Series => ({
            key: l.key,
            label: l.label.replace(/^a: /, ""),
            color: l.color,
            values: chart.map ? l.values.map(chart.map) : l.values,
          }))}
          format={(v) => show(v, chart.unit)}
        />
      ) : (
        <div className="muted infra-nodata">{q.isLoading ? "Loading…" : "No data"}</div>
      )}
    </div>
  );
}

function Detail({ view, row, host, onClose }: { view: View; row: Row; host?: HostInfo; onClose: () => void }) {
  const filter = view.by.map((t) => `${t}:${row.group[t]}`).join(",");
  const name = row.group[view.by[view.by.length - 1]];
  const facts = [...Object.entries(row.group), ...Object.entries(row.info).filter(([, v]) => v)];
  return (
    <SidePanel onClose={onClose} storageKey="infra" header={<strong>{name}</strong>}>
      <div className="infra-detail">
        <div className="infra-facts">
          {facts.map(([k, v]) => (
            <span className="tag" key={k}>
              {k}:{v}
            </span>
          ))}
          {host?.agent_version && <span className="tag">agent:{host.agent_version}</span>}
          {host?.os && <span className="tag">os:{host.os}</span>}
        </div>
        <div className="toolbar" style={{ margin: "10px 0" }}>
          {view.logs && (
            <Link className="btn small" to={`/logs?query=${encodeURIComponent(view.logs(row.group))}`}>
              View logs
            </Link>
          )}
          <Link
            className="btn small"
            to={`/metrics/explorer?q=${encodeURIComponent(view.charts[0].query(filter))}`}
          >
            Open in Metrics Explorer
          </Link>
        </div>
        <div className="infra-charts">
          {view.charts.map((c) => (
            <DetailChart key={c.title} chart={c} filter={filter} />
          ))}
        </div>
        {host && host.tags.length > 0 && (
          <>
            <div className="section-title">Host tags</div>
            <div className="infra-facts">
              {host.tags.map((t) => (
                <span className="tag" key={t}>
                  {t}
                </span>
              ))}
            </div>
          </>
        )}
        {host && host.aliases.length > 0 && (
          <>
            <div className="section-title">Aliases</div>
            <div className="infra-facts">
              {host.aliases.map((t) => (
                <span className="tag" key={t}>
                  {t}
                </span>
              ))}
            </div>
          </>
        )}
      </div>
    </SidePanel>
  );
}

function ResourceTable({ view, hosts }: { view: View; hosts?: HostInfo[] }) {
  const q = useRows(view);
  const [filter, setFilter] = useState("");
  const [sort, setSort] = useState<{ col: number; desc: boolean }>({ col: -1, desc: false });
  const [selected, setSelected] = useState<string | null>(null);
  const infoTags = view.info?.tags ?? [];
  const text = [...view.by].reverse();

  const rows = useMemo(() => {
    const words = filter.toLowerCase().split(/\s+/).filter(Boolean);
    const all = (q.data?.rows ?? []).filter((r) => {
      const hay = [...Object.values(r.group), ...Object.values(r.info)].join(" ").toLowerCase();
      return words.every((w) => hay.includes(w));
    });
    const name = (r: Row) => view.by.map((t) => r.group[t]).join("/");
    return all.sort((a, b) => {
      if (sort.col < 0) return name(a).localeCompare(name(b)) * (sort.desc ? -1 : 1);
      const d = (a.values[sort.col] ?? -Infinity) - (b.values[sort.col] ?? -Infinity);
      return (sort.desc ? -d : d) || name(a).localeCompare(name(b));
    });
  }, [q.data, filter, sort, view]);
  const current = rows.find((r) => r.key === selected);
  const header = (label: string, col: number, num: boolean) => (
    <th
      key={label}
      className={`sortable${num ? " num" : ""}`}
      onClick={() => setSort({ col, desc: sort.col === col ? !sort.desc : col >= 0 })}
    >
      {label}
      {sort.col === col ? (sort.desc ? " ↓" : " ↑") : ""}
    </th>
  );

  return (
    <>
      <div className="filters-row">
        <input
          className="input"
          placeholder="Filter by name, namespace, image…"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          style={{ width: 320 }}
        />
        <div className="spacer" />
        <span className="muted">
          {rows.length} {view.id}
        </span>
      </div>
      <div className="page-scroll">
        {q.data?.error && <div className="empty-state">{q.data.error}</div>}
        {q.data && !q.data.error && rows.length === 0 && (
          <div className="empty-state">
            <h3>Nothing reported</h3>
            These come from the metrics of a Datadog Agent pointed at robine-dog.
          </div>
        )}
        {rows.length > 0 && (
          <table className="list infra-table">
            <thead>
              <tr>
                {header(LABELS[text[0]] ?? text[0], -1, false)}
                {text.slice(1).map((t) => (
                  <th key={t}>{LABELS[t] ?? t}</th>
                ))}
                {infoTags.map((t) => (
                  <th key={t}>{LABELS[t] ?? t}</th>
                ))}
                {view.columns.map((c, i) => header(c.label, i, true))}
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <tr key={r.key} className={r.key === selected ? "selected" : ""} onClick={() => setSelected(r.key)}>
                  {text.map((t, i) => (
                    <td key={t} className={i === 0 ? "infra-name" : "muted"}>
                      {r.group[t]}
                    </td>
                  ))}
                  {infoTags.map((t) => (
                    <td key={t} className={view.info?.bad?.[t]?.(r.info[t] ?? "") ? "infra-bad" : "muted"}>
                      {r.info[t] || "–"}
                    </td>
                  ))}
                  {view.columns.map((c, i) => {
                    const v = r.values[i];
                    return (
                      <td key={c.label} className={`num${v !== undefined && c.bad?.(v) ? " infra-bad" : ""}`}>
                        {show(v, c.unit)}
                      </td>
                    );
                  })}
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
      {current && (
        <Detail
          view={view}
          row={current}
          host={view.id === "hosts" ? hosts?.find((h) => h.name === current.group.host) : undefined}
          onClose={() => setSelected(null)}
        />
      )}
    </>
  );
}

const INFRA_TABS = [
  { to: "/infrastructure", label: "Hosts", end: true },
  { to: "/containers", label: "Containers" },
  { to: "/orchestration/overview/pod", label: "Kubernetes" },
];

export function Hosts() {
  const hosts = useQuery({ queryKey: ["hosts"], queryFn: api.hosts });
  return (
    <>
      <PageHeader title="Infrastructure" tabs={INFRA_TABS} />
      <ResourceTable view={HOSTS} hosts={hosts.data} />
    </>
  );
}

export function Containers() {
  return (
    <>
      <PageHeader title="Infrastructure" tabs={INFRA_TABS} />
      <ResourceTable view={CONTAINERS} />
    </>
  );
}

const KINDS: [string, string][] = [
  ["pod", "Pods"],
  ["deployment", "Deployments"],
  ["statefulset", "StatefulSets"],
  ["daemonset", "DaemonSets"],
  ["node", "Nodes"],
  ["namespace", "Namespaces"],
];

export function Kubernetes({ kind }: { kind: string }) {
  const view = KUBERNETES.find((v) => v.id === `${kind}s`) ?? KUBERNETES[0];
  return (
    <>
      <PageHeader title="Infrastructure" tabs={INFRA_TABS} />
      <div className="infra-kinds">
        {KINDS.map(([k, label]) => (
          <Link key={k} to={`/orchestration/overview/${k}`} className={k === kind ? "active" : ""}>
            {label}
          </Link>
        ))}
      </div>
      <ResourceTable key={view.id} view={view} />
    </>
  );
}
