import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { Modal } from "../components/Common";
import { PlusIcon, TrashIcon } from "../components/Icons";
import { PageHeader } from "../components/Layout";
import { api, ApiError, type ReportDef, type SheetDef } from "../lib/api";
import { ago, compact } from "../lib/format";

const err = (e: unknown) => (e instanceof ApiError ? e.message : String(e));
const SPANS: Record<string, number> = {
  "1h": 3600e3,
  "4h": 4 * 3600e3,
  "1d": 86400e3,
  "1w": 7 * 86400e3,
  "1mo": 30 * 86400e3,
};

// ---------- Monitor Templates ----------

export function MonitorTemplates() {
  const q = useQuery({
    queryKey: ["monitor-templates"],
    queryFn: api.monitorTemplates,
  });
  const navigate = useNavigate();
  const [error, setError] = useState<string | null>(null);
  const cats = [...new Set((q.data ?? []).map((t) => t.category))];
  return (
    <>
      <PageHeader title="Monitor Templates" />
      <div className="page-scroll page-padding">
        <p className="muted" style={{ marginTop: 0 }}>
          “Use” creates the monitor from the template and opens it for editing:
          add recipients (<code>@ops@example.com</code>, <code>@webhook-x</code>
          , <code>@oncall-team</code>, <code>@workflow-x</code>) and adjust the
          scope.
        </p>
        {error && <div className="error-box">{error}</div>}
        {cats.map((c) => (
          <div key={c}>
            <div className="section-title">{c}</div>
            <div className="cards">
              {(q.data ?? [])
                .filter((t) => t.category === c)
                .map((t) => (
                  <div key={t.id} className="card">
                    <div className="card-title">
                      {t.name.replace(/\{\{[^}]+\}\}/g, "…")}
                    </div>
                    <div
                      className="mono faint"
                      style={{ fontSize: 11, wordBreak: "break-all" }}
                    >
                      {t.query}
                    </div>
                    <p className="muted" style={{ fontSize: 12 }}>
                      {t.message}
                    </p>
                    <button
                      className="btn primary"
                      onClick={async () => {
                        try {
                          const m = await api.createMonitor({
                            name: t.name,
                            type: t.type,
                            query: t.query,
                            message: t.message,
                            tags: [],
                            priority: null,
                            options: {},
                          } as never);
                          navigate(`/monitors/${m.id}/edit`);
                        } catch (e) {
                          setError(err(e));
                        }
                      }}
                    >
                      Use
                    </button>
                  </div>
                ))}
            </div>
          </div>
        ))}
      </div>
    </>
  );
}

// ---------- Monitor Quality ----------

