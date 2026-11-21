import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { Modal, SidePanel } from "../components/Common";
import { PlusIcon, TrashIcon } from "../components/Icons";
import { PageHeader } from "../components/Layout";
import {
  api,
  ApiError,
  type CaseItem,
  type EscalationPolicy,
  type OnCallSchedule,
} from "../lib/api";
import { ago, fullDate } from "../lib/format";

const err = (e: unknown) => (e instanceof ApiError ? e.message : String(e));
const PRIORITIES = ["NOT_DEFINED", "P1", "P2", "P3", "P4", "P5"];
const STATUSES = ["OPEN", "IN_PROGRESS", "CLOSED"];
const pill = (s: string) =>
  s === "CLOSED" || s === "resolved"
    ? "ok"
    : s === "IN_PROGRESS" || s === "acknowledged"
      ? "warn"
      : "error";

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

function NewCase({ onClose }: { onClose: (id?: string) => void }) {
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [priority, setPriority] = useState("P3");
  const [project, setProject] = useState("CASE");
  const [error, setError] = useState<string | null>(null);
  return (
    <Modal title="New case" onClose={() => onClose()} width={560}>
      <div className="modal-body form">
        <label className="form-field">
          <span className="form-label">Title</span>
          <input
            className="input"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
          />
        </label>
        <label className="form-field">
          <span className="form-label">Description</span>
          <textarea
            className="input"
            rows={4}
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />
        </label>
        <div style={{ display: "flex", gap: 12 }}>
          <label className="form-field">
            <span className="form-label">Priority</span>
            <select
              className="input"
              value={priority}
              onChange={(e) => setPriority(e.target.value)}
            >
              {PRIORITIES.map((p) => (
                <option key={p}>{p}</option>
              ))}
            </select>
          </label>
          <label className="form-field">
            <span className="form-label">Project</span>
            <input
              className="input"
              value={project}
              onChange={(e) => setProject(e.target.value)}
            />
          </label>
        </div>
        {error && <div className="error-box">{error}</div>}
      </div>
      <div className="modal-footer">
        <button className="btn" onClick={() => onClose()}>
          Cancel
        </button>
        <button
          className="btn primary"
          onClick={async () => {
            try {
              const c = await api.createCase({
                title,
                description,
                priority,
                project,
              });
              onClose(c.id);
            } catch (e) {
              setError(err(e));
            }
          }}
        >
          Create case
        </button>
      </div>
    </Modal>
  );
}

