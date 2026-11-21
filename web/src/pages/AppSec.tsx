import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { Link } from "react-router-dom";
import { ServiceChip } from "../components/Common";
import { PageHeader } from "../components/Layout";
import { TimePicker } from "../components/TimePicker";
import { api } from "../lib/api";
import { ago, thousands } from "../lib/format";
import { useTimeRange } from "../lib/time";

type Time = ReturnType<typeof useTimeRange>;

const logsLink = (query: string, time: Time) =>
  `/logs?${new URLSearchParams({ query, from_ts: String(time.from), to_ts: String(time.to), live: "false" })}`;

/** Counts of the logs matching `query`, by one facet. */
function TopList({
  title,
  query,
  facet,
  time,
}: {
  title: string;
  query: string;
  facet: string;
  time: Time;
}) {
  const q = useQuery({
    queryKey: ["toplist", query, facet, time.from, time.to],
    queryFn: () =>
      api.aggregate("logs", {
        compute: [{ aggregation: "count" }],
        filter: { query, from: time.from, to: time.to },
        group_by: [{ facet, limit: 10 }],
      }),
    placeholderData: keepPreviousData,
  });
  const rows = (q.data?.data.buckets ?? [])
    .map((b) => ({
      v: String(b.by[facet] ?? ""),
      n: Number(b.computes.c0 ?? 0),
    }))
    .sort((a, b) => b.n - a.n);
  const clause = (v: string) =>
    `${facet}:"${v.replace(/"/g, '\\"')}"`;
  return (
    <div className="card">
      <div className="card-title">{title}</div>
      {!rows.length && <div className="muted">Nothing.</div>}
      {rows.map((r) => (
        <div key={r.v} style={{ display: "flex", gap: 8 }}>
          <Link
            className="ellipsis"
            to={logsLink(`${query} ${clause(r.v)}`, time)}
          >
            {r.v || "(none)"}
          </Link>
          <div className="spacer" />
          <span className="mono">{thousands(r.n)}</span>
        </div>
      ))}
    </div>
  );
}

const grid = {
  display: "grid",
  gridTemplateColumns: "repeat(auto-fit, minmax(260px, 1fr))",
  gap: 12,
} as const;

