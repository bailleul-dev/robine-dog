import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { TimeChart, type Series } from "../components/Charts";
import { Modal, SidePanel } from "../components/Common";
import { PlusIcon } from "../components/Icons";
import { PageHeader } from "../components/Layout";
import { api, ApiError, type SyntheticAssertion, type SyntheticTest } from "../lib/api";
import { ago } from "../lib/format";
import { lines } from "./Metrics";

const FREQUENCIES: [string, number][] = [
  ["30 seconds", 30],
  ["1 minute", 60],
  ["5 minutes", 300],
  ["15 minutes", 900],
  ["1 hour", 3600],
];

const blank = (): SyntheticTest => ({
  name: "",
  status: "live",
  config: { request: { method: "GET", url: "", headers: {}, timeout: 10 }, assertions: [{ type: "statusCode", operator: "is", target: 200 }] },
  options: { tick_every: 60 },
  message: "",
  tags: [],
});

const target = (t: SyntheticTest, type: string, operator?: string) =>
  t.config.assertions.find((a) => a.type === type && (!operator || a.operator === operator))?.target;

function TestModal({ test, onClose }: { test: SyntheticTest; onClose: () => void }) {
  const qc = useQueryClient();
  const r = test.config.request;
  const [name, setName] = useState(test.name);
  const [method, setMethod] = useState(r.method || "GET");
  const [url, setUrl] = useState(r.url);
  const [headers, setHeaders] = useState(
    Object.entries(r.headers ?? {})
      .map(([k, v]) => `${k}: ${v}`)
      .join("\n"),
  );
  const [body, setBody] = useState(r.body ?? "");
  const [status, setStatus] = useState(String(target(test, "statusCode") ?? ""));
  const [maxMs, setMaxMs] = useState(String(target(test, "responseTime") ?? ""));
  const [contains, setContains] = useState(String(target(test, "body", "contains") ?? ""));
  const [every, setEvery] = useState(test.options.tick_every ?? 60);
  const [tags, setTags] = useState(test.tags.join(", "));
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const save = async () => {
    // What the form does not edit (header assertions set through the API…) is kept.
    const assertions: SyntheticAssertion[] = test.config.assertions.filter(
      (a) => !(a.type === "statusCode" || a.type === "responseTime" || (a.type === "body" && a.operator === "contains")),
    );
    if (status.trim()) assertions.push({ type: "statusCode", operator: "is", target: Number(status) });
    if (maxMs.trim()) assertions.push({ type: "responseTime", operator: "lessThan", target: Number(maxMs) });
    if (contains) assertions.push({ type: "body", operator: "contains", target: contains });
    const next: SyntheticTest = {
      ...test,
      name,
      config: {
        request: {
          ...r,
          method,
          url,
          body: body || undefined,
          headers: Object.fromEntries(
            headers
              .split("\n")
              .map((l) => l.split(/:(.*)/s))
              .filter((p) => p[0].trim() && p[1] !== undefined)
              .map((p) => [p[0].trim(), p[1].trim()]),
          ),
        },
        assertions,
      },
      options: { ...test.options, tick_every: every },
      tags: tags
        .split(",")
        .map((t) => t.trim())
        .filter(Boolean),
    };
    setSaving(true);
    try {
      if (test.public_id) await api.updateSynthetic(test.public_id, next);
      else await api.createSynthetic(next);
      await qc.invalidateQueries({ queryKey: ["synthetics"] });
      onClose();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : String(e));
      setSaving(false);
    }
  };

  return (
    <Modal title={test.public_id ? "Edit API Test" : "New API Test"} onClose={onClose} width={620}>
      <div className="modal-body form">
        <label className="form-field">
          <span className="form-label">Name</span>
          <input className="input" autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="Checkout API is up" />
        </label>
        <div className="form-field">
          <span className="form-label">Request</span>
          <div style={{ display: "flex", gap: 8 }}>
            <select className="input" value={method} onChange={(e) => setMethod(e.target.value)} style={{ width: 110 }} aria-label="Method">
              {["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"].map((m) => (
                <option key={m}>{m}</option>
              ))}
            </select>
            <input className="input mono" style={{ flex: 1 }} value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://example.com/health" aria-label="URL" />
          </div>
        </div>
        <label className="form-field">
          <span className="form-label">Headers</span>
          <textarea className="input mono" rows={2} value={headers} onChange={(e) => setHeaders(e.target.value)} placeholder="Authorization: Bearer …" />
        </label>
        {!["GET", "HEAD"].includes(method) && (
          <label className="form-field">
            <span className="form-label">Body</span>
            <textarea className="input mono" rows={3} value={body} onChange={(e) => setBody(e.target.value)} />
          </label>
        )}
        <div className="form-field">
          <span className="form-label">Assertions</span>
          <div className="synth-assertions">
            <label>
              Status code is
              <input className="input" value={status} onChange={(e) => setStatus(e.target.value)} placeholder="any" />
            </label>
            <label>
              Response time under (ms)
              <input className="input" value={maxMs} onChange={(e) => setMaxMs(e.target.value)} placeholder="any" />
            </label>
            <label>
              Body contains
              <input className="input" value={contains} onChange={(e) => setContains(e.target.value)} placeholder="anything" />
            </label>
          </div>
        </div>
        <div style={{ display: "flex", gap: 12 }}>
          <label className="form-field" style={{ width: 180 }}>
            <span className="form-label">Run every</span>
            <select className="input" value={every} onChange={(e) => setEvery(Number(e.target.value))}>
              {FREQUENCIES.map(([label, s]) => (
                <option key={s} value={s}>
                  {label}
                </option>
              ))}
            </select>
          </label>
          <label className="form-field" style={{ flex: 1 }}>
            <span className="form-label">Tags</span>
            <input className="input mono" value={tags} onChange={(e) => setTags(e.target.value)} placeholder="env:prod, team:shop" />
          </label>
        </div>
        {error && <div className="form-error">{error}</div>}
      </div>
      <div className="modal-footer">
        <button className="btn" onClick={onClose}>
          Cancel
        </button>
        <button className="btn primary" onClick={save} disabled={saving}>
          {test.public_id ? "Save" : "Create & run"}
        </button>
      </div>
    </Modal>
  );
}

