import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { TimeChart, type Series } from "../components/Charts";
import { Modal } from "../components/Common";
import { PlusIcon, TrashIcon } from "../components/Icons";
import { PageHeader } from "../components/Layout";
import {
  api,
  ApiError,
  type Slo,
  type SloCorrection,
  type SloStatus,
  type SloThreshold,
} from "../lib/api";
import { compact, fullDate } from "../lib/format";

const FRAMES: Record<string, number> = { "7d": 7, "30d": 30, "90d": 90 };
const TYPE_LABEL: Record<string, string> = {
  metric: "Metric",
  monitor: "Monitor",
  time_slice: "Time slice",
};
const CATEGORIES = [
  "Scheduled Maintenance",
  "Outside Business Hours",
  "Deployment",
  "Other",
];

const errText = (e: unknown) => (e instanceof ApiError ? e.message : String(e));
const pct = (v: number) => `${v.toFixed(v >= 99.95 || v < 10 ? 3 : 2)} %`;
const stateClass = (s: SloStatus["state"]) =>
  s === "breached"
    ? "slo-breached"
    : s === "warning"
      ? "slo-warning"
      : s === "ok"
        ? "slo-ok"
        : "muted";

/** Seconds as `2h 5m`. */
function duration(s: number): string {
  s = Math.max(0, Math.round(s));
  if (s < 60) return `${s}s`;
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  return (
    [d && `${d}d`, h && `${h}h`, m && `${m}m`].filter(Boolean).join(" ") || "0m"
  );
}

const amount = (v: number, unit: string) =>
  unit === "seconds" ? duration(v) : compact(Math.round(v));

function useStatus(id: string | undefined) {
  return useQuery({
    queryKey: ["sloStatus", id],
    queryFn: () => api.sloStatus(id!),
    enabled: !!id,
    staleTime: 60_000,
    refetchInterval: 120_000,
  });
}

function StatusCell({ s }: { s?: SloStatus }) {
  if (!s) return <span className="muted">…</span>;
  if (s.error)
    return (
      <span className="infra-bad" title={s.error}>
        error
      </span>
    );
  if (s.sli === null) return <span className="muted">No data</span>;
  return <span className={stateClass(s.state)}>{pct(s.sli)}</span>;
}

function BudgetCell({ s }: { s?: SloStatus }) {
  if (!s || s.error_budget_remaining === null)
    return <span className="muted">–</span>;
  const left = s.error_budget_remaining;
  const allowed = (1 - s.target / 100) * s.total;
  const bad = Math.max(0, s.total - s.good);
  return (
    <span
      className={
        left < 0 ? "slo-breached" : left < 25 ? "slo-warning" : "slo-ok"
      }
    >
      {left.toFixed(0)} %{" "}
      <span className="muted">
        ({amount(bad, s.unit)} of {amount(allowed, s.unit)}{" "}
        {s.unit === "seconds" ? "allowed downtime" : "allowed failures"})
      </span>
    </span>
  );
}

// --- editor ------------------------------------------------------------------------------

type SliceForm = {
  query: string;
  comparator: string;
  threshold: string;
  interval: string;
};

