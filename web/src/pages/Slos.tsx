import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Modal, SidePanel } from "../components/Common";
import { PlusIcon } from "../components/Icons";
import { PageHeader } from "../components/Layout";
import { api, ApiError, type Slo } from "../lib/api";

const FRAMES: Record<string, number> = { "7d": 7, "30d": 30, "90d": 90 };
type Sli = { good: number; total: number; sli: number | null };

/** The events a query counted over the window: the sum of its points. */
async function total(query: string, from: number, to: number, signal?: AbortSignal): Promise<number> {
  const terms = query.split(" - ").map((t) => t.trim());
  const data = await api.queryMetrics(terms, from, to, signal);
  const failed = data.results.find((r) => r.error && !r.error.startsWith("no data"));
  if (failed) throw new Error(failed.error!);
  const sums = terms.map((_, i) =>
    data.series.filter((s) => s.query_index === i).reduce((a, s) => a + s.points.reduce((b, p) => b + p[1], 0), 0),
  );
  return sums.slice(1).reduce((a, b) => a - b, sums[0]);
}

function useSli(slo: Slo, timeframe: string) {
  return useQuery({
    queryKey: ["sli", slo.id, slo.query.numerator, slo.query.denominator, timeframe],
    staleTime: 60_000,
    refetchInterval: 120_000,
    queryFn: async ({ signal }): Promise<Sli> => {
      const to = Date.now();
      const from = to - FRAMES[timeframe] * 86_400_000;
      const [good, all] = await Promise.all([
        total(slo.query.numerator, from, to, signal),
        total(slo.query.denominator, from, to, signal),
      ]);
      return { good, total: all, sli: all > 0 ? (good / all) * 100 : null };
    },
  });
}

const pct = (v: number) => `${v.toFixed(v >= 99.95 || v < 10 ? 3 : 2)} %`;

function Status({ slo, timeframe }: { slo: Slo; timeframe: string }) {
  const q = useSli(slo, timeframe);
  const th = slo.thresholds.find((t) => t.timeframe === timeframe);
  if (q.isLoading) return <span className="muted">…</span>;
  if (q.error) return <span className="infra-bad" title={String(q.error)}>error</span>;
  const sli = q.data?.sli;
  if (sli === null || sli === undefined) return <span className="muted">No data</span>;
  const cls = th ? (sli < th.target ? "slo-breached" : th.warning && sli < th.warning ? "slo-warning" : "slo-ok") : "";
  return <span className={cls}>{pct(sli)}</span>;
}

/** What is left of the errors the target allows, in percent of that allowance. */
function Budget({ slo, timeframe }: { slo: Slo; timeframe: string }) {
  const q = useSli(slo, timeframe);
  const th = slo.thresholds.find((t) => t.timeframe === timeframe);
  if (!th || !q.data || q.data.sli === null) return <span className="muted">–</span>;
  const allowed = (1 - th.target / 100) * q.data.total;
  const bad = q.data.total - q.data.good;
  const left = allowed > 0 ? ((allowed - bad) / allowed) * 100 : 0;
  return (
    <span className={left < 0 ? "slo-breached" : left < 25 ? "slo-warning" : "slo-ok"}>
      {left.toFixed(0)} % <span className="muted">({Math.round(bad)} bad of {Math.round(allowed)} allowed)</span>
    </span>
  );
}

function SloModal({ slo, onClose }: { slo: Slo; onClose: () => void }) {
  const qc = useQueryClient();
  const first = slo.thresholds[0];
  const [name, setName] = useState(slo.name);
  const [description, setDescription] = useState(slo.description ?? "");
  const [numerator, setNumerator] = useState(slo.query.numerator);
  const [denominator, setDenominator] = useState(slo.query.denominator);
  const [timeframe, setTimeframe] = useState(first?.timeframe ?? "30d");
  const [target, setTarget] = useState(String(first?.target ?? 99.9));
  const [warning, setWarning] = useState(first?.warning ? String(first.warning) : "");
  const [error, setError] = useState<string | null>(null);
  const save = async () => {
    const next: Slo = {
      ...slo,
      name,
      description: description || null,
      query: { numerator: numerator.trim(), denominator: denominator.trim() },
      thresholds: [{ timeframe, target: Number(target), warning: warning ? Number(warning) : null }],
    };
    try {
      if (slo.id) await api.updateSlo(slo.id, next);
      else await api.createSlo(next);
      await qc.invalidateQueries({ queryKey: ["slos"] });
      onClose();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : String(e));
    }
  };
  return (
    <Modal title={slo.id ? "Edit SLO" : "New SLO"} onClose={onClose} width={640}>
      <div className="modal-body form">
        <label className="form-field">
          <span className="form-label">Name</span>
          <input className="input" autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="Checkout availability" />
        </label>
        <label className="form-field">
          <span className="form-label">Good events (numerator)</span>
          <input className="input mono" value={numerator} onChange={(e) => setNumerator(e.target.value)} />
          <span className="form-hint">
            A metric query counting events, or all minus bad:{" "}
            <code>sum:trace.http.request.hits{"{service:api}"}.as_count() - sum:trace.http.request.errors{"{service:api}"}.as_count()</code>
          </span>
        </label>
        <label className="form-field">
          <span className="form-label">Total events (denominator)</span>
          <input className="input mono" value={denominator} onChange={(e) => setDenominator(e.target.value)} />
        </label>
        <div style={{ display: "flex", gap: 12 }}>
          <label className="form-field" style={{ flex: 1 }}>
            <span className="form-label">Time frame</span>
            <select className="input" value={timeframe} onChange={(e) => setTimeframe(e.target.value)}>
              {Object.keys(FRAMES).map((f) => (
                <option key={f} value={f}>
                  {FRAMES[f]} days
                </option>
              ))}
            </select>
          </label>
          <label className="form-field" style={{ flex: 1 }}>
            <span className="form-label">Target (%)</span>
            <input className="input" value={target} onChange={(e) => setTarget(e.target.value)} />
          </label>
          <label className="form-field" style={{ flex: 1 }}>
            <span className="form-label">Warning (%)</span>
            <input className="input" value={warning} onChange={(e) => setWarning(e.target.value)} placeholder="optional" />
          </label>
        </div>
        <label className="form-field">
          <span className="form-label">Description</span>
          <input className="input" value={description} onChange={(e) => setDescription(e.target.value)} />
        </label>
        {error && <div className="form-error">{error}</div>}
      </div>
      <div className="modal-footer">
        <button className="btn" onClick={onClose}>
          Cancel
        </button>
        <button className="btn primary" onClick={save}>
          Save
        </button>
      </div>
    </Modal>
  );
}

