import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { ServiceChip } from "../components/Common";
import { PageHeader } from "../components/Layout";
import { TimePicker } from "../components/TimePicker";
import { api, Deployment, DoraMetrics } from "../lib/api";
import { ago, percent } from "../lib/format";
import { useTimeRange } from "../lib/time";
import { CI_TABS } from "./CiVisibility";

/** Milliseconds as minutes, hours or days. */
const span = (ms: number | null | undefined) => {
  if (ms == null) return "—";
  const m = ms / 60_000;
  if (m < 90) return `${Math.round(m)} min`;
  if (m < 48 * 60) return `${(m / 60).toFixed(1)} h`;
  return `${(m / 1440).toFixed(1)} d`;
};

function Filters({
  service,
  env,
  setService,
  setEnv,
}: {
  service: string;
  env: string;
  setService: (s: string) => void;
  setEnv: (s: string) => void;
}) {
  return (
    <>
      <input
        className="input"
        placeholder="Service"
        defaultValue={service}
        onBlur={(e) => setService(e.target.value.trim())}
      />
      <input
        className="input"
        placeholder="Env"
        defaultValue={env}
        onBlur={(e) => setEnv(e.target.value.trim())}
      />
    </>
  );
}

/** CD Visibility: every deployment, from the DORA API, datadog-ci or webhooks. */
export function Deployments() {
  const time = useTimeRange("1w", 60_000);
  const [service, setService] = useState("");
  const [env, setEnv] = useState("");
  const q = useQuery({
    queryKey: ["deployments", time.from, time.to, service, env],
    queryFn: () =>
      api.deployments({ from: time.from, to: time.to, service, env }),
    placeholderData: keepPreviousData,
  });
  const rows = q.data?.data ?? [];
  return (
    <>
      <PageHeader title="CD Visibility" tabs={CI_TABS}>
        <TimePicker time={time} />
      </PageHeader>
      <div className="filters-row">
        <Filters
          service={service}
          env={env}
          setService={setService}
          setEnv={setEnv}
        />
        <div className="spacer" />
        <span className="muted">
          <code>datadog-ci deployment mark</code>,{" "}
          <code>/api/v2/dora/deployment</code>, or GitHub{" "}
          <code>deployment_status</code> / GitLab Deployment webhooks to{" "}
          <code>/api/v2/webhook</code>
        </span>
      </div>
      <div className="page-scroll page-padding">
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Service</th>
                <th>Env</th>
                <th>Version</th>
                <th>Commit</th>
                <th>Status</th>
                <th className="num">Duration</th>
                <th>Source</th>
                <th>Finished</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((d: Deployment) => (
                <tr
                  key={d.id}
                  onClick={() => d.url && window.open(d.url, "_blank")}
                >
                  <td>
                    <ServiceChip name={d.service} />
                  </td>
                  <td>{d.env}</td>
                  <td className="mono">{d.version ?? ""}</td>
                  <td className="mono">{d.commit_sha?.slice(0, 8) ?? ""}</td>
                  <td>
                    <span
                      className={`status-pill ${d.status === "success" ? "ok" : "error"}`}
                    >
                      {d.status}
                    </span>
                    {d.rollback && <span className="tag">rollback</span>}
                  </td>
                  <td className="num mono">
                    {span(d.finished_ms - d.started_ms)}
                  </td>
                  <td className="muted">{d.source}</td>
                  <td className="muted">{ago(d.finished_ms)}</td>
                </tr>
              ))}
              {!rows.length && (
                <tr>
                  <td colSpan={8} className="muted">
                    No deployment in this range.
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

function Tiles({ m }: { m: DoraMetrics }) {
  const tiles: [string, string, string][] = [
    [
      "Deployment frequency",
      `${m.deployment_frequency_per_day.toFixed(2)} / day`,
      `${m.deployments} deployments`,
    ],
    [
      "Lead time for changes",
      span(m.lead_time_ms),
      "median, first CI pipeline of the commit → deployed",
    ],
    [
      "Change failure rate",
      percent(m.change_failure_rate),
      `${m.failed_deployments} failed, rolled back or followed by an incident`,
    ],
    [
      "Time to restore",
      span(m.time_to_restore_ms),
      `median over ${m.failures} incidents`,
    ],
  ];
  return (
    <div
      className="stat-row"
      style={{
        display: "grid",
        gridTemplateColumns: "repeat(auto-fit, minmax(200px, 1fr))",
        gap: 12,
      }}
    >
      {tiles.map(([t, v, h]) => (
        <div className="card" key={t}>
          <div className="muted">{t}</div>
          <div style={{ fontSize: 26, fontWeight: 600, margin: "4px 0" }}>
            {v}
          </div>
          <div className="faint" style={{ fontSize: 12 }}>
            {h}
          </div>
        </div>
      ))}
    </div>
  );
}

/** DORA Metrics: the four keys, overall and per service. */
export function Dora() {
  const time = useTimeRange("30d", 60_000);
  const [service, setService] = useState("");
  const [env, setEnv] = useState("");
  const q = useQuery({
    queryKey: ["dora", time.from, time.to, service, env],
    queryFn: () => api.dora({ from: time.from, to: time.to, service, env }),
    placeholderData: keepPreviousData,
  });
  const d = q.data;
  return (
    <>
      <PageHeader title="DORA Metrics" tabs={CI_TABS}>
        <TimePicker time={time} />
      </PageHeader>
      <div className="filters-row">
        <Filters
          service={service}
          env={env}
          setService={setService}
          setEnv={setEnv}
        />
        <div className="spacer" />
        <span className="muted">
          Failures: <code>/api/v2/dora/incident</code> and robine-dog incidents
          on the service
        </span>
      </div>
      <div className="page-scroll page-padding">
        {d && <Tiles m={d.overall} />}
        <div className="section-title">By service</div>
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Service</th>
                <th>Team</th>
                <th className="num">Deployments / day</th>
                <th className="num">Lead time</th>
                <th className="num">Change failure rate</th>
                <th className="num">Time to restore</th>
              </tr>
            </thead>
            <tbody>
              {d?.services.map((s) => (
                <tr key={s.service}>
                  <td>
                    <ServiceChip name={s.service ?? null} />
                  </td>
                  <td>{s.team ?? ""}</td>
                  <td className="num">
                    {s.deployment_frequency_per_day.toFixed(2)}
                  </td>
                  <td className="num">{span(s.lead_time_ms)}</td>
                  <td className="num">{percent(s.change_failure_rate)}</td>
                  <td className="num">{span(s.time_to_restore_ms)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    </>
  );
}

/** Code Coverage: datadog-ci coverage upload, by repository and branch. */
export function Coverage() {
  const q = useQuery({ queryKey: ["coverage"], queryFn: api.coverage });
  const all = q.data?.data ?? [];
  const latest = new Map<string, (typeof all)[number]>();
  for (const r of all) {
    const k = `${r.repository} ${r.branch ?? ""} ${r.flags ?? ""}`;
    if (!latest.has(k)) latest.set(k, r);
  }
  return (
    <>
      <PageHeader title="Code Coverage" tabs={CI_TABS} />
      <div className="filters-row">
        <span className="muted">
          <code>datadog-ci coverage upload</code> (lcov, Cobertura, JaCoCo) with{" "}
          <code>DD_SITE</code> pointing here
        </span>
      </div>
      <div className="page-scroll page-padding">
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Repository</th>
                <th>Branch</th>
                <th>Flags</th>
                <th className="num">Coverage</th>
                <th>Trend</th>
                <th>Commit</th>
                <th>Uploaded</th>
              </tr>
            </thead>
            <tbody>
              {[...latest.entries()].map(([k, r]) => {
                const history = all
                  .filter(
                    (x) =>
                      x.repository === r.repository &&
                      x.branch === r.branch &&
                      x.flags === r.flags,
                  )
                  .slice(0, 10)
                  .reverse();
                return (
                  <tr key={k}>
                    <td className="mono">
                      {r.repository.replace(/^https?:\/\//, "")}
                    </td>
                    <td>{r.branch ?? ""}</td>
                    <td className="muted">{r.flags ?? ""}</td>
                    <td className="num">
                      <b>{percent(r.covered / r.total)}</b>{" "}
                      <span className="faint">
                        {r.covered}/{r.total} lines
                      </span>
                    </td>
                    <td className="mono muted">
                      {history
                        .map((h) => Math.round((h.covered / h.total) * 100))
                        .join(" → ")}
                      %
                    </td>
                    <td className="mono">{r.commit_sha?.slice(0, 8) ?? ""}</td>
                    <td className="muted">{ago(r.ts)}</td>
                  </tr>
                );
              })}
              {!latest.size && (
                <tr>
                  <td colSpan={7} className="muted">
                    No coverage report yet.
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
