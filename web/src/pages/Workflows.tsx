import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Modal, SidePanel } from "../components/Common";
import { PlusIcon, TrashIcon } from "../components/Icons";
import { PageHeader } from "../components/Layout";
import {
  api,
  ApiError,
  type AppDef,
  type Workflow,
  type WorkflowRun,
  type WorkflowStep,
} from "../lib/api";
import { ago, fullDate } from "../lib/format";

const err = (e: unknown) => (e instanceof ApiError ? e.message : String(e));
const pill = (s: string) =>
  s === "succeeded" || s === "approved"
    ? "ok"
    : s === "running" ||
        s === "waiting_approval" ||
        s === "waiting" ||
        s === "skipped"
      ? "warn"
      : "error";

/** An example of each action's parameters; `{{ trigger.* }}` and `{{ steps.<name>.* }}` are replaced. */
const EXAMPLES: Record<string, unknown> = {
  http: {
    method: "POST",
    url: "https://hooks.example.com/…",
    headers: {},
    body: '{"text": "{{ trigger.title }} is {{ trigger.status }}"}',
  },
  "k8s.restart": { namespace: "default", kind: "deployment", name: "api" },
  "k8s.scale": {
    namespace: "default",
    kind: "deployment",
    name: "api",
    replicas: 3,
  },
  "k8s.delete_pod": { namespace: "default", name: "api-6d9f…" },
  "case.create": {
    title: "{{ trigger.title }}",
    description: "{{ trigger.message }}",
    priority: "P2",
    project: "OPS",
  },
  "downtime.create": {
    scope: "*",
    monitor_id: "{{ trigger.monitor_id }}",
    minutes: 60,
    message: "Silenced by a workflow",
  },
  "monitor.mute": {
    monitor_id: "{{ trigger.monitor_id }}",
    scope: "*",
    minutes: 30,
  },
  "oncall.page": {
    team: "ops",
    title: "{{ trigger.title }}",
    message: "{{ trigger.message }}",
  },
  approval: { message: "Restart {{ trigger.group }}?" },
  sleep: { seconds: 30 },
};

function StepEditor({
  step,
  actions,
  onChange,
  onRemove,
}: {
  step: WorkflowStep & { raw: string };
  actions: string[];
  onChange: (s: WorkflowStep & { raw: string }) => void;
  onRemove: () => void;
}) {
  return (
    <div className="card" style={{ padding: 10, marginBottom: 8 }}>
      <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
        <input
          className="input mono"
          style={{ width: 160 }}
          placeholder="name"
          value={step.name}
          onChange={(e) => onChange({ ...step, name: e.target.value })}
        />
        <select
          className="input"
          value={step.action}
          onChange={(e) =>
            onChange({
              ...step,
              action: e.target.value,
              raw: JSON.stringify(EXAMPLES[e.target.value] ?? {}, null, 2),
            })
          }
        >
          {actions.map((a) => (
            <option key={a}>{a}</option>
          ))}
        </select>
        <input
          className="input mono"
          style={{ flex: 1 }}
          placeholder="if (optional): {{ trigger.status }} == Alert"
          value={step.if ?? ""}
          onChange={(e) => onChange({ ...step, if: e.target.value })}
        />
        <label className="muted" style={{ whiteSpace: "nowrap", fontSize: 12 }}>
          <input
            type="checkbox"
            checked={!!step.continue_on_error}
            onChange={(e) =>
              onChange({ ...step, continue_on_error: e.target.checked })
            }
          />{" "}
          continue on error
        </label>
        <button
          className="btn icon ghost"
          onClick={onRemove}
          title="Remove the step"
        >
          <TrashIcon />
        </button>
      </div>
      <textarea
        className="input mono"
        rows={Math.min(10, step.raw.split("\n").length + 1)}
        style={{ width: "100%", marginTop: 6, fontSize: 12 }}
        value={step.raw}
        onChange={(e) => onChange({ ...step, raw: e.target.value })}
      />
    </div>
  );
}