/** App & API Protection: attacks detected by the tracers' WAF, and the API inventory. */
export function AppProtection() {
  const time = useTimeRange("1d", 60_000);
  const eps = useQuery({
    queryKey: ["appsec-endpoints", time.from, time.to],
    queryFn: () => api.appsecEndpoints(time.from, time.to),
    placeholderData: keepPreviousData,
  });
  const attacked = useQuery({
    queryKey: ["appsec-attacked", time.from, time.to],
    queryFn: () =>
      api.aggregate("logs", {
        compute: [{ aggregation: "count" }],
        filter: { query: "source:appsec", from: time.from, to: time.to },
        group_by: [
          { facet: "service", limit: 100 },
          { facet: "@http.route", limit: 500 },
        ],
      }),
  });
  const hits = new Map<string, number>();
  for (const b of attacked.data?.data.buckets ?? [])
    hits.set(
      `${b.by.service} ${b.by["@http.route"]}`,
      Number(b.computes.c0 ?? 0),
    );
  return (
    <>
      <PageHeader title="App & API Protection">
        <TimePicker time={time} />
      </PageHeader>
      <div className="filters-row">
        <span className="muted">
          Tracers with <code>DD_APPSEC_ENABLED=true</code>; each attack is a{" "}
          <code>source:appsec</code> log, and the{" "}
          <Link to="/security/rules">“Attacks from one address”</Link> rule
          opens signals.
        </span>
        <div className="spacer" />
        <Link className="btn small" to={logsLink("source:appsec", time)}>
          All attacks
        </Link>
      </div>
      <div className="page-scroll page-padding">
        <div style={grid}>
          <TopList
            title="Attack types"
            query="source:appsec"
            facet="@appsec.type"
            time={time}
          />
          <TopList
            title="Attackers"
            query="source:appsec"
            facet="@network.client.ip"
            time={time}
          />
          <TopList
            title="Targeted services"
            query="source:appsec"
            facet="service"
            time={time}
          />
          <TopList
            title="Rules"
            query="source:appsec"
            facet="@appsec.rule.name"
            time={time}
          />
        </div>
        <div className="section-title">API endpoints</div>
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Service</th>
                <th>Endpoint</th>
                <th className="num">Requests</th>
                <th className="num">Errors</th>
                <th className="num">Attacks</th>
                <th>Last seen</th>
              </tr>
            </thead>
            <tbody>
              {eps.data?.data.map((e) => {
                const n = hits.get(`${e.service} ${e.route}`) ?? 0;
                return (
                  <tr key={`${e.service} ${e.method} ${e.route}`}>
                    <td>
                      <ServiceChip name={e.service} />
                    </td>
                    <td className="mono">
                      {e.method} {e.route}
                    </td>
                    <td className="num">{thousands(e.requests)}</td>
                    <td className="num">{thousands(e.errors)}</td>
                    <td className="num">
                      {n ? (
                        <Link
                          to={logsLink(
                            `source:appsec service:"${e.service}" @http.route:"${e.route}"`,
                            time,
                          )}
                        >
                          {n}
                        </Link>
                      ) : (
                        ""
                      )}
                    </td>
                    <td className="muted">{ago(e.last_seen)}</td>
                  </tr>
                );
              })}
              {eps.data && !eps.data.data.length && (
                <tr>
                  <td colSpan={6} className="muted">
                    No service entry span with <code>http.route</code> in this
                    range.
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

/** Code Security (SCA): vulnerable libraries loaded by services, from OSV. */
export function CodeSecurity() {
  const q = useQuery({
    queryKey: ["code-security"],
    queryFn: api.codeSecurity,
  });
  const d = q.data;
  return (
    <>
      <PageHeader title="Code Security" />
      <div className="filters-row">
        <span className="muted">
          {d
            ? `${d.libraries} libraries loaded by ${d.services} services, ${d.checked} checked against OSV.`
            : ""}{" "}
          The tracers report their libraries with telemetry (on by default);{" "}
          {d && !d.osv && (
            <b>OSV lookups are off (ROBINE_DOG_OSV_URL is empty). </b>
          )}
        </span>
      </div>
      <div className="page-scroll page-padding">
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Service</th>
                <th>Library</th>
                <th>Vulnerability</th>
                <th>Severity</th>
                <th>Fixed in</th>
              </tr>
            </thead>
            <tbody>
              {d?.data.map((v) => (
                <tr key={`${v.service} ${v.env} ${v.library} ${v.id}`}>
                  <td>
                    <ServiceChip name={v.service} />{" "}
                    <span className="faint">{v.env}</span>
                  </td>
                  <td className="mono">
                    {v.library} {v.version}{" "}
                    <span className="faint">{v.language}</span>
                  </td>
                  <td>
                    <a
                      href={`https://osv.dev/vulnerability/${v.id}`}
                      target="_blank"
                      rel="noreferrer"
                    >
                      {v.id}
                    </a>{" "}
                    <span className="faint">{v.aliases}</span>
                    <div className="muted">{v.summary}</div>
                  </td>
                  <td>{v.severity ?? ""}</td>
                  <td className="mono">{v.fixed ?? ""}</td>
                </tr>
              ))}
              {d && !d.data.length && (
                <tr>
                  <td colSpan={5} className="muted">
                    No known vulnerability in the libraries seen.
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

/** Findings the Datadog security-agent sends to robine-dog's logs intake. */
function AgentFindings({
  title,
  source,
  facets,
  config,
}: {
  title: string;
  source: string;
  facets: [string, string][];
  config: string;
}) {
  const time = useTimeRange("1d", 60_000);
  const query = `source:${source}`;
  return (
    <>
      <PageHeader title={title}>
        <TimePicker time={time} />
      </PageHeader>
      <div className="filters-row">
        <span className="muted">
          security-agent with <code>{config}</code> pointing to robine-dog;
          stored as <code>{query}</code> logs, usable in detection rules.
        </span>
        <div className="spacer" />
        <Link className="btn small" to={logsLink(query, time)}>
          All events
        </Link>
      </div>
      <div className="page-scroll page-padding">
        <div style={grid}>
          {facets.map(([t, f]) => (
            <TopList key={f} title={t} query={query} facet={f} time={time} />
          ))}
        </div>
      </div>
    </>
  );
}

export const WorkloadProtection = () => (
  <AgentFindings
    title="Workload Protection"
    source="runtime-security-agent"
    config="runtime_security_config.endpoints.logs_dd_url"
    facets={[
      ["Rules", "@agent.rule_id"],
      ["Hosts", "host"],
      ["Processes", "@process.file.path"],
      ["Containers", "@container.id"],
    ]}
  />
);

export const CloudSecurity = () => (
  <AgentFindings
    title="Cloud Security"
    source="compliance-agent"
    config="compliance_config.endpoints.logs_dd_url"
    facets={[
      ["Results", "@result"],
      ["Failing rules", "@agent_rule_id"],
      ["Frameworks", "@agent_framework_id"],
      ["Resources", "@resource_type"],
    ]}
  />
);
