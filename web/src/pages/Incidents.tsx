import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { Link, useNavigate, useParams, useSearchParams } from "react-router-dom";
import { Modal } from "../components/Common";
import { PlusIcon, TrashIcon } from "../components/Icons";
import { PageHeader } from "../components/Layout";
import { api, ApiError, type Incident, type StatusComponent, type StatusPage } from "../lib/api";
import { ago, fullDate } from "../lib/format";

const TABS = [
  { to: "/incidents", label: "Incidents", end: true },
  { to: "/status-pages", label: "Status Page" },
];
const SEVERITIES = ["SEV-1", "SEV-2", "SEV-3", "SEV-4", "SEV-5", "UNKNOWN"];
const STATES = ["active", "stable", "resolved"];
const stateClass = (s: string) => (s === "resolved" ? "ok" : s === "stable" ? "stable" : "alert");

function duration(ms: number) {
  const m = Math.round(ms / 60_000);
  if (m < 60) return `${m} min`;
  if (m < 48 * 60) return `${Math.floor(m / 60)} h ${m % 60} min`;
  return `${Math.round(m / 1440)} days`;
}

function DeclareModal({ title: initial, onClose }: { title: string; onClose: () => void }) {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const [title, setTitle] = useState(initial);
  const [severity, setSeverity] = useState("UNKNOWN");
  const [summary, setSummary] = useState("");
  const [services, setServices] = useState("");
  const [error, setError] = useState<string | null>(null);
  const save = async () => {
    try {
      const inc = await api.declareIncident({ title, severity, summary, services: services.split(",") });
      await qc.invalidateQueries({ queryKey: ["incidents"] });
      navigate(`/incidents/${inc.id}`);
    } catch (e) {
      setError(e instanceof ApiError ? e.message : String(e));
    }
  };
  return (
    <Modal title="Declare Incident" onClose={onClose} width={520}>
      <div className="modal-body form">
        <label className="form-field">
          <span className="form-label">Title</span>
          <input className="input" autoFocus value={title} onChange={(e) => setTitle(e.target.value)} placeholder="What is happening" />
        </label>
        <label className="form-field">
          <span className="form-label">Severity</span>
          <select className="input" value={severity} onChange={(e) => setSeverity(e.target.value)}>
            {SEVERITIES.map((s) => (
              <option key={s}>{s}</option>
            ))}
          </select>
        </label>
        <label className="form-field">
          <span className="form-label">Services</span>
          <input className="input mono" value={services} onChange={(e) => setServices(e.target.value)} placeholder="shop-api, shop-web" />
        </label>
        <label className="form-field">
          <span className="form-label">What we know</span>
          <textarea className="input" rows={3} value={summary} onChange={(e) => setSummary(e.target.value)} />
        </label>
        {error && <div className="form-error">{error}</div>}
      </div>
      <div className="modal-footer">
        <button className="btn ghost" onClick={onClose}>
          Cancel
        </button>
        <button className="btn primary" onClick={save}>
          Declare Incident
        </button>
      </div>
    </Modal>
  );
}