function SloModal({ slo, onClose }: { slo: Slo; onClose: () => void }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const [type, setType] = useState(slo.type || "metric");
  const [name, setName] = useState(slo.name);
  const [description, setDescription] = useState(slo.description ?? "");
  const [numerator, setNumerator] = useState(slo.query?.numerator ?? "");
  const [denominator, setDenominator] = useState(slo.query?.denominator ?? "");
  const [monitorIds, setMonitorIds] = useState<number[]>(slo.monitor_ids ?? []);
  const [groups, setGroups] = useState((slo.groups ?? []).join("; "));
  const ts = slo.sli_specification?.time_slice;
  const [slice, setSlice] = useState<SliceForm>({
    query:
      ts?.query.queries[0]?.query ??
      "avg:trace.http.request.duration{service:web-store}",
    comparator: ts?.comparator ?? "<",
    threshold: String(ts?.threshold ?? 0.5),
    interval: String(ts?.query_interval_seconds ?? 300),
  });
  const [thresholds, setThresholds] = useState<
    { timeframe: string; target: string; warning: string }[]
  >(
    slo.thresholds.map((t) => ({
      timeframe: t.timeframe,
      target: String(t.target),
      warning: t.warning ? String(t.warning) : "",
    })),
  );
  const [tags, setTags] = useState(slo.tags.join(", "));
  const [error, setError] = useState<string | null>(null);
  const monitors = useQuery({
    queryKey: ["monitors"],
    queryFn: () => api.monitors(),
    enabled: type === "monitor",
  });
  const [monitorSearch, setMonitorSearch] = useState("");

  const save = async () => {
    const th: SloThreshold[] = thresholds.map((t) => ({
      timeframe: t.timeframe,
      target: Number(t.target),
      warning: t.warning ? Number(t.warning) : null,
    }));
    const next: Slo = {
      id: slo.id,
      name,
      description: description || null,
      type,
      thresholds: th,
      tags: tags
        .split(",")
        .map((t) => t.trim())
        .filter(Boolean),
      query:
        type === "metric"
          ? { numerator: numerator.trim(), denominator: denominator.trim() }
          : null,
      monitor_ids: type === "monitor" ? monitorIds : [],
      groups:
        type === "monitor"
          ? groups
              .split(";")
              .map((g) => g.trim())
              .filter(Boolean)
          : [],
      sli_specification:
        type === "time_slice"
          ? {
              time_slice: {
                query: {
                  formulas: [{ formula: "query1" }],
                  queries: [
                    {
                      data_source: "metrics",
                      name: "query1",
                      query: slice.query.trim(),
                    },
                  ],
                },
                comparator: slice.comparator,
                threshold: Number(slice.threshold),
                query_interval_seconds: Number(slice.interval),
              },
            }
          : null,
    };
    try {
      const res = (
        slo.id ? await api.updateSlo(slo.id, next) : await api.createSlo(next)
      ) as { data: Slo[] };
      await qc.invalidateQueries({ queryKey: ["slos"] });
      await qc.invalidateQueries({ queryKey: ["slo"] });
      await qc.invalidateQueries({ queryKey: ["sloStatus"] });
      onClose();
      if (!slo.id && res.data[0]?.id) navigate(`/slo/${res.data[0].id}`);
    } catch (e) {
      setError(errText(e));
    }
  };

  const shown = (monitors.data ?? []).filter(
    (m) =>
      m.type !== "composite" &&
      m.type !== "slo alert" &&
      m.type !== "synthetics alert" &&
      (!monitorSearch ||
        m.name.toLowerCase().includes(monitorSearch.toLowerCase()) ||
        monitorIds.includes(m.id)),
  );

  return (
    <Modal
      title={slo.id ? "Edit SLO" : "New SLO"}
      onClose={onClose}
      width={720}
    >
      <div className="modal-body form">
        {!slo.id && (
          <div className="segmented" style={{ marginBottom: 12 }}>
            {Object.entries(TYPE_LABEL).map(([k, l]) => (
              <button
                key={k}
                className={`btn small${type === k ? " primary" : ""}`}
                onClick={() => setType(k)}
              >
                {l} based
              </button>
            ))}
          </div>
        )}
        {type === "metric" && (
          <>
            <label className="form-field">
              <span className="form-label">Good events (numerator)</span>
              <input
                className="input mono"
                value={numerator}
                onChange={(e) => setNumerator(e.target.value)}
              />
              <span className="form-hint">
                A count query, or all minus bad:{" "}
                <code>
                  sum:trace.http.request.hits{"{service:api}"}.as_count() -
                  sum:trace.http.request.errors
                  {"{service:api}"}.as_count()
                </code>
              </span>
            </label>
            <label className="form-field">
              <span className="form-label">Total events (denominator)</span>
              <input
                className="input mono"
                value={denominator}
                onChange={(e) => setDenominator(e.target.value)}
              />
            </label>
          </>
        )}
        {type === "monitor" && (
          <>
            <div className="form-field">
              <span className="form-label">
                Monitors (the SLO is down while any is in Alert)
              </span>
              <input
                className="input"
                placeholder="Filter monitors"
                value={monitorSearch}
                onChange={(e) => setMonitorSearch(e.target.value)}
              />
              <div className="slo-monitor-picker">
                {shown.map((m) => (
                  <label key={m.id} className="form-check">
                    <input
                      type="checkbox"
                      checked={monitorIds.includes(m.id)}
                      onChange={(e) =>
                        setMonitorIds(
                          e.target.checked
                            ? [...monitorIds, m.id]
                            : monitorIds.filter((x) => x !== m.id),
                        )
                      }
                    />
                    <span className="mono faint">{m.id}</span> {m.name}
                  </label>
                ))}
                {monitors.data && !shown.length && (
                  <span className="muted">No monitor.</span>
                )}
              </div>
            </div>
            <label className="form-field">
              <span className="form-label">Only these groups (optional)</span>
              <input
                className="input mono"
                value={groups}
                onChange={(e) => setGroups(e.target.value)}
                placeholder="host:web-1; host:web-2"
              />
              <span className="form-hint">
                Multi-alert monitors: group names as shown on the monitor,
                separated by `;`.
              </span>
            </label>
          </>
        )}
        {type === "time_slice" && (
          <>
            <label className="form-field">
              <span className="form-label">Metric query</span>
              <input
                className="input mono"
                value={slice.query}
                onChange={(e) => setSlice({ ...slice, query: e.target.value })}
              />
            </label>
            <div className="form-row inline">
              A
              <select
                className="input"
                value={slice.interval}
                onChange={(e) =>
                  setSlice({ ...slice, interval: e.target.value })
                }
              >
                <option value="60">1-minute</option>
                <option value="300">5-minute</option>
              </select>
              slice is good when its value is
              <select
                className="input"
                value={slice.comparator}
                onChange={(e) =>
                  setSlice({ ...slice, comparator: e.target.value })
                }
              >
                {["<", "<=", ">", ">="].map((c) => (
                  <option key={c}>{c}</option>
                ))}
              </select>
              <input
                className="input"
                type="number"
                value={slice.threshold}
                onChange={(e) =>
                  setSlice({ ...slice, threshold: e.target.value })
                }
                style={{ width: 100 }}
              />
            </div>
            <span className="form-hint">
              Slices without data count as good.
            </span>
          </>
        )}
        <div className="form-field">
          <span className="form-label">Targets</span>
          {thresholds.map((t, i) => (
            <div className="form-row inline" key={i}>
              <select
                className="input"
                value={t.timeframe}
                onChange={(e) =>
                  setThresholds(
                    thresholds.map((x, j) =>
                      j === i ? { ...x, timeframe: e.target.value } : x,
                    ),
                  )
                }
              >
                {Object.keys(FRAMES).map((f) => (
                  <option key={f} value={f}>
                    {FRAMES[f]} days
                  </option>
                ))}
              </select>
              target
              <input
                className="input"
                value={t.target}
                onChange={(e) =>
                  setThresholds(
                    thresholds.map((x, j) =>
                      j === i ? { ...x, target: e.target.value } : x,
                    ),
                  )
                }
                style={{ width: 90 }}
              />
              % warning
              <input
                className="input"
                value={t.warning}
                placeholder="optional"
                onChange={(e) =>
                  setThresholds(
                    thresholds.map((x, j) =>
                      j === i ? { ...x, warning: e.target.value } : x,
                    ),
                  )
                }
                style={{ width: 90 }}
              />
              %
              {thresholds.length > 1 && (
                <button
                  className="btn small ghost"
                  onClick={() =>
                    setThresholds(thresholds.filter((_, j) => j !== i))
                  }
                >
                  <TrashIcon />
                </button>
              )}
            </div>
          ))}
          {thresholds.length < 3 && (
            <button
              className="btn small ghost"
              onClick={() =>
                setThresholds([
                  ...thresholds,
                  {
                    timeframe:
                      Object.keys(FRAMES).find(
                        (f) => !thresholds.some((t) => t.timeframe === f),
                      ) ?? "7d",
                    target: thresholds[0]?.target ?? "99.9",
                    warning: "",
                  },
                ])
              }
            >
              <PlusIcon /> Add a time frame
            </button>
          )}
        </div>
        <label className="form-field">
          <span className="form-label">Name</span>
          <input
            className="input"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Checkout availability"
          />
        </label>
        <label className="form-field">
          <span className="form-label">Description</span>
          <input
            className="input"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />
        </label>
        <label className="form-field">
          <span className="form-label">Tags</span>
          <input
            className="input"
            value={tags}
            onChange={(e) => setTags(e.target.value)}
            placeholder="team:shop, env:prod"
          />
        </label>
        {error && <div className="form-error">{error}</div>}
      </div>
      <div className="modal-footer">
        <button className="btn" onClick={onClose}>
          Cancel
        </button>
        <button className="btn primary" onClick={save} disabled={!name.trim()}>
          Save
        </button>
      </div>
    </Modal>
  );
}