function Graph({ title, query, unit }: { title: string; query: string; unit: string }) {
  const q = useQuery({
    queryKey: ["synth-graph", query],
    refetchInterval: 60_000,
    queryFn: async ({ signal }) => {
      const to = Date.now();
      const from = to - 4 * 3_600_000;
      return { from, to, data: await api.queryMetrics([query], from, to, signal) };
    },
  });
  const built = useMemo(() => (q.data ? lines(q.data.data, q.data.from, q.data.to) : null), [q.data]);
  return (
    <div className="card">
      <div className="card-title">
        {title} <span className="muted">past 4 hours</span>
      </div>
      {built && built.lines.length > 0 ? (
        <TimeChart
          times={built.times}
          interval={q.data!.data.interval_ms}
          from={q.data!.from}
          to={q.data!.to}
          kind={unit ? "lines" : "bars"}
          height={130}
          series={built.lines.map((l): Series => {
            const label = l.label.replace(/^a: /, "").replace("status:", "");
            const color = label === "failed" ? "var(--red)" : label === "passed" ? "var(--green)" : "#8b6fd0";
            return { key: l.key, label, color, values: unit ? l.values : l.values.map((v) => (Number.isNaN(v) ? 0 : v)) };
          })}
          format={(v) => (unit ? `${Math.round(v)} ${unit}` : String(Math.round(v)))}
        />
      ) : (
        <div className="muted infra-nodata">{q.isLoading ? "Loading…" : "No data yet"}</div>
      )}
    </div>
  );
}

function Outcome({ t }: { t: SyntheticTest }) {
  if (t.status === "paused") return <span className="synth-state paused">Paused</span>;
  if (!t.last_result) return <span className="synth-state paused">No data</span>;
  return t.last_result.passed ? <span className="synth-state ok">OK</span> : <span className="synth-state alert">Alert</span>;
}