function WorkflowModal({
  wf,
  actions,
  onClose,
}: {
  wf: Workflow | null;
  actions: string[];
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const a = wf?.attributes;
  const [name, setName] = useState(a?.name ?? "");
  const [description, setDescription] = useState(a?.description ?? "");
  const [enabled, setEnabled] = useState(a?.enabled ?? true);
  const [signal, setSignal] = useState(!!a?.triggers?.security_signal);
  const [rule, setRule] = useState(a?.triggers?.security_rule ?? "");
  const [schedule, setSchedule] = useState(a?.triggers?.schedule ?? "");
  const [steps, setSteps] = useState<(WorkflowStep & { raw: string })[]>(
    (
      a?.steps ?? [
        {
          name: "notify",
          action: "http",
          params: EXAMPLES.http as Record<string, unknown>,
        },
      ]
    ).map((s) => ({
      ...s,
      raw: JSON.stringify(s.params ?? {}, null, 2),
    })),
  );
  const [error, setError] = useState<string | null>(null);
  const save = async () => {
    try {
      const parsed = steps.map((s, i) => {
        let params: Record<string, unknown>;
        try {
          params = JSON.parse(s.raw || "{}");
        } catch {
          throw new Error(
            `step ${i + 1} (${s.name}): the parameters are not valid JSON`,
          );
        }
        return {
          name: s.name,
          action: s.action,
          params,
          if: s.if || undefined,
          continue_on_error: s.continue_on_error,
        };
      });
      await api.saveWorkflow(wf?.id ?? null, {
        name,
        description,
        enabled,
        triggers: { security_signal: signal, security_rule: rule, schedule },
        steps: parsed,
      });
      await qc.invalidateQueries({ queryKey: ["workflows"] });
      onClose();
    } catch (e) {
      setError(
        e instanceof Error && !(e instanceof ApiError) ? e.message : err(e),
      );
    }
  };
  return (
    <Modal
      title={wf ? `Edit ${a?.name}` : "New workflow"}
      onClose={onClose}
      width={860}
    >
      <div className="modal-body form">
        <div style={{ display: "flex", gap: 12 }}>
          <label className="form-field" style={{ flex: 1 }}>
            <span className="form-label">
              Name (a monitor runs it with @workflow-{name || "<name>"})
            </span>
            <input
              className="input mono"
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
          </label>
          <label className="form-field" style={{ flex: 2 }}>
            <span className="form-label">Description</span>
            <input
              className="input"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
            />
          </label>
          <label className="form-field">
            <span className="form-label">Enabled</span>
            <input
              type="checkbox"
              checked={enabled}
              onChange={(e) => setEnabled(e.target.checked)}
            />
          </label>
        </div>
        <div className="section-title" style={{ marginTop: 4 }}>
          Triggers
        </div>
        <div className="muted" style={{ fontSize: 12 }}>
          Always: by hand, from an app, and from the API (
          <code>POST /api/v2/workflows/{"{id}"}/instances</code>). A monitor
          runs it when its message names{" "}
          <code>@workflow-{name || "<name>"}</code>.
        </div>
        <div style={{ display: "flex", gap: 12, alignItems: "center" }}>
          <label style={{ whiteSpace: "nowrap" }}>
            <input
              type="checkbox"
              checked={signal}
              onChange={(e) => setSignal(e.target.checked)}
            />{" "}
            On new security signals
          </label>
          <input
            className="input mono"
            placeholder="of this rule id (empty: all rules)"
            value={rule}
            disabled={!signal}
            onChange={(e) => setRule(e.target.value)}
          />
          <input
            className="input mono"
            placeholder="Schedule (cron, UTC): */15 * * * *"
            value={schedule}
            onChange={(e) => setSchedule(e.target.value)}
          />
        </div>
        <div className="section-title">Steps</div>
        <div className="muted" style={{ fontSize: 12, marginBottom: 6 }}>
          In parameters, <code>{"{{ trigger.title }}"}</code>,{" "}
          <code>{"{{ inputs.<field> }}"}</code> and{" "}
          <code>{"{{ steps.<name>.body.<field> }}"}</code> are replaced;{" "}
          <code>{"{{ x | json }}"}</code> quotes as JSON. An HTTP step reaches
          any webhook (Slack, Discord, ntfy…).
        </div>
        {steps.map((s, i) => (
          <StepEditor
            key={i}
            step={s}
            actions={actions}
            onChange={(n) => setSteps(steps.map((x, j) => (j === i ? n : x)))}
            onRemove={() => setSteps(steps.filter((_, j) => j !== i))}
          />
        ))}
        <button
          className="btn"
          onClick={() =>
            setSteps([
              ...steps,
              {
                name: `step${steps.length + 1}`,
                action: "http",
                params: {},
                raw: JSON.stringify(EXAMPLES.http, null, 2),
              },
            ])
          }
        >
          <PlusIcon /> Step
        </button>
        {error && <div className="error-box">{error}</div>}
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

function RunPanel({ id, onClose }: { id: string; onClose: () => void }) {
  const qc = useQueryClient();
  const q = useQuery({
    queryKey: ["workflow-run", id],
    queryFn: () => api.workflowRun(id),
    refetchInterval: (q) =>
      q.state.data?.data.status === "running" ? 1500 : false,
  });
  const r = q.data?.data;
  const decide = async (what: "approve" | "reject") => {
    await api.decideRun(id, what);
    await qc.invalidateQueries({ queryKey: ["workflow-run", id] });
    await qc.invalidateQueries({ queryKey: ["workflow-runs"] });
  };
  return (
    <SidePanel
      onClose={onClose}
      storageKey="workflow-run"
      header={
        <>
          <span style={{ fontWeight: 600 }}>{r?.workflow_name}</span>
          {r && <span className={`pill ${pill(r.status)}`}>{r.status}</span>}
        </>
      }
    >
      {r && (
        <>
          <div className="panel-section">
            <div className="muted">
              Run #{r.id} · {r.trigger_type} · {fullDate(r.started_at)}
              {r.finished_at &&
                ` · ${((r.finished_at - r.started_at) / 1000).toFixed(1)} s`}
            </div>
            {r.error && <div className="error-box">{r.error}</div>}
            {r.status === "waiting_approval" && (
              <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
                <button
                  className="btn primary"
                  onClick={() => decide("approve")}
                >
                  Approve
                </button>
                <button className="btn" onClick={() => decide("reject")}>
                  Reject
                </button>
              </div>
            )}
          </div>
          <div className="panel-section">
            <h4>Trigger</h4>
            <pre
              className="mono"
              style={{ fontSize: 11, whiteSpace: "pre-wrap" }}
            >
              {JSON.stringify(r.trigger, null, 2)}
            </pre>
          </div>
          <div className="panel-section">
            <h4>Steps</h4>
            {r.steps.map((s, i) => (
              <details
                key={i}
                className="llm-message"
                open={s.status === "failed" || s.status === "waiting"}
              >
                <summary>
                  <span className={`pill ${pill(s.status)}`}>{s.status}</span>{" "}
                  <b>{s.name}</b> <span className="faint">{s.action}</span>
                  {s.duration_ms !== undefined && (
                    <span className="faint"> · {s.duration_ms} ms</span>
                  )}
                  {s.by && <span className="faint"> · by {s.by}</span>}
                </summary>
                {s.message && <p>{s.message}</p>}
                {s.error && <div className="error-box">{s.error}</div>}
                {s.params !== undefined && (
                  <pre
                    className="mono"
                    style={{ fontSize: 11, whiteSpace: "pre-wrap" }}
                  >
                    {JSON.stringify(s.params, null, 2)}
                  </pre>
                )}
                {s.output !== undefined && (
                  <pre
                    className="mono"
                    style={{
                      fontSize: 11,
                      whiteSpace: "pre-wrap",
                      maxHeight: 300,
                      overflow: "auto",
                    }}
                  >
                    {JSON.stringify(s.output, null, 2)}
                  </pre>
                )}
              </details>
            ))}
          </div>
        </>
      )}
    </SidePanel>
  );
}

function Runs({
  runs,
  onOpen,
}: {
  runs: WorkflowRun[];
  onOpen: (id: string) => void;
}) {
  return (
    <table className="list">
      <thead>
        <tr>
          <th>Started</th>
          <th>Workflow</th>
          <th>Trigger</th>
          <th>Status</th>
          <th>Steps</th>
          <th>Error</th>
        </tr>
      </thead>
      <tbody>
        {runs.map((r) => (
          <tr
            key={r.id}
            style={{ cursor: "pointer" }}
            onClick={() => onOpen(r.id)}
          >
            <td className="muted">{ago(r.started_at)}</td>
            <td>{r.workflow_name}</td>
            <td>{r.trigger_type}</td>
            <td>
              <span className={`pill ${pill(r.status)}`}>{r.status}</span>
            </td>
            <td>{r.steps.length}</td>
            <td className="llm-cell" style={{ maxWidth: 360 }}>
              {r.error ?? ""}
            </td>
          </tr>
        ))}
        {!runs.length && (
          <tr>
            <td colSpan={6} className="faint">
              No run yet.
            </td>
          </tr>
        )}
      </tbody>
    </table>
  );
}

export function Workflows() {
  const qc = useQueryClient();
  const wfs = useQuery({ queryKey: ["workflows"], queryFn: api.workflows });
  const runs = useQuery({
    queryKey: ["workflow-runs"],
    queryFn: () => api.workflowRuns(),
    refetchInterval: 5000,
  });
  const [editing, setEditing] = useState<{ wf: Workflow | null } | null>(null);
  const [run, setRun] = useState<string | null>(null);
  const actions = wfs.data?.actions ?? Object.keys(EXAMPLES);
  return (
    <>
      <PageHeader title="Workflow Automation">
        <button
          className="btn primary"
          onClick={() => setEditing({ wf: null })}
        >
          <PlusIcon /> New Workflow
        </button>
      </PageHeader>
      <div className="page-scroll page-padding">
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Name</th>
                <th>Triggers</th>
                <th>Steps</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {(wfs.data?.data ?? []).map((w) => {
                const t = w.attributes.triggers ?? {};
                return (
                  <tr key={w.id}>
                    <td>
                      <b>{w.attributes.name}</b>{" "}
                      {w.attributes.enabled === false && (
                        <span className="pill">disabled</span>
                      )}
                      <div className="faint" style={{ fontSize: 12 }}>
                        {w.attributes.description}
                      </div>
                    </td>
                    <td className="mono" style={{ fontSize: 12 }}>
                      @workflow-{w.attributes.name}
                      {t.security_signal &&
                        ` · signals${t.security_rule ? ` (${t.security_rule})` : ""}`}
                      {t.schedule && ` · cron ${t.schedule}`}
                    </td>
                    <td style={{ fontSize: 12 }}>
                      {w.attributes.steps.map((s) => s.action).join(" → ")}
                    </td>
                    <td style={{ textAlign: "right", whiteSpace: "nowrap" }}>
                      <button
                        className="btn"
                        onClick={async () => {
                          const r = await api.runWorkflow(w.id, {});
                          await qc.invalidateQueries({
                            queryKey: ["workflow-runs"],
                          });
                          setRun(r.data.id);
                        }}
                      >
                        Run
                      </button>{" "}
                      <button
                        className="btn"
                        onClick={() => setEditing({ wf: w })}
                      >
                        Edit
                      </button>
                      <button
                        className="btn icon ghost"
                        onClick={() =>
                          api
                            .deleteWorkflow(w.id)
                            .then(() =>
                              qc.invalidateQueries({ queryKey: ["workflows"] }),
                            )
                        }
                      >
                        <TrashIcon />
                      </button>
                    </td>
                  </tr>
                );
              })}
              {wfs.isSuccess && !wfs.data.data.length && (
                <tr>
                  <td colSpan={4} className="muted">
                    No workflow yet. A workflow chains HTTP calls, Kubernetes
                    actions (restart, scale), cases, downtimes, pages and
                    approvals.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
        <div className="section-title">Runs</div>
        <div className="card" style={{ padding: 0 }}>
          <Runs runs={runs.data?.data ?? []} onOpen={setRun} />
        </div>
      </div>
      {editing && (
        <WorkflowModal
          wf={editing.wf}
          actions={actions}
          onClose={() => setEditing(null)}
        />
      )}
      {run && <RunPanel id={run} onClose={() => setRun(null)} />}
    </>
  );
}

function AppModal({
  app,
  workflows,
  onClose,
}: {
  app: AppDef | null;
  workflows: Workflow[];
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const a = app?.attributes;
  const [name, setName] = useState(a?.name ?? "");
  const [description, setDescription] = useState(a?.description ?? "");
  const [fields, setFields] = useState(
    (a?.fields ?? []).map((f) => f.name).join(", "),
  );
  const [buttons, setButtons] = useState(
    a?.buttons ?? [{ label: "Run", workflow_id: workflows[0]?.id ?? "" }],
  );
  const [error, setError] = useState<string | null>(null);
  return (
    <Modal
      title={app ? `Edit ${a?.name}` : "New app"}
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
          <span className="form-label">Description</span>
          <input
            className="input"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />
        </label>
        <label className="form-field">
          <span className="form-label">
            Fields, comma-separated (the workflow reads them as{" "}
            {"{{ inputs.values.<field> }}"})
          </span>
          <input
            className="input mono"
            value={fields}
            onChange={(e) => setFields(e.target.value)}
          />
        </label>
        <span className="form-label">Buttons</span>
        {buttons.map((b, i) => (
          <div key={i} style={{ display: "flex", gap: 8 }}>
            <input
              className="input"
              value={b.label}
              onChange={(e) =>
                setButtons(
                  buttons.map((x, j) =>
                    j === i ? { ...x, label: e.target.value } : x,
                  ),
                )
              }
            />
            <select
              className="input"
              value={b.workflow_id}
              onChange={(e) =>
                setButtons(
                  buttons.map((x, j) =>
                    j === i ? { ...x, workflow_id: e.target.value } : x,
                  ),
                )
              }
            >
              {workflows.map((w) => (
                <option key={w.id} value={w.id}>
                  {w.attributes.name}
                </option>
              ))}
            </select>
            <button
              className="btn icon ghost"
              onClick={() => setButtons(buttons.filter((_, j) => j !== i))}
            >
              <TrashIcon />
            </button>
          </div>
        ))}
        <button
          className="btn"
          onClick={() =>
            setButtons([
              ...buttons,
              { label: "Run", workflow_id: workflows[0]?.id ?? "" },
            ])
          }
        >
          <PlusIcon /> Button
        </button>
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
              await api.saveApp(app?.id ?? null, {
                name,
                description,
                fields: fields
                  .split(",")
                  .map((f) => f.trim())
                  .filter(Boolean)
                  .map((f) => ({ name: f, label: f, type: "text" })),
                buttons,
              });
              await qc.invalidateQueries({ queryKey: ["apps"] });
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

function AppCard({
  app,
  onEdit,
  onRun,
}: {
  app: AppDef;
  onEdit: () => void;
  onRun: (run: string) => void;
}) {
  const qc = useQueryClient();
  const [values, setValues] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const a = app.attributes;
  return (
    <div className="card">
      <div
        className="card-title"
        style={{ display: "flex", alignItems: "center" }}
      >
        <span style={{ flex: 1 }}>{a.name}</span>
        <button className="btn" onClick={onEdit}>
          Edit
        </button>
        <button
          className="btn icon ghost"
          onClick={() =>
            api
              .deleteApp(app.id)
              .then(() => qc.invalidateQueries({ queryKey: ["apps"] }))
          }
        >
          <TrashIcon />
        </button>
      </div>
      {a.description && <p className="muted">{a.description}</p>}
      <div className="form">
        {a.fields.map((f) => (
          <label key={f.name} className="form-field">
            <span className="form-label">{f.label ?? f.name}</span>
            <input
              className="input"
              value={values[f.name] ?? ""}
              onChange={(e) =>
                setValues({ ...values, [f.name]: e.target.value })
              }
            />
          </label>
        ))}
        <div style={{ display: "flex", gap: 8 }}>
          {a.buttons.map((b, i) => (
            <button
              key={i}
              className="btn primary"
              onClick={async () => {
                try {
                  setError(null);
                  const r = await api.runApp(app.id, i, values);
                  onRun(r.data.id);
                } catch (e) {
                  setError(err(e));
                }
              }}
            >
              {b.label}
            </button>
          ))}
        </div>
        {error && <div className="error-box">{error}</div>}
      </div>
    </div>
  );
}

export function Apps() {
  const apps = useQuery({ queryKey: ["apps"], queryFn: api.apps });
  const wfs = useQuery({ queryKey: ["workflows"], queryFn: api.workflows });
  const [editing, setEditing] = useState<{ app: AppDef | null } | null>(null);
  const [run, setRun] = useState<string | null>(null);
  return (
    <>
      <PageHeader title="App Builder">
        <button
          className="btn primary"
          disabled={!wfs.data?.data.length}
          onClick={() => setEditing({ app: null })}
        >
          <PlusIcon /> New App
        </button>
      </PageHeader>
      <div className="page-scroll page-padding">
        {apps.isSuccess && !apps.data.data.length && (
          <div className="card muted">
            No app yet. An app is a form whose buttons run workflows with what
            was typed — e.g. “Restart a deployment” with a namespace and a name.{" "}
            {!wfs.data?.data.length && "Create a workflow first."}
          </div>
        )}
        <div className="cards">
          {(apps.data?.data ?? []).map((a) => (
            <AppCard
              key={a.id}
              app={a}
              onEdit={() => setEditing({ app: a })}
              onRun={setRun}
            />
          ))}
        </div>
      </div>
      {editing && (
        <AppModal
          app={editing.app}
          workflows={wfs.data?.data ?? []}
          onClose={() => setEditing(null)}
        />
      )}
      {run && <RunPanel id={run} onClose={() => setRun(null)} />}
    </>
  );
}