// --- detail ------------------------------------------------------------------------------

function CorrectionModal({
  sloId,
  onClose,
}: {
  sloId: string;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const local = (d: Date) =>
    new Date(d.getTime() - d.getTimezoneOffset() * 60_000)
      .toISOString()
      .slice(0, 16);
  const now = new Date();
  const [start, setStart] = useState(
    local(new Date(now.getTime() - 3_600_000)),
  );
  const [end, setEnd] = useState(local(now));
  const [recurring, setRecurring] = useState(false);
  const [rrule, setRrule] = useState("FREQ=WEEKLY;BYDAY=SA,SU");
  const [durationMin, setDurationMin] = useState("60");
  const [category, setCategory] = useState("Scheduled Maintenance");
  const [description, setDescription] = useState("");
  const [error, setError] = useState<string | null>(null);
  const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  const save = async () => {
    const s = Math.floor(new Date(start).getTime() / 1000);
    try {
      await api.createSloCorrection({
        slo_id: sloId,
        start: s,
        end: recurring ? null : Math.floor(new Date(end).getTime() / 1000),
        duration: recurring ? Number(durationMin) * 60 : null,
        rrule: recurring ? rrule : null,
        timezone: recurring ? zone : null,
        category,
        description: description || null,
      });
      await qc.invalidateQueries({ queryKey: ["sloCorrections", sloId] });
      await qc.invalidateQueries({ queryKey: ["sloStatus", sloId] });
      await qc.invalidateQueries({ queryKey: ["sloHistory", sloId] });
      onClose();
    } catch (e) {
      setError(errText(e));
    }
  };
  return (
    <Modal title="New status correction" onClose={onClose} width={560}>
      <div className="modal-body form">
        <p className="muted" style={{ marginTop: 0 }}>
          The corrected period is left out of the SLO, both its good and its
          bad.
        </p>
        <label className="form-check">
          <input
            type="checkbox"
            checked={recurring}
            onChange={(e) => setRecurring(e.target.checked)}
          />{" "}
          Recurring
        </label>
        <div className="form-row">
          <label className="form-field">
            <span className="form-label">
              {recurring ? "First start" : "Start"}
            </span>
            <input
              className="input"
              type="datetime-local"
              value={start}
              onChange={(e) => setStart(e.target.value)}
            />
          </label>
          {recurring ? (
            <label className="form-field">
              <span className="form-label">Duration (minutes)</span>
              <input
                className="input"
                type="number"
                value={durationMin}
                onChange={(e) => setDurationMin(e.target.value)}
              />
            </label>
          ) : (
            <label className="form-field">
              <span className="form-label">End</span>
              <input
                className="input"
                type="datetime-local"
                value={end}
                onChange={(e) => setEnd(e.target.value)}
              />
            </label>
          )}
        </div>
        {recurring && (
          <label className="form-field">
            <span className="form-label">Repeat (RRULE, in {zone})</span>
            <input
              className="input mono"
              value={rrule}
              onChange={(e) => setRrule(e.target.value)}
            />
            <span className="form-hint">
              <code>FREQ=DAILY</code>, <code>FREQ=WEEKLY;BYDAY=MO,FR</code>,{" "}
              <code>FREQ=MONTHLY;COUNT=6</code>, <code>UNTIL=20261231</code>
            </span>
          </label>
        )}
        <label className="form-field">
          <span className="form-label">Category</span>
          <select
            className="input"
            value={category}
            onChange={(e) => setCategory(e.target.value)}
          >
            {CATEGORIES.map((c) => (
              <option key={c}>{c}</option>
            ))}
          </select>
        </label>
        <label className="form-field">
          <span className="form-label">Description</span>
          <input
            className="input"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />
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

/** The budget left over the window, the burn rate over the last hour, and
 * the bad per bucket, from the history buckets. */
function useCharts(
  good: number[],
  total: number[],
  step: number,
  target: number,
) {
  return useMemo(() => {
    const allowedShare = 1 - target / 100;
    const allTotal = total.reduce((a, b) => a + b, 0);
    const allowed = allowedShare * allTotal;
    let bad = 0;
    const budget: number[] = [];
    const burn: number[] = [];
    const badPer: number[] = [];
    const span = Math.max(1, Math.round(3_600_000 / step));
    for (let i = 0; i < total.length; i++) {
      const b = Math.max(0, total[i] - good[i]);
      bad += b;
      badPer.push(b);
      budget.push(
        allowed > 0 ? Math.max(0, ((allowed - bad) / allowed) * 100) : NaN,
      );
      let g = 0;
      let t = 0;
      for (let j = Math.max(0, i - span + 1); j <= i; j++) {
        g += good[j];
        t += total[j];
      }
      burn.push(t > 0 && allowedShare > 0 ? (1 - g / t) / allowedShare : NaN);
    }
    return { budget, burn, badPer };
  }, [good, total, step, target]);
}

export function SloPage() {
  const { id } = useParams();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const slo = useQuery({ queryKey: ["slo", id], queryFn: () => api.slo(id!) });
  const status = useStatus(id);
  const s = slo.data?.data;
  const [frame, setFrame] = useState<string | null>(null);
  const tf = frame ?? s?.thresholds[0]?.timeframe ?? "30d";
  const th = s?.thresholds.find((t) => t.timeframe === tf) ?? s?.thresholds[0];
  const [range] = useState(() => Date.now());
  const from = range - FRAMES[tf] * 86_400_000;
  const history = useQuery({
    queryKey: ["sloHistory", id, tf, range],
    queryFn: () => api.sloHistory(id!, from, range, th?.target),
    enabled: !!s,
  });
  const corrections = useQuery({
    queryKey: ["sloCorrections", id],
    queryFn: () => api.sloCorrections(id!),
  });
  const [editing, setEditing] = useState(false);
  const [correcting, setCorrecting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const series = history.data?.data.series;
  const step = (series?.interval ?? 60) * 1000;
  const charts = useCharts(
    series?.numerator.values ?? [],
    series?.denominator.values ?? [],
    step,
    th?.target ?? 99.9,
  );

  if (slo.error)
    return <div className="page-padding error-box">{errText(slo.error)}</div>;
  if (!s) return <div className="page-padding faint">Loading…</div>;
  const times = series?.times ?? [];
  const chart = (values: number[], label: string, color: string): Series[] => [
    { key: label, label, color, values },
  ];
  const unit = series?.unit ?? (s.type === "metric" ? "events" : "seconds");

  return (
    <>
      <PageHeader title="Service Level Objectives" />
      <div className="page-scroll page-padding">
        <div className="monitor-header">
          <Link className="btn small ghost" to="/slo/manage">
            ← SLOs
          </Link>
          <h2>{s.name}</h2>
          <span className="tag">{TYPE_LABEL[s.type ?? "metric"]}</span>
          <div className="spacer" />
          <button className="btn" onClick={() => setCorrecting(true)}>
            Add a correction
          </button>
          <button className="btn primary" onClick={() => setEditing(true)}>
            Edit
          </button>
          <button
            className="btn"
            title="Delete"
            onClick={async () => {
              const alerts = s.configured_alert_ids;
              const msg = alerts.length
                ? `Delete "${s.name}" and leave its ${alerts.length} alert monitor(s) without SLO?`
                : `Delete "${s.name}"?`;
              if (!window.confirm(msg)) return;
              try {
                await api.deleteSlo(id!, alerts.length > 0);
                await qc.invalidateQueries({ queryKey: ["slos"] });
                navigate("/slo/manage");
              } catch (e) {
                setError(errText(e));
              }
            }}
          >
            <TrashIcon />
          </button>
        </div>
        {error && <div className="error-box">{error}</div>}
        {s.description && <p className="muted">{s.description}</p>}
        <div className="slo-tiles">
          {(status.data ?? s.thresholds.map(() => undefined)).map((st, i) => {
            const t = s.thresholds[i];
            return (
              <button
                key={t.timeframe}
                className={`card slo-tile${t.timeframe === tf ? " selected" : ""}`}
                onClick={() => setFrame(t.timeframe)}
              >
                <div className="card-title">
                  {FRAMES[t.timeframe]} days{" "}
                  <span className="muted">target {t.target} %</span>
                </div>
                <div className="slo-big">
                  <StatusCell s={st} />
                </div>
                <div>
                  <span className="faint">Error budget</span>{" "}
                  <BudgetCell s={st} />
                </div>
              </button>
            );
          })}
        </div>
        {history.error && (
          <div className="error-box">{errText(history.error)}</div>
        )}
        <div className="slo-charts">
          <div className="card">
            <div className="card-title">
              Error budget remaining ({FRAMES[tf]} days)
            </div>
            <TimeChart
              times={times}
              interval={step}
              from={from}
              to={range}
              kind="lines"
              height={140}
              format={(v) => `${v.toFixed(0)} %`}
              series={chart(charts.budget, "budget left", "#8b6fd0")}
            />
          </div>
          <div className="card">
            <div className="card-title">Burn rate (past hour)</div>
            <TimeChart
              times={times}
              interval={step}
              from={from}
              to={range}
              kind="lines"
              height={140}
              format={(v) => `${v.toFixed(1)}×`}
              markers={[
                {
                  value: 1,
                  color: "#f5a524",
                  label: "1× (budget lasts the window)",
                },
                { value: 14.4, color: "#e74c50", label: "14.4×" },
              ]}
              series={chart(charts.burn, "burn rate", "#e5484d")}
            />
          </div>
          <div className="card">
            <div className="card-title">
              {unit === "seconds"
                ? "Downtime per interval"
                : "Bad events per interval"}
            </div>
            <TimeChart
              times={times}
              interval={step}
              from={from}
              to={range}
              height={110}
              format={(v) => amount(v, unit)}
              series={chart(
                charts.badPer,
                unit === "seconds" ? "downtime" : "bad events",
                "#e5484d",
              )}
            />
          </div>
        </div>
        <div className="section-title">Definition</div>
        {s.type === "monitor" ? (
          <p>
            Up while none of{" "}
            {(s.monitor_ids ?? []).map((m, i) => (
              <span key={m}>
                {i > 0 && ", "}
                <Link to={`/monitors/${m}`}>monitor {m}</Link>
              </span>
            ))}{" "}
            is in Alert
            {s.groups?.length ? ` (groups ${s.groups.join(", ")})` : ""}.
          </p>
        ) : s.type === "time_slice" ? (
          <p className="mono">
            {s.sli_specification?.time_slice.query.queries[0]?.query}{" "}
            {s.sli_specification?.time_slice.comparator}{" "}
            {s.sli_specification?.time_slice.threshold} per{" "}
            {(s.sli_specification?.time_slice.query_interval_seconds ?? 300) /
              60}{" "}
            min
          </p>
        ) : (
          <>
            <p className="mono">good: {s.query?.numerator}</p>
            <p className="mono">total: {s.query?.denominator}</p>
          </>
        )}
        <div className="section-title">
          Alerts
          <Link
            className="btn small"
            style={{ marginLeft: 12 }}
            to={`/monitors/create?type=slo&slo=${s.id}&timeframe=${tf}`}
          >
            <PlusIcon /> New SLO alert
          </Link>
        </div>
        {s.configured_alert_ids.length ? (
          <ul>
            {s.configured_alert_ids.map((m) => (
              <li key={m}>
                <Link to={`/monitors/${m}`}>Monitor {m}</Link>
              </li>
            ))}
          </ul>
        ) : (
          <p className="muted">
            No alert: a burn-rate alert warns while the budget burns too fast,
            an error-budget one when too much of it is spent.
          </p>
        )}
        <div className="section-title">Status corrections</div>
        <Corrections list={corrections.data?.data ?? []} sloId={id!} />
      </div>
      {editing && <SloModal slo={s} onClose={() => setEditing(false)} />}
      {correcting && (
        <CorrectionModal sloId={id!} onClose={() => setCorrecting(false)} />
      )}
    </>
  );
}

function Corrections({
  list,
  sloId,
}: {
  list: SloCorrection[];
  sloId: string;
}) {
  const qc = useQueryClient();
  if (!list.length)
    return (
      <p className="muted">
        None: maintenance windows and the like can be left out here.
      </p>
    );
  return (
    <table className="list">
      <thead>
        <tr>
          <th>When</th>
          <th>Category</th>
          <th>Description</th>
          <th style={{ width: 40 }} />
        </tr>
      </thead>
      <tbody>
        {list.map((c) => {
          const a = c.attributes;
          return (
            <tr key={c.id}>
              <td>
                {fullDate(a.start * 1000)}
                {a.rrule ? (
                  <span className="muted">
                    {" "}
                    for {duration(a.duration ?? 0)}, <code>{a.rrule}</code> (
                    {a.timezone ?? "UTC"})
                  </span>
                ) : (
                  a.end && (
                    <span className="muted"> → {fullDate(a.end * 1000)}</span>
                  )
                )}
              </td>
              <td>{a.category}</td>
              <td className="muted">{a.description}</td>
              <td>
                <button
                  className="btn small ghost"
                  title="Delete"
                  onClick={async () => {
                    await api.deleteSloCorrection(c.id);
                    await qc.invalidateQueries({
                      queryKey: ["sloCorrections", sloId],
                    });
                    await qc.invalidateQueries({
                      queryKey: ["sloStatus", sloId],
                    });
                    await qc.invalidateQueries({
                      queryKey: ["sloHistory", sloId],
                    });
                  }}
                >
                  <TrashIcon />
                </button>
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

// --- list --------------------------------------------------------------------------------

function Row({ s }: { s: Slo }) {
  const navigate = useNavigate();
  const status = useStatus(s.id);
  const th = s.thresholds[0];
  const main = status.data?.find((x) => x.timeframe === th.timeframe);
  return (
    <tr onClick={() => navigate(`/slo/${s.id}`)} style={{ cursor: "pointer" }}>
      <td className="infra-name ellipsis">
        {s.name}
        {s.tags.map((t) => (
          <span key={t} className="tag small">
            {t}
          </span>
        ))}
      </td>
      <td className="muted">{TYPE_LABEL[s.type ?? "metric"] ?? s.type}</td>
      <td>{th.target} %</td>
      <td className="muted">{FRAMES[th.timeframe]} days</td>
      <td>
        <StatusCell s={main} />
      </td>
      <td>
        <BudgetCell s={main} />
      </td>
    </tr>
  );
}

const blank = (): Slo => ({
  name: "",
  type: "metric",
  query: { numerator: "", denominator: "" },
  thresholds: [{ timeframe: "30d", target: 99.9 }],
  tags: [],
});

export function Slos() {
  const list = useQuery({ queryKey: ["slos"], queryFn: api.slos });
  const [editing, setEditing] = useState<Slo | null>(null);
  const [search, setSearch] = useState("");
  const slos = (list.data?.data ?? []).filter((s) => {
    const q = search.trim().toLowerCase();
    return (
      !q ||
      q.split(/\s+/).every((w) => {
        if (w.startsWith("type:")) return (s.type ?? "metric") === w.slice(5);
        return (
          s.name.toLowerCase().includes(w) ||
          s.tags.some((t) => t.toLowerCase() === w)
        );
      })
    );
  });
  return (
    <>
      <PageHeader title="Service Level Objectives">
        <button className="btn primary" onClick={() => setEditing(blank())}>
          <PlusIcon /> New SLO
        </button>
      </PageHeader>
      <div className="page-scroll page-padding">
        <div className="toolbar" style={{ marginBottom: 12 }}>
          <input
            className="input"
            style={{ width: 420 }}
            placeholder="Search: words of the name, team:shop, type:monitor"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        </div>
        <div className="card" style={{ padding: 0 }}>
          <table className="list infra-table">
            <thead>
              <tr>
                <th>Name</th>
                <th style={{ width: 100 }}>Type</th>
                <th style={{ width: 90 }}>Target</th>
                <th style={{ width: 100 }}>Time frame</th>
                <th style={{ width: 110 }}>Status</th>
                <th style={{ width: "34%" }}>Error budget left</th>
              </tr>
            </thead>
            <tbody>
              {slos.map((s) => (
                <Row key={s.id} s={s} />
              ))}
            </tbody>
          </table>
          {list.data && slos.length === 0 && (
            <div className="empty-state">
              <h3>{search ? "No SLO matches" : "No SLO yet"}</h3>
              Metric-based (good among all events), monitor-based (time out of
              alert) or time-slice (minutes whose latency stays under a
              threshold).
            </div>
          )}
        </div>
      </div>
      {editing && <SloModal slo={editing} onClose={() => setEditing(null)} />}
    </>
  );
}