function Detail({ slo, onClose, onEdit }: { slo: Slo; onClose: () => void; onEdit: () => void }) {
  const qc = useQueryClient();
  const main = slo.thresholds[0];
  return (
    <SidePanel onClose={onClose} storageKey="slo" header={<strong>{slo.name}</strong>}>
      <div className="infra-detail">
        {slo.description && <p className="muted" style={{ marginTop: 0 }}>{slo.description}</p>}
        <div className="toolbar" style={{ marginBottom: 12 }}>
          <button className="btn small" onClick={onEdit}>
            Edit
          </button>
          <div className="spacer" />
          <button
            className="btn small ghost"
            onClick={async () => {
              await api.deleteSlo(slo.id!);
              await qc.invalidateQueries({ queryKey: ["slos"] });
              onClose();
            }}
          >
            Delete
          </button>
        </div>
        <div className="slo-tiles">
          {Object.keys(FRAMES).map((f) => (
            <div className="card" key={f}>
              <div className="card-title">
                {FRAMES[f]} days {f === main.timeframe && <span className="muted">target {main.target} %</span>}
              </div>
              <div className="slo-big">
                <Status slo={slo} timeframe={f} />
              </div>
            </div>
          ))}
        </div>
        <div className="section-title">Error budget remaining ({FRAMES[main.timeframe]} days)</div>
        <p>
          <Budget slo={slo} timeframe={main.timeframe} />
        </p>
        <div className="section-title">Queries</div>
        <p className="mono">good: {slo.query.numerator}</p>
        <p className="mono">total: {slo.query.denominator}</p>
      </div>
    </SidePanel>
  );
}

const blank = (): Slo => ({
  name: "",
  query: { numerator: "", denominator: "" },
  thresholds: [{ timeframe: "30d", target: 99.9 }],
  tags: [],
});

export function Slos() {
  const list = useQuery({ queryKey: ["slos"], queryFn: api.slos });
  const [editing, setEditing] = useState<Slo | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const slos = list.data?.data ?? [];
  const current = slos.find((s) => s.id === selected);
  return (
    <>
      <PageHeader title="Service Level Objectives">
        <button className="btn primary" onClick={() => setEditing(blank())}>
          <PlusIcon /> New SLO
        </button>
      </PageHeader>
      <div className="page-scroll page-padding">
        <p className="muted" style={{ maxWidth: 900, marginTop: 0 }}>
          Metric-based SLOs: the share of good events among all events over a time frame, against a target. The error
          budget is what the target still allows to fail.
        </p>
        <div className="card" style={{ padding: 0 }}>
          <table className="list infra-table">
            <thead>
              <tr>
                <th>Name</th>
                <th style={{ width: 110 }}>Target</th>
                <th style={{ width: 120 }}>Time frame</th>
                <th style={{ width: 130 }}>Status</th>
                <th style={{ width: "32%" }}>Error budget left</th>
              </tr>
            </thead>
            <tbody>
              {slos.map((s) => {
                const th = s.thresholds[0];
                return (
                  <tr key={s.id} className={s.id === selected ? "selected" : ""} onClick={() => setSelected(s.id!)}>
                    <td className="infra-name ellipsis">{s.name}</td>
                    <td>{th.target} %</td>
                    <td className="muted">{FRAMES[th.timeframe]} days</td>
                    <td>
                      <Status slo={s} timeframe={th.timeframe} />
                    </td>
                    <td>
                      <Budget slo={s} timeframe={th.timeframe} />
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          {list.data && slos.length === 0 && (
            <div className="empty-state">
              <h3>No SLO yet</h3>
              For instance the share of passed runs of a synthetic test, or of requests of a service without error.
            </div>
          )}
        </div>
      </div>
      {current && !editing && <Detail slo={current} onClose={() => setSelected(null)} onEdit={() => setEditing(current)} />}
      {editing && <SloModal slo={editing} onClose={() => setEditing(null)} />}
    </>
  );
}
