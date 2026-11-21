import {
  keepPreviousData,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { Modal, SidePanel } from "../components/Common";
import { PlusIcon, TrashIcon } from "../components/Icons";
import { PageHeader } from "../components/Layout";
import { TimePicker } from "../components/TimePicker";
import {
  api,
  ApiError,
  type SecurityRule,
  type SecuritySignal,
} from "../lib/api";
import { ago, compact, fullDate } from "../lib/format";
import { useTimeRange } from "../lib/time";

const TABS = [
  { to: "/security/signals", label: "Signals" },
  { to: "/security/rules", label: "Detection Rules" },
  { to: "/audit", label: "Audit Trail" },
];
const SEVERITIES = ["critical", "high", "medium", "low", "info"];
const sevClass = (s: string) =>
  s === "critical" || s === "high" ? "error" : s === "medium" ? "warn" : "";

function useParam(k: string) {
  const [params, setParams] = useSearchParams();
  const set = (v: string) => {
    const p = new URLSearchParams(params);
    if (v) p.set(k, v);
    else p.delete(k);
    setParams(p, { replace: true });
  };
  return [params.get(k) ?? "", set] as const;
}

function SignalPanel({
  s,
  onClose,
}: {
  s: SecuritySignal;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const a = s.attributes;
  const [assignee, setAssignee] = useState(a.assignee ?? "");
  const run = async (
    what: "state" | "assignee",
    attrs: Record<string, unknown>,
  ) => {
    await api.triageSignal(s.id, what, attrs);
    await qc.invalidateQueries({ queryKey: ["signals"] });
  };
  const logQuery = [
    a.query,
    ...Object.entries(a.group_tags).map(([k, v]) => `${k}:"${v}"`),
  ].join(" ");
  return (
    <SidePanel
      onClose={onClose}
      storageKey="signal"
      header={
        <>
          <span className={`pill ${sevClass(a.severity)}`}>{a.severity}</span>
          <span style={{ fontWeight: 600 }}>{a.title}</span>
        </>
      }
    >
      <div className="panel-section">
        <p style={{ marginTop: 0 }}>{a.message}</p>
        <dl className="kv-grid">
          <dt>Group</dt>
          <dd className="mono">{a.group}</dd>
          <dt>Value</dt>
          <dd>{compact(a.value)}</dd>
          <dt>First seen</dt>
          <dd>{fullDate(a.first_seen)}</dd>
          <dt>Last seen</dt>
          <dd>{fullDate(a.timestamp)}</dd>
          <dt>Status</dt>
          <dd>
            {a.status}
            {a.archive_reason ? ` (${a.archive_reason})` : ""}
          </dd>
        </dl>
        <div
          style={{ display: "flex", gap: 8, marginTop: 12, flexWrap: "wrap" }}
        >
          <button
            className="btn"
            onClick={() => run("state", { state: "under_review" })}
          >
            Under review
          </button>
          <button
            className="btn"
            onClick={() =>
              run("state", {
                state: "archived",
                archive_reason: "false_positive",
              })
            }
          >
            Archive (false positive)
          </button>
          <button
            className="btn"
            onClick={() =>
              run("state", {
                state: "archived",
                archive_reason: "true_positive_benign",
              })
            }
          >
            Archive (benign)
          </button>
          <button
            className="btn"
            onClick={() => run("state", { state: "open" })}
          >
            Reopen
          </button>
          <Link
            className="btn"
            to={`/logs?${new URLSearchParams({ query: logQuery, from_ts: String(a.first_seen - 600_000), to_ts: String(a.timestamp + 60_000) })}`}
          >
            View logs
          </Link>
        </div>
        <form
          style={{ display: "flex", gap: 8, marginTop: 12 }}
          onSubmit={(e) => {
            e.preventDefault();
            run("assignee", { assignee: assignee.trim() });
          }}
        >
          <input
            className="input"
            placeholder="Assignee"
            value={assignee}
            onChange={(e) => setAssignee(e.target.value)}
          />
          <button className="btn">Assign</button>
        </form>
      </div>
    </SidePanel>
  );
}

export function SecuritySignals() {
  const time = useTimeRange("1d", 30_000);
  const [status, setStatus] = useParam("status");
  const [severity, setSeverity] = useParam("severity");
  const [open, setOpen] = useParam("signal");
  const f = {
    from: time.from,
    to: time.to,
    status: status || undefined,
    severity: severity || undefined,
  };
  const q = useQuery({
    queryKey: ["signals", f],
    queryFn: () => api.securitySignals(f),
    placeholderData: keepPreviousData,
  });
  const rows = q.data ?? [];
  const sel = rows.find((r) => r.id === open);
  return (
    <>
      <PageHeader title="Cloud SIEM" tabs={TABS}>
        <TimePicker time={time} />
      </PageHeader>
      <div className="filters-row">
        <select
          className="input"
          value={status}
          onChange={(e) => setStatus(e.target.value)}
        >
          <option value="">Any status</option>
          <option value="open">open</option>
          <option value="under_review">under review</option>
          <option value="archived">archived</option>
        </select>
        <select
          className="input"
          value={severity}
          onChange={(e) => setSeverity(e.target.value)}
        >
          <option value="">Any severity</option>
          {SEVERITIES.map((s) => (
            <option key={s}>{s}</option>
          ))}
        </select>
        <div className="spacer" />
        <span className="muted">{rows.length} signals</span>
      </div>
      <div className="page-scroll">
        <table className="list">
          <thead>
            <tr>
              <th style={{ width: 90 }}>Severity</th>
              <th>Signal</th>
              <th>Group</th>
              <th className="num">Value</th>
              <th>Status</th>
              <th>Assignee</th>
              <th>Last seen</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((s) => (
              <tr
                key={s.id}
                className="signal-row"
                style={{ cursor: "pointer" }}
                onClick={() => setOpen(s.id)}
              >
                <td>
                  <span className={`pill ${sevClass(s.attributes.severity)}`}>
                    {s.attributes.severity}
                  </span>
                </td>
                <td>{s.attributes.title}</td>
                <td className="mono">{s.attributes.group}</td>
                <td className="num">{compact(s.attributes.value)}</td>
                <td>{s.attributes.status}</td>
                <td>{s.attributes.assignee ?? "—"}</td>
                <td className="muted">{ago(s.attributes.timestamp)}</td>
              </tr>
            ))}
            {q.isSuccess && !rows.length && (
              <tr>
                <td colSpan={7} className="muted">
                  No signal: detection rules run every minute on logs.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      {sel && <SignalPanel s={sel} onClose={() => setOpen("")} />}
    </>
  );
}

function RuleModal({
  rule,
  onClose,
}: {
  rule: SecurityRule | null;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const q0 = rule?.queries[0];
  const [name, setName] = useState(rule?.name ?? "");
  const [query, setQuery] = useState(q0?.query ?? "");
  const [type, setType] = useState(q0?.aggregation ?? "count");
  const [groupBy, setGroupBy] = useState((q0?.groupByFields ?? []).join(", "));
  const [field, setField] = useState(q0?.metrics?.[0] ?? "");
  const [cases, setCases] = useState(
    (rule?.cases ?? [{ status: "medium", condition: "a > 10" }])
      .map((c) => `${c.status}: ${c.condition}`)
      .join("\n"),
  );
  const [windowMin, setWindowMin] = useState(
    String((rule?.options.evaluationWindow ?? 300) / 60),
  );
  const [message, setMessage] = useState(rule?.message ?? "");
  const [error, setError] = useState<string | null>(null);
  return (
    <Modal
      title={rule ? `Edit ${rule.name}` : "New detection rule"}
      onClose={onClose}
      width={620}
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
          <span className="form-label">Logs matching</span>
          <input
            className="input mono"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder='service:sshd "Failed password"'
          />
        </label>
        <label className="form-field">
          <span className="form-label">Detection</span>
          <select
            className="input"
            value={type}
            onChange={(e) => setType(e.target.value)}
          >
            <option value="count">Threshold on a count</option>
            <option value="new_value">New value of an attribute</option>
          </select>
        </label>
        {type === "new_value" && (
          <label className="form-field">
            <span className="form-label">Attribute</span>
            <input
              className="input mono"
              value={field}
              onChange={(e) => setField(e.target.value)}
              placeholder="@usr.id"
            />
          </label>
        )}
        <label className="form-field">
          <span className="form-label">Group by (comma separated)</span>
          <input
            className="input mono"
            value={groupBy}
            onChange={(e) => setGroupBy(e.target.value)}
            placeholder="@network.client.ip"
          />
        </label>
        <label className="form-field">
          <span className="form-label">
            Cases, one per line: severity: condition (a is the count; empty for
            always)
          </span>
          <textarea
            className="input mono"
            rows={3}
            value={cases}
            onChange={(e) => setCases(e.target.value)}
          />
        </label>
        <label className="form-field">
          <span className="form-label">Evaluation window (minutes)</span>
          <input
            className="input"
            type="number"
            min={1}
            value={windowMin}
            onChange={(e) => setWindowMin(e.target.value)}
          />
        </label>
        <label className="form-field">
          <span className="form-label">Message</span>
          <textarea
            className="input"
            rows={2}
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
            setError(null);
            try {
              await api.saveSecurityRule(rule?.id ?? null, {
                name: name.trim(),
                message,
                isEnabled: rule?.isEnabled ?? true,
                queries: [
                  {
                    name: "a",
                    query: query.trim(),
                    aggregation: type,
                    groupByFields: groupBy
                      .split(",")
                      .map((x) => x.trim())
                      .filter(Boolean),
                    ...(type === "new_value"
                      ? { metrics: [field.trim()] }
                      : {}),
                  },
                ],
                cases: cases
                  .split("\n")
                  .map((l) => l.trim())
                  .filter(Boolean)
                  .map((l) => {
                    const [status, ...rest] = l.split(":");
                    return {
                      status: status.trim(),
                      condition: rest.join(":").trim(),
                    };
                  }),
                options: {
                  ...(rule?.options ?? {}),
                  evaluationWindow: Math.round(Number(windowMin) * 60),
                },
              });
              await qc.invalidateQueries({ queryKey: ["security-rules"] });
              onClose();
            } catch (e) {
              setError(e instanceof ApiError ? e.message : String(e));
            }
          }}
        >
          Save rule
        </button>
      </div>
    </Modal>
  );
}

export function DetectionRules() {
  const qc = useQueryClient();
  const q = useQuery({
    queryKey: ["security-rules"],
    queryFn: api.securityRules,
  });
  const [editing, setEditing] = useState<{ r: SecurityRule | null } | null>(
    null,
  );
  const refresh = () => qc.invalidateQueries({ queryKey: ["security-rules"] });
  return (
    <>
      <PageHeader title="Cloud SIEM" tabs={TABS}>
        <button className="btn primary" onClick={() => setEditing({ r: null })}>
          <PlusIcon /> New Rule
        </button>
      </PageHeader>
      <div className="page-scroll">
        <table className="list">
          <thead>
            <tr>
              <th>Rule</th>
              <th>Query</th>
              <th>Detection</th>
              <th>Cases</th>
              <th>Enabled</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {(q.data ?? []).map((r) => (
              <tr key={r.id}>
                <td>
                  {r.name}{" "}
                  {r.isDefault && <span className="faint">(default)</span>}
                </td>
                <td className="mono" style={{ fontSize: 12 }}>
                  {r.queries[0]?.query}
                </td>
                <td style={{ fontSize: 12 }}>
                  {r.queries[0]?.aggregation === "new_value"
                    ? `new ${r.queries[0]?.metrics?.[0]}`
                    : "count"}
                  {r.queries[0]?.groupByFields.length
                    ? ` by ${r.queries[0].groupByFields.join(", ")}`
                    : ""}{" "}
                  over {r.options.evaluationWindow / 60} min
                </td>
                <td style={{ fontSize: 12 }}>
                  {r.cases.map((c, i) => (
                    <div key={i}>
                      <span className={`pill ${sevClass(c.status)}`}>
                        {c.status}
                      </span>{" "}
                      {c.condition}
                    </div>
                  ))}
                </td>
                <td>
                  <input
                    type="checkbox"
                    checked={r.isEnabled}
                    onChange={async (e) => {
                      await api.saveSecurityRule(r.id, {
                        isEnabled: e.target.checked,
                      });
                      refresh();
                    }}
                  />
                </td>
                <td style={{ textAlign: "right", whiteSpace: "nowrap" }}>
                  <button className="btn" onClick={() => setEditing({ r })}>
                    Edit
                  </button>
                  <button
                    className="btn icon ghost"
                    title="Delete"
                    onClick={async () => {
                      await api.deleteSecurityRule(r.id);
                      refresh();
                    }}
                  >
                    <TrashIcon />
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {editing && (
        <RuleModal rule={editing.r} onClose={() => setEditing(null)} />
      )}
    </>
  );
}

export function AuditTrail() {
  const time = useTimeRange("1d", 30_000);
  const [query, setQuery] = useParam("query");
  const f = { from: time.from, to: time.to, query: query || undefined };
  const q = useQuery({
    queryKey: ["audit", f],
    queryFn: () => api.auditEvents(f),
    placeholderData: keepPreviousData,
  });
  return (
    <>
      <PageHeader title="Audit Trail" tabs={TABS}>
        <TimePicker time={time} />
      </PageHeader>
      <div className="filters-row">
        <input
          className="input"
          style={{ flex: 1 }}
          placeholder="Filter by user, method or path"
          defaultValue={query}
          onKeyDown={(e) =>
            e.key === "Enter" &&
            setQuery((e.target as HTMLInputElement).value.trim())
          }
        />
        <span className="muted">
          Kept 90 days · also at /api/v2/audit/events
        </span>
      </div>
      <div className="page-scroll">
        <table className="list">
          <thead>
            <tr>
              <th>Date</th>
              <th>User</th>
              <th>Source</th>
              <th>Request</th>
              <th>Status</th>
            </tr>
          </thead>
          <tbody>
            {(q.data ?? []).map((e) => {
              const a = e.attributes.attributes;
              return (
                <tr key={e.id}>
                  <td className="muted">{fullDate(e.attributes.timestamp)}</td>
                  <td>{a.usr.name ?? a.usr.id}</td>
                  <td>{a.evt.source}</td>
                  <td className="mono">
                    {a.http.method} {a.http.url_details.path}
                  </td>
                  <td>
                    <span
                      className={`pill ${a.http.status_code < 400 ? "ok" : "error"}`}
                    >
                      {a.http.status_code}
                    </span>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </>
  );
}