export function IncidentList() {
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  // `?declare=<title>`: how Watchdog and monitors hand an incident over.
  const prefill = params.get("declare");
  const [declaring, setDeclaring] = useState(prefill !== null);
  const q = useQuery({ queryKey: ["incidents"], queryFn: api.incidents, refetchInterval: 60_000 });
  const rows = q.data?.incidents ?? [];
  return (
    <>
      <PageHeader title="Incidents" tabs={TABS}>
        <button className="btn primary" onClick={() => setDeclaring(true)}>
          <PlusIcon /> Declare Incident
        </button>
      </PageHeader>
      <div className="page-scroll page-padding">
        <div className="card" style={{ padding: 0 }}>
          <table className="list infra-table">
            <thead>
              <tr>
                <th style={{ width: 80 }}>ID</th>
                <th style={{ width: 100 }}>State</th>
                <th style={{ width: 90 }}>Severity</th>
                <th>Title</th>
                <th style={{ width: 160 }}>Commander</th>
                <th style={{ width: 130 }}>Declared</th>
                <th style={{ width: 120 }}>Duration</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((i) => (
                <tr key={i.id} onClick={() => navigate(`/incidents/${i.id}`)}>
                  <td className="mono muted">IR-{i.public_id}</td>
                  <td>
                    <span className={`synth-state ${stateClass(i.state)}`}>{i.state}</span>
                  </td>
                  <td className={i.severity === "SEV-1" || i.severity === "SEV-2" ? "infra-bad" : "muted"}>{i.severity}</td>
                  <td className="infra-name">
                    <Link to={`/incidents/${i.id}`}>{i.title}</Link>
                  </td>
                  <td className="muted ellipsis">{i.commander}</td>
                  <td className="muted">{ago(i.created)}</td>
                  <td className="muted">{duration((i.resolved ?? Date.now()) - i.created)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {q.data && rows.length === 0 && (
            <div className="empty-state">
              <h3>No incident</h3>
              Declare one when something is wrong: it keeps the timeline of what was seen and done, and can be shown on
              the status page.
            </div>
          )}
        </div>
      </div>
      {declaring && (
        <DeclareModal
          title={prefill ?? ""}
          onClose={() => {
            setDeclaring(false);
            if (prefill !== null) setParams({}, { replace: true });
          }}
        />
      )}
    </>
  );
}

export function IncidentPage() {
  const { id = "" } = useParams();
  const navigate = useNavigate();
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ["incident", id], queryFn: () => api.incident(id), refetchInterval: 30_000 });
  const [note, setNote] = useState("");
  const [summary, setSummary] = useState<string | null>(null);
  const inc = q.data;
  useEffect(() => setSummary(null), [id]);
  if (q.error) return <div className="empty-state">This incident does not exist.</div>;
  if (!inc) return <div className="loading">Loading…</div>;
  const set = (next: Incident) => {
    qc.setQueryData(["incident", id], next);
    void qc.invalidateQueries({ queryKey: ["incidents"] });
  };
  const patch = async (p: Partial<Incident>) => set(await api.updateIncident(id, p));
  return (
    <>
      <PageHeader
        title={
          <>
            <span className="muted">IR-{inc.public_id}</span> {inc.title}
          </>
        }
      >
        <Link className="btn ghost" to="/incidents">
          All incidents
        </Link>
        <button
          className="btn ghost"
          onClick={async () => {
            await api.deleteIncident(id);
            await qc.invalidateQueries({ queryKey: ["incidents"] });
            navigate("/incidents");
          }}
        >
          Delete
        </button>
        {inc.state !== "resolved" && (
          <button className="btn primary" onClick={() => patch({ state: "resolved" })}>
            Resolve
          </button>
        )}
      </PageHeader>
      <div className="page-scroll page-padding">
        <div className="incident">
          <div>
            <div className="card">
              <div className="card-title">What we know</div>
              <textarea
                className="input"
                rows={4}
                style={{ width: "100%" }}
                value={summary ?? inc.summary}
                onChange={(e) => setSummary(e.target.value)}
                onBlur={() => summary !== null && summary !== inc.summary && patch({ summary })}
                placeholder="Impact, cause, what is being done"
              />
            </div>
            <div className="card" style={{ marginTop: 12 }}>
              <div className="card-title">Timeline</div>
              <form
                className="toolbar"
                onSubmit={async (e) => {
                  e.preventDefault();
                  if (!note.trim()) return;
                  set(await api.noteIncident(id, note));
                  setNote("");
                }}
              >
                <input className="input" style={{ flex: 1 }} value={note} onChange={(e) => setNote(e.target.value)} placeholder="Add a note: what was seen, what was done" />
                <button className="btn">Add</button>
              </form>
              <ul className="incident-timeline">
                {[...inc.timeline].reverse().map((e, i) => (
                  <li key={i} className={e.kind}>
                    <span className="muted">
                      {fullDate(e.ts)}
                      {e.by ? ` · ${e.by}` : ""}
                    </span>
                    <div>{e.kind === "state" ? `State: ${e.text}` : e.kind === "severity" ? `Severity: ${e.text}` : e.text}</div>
                  </li>
                ))}
              </ul>
            </div>
          </div>
          <div className="card incident-side form">
            <label className="form-field">
              <span className="form-label">State</span>
              <select className="input" value={inc.state} onChange={(e) => patch({ state: e.target.value })}>
                {STATES.map((s) => (
                  <option key={s}>{s}</option>
                ))}
              </select>
            </label>
            <label className="form-field">
              <span className="form-label">Severity</span>
              <select className="input" value={inc.severity} onChange={(e) => patch({ severity: e.target.value })}>
                {SEVERITIES.map((s) => (
                  <option key={s}>{s}</option>
                ))}
              </select>
            </label>
            <label className="form-field">
              <span className="form-label">Commander</span>
              <input className="input" defaultValue={inc.commander ?? ""} onBlur={(e) => e.target.value !== (inc.commander ?? "") && patch({ commander: e.target.value })} />
            </label>
            <label className="form-field">
              <span className="form-label">Services</span>
              <input
                className="input mono"
                defaultValue={inc.services.join(", ")}
                onBlur={(e) => e.target.value !== inc.services.join(", ") && patch({ services: e.target.value.split(",") })}
              />
            </label>
            <label className="incident-check">
              <input type="checkbox" checked={inc.public} onChange={(e) => patch({ public: e.target.checked })} /> Show on the
              status page
            </label>
            <div className="muted">
              Declared {fullDate(inc.created)}
              <br />
              {inc.resolved ? `Resolved after ${duration(inc.resolved - inc.created)}` : `Ongoing for ${duration(Date.now() - inc.created)}`}
            </div>
            {inc.services.map((s) => (
              <Link key={s} className="btn small" to={`/logs?query=${encodeURIComponent(`service:${s} status:error`)}&from_ts=${inc.created - 3_600_000}&to_ts=${inc.resolved ?? Date.now()}&live=false`}>
                Error logs of {s}
              </Link>
            ))}
          </div>
        </div>
      </div>
    </>
  );
}

const DOT: Record<string, string> = { operational: "ok", degraded: "stable", outage: "alert", unknown: "paused" };

export function StatusPageSettings() {
  const q = useQuery({ queryKey: ["status-page"], queryFn: api.statusPage });
  const tests = useQuery({ queryKey: ["synthetics"], queryFn: api.synthetics });
  const monitors = useQuery({ queryKey: ["monitors"], queryFn: api.monitors });
  const [page, setPage] = useState<StatusPage | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  useEffect(() => {
    if (q.data && !page) setPage(q.data);
  }, [q.data, page]);
  if (!page) return <div className="loading">Loading…</div>;
  const change = (p: Partial<StatusPage>) => {
    setSaved(false);
    setPage({ ...page, ...p });
  };
  const component = (i: number, c: Partial<StatusComponent>) =>
    change({ components: page.components.map((x, j) => (j === i ? { ...x, ...c } : x)) });
  const save = async () => {
    try {
      setPage(await api.saveStatusPage(page));
      setError(null);
      setSaved(true);
    } catch (e) {
      setError(e instanceof ApiError ? e.message : String(e));
    }
  };
  const url = `${window.location.origin}/status`;
  return (
    <>
      <PageHeader title="Incidents" tabs={TABS}>
        {saved && <span className="muted">Saved</span>}
        <button className="btn primary" onClick={save}>
          Save
        </button>
      </PageHeader>
      <div className="page-scroll page-padding">
        <div className="card form" style={{ maxWidth: 860 }}>
          <label className="incident-check">
            <input type="checkbox" checked={page.enabled} onChange={(e) => change({ enabled: e.target.checked })} /> Publish the
            status page
          </label>
          <div className="form-hint">
            Once published, anyone can read{" "}
            <a href={url} target="_blank" rel="noreferrer">
              {url}
            </a>{" "}
            without signing in: the components below and the incidents marked "Show on the status page".
          </div>
          <label className="form-field">
            <span className="form-label">Title</span>
            <input className="input" value={page.title} onChange={(e) => change({ title: e.target.value })} />
          </label>
          <label className="form-field">
            <span className="form-label">Description</span>
            <input className="input" value={page.description} onChange={(e) => change({ description: e.target.value })} />
          </label>
          <div className="form-label">Components</div>
          {page.components.map((c, i) => (
            <div className="toolbar" key={i}>
              <span className={`synth-state ${DOT[c.status ?? "unknown"] ?? "paused"}`} title={c.status}>
                {c.status ?? "new"}
              </span>
              {c.uptime != null && <span className="muted">{c.uptime.toFixed(2)}%</span>}
              <input className="input" style={{ flex: 1 }} placeholder="Name shown" value={c.name} onChange={(e) => component(i, { name: e.target.value })} />
              <select
                className="input"
                style={{ flex: 1 }}
                value={`${c.kind}:${c.ref}`}
                onChange={(e) => {
                  const [kind, ...ref] = e.target.value.split(":");
                  component(i, { kind: kind as StatusComponent["kind"], ref: ref.join(":") });
                }}
              >
                <option value=":">Its state comes from…</option>
                <optgroup label="Synthetic tests">
                  {(tests.data?.tests ?? []).map((t) => (
                    <option key={t.public_id} value={`synthetic:${t.public_id}`}>
                      {t.name}
                    </option>
                  ))}
                </optgroup>
                <optgroup label="Monitors">
                  {(monitors.data ?? []).map((m) => (
                    <option key={m.id} value={`monitor:${m.id}`}>
                      {m.name}
                    </option>
                  ))}
                </optgroup>
              </select>
              <button className="btn ghost small icon" title="Remove" onClick={() => change({ components: page.components.filter((_, j) => j !== i) })}>
                <TrashIcon />
              </button>
            </div>
          ))}
          <div>
            <button className="btn small" onClick={() => change({ components: [...page.components, { name: "", kind: "synthetic", ref: "" }] })}>
              <PlusIcon /> Add a component
            </button>
          </div>
          {error && <div className="form-error">{error}</div>}
        </div>
      </div>
    </>
  );
}