function Detail({ t, onClose, onEdit }: { t: SyntheticTest; onClose: () => void; onEdit: () => void }) {
  const qc = useQueryClient();
  const [busy, setBusy] = useState(false);
  const id = t.public_id!;
  const refresh = () => qc.invalidateQueries({ queryKey: ["synthetics"] });
  const act = async (f: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await f();
      await refresh();
    } finally {
      setBusy(false);
    }
  };
  const res = t.last_result;
  return (
    <SidePanel
      onClose={onClose}
      storageKey="synthetics"
      header={
        <>
          <Outcome t={t} /> <strong style={{ marginLeft: 8 }}>{t.name}</strong>
        </>
      }
    >
      <div className="infra-detail">
        <div className="mono" style={{ marginBottom: 10 }}>
          {t.config.request.method} {t.config.request.url}
        </div>
        <div className="toolbar" style={{ marginBottom: 12 }}>
          <button className="btn small primary" disabled={busy} onClick={() => act(() => api.triggerSynthetic(id))}>
            Run now
          </button>
          <button className="btn small" onClick={onEdit}>
            Edit
          </button>
          <button
            className="btn small"
            disabled={busy}
            onClick={() => act(() => api.updateSynthetic(id, { ...t, status: t.status === "paused" ? "live" : "paused" }))}
          >
            {t.status === "paused" ? "Resume" : "Pause"}
          </button>
          <Link className="btn small" to={`/logs?query=${encodeURIComponent(`source:synthetics @synthetics.check_id:${id}`)}`}>
            View results in Logs
          </Link>
          <div className="spacer" />
          <button
            className="btn small ghost"
            disabled={busy}
            onClick={() =>
              act(async () => {
                await api.deleteSynthetic(id);
                onClose();
              })
            }
          >
            Delete
          </button>
        </div>
        {res && (
          <div className={`synth-last ${res.passed ? "ok" : "alert"}`}>
            <strong>{res.passed ? "Passed" : "Failed"}</strong> {ago(res.ts)}
            {res.status_code ? ` · HTTP ${res.status_code}` : ""} · {Math.round(res.duration_ms)} ms
            {res.failure && <div className="mono">{res.failure}</div>}
          </div>
        )}
        <div className="infra-charts" style={{ gridTemplateColumns: "1fr" }}>
          <Graph title="Response time" query={`avg:synthetics.http.response.time{check_id:${id}}`} unit="ms" />
          <Graph title="Runs" query={`sum:synthetics.test_runs{check_id:${id}} by {status}.as_count()`} unit="" />
        </div>
        <div className="section-title">Assertions</div>
        <ul className="synth-list">
          {t.config.assertions.map((a, i) => (
            <li key={i} className="mono">
              {a.type}
              {a.property ? ` ${a.property}` : ""} {a.operator} {String(a.target)}
            </li>
          ))}
          {t.config.assertions.length === 0 && <li className="muted">None: any response passes.</li>}
        </ul>
        <div className="section-title">Alerting</div>
        <p className="muted">
          Create a metric monitor on{" "}
          <code>
            sum(last_5m):sum:synthetics.test_runs{"{"}check_id:{id},status:failed{"}"}.as_count() &gt; 0
          </code>{" "}
          to be notified when this test fails.
        </p>
      </div>
    </SidePanel>
  );
}

export function Synthetics() {
  const list = useQuery({ queryKey: ["synthetics"], queryFn: api.synthetics, refetchInterval: 15_000 });
  const [editing, setEditing] = useState<SyntheticTest | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const tests = list.data?.tests ?? [];
  const current = tests.find((t) => t.public_id === selected);
  return (
    <>
      <PageHeader title="Synthetic Monitoring">
        <button className="btn primary" onClick={() => setEditing(blank())}>
          <PlusIcon /> New API Test
        </button>
      </PageHeader>
      <div className="page-scroll page-padding">
        <p className="muted" style={{ maxWidth: 900, marginTop: 0 }}>
          HTTP requests sent on a schedule from robine-dog, with assertions on the response. Same test format as Datadog's
          Synthetics API (<code>/api/v1/synthetics/tests</code>).
        </p>
        <div className="card" style={{ padding: 0 }}>
          <table className="list infra-table">
            <thead>
              <tr>
                <th style={{ width: 90 }}>Status</th>
                <th style={{ width: "26%" }}>Name</th>
                <th>Request</th>
                <th className="num" style={{ width: 120 }}>
                  Response
                </th>
                <th style={{ width: 130 }}>Last run</th>
                <th style={{ width: 110 }}>Frequency</th>
              </tr>
            </thead>
            <tbody>
              {tests.map((t) => (
                <tr key={t.public_id} className={t.public_id === selected ? "selected" : ""} onClick={() => setSelected(t.public_id!)}>
                  <td>
                    <Outcome t={t} />
                  </td>
                  <td className="infra-name ellipsis">{t.name}</td>
                  <td className="mono ellipsis muted">
                    {t.config.request.method} {t.config.request.url}
                  </td>
                  <td className="num">{t.last_result ? `${Math.round(t.last_result.duration_ms)} ms` : "–"}</td>
                  <td className="muted">{t.last_result ? ago(t.last_result.ts) : "–"}</td>
                  <td className="muted">{FREQUENCIES.find(([, s]) => s === (t.options.tick_every ?? 60))?.[0] ?? `${t.options.tick_every} s`}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {list.data && tests.length === 0 && (
            <div className="empty-state">
              <h3>No test yet</h3>
              Create one to watch an endpoint: its availability and response time become metrics and logs.
            </div>
          )}
        </div>
      </div>
      {current && !editing && <Detail t={current} onClose={() => setSelected(null)} onEdit={() => setEditing(current)} />}
      {editing && <TestModal test={editing} onClose={() => setEditing(null)} />}
    </>
  );
}
