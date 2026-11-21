import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useSearchParams } from "react-router-dom";
import { ServiceChip } from "../components/Common";
import { PageHeader } from "../components/Layout";
import { TimePicker } from "../components/TimePicker";
import { api } from "../lib/api";
import { ago, compact, duration, percent } from "../lib/format";
import { useTimeRange } from "../lib/time";

const CI_TABS = [
  { to: "/ci/pipelines", label: "Pipelines" },
  { to: "/ci/tests", label: "Tests" },
];

const ms = (v: number) => duration(v * 1_000_000);

function Status({ s }: { s: string }) {
  const cls =
    s === "success" || s === "pass"
      ? "ok"
      : s === "error" || s === "fail"
        ? "error"
        : "warn";
  return <span className={`pill ${cls}`}>{s}</span>;
}

function useParam(k: string) {
  const [params, setParams] = useSearchParams();
  const v = params.get(k) ?? "";
  const set = (x: string) => {
    const p = new URLSearchParams(params);
    if (x) p.set(k, x);
    else p.delete(k);
    setParams(p, { replace: true });
  };
  return [v, set] as const;
}

function Jobs({ provider, id }: { provider: string; id: string }) {
  const q = useQuery({
    queryKey: ["ci-jobs", provider, id],
    queryFn: () => api.ciJobs(provider, id),
  });
  if (!q.data) return <div className="faint">Loading…</div>;
  if (!q.data.length)
    return <div className="faint">No job reported for this execution.</div>;
  return (
    <table className="list">
      <tbody>
        {q.data.map((j, i) => (
          <tr key={i}>
            <td className="muted">{j.stage ?? j.level}</td>
            <td>
              {j.url ? (
                <a href={j.url} target="_blank" rel="noreferrer">
                  {j.name}
                </a>
              ) : (
                j.name
              )}
            </td>
            <td>
              <Status s={j.status} />
            </td>
            <td className="num mono">{ms(j.end - j.start)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export function CiPipelinesPage() {
  const time = useTimeRange("1d", 60_000);
  const [name, setName] = useParam("pipeline");
  const [status, setStatus] = useParam("status");
  const [branch, setBranch] = useParam("branch");
  const [open, setOpen] = useState<string | null>(null);
  const f = {
    from: time.from,
    to: time.to,
    service: name || undefined,
    status: status || undefined,
    branch: branch || undefined,
  };
  const q = useQuery({
    queryKey: ["ci-pipelines", f],
    queryFn: () => api.ciPipelines(f),
    placeholderData: keepPreviousData,
  });
  const d = q.data;
  return (
    <>
      <PageHeader title="CI Visibility" tabs={CI_TABS}>
        <TimePicker time={time} />
      </PageHeader>
      <div className="filters-row">
        <select
          className="input"
          value={name}
          onChange={(e) => setName(e.target.value)}
        >
          <option value="">All pipelines</option>
          {d?.pipelines.map((p) => (
            <option key={p.provider + p.name}>{p.name}</option>
          ))}
        </select>
        <select
          className="input"
          value={status}
          onChange={(e) => setStatus(e.target.value)}
        >
          <option value="">Any status</option>
          {["success", "error", "canceled", "skipped", "blocked"].map((s) => (
            <option key={s}>{s}</option>
          ))}
        </select>
        <input
          className="input"
          placeholder="Branch"
          defaultValue={branch}
          onBlur={(e) => setBranch(e.target.value.trim())}
        />
        <div className="spacer" />
        <span className="muted">
          GitHub / GitLab webhooks to <code>/api/v2/webhook</code>, others to{" "}
          <code>/api/v2/ci/pipeline</code>
        </span>
      </div>
      <div className="page-scroll page-padding">
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Pipeline</th>
                <th>Provider</th>
                <th className="num">Executions</th>
                <th className="num">Success rate</th>
                <th className="num">P50</th>
                <th className="num">P95</th>
                <th>Last</th>
              </tr>
            </thead>
            <tbody>
              {d?.pipelines.map((p) => (
                <tr
                  key={p.provider + p.name}
                  onClick={() => setName(p.name)}
                  style={{ cursor: "pointer" }}
                >
                  <td>{p.name}</td>
                  <td className="muted">{p.provider}</td>
                  <td className="num">{compact(p.executions)}</td>
                  <td className="num">
                    {percent(p.successes / Math.max(1, p.executions))}
                  </td>
                  <td className="num">{ms(p.p50)}</td>
                  <td className="num">{ms(p.p95)}</td>
                  <td>
                    <Status s={p.last_status} />
                  </td>
                </tr>
              ))}
              {d && !d.pipelines.length && (
                <tr>
                  <td colSpan={7} className="muted">
                    No pipeline execution in this period.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
        <div className="section-title">Executions</div>
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Finished</th>
                <th>Pipeline</th>
                <th>Status</th>
                <th>Branch</th>
                <th>Commit</th>
                <th className="num">Duration</th>
              </tr>
            </thead>
            <tbody>
              {d?.executions.flatMap((e) => {
                const key = `${e.provider}/${e.pipeline_id}`;
                return [
                  <tr
                    key={key}
                    onClick={() => setOpen(open === key ? null : key)}
                    style={{ cursor: "pointer" }}
                  >
                    <td className="muted">{ago(e.end)}</td>
                    <td>
                      {e.url ? (
                        <a href={e.url} target="_blank" rel="noreferrer">
                          {e.name}
                        </a>
                      ) : (
                        e.name
                      )}
                    </td>
                    <td>
                      <Status s={e.status} />
                    </td>
                    <td className="mono">{e.branch ?? "—"}</td>
                    <td className="mono">{e.commit?.slice(0, 8) ?? "—"}</td>
                    <td className="num mono">{ms(e.end - e.start)}</td>
                  </tr>,
                  open === key && (
                    <tr key={key + "-jobs"}>
                      <td
                        colSpan={6}
                        style={{ background: "var(--purple-soft)" }}
                      >
                        <Jobs provider={e.provider} id={e.pipeline_id} />
                      </td>
                    </tr>
                  ),
                ];
              })}
            </tbody>
          </table>
        </div>
      </div>
    </>
  );
}

export function CiTestsPage() {
  const time = useTimeRange("1d", 60_000);
  const [service, setService] = useParam("service");
  const [status, setStatus] = useParam("status");
  const [query, setQuery] = useParam("query");
  const f = {
    from: time.from,
    to: time.to,
    service: service || undefined,
    status: status || undefined,
    query: query || undefined,
  };
  const q = useQuery({
    queryKey: ["ci-tests", f],
    queryFn: () => api.ciTests(f),
    placeholderData: keepPreviousData,
  });
  const d = q.data;
  return (
    <>
      <PageHeader title="Test Optimization" tabs={CI_TABS}>
        <TimePicker time={time} />
      </PageHeader>
      <div className="filters-row">
        <select
          className="input"
          value={service}
          onChange={(e) => setService(e.target.value)}
        >
          <option value="">All test services</option>
          {d?.services.map((s) => (
            <option key={s.service}>{s.service}</option>
          ))}
        </select>
        <select
          className="input"
          value={status}
          onChange={(e) => setStatus(e.target.value)}
        >
          <option value="">Any status</option>
          <option>pass</option>
          <option>fail</option>
          <option>skip</option>
        </select>
        <input
          className="input"
          style={{ flex: 1 }}
          placeholder="Search test names and errors"
          defaultValue={query}
          onKeyDown={(e) =>
            e.key === "Enter" &&
            setQuery((e.target as HTMLInputElement).value.trim())
          }
        />
      </div>
      <div className="page-scroll page-padding">
        {d && !d.services.length && (
          <div className="card muted">
            No test run in this period. Run tests with ddtrace's test
            integrations (<code>pytest --ddtrace</code>,{" "}
            <code>DD_CIVISIBILITY_AGENTLESS_ENABLED</code>) or upload JUnit
            reports with <code>datadog-ci junit upload</code>.
          </div>
        )}
        {d && d.services.length > 0 && (
          <div className="stat-row" style={{ padding: 0, marginBottom: 12 }}>
            {d.services.map((s) => (
              <div className="stat" key={s.service}>
                <span className="label">{s.service}</span>
                <span className="value">{compact(s.tests)} tests</span>
                <span className="faint">
                  {s.failed} failed · {s.skipped} skipped · avg{" "}
                  {duration(s.avg)}
                </span>
              </div>
            ))}
          </div>
        )}
        <div className="cards">
          <div className="card">
            <div className="card-title">
              Flaky tests (passed and failed on the same commit)
            </div>
            <table className="list">
              <tbody>
                {d?.flaky.map((t) => (
                  <tr key={t.service + t.suite + t.name} title={t.error ?? ""}>
                    <td>
                      <span className="mono">{t.name}</span>
                      <div className="faint" style={{ fontSize: 11 }}>
                        {t.service} · {t.suite}
                      </div>
                    </td>
                    <td className="num">{t.commits} commits</td>
                    <td className="muted">{ago(t.last)}</td>
                  </tr>
                ))}
                {d && !d.flaky.length && (
                  <tr>
                    <td className="faint">No flaky test.</td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
          <div className="card">
            <div className="card-title">Slowest tests</div>
            <table className="list">
              <tbody>
                {d?.slowest.map((t) => (
                  <tr key={t.service + t.suite + t.name}>
                    <td>
                      <span className="mono">{t.name}</span>
                      <div className="faint" style={{ fontSize: 11 }}>
                        {t.service} · {t.suite}
                      </div>
                    </td>
                    <td className="num mono">{duration(t.avg)}</td>
                    <td className="num muted">{t.runs} runs</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
        <div className="section-title">Test runs</div>
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Date</th>
                <th>Test</th>
                <th>Status</th>
                <th>Branch</th>
                <th className="num">Duration</th>
                <th>Error</th>
              </tr>
            </thead>
            <tbody>
              {d?.runs.map((r) => (
                <tr key={r.id}>
                  <td className="muted">{ago(r.start)}</td>
                  <td>
                    <span className="mono">{r.name}</span>
                    <div className="faint" style={{ fontSize: 11 }}>
                      <ServiceChip name={r.service} /> {r.suite}
                    </div>
                  </td>
                  <td>
                    <Status s={r.status} />
                  </td>
                  <td className="mono">{r.branch ?? "—"}</td>
                  <td className="num mono">{duration(r.duration)}</td>
                  <td className="llm-cell" title={r.error ?? ""}>
                    {r.error}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    </>
  );
}