function CasePanel({ c, onClose }: { c: CaseItem; onClose: () => void }) {
  const qc = useQueryClient();
  const a = c.attributes;
  const [comment, setComment] = useState("");
  const [assignee, setAssignee] = useState(a.assignee ?? "");
  const act = async (what: string, attrs: Record<string, unknown>) => {
    await api.caseAction(c.id, what, attrs);
    await qc.invalidateQueries({ queryKey: ["cases"] });
  };
  return (
    <SidePanel
      onClose={onClose}
      storageKey="case"
      header={
        <>
          <span className="mono faint">{a.key}</span>
          <span style={{ fontWeight: 600 }}>{a.title}</span>
        </>
      }
    >
      <div className="panel-section">
        {a.description && (
          <p style={{ marginTop: 0, whiteSpace: "pre-wrap" }}>
            {a.description}
          </p>
        )}
        <div
          style={{
            display: "flex",
            gap: 8,
            flexWrap: "wrap",
            alignItems: "center",
          }}
        >
          <select
            className="input"
            value={a.status}
            onChange={(e) => act("status", { status: e.target.value })}
          >
            {STATUSES.map((s) => (
              <option key={s}>{s}</option>
            ))}
          </select>
          <select
            className="input"
            value={a.priority}
            onChange={(e) => act("priority", { priority: e.target.value })}
          >
            {PRIORITIES.map((p) => (
              <option key={p}>{p}</option>
            ))}
          </select>
          <form
            style={{ display: "flex", gap: 6 }}
            onSubmit={(e) => {
              e.preventDefault();
              if (assignee.trim())
                act("assign", { assignee_id: assignee.trim() });
              else act("unassign", {});
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
        {a.origin?.url && (
          <p>
            From {a.origin.type?.replace("_", " ")}:{" "}
            <Link to={a.origin.url}>{a.origin.id}</Link>
          </p>
        )}
      </div>
      <div className="panel-section">
        <h4>Timeline</h4>
        {a.timeline.map((t, i) => (
          <div key={i} className="case-event">
            <span className="faint">{fullDate(t.ts)}</span> <b>{t.author}</b>{" "}
            <span style={{ whiteSpace: "pre-wrap" }}>{t.text}</span>
          </div>
        ))}
        <form
          onSubmit={async (e) => {
            e.preventDefault();
            if (!comment.trim()) return;
            await act("comment", { comment });
            setComment("");
          }}
          style={{ display: "flex", gap: 6, marginTop: 8 }}
        >
          <input
            className="input"
            style={{ flex: 1 }}
            placeholder="Add a comment"
            value={comment}
            onChange={(e) => setComment(e.target.value)}
          />
          <button className="btn">Comment</button>
        </form>
      </div>
    </SidePanel>
  );
}

export function Cases() {
  const [status, setStatus] = useParam("status");
  const [open, setOpen] = useParam("case");
  const [creating, setCreating] = useState(false);
  const q = useQuery({
    queryKey: ["cases", status],
    queryFn: () => api.cases(status || undefined),
  });
  const rows = q.data ?? [];
  const sel = rows.find((c) => c.id === open);
  return (
    <>
      <PageHeader title="Case Management">
        <button className="btn primary" onClick={() => setCreating(true)}>
          <PlusIcon /> New Case
        </button>
      </PageHeader>
      <div className="filters-row">
        <select
          className="input"
          value={status}
          onChange={(e) => setStatus(e.target.value)}
        >
          <option value="">Any status</option>
          {STATUSES.map((s) => (
            <option key={s}>{s}</option>
          ))}
        </select>
        <div className="spacer" />
        <span className="muted">
          {rows.length} cases · same API as Datadog: /api/v2/cases
        </span>
      </div>
      <div className="page-scroll">
        <table className="list">
          <thead>
            <tr>
              <th style={{ width: 100 }}>Key</th>
              <th>Title</th>
              <th>Status</th>
              <th>Priority</th>
              <th>Assignee</th>
              <th>Updated</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((c) => (
              <tr
                key={c.id}
                style={{ cursor: "pointer" }}
                onClick={() => setOpen(c.id)}
              >
                <td className="mono">{c.attributes.key}</td>
                <td>{c.attributes.title}</td>
                <td>
                  <span className={`pill ${pill(c.attributes.status)}`}>
                    {c.attributes.status}
                  </span>
                </td>
                <td>{c.attributes.priority}</td>
                <td>{c.attributes.assignee ?? "—"}</td>
                <td className="muted">{ago(c.attributes.modified_at)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {creating && (
        <NewCase
          onClose={(id) => {
            setCreating(false);
            q.refetch();
            if (id) setOpen(id);
          }}
        />
      )}
      {sel && <CasePanel c={sel} onClose={() => setOpen("")} />}
    </>
  );
}

function ScheduleModal({
  s,
  onClose,
}: {
  s: OnCallSchedule | null;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const [name, setName] = useState(s?.name ?? "");
  const [members, setMembers] = useState((s?.members ?? []).join("\n"));
  const [hours, setHours] = useState(String(s?.length_hours ?? 168));
  const [error, setError] = useState<string | null>(null);
  return (
    <Modal
      title={s ? `Edit ${s.name}` : "New schedule"}
      onClose={onClose}
      width={480}
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
          <span className="form-label">
            Members, in rotation order (one e-mail per line)
          </span>
          <textarea
            className="input mono"
            rows={4}
            value={members}
            onChange={(e) => setMembers(e.target.value)}
          />
        </label>
        <label className="form-field">
          <span className="form-label">Shift length (hours; 168 = weekly)</span>
          <input
            className="input"
            type="number"
            min={1}
            value={hours}
            onChange={(e) => setHours(e.target.value)}
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
              await api.onCallSave("schedules", s?.id ?? null, {
                name,
                members: members
                  .split("\n")
                  .map((m) => m.trim())
                  .filter(Boolean),
                length_hours: Number(hours),
                start: s?.start ?? Date.now(),
              });
              await qc.invalidateQueries({ queryKey: ["oncall"] });
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

function PolicyModal({
  p,
  schedules,
  onClose,
}: {
  p: EscalationPolicy | null;
  schedules: OnCallSchedule[];
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const [name, setName] = useState(p?.name ?? "");
  const [team, setTeam] = useState(p?.team ?? "");
  const [steps, setSteps] = useState(
    (
      p?.steps ?? [
        {
          targets: schedules[0] ? [`schedule:${schedules[0].id}`] : [],
          delay_minutes: 5,
        },
      ]
    )
      .map((s) => `${s.delay_minutes}: ${s.targets.join(", ")}`)
      .join("\n"),
  );
  const [repeat, setRepeat] = useState(String(p?.repeat ?? 1));
  const [error, setError] = useState<string | null>(null);
  return (
    <Modal
      title={p ? `Edit ${p.name}` : "New escalation policy"}
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
          <span className="form-label">
            Team handle (monitors page it with @oncall-handle)
          </span>
          <input
            className="input mono"
            value={team}
            onChange={(e) => setTeam(e.target.value)}
          />
        </label>
        <label className="form-field">
          <span className="form-label">
            Steps, one per line: minutes before the next step: targets
            (schedule:id or e-mails). Schedules:{" "}
            {schedules.map((s) => `${s.name} = schedule:${s.id}`).join(", ") ||
              "none yet"}
          </span>
          <textarea
            className="input mono"
            rows={4}
            value={steps}
            onChange={(e) => setSteps(e.target.value)}
          />
        </label>
        <label className="form-field">
          <span className="form-label">Repeat the policy (times)</span>
          <input
            className="input"
            type="number"
            min={0}
            value={repeat}
            onChange={(e) => setRepeat(e.target.value)}
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
              await api.onCallSave("escalation-policies", p?.id ?? null, {
                name,
                team: team.trim(),
                repeat: Number(repeat),
                steps: steps
                  .split("\n")
                  .map((l) => l.trim())
                  .filter(Boolean)
                  .map((l) => {
                    const [d, ...rest] = l.split(":");
                    const t = rest.join(":");
                    return {
                      delay_minutes: Number(d) || 5,
                      targets: t
                        .split(",")
                        .map((x) => x.trim())
                        .filter(Boolean),
                    };
                  }),
              });
              await qc.invalidateQueries({ queryKey: ["oncall"] });
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

export function OnCall() {
  const qc = useQueryClient();
  const [highlight] = useParam("page");
  const pages = useQuery({
    queryKey: ["oncall", "pages"],
    queryFn: api.onCallPages,
    refetchInterval: 15000,
  });
  const schedules = useQuery({
    queryKey: ["oncall", "schedules"],
    queryFn: () => api.onCallList<OnCallSchedule>("schedules"),
  });
  const policies = useQuery({
    queryKey: ["oncall", "policies"],
    queryFn: () => api.onCallList<EscalationPolicy>("escalation-policies"),
  });
  const [schedule, setSchedule] = useState<{ s: OnCallSchedule | null } | null>(
    null,
  );
  const [policy, setPolicy] = useState<{ p: EscalationPolicy | null } | null>(
    null,
  );
  const refresh = () => qc.invalidateQueries({ queryKey: ["oncall"] });
  return (
    <>
      <PageHeader title="On-Call">
        <button className="btn" onClick={() => setSchedule({ s: null })}>
          <PlusIcon /> Schedule
        </button>
        <button className="btn primary" onClick={() => setPolicy({ p: null })}>
          <PlusIcon /> Escalation Policy
        </button>
      </PageHeader>
      <div className="page-scroll page-padding">
        <div className="section-title" style={{ marginTop: 0 }}>
          Pages
        </div>
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Triggered</th>
                <th>Team</th>
                <th>Title</th>
                <th>Status</th>
                <th>Latest</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {(pages.data ?? []).map((p) => (
                <tr
                  key={p.id}
                  style={
                    p.id === highlight
                      ? { background: "var(--purple-soft)" }
                      : undefined
                  }
                >
                  <td className="muted">{ago(p.created_at)}</td>
                  <td className="mono">@oncall-{p.team}</td>
                  <td>{p.title}</td>
                  <td>
                    <span className={`pill ${pill(p.status)}`}>{p.status}</span>
                  </td>
                  <td style={{ fontSize: 12 }}>
                    {p.timeline[p.timeline.length - 1]?.text}
                  </td>
                  <td style={{ textAlign: "right", whiteSpace: "nowrap" }}>
                    {p.status === "triggered" && (
                      <button
                        className="btn primary"
                        onClick={() =>
                          api.pageAction(p.id, "acknowledge").then(refresh)
                        }
                      >
                        Acknowledge
                      </button>
                    )}{" "}
                    {p.status !== "resolved" && (
                      <button
                        className="btn"
                        onClick={() =>
                          api.pageAction(p.id, "resolve").then(refresh)
                        }
                      >
                        Resolve
                      </button>
                    )}
                  </td>
                </tr>
              ))}
              {pages.isSuccess && !pages.data.length && (
                <tr>
                  <td colSpan={6} className="muted">
                    No page. A monitor pages a team when its message names
                    @oncall-&lt;team&gt;.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
        <div className="cards" style={{ marginTop: 12 }}>
          <div className="card">
            <div className="card-title">Schedules</div>
            <table className="list">
              <tbody>
                {(schedules.data ?? []).map((s) => (
                  <tr key={s.id}>
                    <td>
                      {s.name}
                      <div className="faint" style={{ fontSize: 11 }}>
                        {s.members.join(" → ")} · {s.length_hours} h shifts
                      </div>
                    </td>
                    <td>
                      On call: <b>{s.on_call ?? "—"}</b>
                    </td>
                    <td style={{ textAlign: "right", whiteSpace: "nowrap" }}>
                      <button
                        className="btn"
                        onClick={() => setSchedule({ s })}
                      >
                        Edit
                      </button>
                      <button
                        className="btn icon ghost"
                        onClick={() =>
                          api.onCallDelete("schedules", s.id).then(refresh)
                        }
                      >
                        <TrashIcon />
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div className="card">
            <div className="card-title">Escalation policies</div>
            <table className="list">
              <tbody>
                {(policies.data ?? []).map((p) => (
                  <tr key={p.id}>
                    <td>
                      {p.name}{" "}
                      <span className="mono faint">@oncall-{p.team}</span>
                      {p.steps.map((s, i) => (
                        <div key={i} className="faint" style={{ fontSize: 11 }}>
                          {i + 1}. {s.targets.join(", ")} · then{" "}
                          {s.delay_minutes} min
                        </div>
                      ))}
                    </td>
                    <td style={{ textAlign: "right", whiteSpace: "nowrap" }}>
                      <button className="btn" onClick={() => setPolicy({ p })}>
                        Edit
                      </button>
                      <button
                        className="btn icon ghost"
                        onClick={() =>
                          api
                            .onCallDelete("escalation-policies", p.id)
                            .then(refresh)
                        }
                      >
                        <TrashIcon />
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      </div>
      {schedule && (
        <ScheduleModal s={schedule.s} onClose={() => setSchedule(null)} />
      )}
      {policy && (
        <PolicyModal
          p={policy.p}
          schedules={schedules.data ?? []}
          onClose={() => setPolicy(null)}
        />
      )}
    </>
  );
}