export function MonitorQuality() {
  const q = useQuery({
    queryKey: ["monitor-quality"],
    queryFn: api.monitorQuality,
  });
  const d = q.data;
  return (
    <>
      <PageHeader title="Monitor Quality" />
      <div className="page-scroll page-padding">
        {d && (
          <p className="muted" style={{ marginTop: 0 }}>
            {d.data.length} of {d.monitors} monitors need attention.
          </p>
        )}
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Monitor</th>
                <th>Status</th>
                <th>Issues</th>
              </tr>
            </thead>
            <tbody>
              {(d?.data ?? []).map((m) => (
                <tr key={m.id}>
                  <td>
                    <Link to={`/monitors/${m.id}`}>{m.name}</Link>
                    <div className="faint" style={{ fontSize: 11 }}>
                      {m.type}
                    </div>
                  </td>
                  <td>{m.overall_state}</td>
                  <td>
                    {m.issues.map((i) => (
                      <div key={i.kind + i.text}>
                        <span className="pill warn">
                          {i.kind.replace(/_/g, " ")}
                        </span>{" "}
                        {i.text}
                      </div>
                    ))}
                  </td>
                </tr>
              ))}
              {d && !d.data.length && (
                <tr>
                  <td colSpan={3} className="muted">
                    Every monitor notifies someone, none is muted forever or
                    stuck alerting.
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

// ---------- Reports ----------

function ReportModal({
  r,
  onClose,
}: {
  r: ReportDef | null;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const dashes = useQuery({
    queryKey: ["dashboards"],
    queryFn: api.dashboards,
  });
  const a = r?.attributes;
  const [name, setName] = useState(a?.name ?? "");
  const [dash, setDash] = useState(a?.dashboard_id ?? "");
  const [span, setSpan] = useState(a?.live_span ?? "1w");
  const [schedule, setSchedule] = useState(a?.schedule ?? "0 8 * * 1");
  const [to, setTo] = useState((a?.recipients ?? []).join(", "));
  const [message, setMessage] = useState(a?.message ?? "");
  const [error, setError] = useState<string | null>(null);
  const list = dashes.data?.dashboards ?? [];
  return (
    <Modal
      title={r ? `Edit ${a?.name}` : "New report"}
      onClose={onClose}
      width={560}
    >
      <div className="modal-body form">
        <label className="form-field">
          <span className="form-label">Name</span>
          <input
            className="input"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </label>
        <label className="form-field">
          <span className="form-label">Dashboard</span>
          <select
            className="input"
            value={dash}
            onChange={(e) => setDash(e.target.value)}
          >
            <option value="">—</option>
            {list.map((d) => (
              <option key={d.id} value={d.id}>
                {d.title}
              </option>
            ))}
          </select>
        </label>
        <div style={{ display: "flex", gap: 12 }}>
          <label className="form-field">
            <span className="form-label">Time span</span>
            <select
              className="input"
              value={span}
              onChange={(e) => setSpan(e.target.value)}
            >
              {Object.keys(SPANS).map((s) => (
                <option key={s}>{s}</option>
              ))}
            </select>
          </label>
          <label className="form-field" style={{ flex: 1 }}>
            <span className="form-label">
              Schedule (cron, UTC; empty: by hand only)
            </span>
            <input
              className="input mono"
              value={schedule}
              onChange={(e) => setSchedule(e.target.value)}
            />
          </label>
        </div>
        <label className="form-field">
          <span className="form-label">
            Recipients (e-mails, comma-separated)
          </span>
          <input
            className="input"
            value={to}
            onChange={(e) => setTo(e.target.value)}
          />
        </label>
        <label className="form-field">
          <span className="form-label">Message</span>
          <textarea
            className="input"
            rows={3}
            value={message}
            onChange={(e) => setMessage(e.target.value)}
          />
        </label>
        {error && <div className="error-box">{error}</div>}
      </div>
      <div className="modal-footer">
        <button className="btn" onClick={onClose}>
          Cancel
        </button>
        <button
          className="btn primary"
          onClick={async () => {
            try {
              await api.saveReport(r?.id ?? null, {
                name:
                  name || list.find((d) => d.id === dash)?.title || "Report",
                dashboard_id: dash,
                live_span: span,
                schedule,
                recipients: to
                  .split(",")
                  .map((x) => x.trim())
                  .filter(Boolean),
                message,
              });
              await qc.invalidateQueries({ queryKey: ["reports"] });
              onClose();
            } catch (e) {
              setError(err(e));
            }
          }}
        >
          Save
        </button>
      </div>
    </Modal>
  );
}

export function Reports() {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ["reports"], queryFn: api.reports });
  const [editing, setEditing] = useState<{ r: ReportDef | null } | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  return (
    <>
      <PageHeader title="Dashboard Reports">
        <button className="btn primary" onClick={() => setEditing({ r: null })}>
          <PlusIcon /> New Report
        </button>
      </PageHeader>
      <div className="page-scroll page-padding">
        <p className="muted" style={{ marginTop: 0 }}>
          A report prints a dashboard to PDF (Chromium) and e-mails it on its
          schedule. It reads the dashboard through a public link created for it,
          deleted with the report.
        </p>
        {note && <div className="card">{note}</div>}
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Name</th>
                <th>Schedule</th>
                <th>Recipients</th>
                <th>Last sent</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {(q.data?.data ?? []).map((r) => (
                <tr key={r.id}>
                  <td>
                    <b>{r.attributes.name}</b>{" "}
                    <span className="faint">last {r.attributes.live_span}</span>
                  </td>
                  <td className="mono">{r.attributes.schedule || "by hand"}</td>
                  <td>{r.attributes.recipients.join(", ")}</td>
                  <td className="muted">
                    {r.attributes.last_sent
                      ? `${ago(r.attributes.last_sent)} · ${r.attributes.last_result}`
                      : "—"}
                  </td>
                  <td style={{ textAlign: "right", whiteSpace: "nowrap" }}>
                    <a
                      className="btn"
                      href={`/api/ui/reports/${r.id}/pdf`}
                      target="_blank"
                      rel="noreferrer"
                    >
                      Preview PDF
                    </a>{" "}
                    <button
                      className="btn"
                      disabled={busy === r.id}
                      onClick={async () => {
                        setBusy(r.id);
                        try {
                          const s = await api.sendReport(r.id);
                          setNote(
                            s.sent.length
                              ? `Sent to ${s.sent.join(", ")}`
                              : "Not sent: is ROBINE_DOG_SMTP_URL set?",
                          );
                        } catch (e) {
                          setNote(err(e));
                        }
                        setBusy(null);
                        qc.invalidateQueries({ queryKey: ["reports"] });
                      }}
                    >
                      {busy === r.id ? "Sending…" : "Send now"}
                    </button>{" "}
                    <button className="btn" onClick={() => setEditing({ r })}>
                      Edit
                    </button>
                    <button
                      className="btn icon ghost"
                      onClick={() =>
                        api
                          .deleteReport(r.id)
                          .then(() =>
                            qc.invalidateQueries({ queryKey: ["reports"] }),
                          )
                      }
                    >
                      <TrashIcon />
                    </button>
                  </td>
                </tr>
              ))}
              {q.isSuccess && !q.data.data.length && (
                <tr>
                  <td colSpan={5} className="muted">
                    No report yet.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </div>
      {editing && (
        <ReportModal r={editing.r} onClose={() => setEditing(null)} />
      )}
    </>
  );
}

// ---------- Sheets ----------

/** `[count] / [avg:@duration] * 100`: + - * / and parentheses over columns. */
export function evaluate(
  expr: string,
  row: Record<string, number | null>,
): number | null {
  const tokens = expr.match(/\[[^\]]+\]|\d+(?:\.\d+)?|[-+*/()]/g) ?? [];
  let i = 0;
  const atom = (): number | null => {
    const t = tokens[i++];
    if (t === "(") {
      const v = sum();
      i++;
      return v;
    }
    if (t === "-") {
      const v = atom();
      return v === null ? null : -v;
    }
    if (t?.startsWith("[")) return row[t.slice(1, -1)] ?? null;
    return t === undefined ? null : Number(t);
  };
  const product = (): number | null => {
    let v = atom();
    while (tokens[i] === "*" || tokens[i] === "/") {
      const op = tokens[i++];
      const r = atom();
      v =
        v === null || r === null
          ? null
          : op === "*"
            ? v * r
            : r === 0
              ? null
              : v / r;
    }
    return v;
  };
  const sum = (): number | null => {
    let v = product();
    while (tokens[i] === "+" || tokens[i] === "-") {
      const op = tokens[i++];
      const r = product();
      v = v === null || r === null ? null : op === "+" ? v + r : v - r;
    }
    return v;
  };
  return sum();
}

const blankSheet = (): SheetDef["attributes"] => ({
  name: "New sheet",
  dataset: "logs",
  query: "*",
  group_by: ["service"],
  computes: ["count"],
  columns: [],
  live_span: "1d",
  pivot: false,
});

function SheetView({
  sheet,
  onSaved,
}: {
  sheet: SheetDef | null;
  onSaved: (s: SheetDef) => void;
}) {
  const qc = useQueryClient();
  const [s, setS] = useState<SheetDef["attributes"]>(
    sheet?.attributes ?? blankSheet(),
  );
  const [cols, setCols] = useState(
    s.columns.map((c) => `${c.name} = ${c.expr}`).join("\n"),
  );
  const [error, setError] = useState<string | null>(null);
  const [run, setRun] = useState(s);
  const columns = useMemo(
    () =>
      cols
        .split("\n")
        .map((l) => l.split("="))
        .filter((p) => p.length >= 2 && p[0].trim())
        .map(([n, ...e]) => ({ name: n.trim(), expr: e.join("=").trim() })),
    [cols],
  );
  const data = useQuery({
    queryKey: ["sheet-data", run],
    queryFn: () => {
      const to = Date.now();
      return api.aggregate(run.dataset, {
        compute: run.computes.map((c) => {
          const [aggregation, metric] = c.split(":");
          return metric ? { aggregation, metric } : { aggregation };
        }),
        filter: {
          query: run.query,
          from: to - (SPANS[run.live_span] ?? SPANS["1d"]),
          to,
        },
        group_by: run.group_by.map((facet) => ({ facet, limit: 100 })),
      });
    },
  });
  const rows = (data.data?.data.buckets ?? []).map((b) => {
    const r: Record<string, number | null> = {};
    run.computes.forEach((c, i) => {
      const v = b.computes[`c${i}`];
      r[c] = typeof v === "number" ? v : null;
    });
    columns.forEach((c) => (r[c.name] = evaluate(c.expr, r)));
    return { by: b.by, values: r };
  });
  const pivot = run.pivot && run.group_by.length === 2;
  const pivotCols = pivot
    ? [...new Set(rows.map((r) => r.by[run.group_by[1]]))]
    : [];
  const pivotRows = pivot
    ? [...new Set(rows.map((r) => r.by[run.group_by[0]]))]
    : [];
  const measure = run.computes[0];
  const fmt = (v: number | null | undefined) =>
    v === null || v === undefined ? "—" : compact(Math.round(v * 100) / 100);
  return (
    <div>
      <div className="card form" style={{ marginBottom: 12 }}>
        <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
          <input
            className="input"
            style={{ width: 180 }}
            value={s.name}
            onChange={(e) => setS({ ...s, name: e.target.value })}
          />
          <select
            className="input"
            value={s.dataset}
            onChange={(e) =>
              setS({
                ...s,
                dataset: e.target.value as SheetDef["attributes"]["dataset"],
              })
            }
          >
            <option value="logs">Logs</option>
            <option value="spans">Spans</option>
            <option value="rum">RUM</option>
          </select>
          <input
            className="input mono"
            style={{ flex: 1, minWidth: 200 }}
            placeholder="Search query"
            value={s.query}
            onChange={(e) => setS({ ...s, query: e.target.value })}
          />
          <select
            className="input"
            value={s.live_span}
            onChange={(e) => setS({ ...s, live_span: e.target.value })}
          >
            {Object.keys(SPANS).map((x) => (
              <option key={x}>{x}</option>
            ))}
          </select>
        </div>
        <div
          style={{
            display: "flex",
            gap: 8,
            flexWrap: "wrap",
            alignItems: "center",
          }}
        >
          <input
            className="input mono"
            style={{ flex: 1 }}
            title="Group by facets, comma-separated"
            placeholder="group by: service, @http.status_code"
            value={s.group_by.join(", ")}
            onChange={(e) =>
              setS({
                ...s,
                group_by: e.target.value
                  .split(",")
                  .map((x) => x.trim())
                  .filter(Boolean),
              })
            }
          />
          <input
            className="input mono"
            style={{ flex: 1 }}
            title="Measures: count, avg:@duration, p95:@duration, cardinality:@usr.id"
            placeholder="measures: count, avg:@duration"
            value={s.computes.join(", ")}
            onChange={(e) =>
              setS({
                ...s,
                computes: e.target.value
                  .split(",")
                  .map((x) => x.trim())
                  .filter(Boolean),
              })
            }
          />
          <label className="muted" style={{ whiteSpace: "nowrap" }}>
            <input
              type="checkbox"
              checked={!!s.pivot}
              onChange={(e) => setS({ ...s, pivot: e.target.checked })}
            />{" "}
            Pivot the 2nd group
          </label>
        </div>
        <textarea
          className="input mono"
          rows={2}
          placeholder="Computed columns, one per line: error rate = [errors] / [count] * 100"
          value={cols}
          onChange={(e) => setCols(e.target.value)}
        />
        <div style={{ display: "flex", gap: 8 }}>
          <button className="btn" onClick={() => setRun({ ...s, columns })}>
            Run
          </button>
          <button
            className="btn primary"
            onClick={async () => {
              try {
                const r = await api.saveSheet(sheet?.id ?? null, {
                  ...s,
                  columns,
                });
                await qc.invalidateQueries({ queryKey: ["sheets"] });
                setRun({ ...s, columns });
                onSaved(r.data);
              } catch (e) {
                setError(err(e));
              }
            }}
          >
            Save
          </button>
          {error && <span className="error-box">{error}</span>}
        </div>
      </div>
      <div className="card" style={{ padding: 0, overflow: "auto" }}>
        {data.isError && <div className="error-box">{err(data.error)}</div>}
        {pivot ? (
          <table className="list">
            <thead>
              <tr>
                <th>
                  {run.group_by[0]} \ {run.group_by[1]} ({measure})
                </th>
                {pivotCols.map((c) => (
                  <th key={c} className="num">
                    {c}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {pivotRows.map((pr) => (
                <tr key={pr}>
                  <td>{pr}</td>
                  {pivotCols.map((pc) => (
                    <td key={pc} className="num">
                      {fmt(
                        rows.find(
                          (r) =>
                            r.by[run.group_by[0]] === pr &&
                            r.by[run.group_by[1]] === pc,
                        )?.values[measure],
                      )}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <table className="list">
            <thead>
              <tr>
                {run.group_by.map((g) => (
                  <th key={g}>{g}</th>
                ))}
                {[...run.computes, ...columns.map((c) => c.name)].map((c) => (
                  <th key={c} className="num">
                    {c}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((r, i) => (
                <tr key={i}>
                  {run.group_by.map((g) => (
                    <td key={g}>{r.by[g]}</td>
                  ))}
                  {[...run.computes, ...columns.map((c) => c.name)].map((c) => (
                    <td key={c} className="num">
                      {fmt(r.values[c])}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}

export function Sheets() {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ["sheets"], queryFn: api.sheets });
  const [open, setOpen] = useState<string | "new" | null>(null);
  const sheet = q.data?.data.find((s) => s.id === open) ?? null;
  return (
    <>
      <PageHeader title="Sheets">
        <button className="btn primary" onClick={() => setOpen("new")}>
          <PlusIcon /> New Sheet
        </button>
      </PageHeader>
      <div
        className="page-scroll page-padding"
        style={{ display: "flex", gap: 12, alignItems: "flex-start" }}
      >
        <div className="card" style={{ width: 220, padding: 0, flexShrink: 0 }}>
          {(q.data?.data ?? []).map((s) => (
            <div
              key={s.id}
              className="nav-entry"
              style={{
                cursor: "pointer",
                padding: "8px 10px",
                fontWeight: s.id === open ? 600 : undefined,
                display: "flex",
              }}
              onClick={() => setOpen(s.id)}
            >
              <span style={{ flex: 1 }}>{s.attributes.name}</span>
              <button
                className="btn icon ghost"
                onClick={(e) => {
                  e.stopPropagation();
                  api
                    .deleteSheet(s.id)
                    .then(() => qc.invalidateQueries({ queryKey: ["sheets"] }));
                  if (open === s.id) setOpen(null);
                }}
              >
                <TrashIcon />
              </button>
            </div>
          ))}
          {q.isSuccess && !q.data.data.length && (
            <div className="muted" style={{ padding: 10 }}>
              No sheet yet.
            </div>
          )}
        </div>
        <div style={{ flex: 1, minWidth: 0 }}>
          {open ? (
            <SheetView
              key={open}
              sheet={sheet}
              onSaved={(s) => setOpen(s.id)}
            />
          ) : (
            <div className="card muted">
              A sheet is a table over logs, spans or RUM: groups, measures,
              computed columns (<code>[errors] / [count] * 100</code>) and a
              pivot.
            </div>
          )}
        </div>
      </div>
    </>
  );
}
