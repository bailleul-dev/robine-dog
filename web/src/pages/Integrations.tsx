import { useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { PageHeader } from "../components/Layout";
import { api } from "../lib/api";
import { formatValue } from "../lib/metrics";

type Row = { name: string; logs: number; points: number; metrics: number; pipeline: boolean };

/** Names that are the same integration under a log source and a metric prefix. */
const ALIASES: Record<string, string> = {
  postgres: "postgresql",
  redisdb: "redis",
  kubernetes_state: "kubernetes",
  kubelet: "kubernetes",
  "datadog-agent": "agent",
  datadog: "agent",
};
const canon = (n: string) => ALIASES[n.toLowerCase()] ?? n.toLowerCase();

/**
 * What reports to robine-dog, found from the data itself: a log source or a
 * metric prefix is an integration, as with a Datadog Agent's checks.
 */
export function Integrations() {
  const [filter, setFilter] = useState("");
  const window = useMemo(() => {
    const to = Date.now();
    return { from: to - 3_600_000, to };
  }, []);
  const sources = useQuery({
    queryKey: ["integrations", "sources"],
    queryFn: () => api.facetValues("logs", { query: "", ...window }, ["source"], 200),
  });
  const volume = useQuery({
    queryKey: ["integrations", "volume"],
    queryFn: () => api.metricsVolume(window.from, window.to),
  });
  const pipelines = useQuery({ queryKey: ["pipelines"], queryFn: api.pipelines });
  const hosts = useQuery({ queryKey: ["hosts"], queryFn: api.hosts });

  const rows = useMemo(() => {
    const map = new Map<string, Row>();
    const row = (name: string) => {
      const k = canon(name);
      let r = map.get(k);
      if (!r) map.set(k, (r = { name: k, logs: 0, points: 0, metrics: 0, pipeline: false }));
      return r;
    };
    for (const v of sources.data?.facets[0]?.values ?? []) if (v.value) row(v.value).logs += v.count;
    for (const m of volume.data ?? []) {
      const r = row(m.metric.split(".")[0]);
      r.points += m.points;
      r.metrics += 1;
    }
    for (const p of pipelines.data ?? []) {
      const source = /source:([\w.-]+)/.exec(p.filter?.query ?? "")?.[1];
      if (source && map.has(canon(source))) map.get(canon(source))!.pipeline = true;
    }
    const f = filter.trim().toLowerCase();
    return [...map.values()].filter((r) => r.name.includes(f)).sort((a, b) => a.name.localeCompare(b.name));
  }, [sources.data, volume.data, pipelines.data, filter]);

  return (
    <>
      <PageHeader
        title="Integrations"
        tabs={[
          { to: "/integrations", label: "Reporting", end: true },
          { to: "/monitors/webhooks", label: "Webhooks" },
        ]}
      />
      <div className="filters-row">
        <input className="input" placeholder="Search integrations" value={filter} onChange={(e) => setFilter(e.target.value)} style={{ width: 280 }} />
        <div className="spacer" />
        <span className="muted">{rows.length} reporting over the past hour</span>
      </div>
      <div className="page-scroll page-padding">
        {(hosts.data ?? []).length > 0 && (
          <div className="card" style={{ marginBottom: 14 }}>
            <div className="card-title">Agents</div>
            {(hosts.data ?? []).map((h) => (
              <div key={h.name}>
                <Link to="/infrastructure">{h.name}</Link>{" "}
                <span className="muted">
                  Datadog Agent {h.agent_version ?? "?"}
                  {h.os ? ` · ${h.os}` : ""}
                </span>
              </div>
            ))}
          </div>
        )}
        <div className="integration-grid">
          {rows.map((r) => (
            <div className="card integration" key={r.name}>
              <div className="integration-name">{r.name}</div>
              <div className="integration-facts">
                {r.logs > 0 ? (
                  <Link to={`/logs?query=${encodeURIComponent(`source:${r.name}`)}`}>{formatValue(r.logs)} logs</Link>
                ) : (
                  <span className="muted">no logs</span>
                )}
                {r.metrics > 0 ? (
                  <Link to={`/metrics/summary`}>
                    {r.metrics} metrics · {formatValue(r.points)} points
                  </Link>
                ) : (
                  <span className="muted">no metrics</span>
                )}
                {r.pipeline && <Link to="/logs/pipelines">log pipeline</Link>}
              </div>
            </div>
          ))}
        </div>
        {sources.data && volume.data && rows.length === 0 && (
          <div className="empty-state">
            <h3>Nothing reporting</h3>
            Point a Datadog Agent, an OpenTelemetry collector or a tracer at robine-dog.
          </div>
        )}
      </div>
    </>
  );
}
