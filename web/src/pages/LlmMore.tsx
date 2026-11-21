import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { PageHeader } from "../components/Layout";
import { TimePicker } from "../components/TimePicker";
import { api, LlmSpan } from "../lib/api";
import { ago, duration, thousands } from "../lib/format";
import { useTimeRange } from "../lib/time";
import { KindBadge, LLM_TABS } from "./LlmObs";

const show = (v: unknown): string => {
  if (v == null) return "";
  if (typeof v === "number")
    return Number.isInteger(v) ? String(v) : v.toFixed(3);
  if (typeof v === "string") return v;
  return JSON.stringify(v);
};

/** Experiments: runs of a task over a dataset, compared by their evaluations. */
export function LlmExperiments() {
  const q = useQuery({
    queryKey: ["llm-experiments"],
    queryFn: api.llmExperiments,
  });
  const [open, setOpen] = useState<string | null>(null);
  const d = q.data;
  const byDataset = new Map<string, NonNullable<typeof d>["experiments"]>();
  for (const e of d?.experiments ?? []) {
    const k = String(e.dataset ?? e.dataset_id);
    byDataset.set(k, [...(byDataset.get(k) ?? []), e]);
  }
  return (
    <>
      <PageHeader title="LLM Observability" tabs={LLM_TABS} />
      <div className="filters-row">
        <span className="muted">
          ddtrace's <code>LLMObs.create_dataset()</code> and{" "}
          <code>LLMObs.experiment()</code>, with <code>DD_SITE</code> pointing
          here or through the Agent. {d?.datasets.length ?? 0} datasets.
        </span>
      </div>
      <div className="page-scroll page-padding">
        {[...byDataset].map(([ds, list]) => {
          const labels = [
            ...new Set(list.flatMap((e) => Object.keys(e.summary))),
          ].sort();
          return (
            <div key={ds}>
              <div className="section-title">Dataset {ds}</div>
              <div className="card" style={{ padding: 0 }}>
                <table className="list">
                  <thead>
                    <tr>
                      <th>Experiment</th>
                      <th>Status</th>
                      <th className="num">Rows</th>
                      <th className="num">Errors</th>
                      {labels.map((l) => (
                        <th className="num" key={l}>
                          {l}
                        </th>
                      ))}
                      <th>Created</th>
                    </tr>
                  </thead>
                  <tbody>
                    {list.map((e) => (
                      <tr
                        key={e.id}
                        onClick={() => setOpen(open === e.id ? null : e.id)}
                      >
                        <td>
                          <b>{e.name}</b>{" "}
                          <span className="faint">
                            {e.config ? JSON.stringify(e.config) : ""}
                          </span>
                        </td>
                        <td>{e.status}</td>
                        <td className="num">{e.spans}</td>
                        <td className="num">{e.errors || ""}</td>
                        {labels.map((l) => (
                          <td className="num mono" key={l}>
                            {show(e.summary[l])}
                          </td>
                        ))}
                        <td className="muted">
                          {e.created_at ? ago(e.created_at) : ""}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              {open && list.some((e) => e.id === open) && (
                <ExperimentRows id={open} />
              )}
            </div>
          );
        })}
        {d && !d.experiments.length && (
          <div className="muted">No experiment yet.</div>
        )}
      </div>
    </>
  );
}

function ExperimentRows({ id }: { id: string }) {
  const q = useQuery({
    queryKey: ["llm-experiment", id],
    queryFn: () => api.llmExperiment(id),
  });
  const rows = q.data?.rows ?? [];
  const labels = [...new Set(rows.flatMap((r) => Object.keys(r.evals)))].sort();
  return (
    <div className="card" style={{ padding: 0, marginTop: 8 }}>
      <table className="list">
        <thead>
          <tr>
            <th>Input</th>
            <th>Output</th>
            <th>Expected</th>
            {labels.map((l) => (
              <th key={l}>{l}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.span_id}>
              <td
                className="mono"
                style={{ maxWidth: 320, whiteSpace: "pre-wrap" }}
              >
                {show(r.input)}
              </td>
              <td
                className="mono"
                style={{ maxWidth: 320, whiteSpace: "pre-wrap" }}
              >
                {r.error ? (
                  <span className="status-pill error">{show(r.error)}</span>
                ) : (
                  show(r.output)
                )}
              </td>
              <td
                className="mono muted"
                style={{ maxWidth: 240, whiteSpace: "pre-wrap" }}
              >
                {show(r.expected_output)}
              </td>
              {labels.map((l) => (
                <td className="mono" key={l}>
                  {show(r.evals[l])}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** Agent Observability: agents, the tools they call, and each run's steps. */
export function LlmAgents() {
  const time = useTimeRange("1d", 60_000);
  const q = useQuery({
    queryKey: ["llm-agents", time.from, time.to],
    queryFn: () => api.llmAgents(time.from, time.to),
    placeholderData: keepPreviousData,
  });
  const [run, setRun] = useState<string | null>(null);
  const d = q.data;
  return (
    <>
      <PageHeader title="LLM Observability" tabs={LLM_TABS}>
        <TimePicker time={time} />
      </PageHeader>
      <div className="page-scroll page-padding">
        <div className="section-title">Agents</div>
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Agent</th>
                <th>App</th>
                <th className="num">Runs</th>
                <th className="num">Errors</th>
                <th className="num">Avg duration</th>
                <th className="num">Tool calls / run</th>
                <th className="num">LLM calls / run</th>
                <th className="num">Tokens / run</th>
              </tr>
            </thead>
            <tbody>
              {d?.agents.map((a) => (
                <tr
                  key={a.ml_app + a.name}
                  onClick={() => setRun(a.sample_trace)}
                >
                  <td>
                    <b>{a.name}</b>
                  </td>
                  <td className="muted">{a.ml_app}</td>
                  <td className="num">{thousands(a.runs)}</td>
                  <td className="num">{a.errors || ""}</td>
                  <td className="num">{duration(a.avg_ns)}</td>
                  <td className="num">{a.tools.toFixed(1)}</td>
                  <td className="num">{a.llms.toFixed(1)}</td>
                  <td className="num">{thousands(Math.round(a.tokens))}</td>
                </tr>
              ))}
              {d && !d.agents.length && (
                <tr>
                  <td colSpan={8} className="muted">
                    No <code>agent</code> span in this range (
                    <code>@agent</code> / <code>LLMObs.agent()</code>).
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
        {run && <AgentRun traceId={run} />}
        <div className="section-title">Tools</div>
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Tool</th>
                <th>App</th>
                <th className="num">Calls</th>
                <th className="num">Errors</th>
                <th className="num">Avg duration</th>
              </tr>
            </thead>
            <tbody>
              {d?.tools.map((t) => (
                <tr key={t.ml_app + t.name}>
                  <td>{t.name}</td>
                  <td className="muted">{t.ml_app}</td>
                  <td className="num">{thousands(t.calls)}</td>
                  <td className="num">{t.errors || ""}</td>
                  <td className="num">{duration(t.avg_ns)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <div className="section-title">Recent runs</div>
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <tbody>
              {d?.runs.map((r) => (
                <tr
                  key={r.trace_id}
                  onClick={() => setRun(r.trace_id)}
                  className={run === r.trace_id ? "selected" : ""}
                >
                  <td>{r.name}</td>
                  <td className="muted">{r.ml_app}</td>
                  <td className="num">{duration(r.duration)}</td>
                  <td>
                    {r.error && (
                      <span className="status-pill error">error</span>
                    )}
                  </td>
                  <td className="muted">{ago(r.start_ns / 1e6)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    </>
  );
}

/** One run as a graph: each step under its parent, in order. */
function AgentRun({ traceId }: { traceId: string }) {
  const q = useQuery({
    queryKey: ["llm-trace", traceId],
    queryFn: () => api.llmTrace(traceId),
  });
  const spans = q.data?.spans ?? [];
  const children = new Map<string, LlmSpan[]>();
  const ids = new Set(spans.map((s) => s.span_id));
  const roots: LlmSpan[] = [];
  for (const s of spans) {
    if (s.parent_id && ids.has(s.parent_id))
      children.set(s.parent_id, [...(children.get(s.parent_id) ?? []), s]);
    else roots.push(s);
  }
  const node = (s: LlmSpan): React.ReactNode => {
    const kids = children.get(s.span_id) ?? [];
    return (
      <div key={s.span_id} className="agent-node">
        <div
          className={`agent-step${s.error ? " error" : ""}`}
          title={s.output.slice(0, 400)}
        >
          <KindBadge kind={s.kind} /> <b>{s.name}</b>{" "}
          <span className="faint">
            {duration(s.duration)}
            {s.total_tokens ? ` · ${s.total_tokens} tokens` : ""}
            {s.model_name ? ` · ${s.model_name}` : ""}
          </span>
        </div>
        {kids.length > 0 && (
          <div className="agent-children">{kids.map(node)}</div>
        )}
      </div>
    );
  };
  return (
    <div className="card" style={{ marginTop: 8 }}>
      <div className="card-title">Run {traceId}</div>
      {roots.map(node)}
    </div>
  );
}
