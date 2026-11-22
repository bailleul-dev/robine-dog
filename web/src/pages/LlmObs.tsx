import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { TimeChart } from "../components/Charts";
import { SidePanel, StatusPill } from "../components/Common";
import { PageHeader } from "../components/Layout";
import { TimePicker } from "../components/TimePicker";
import { api, type LlmEval, type LlmSpan } from "../lib/api";
import { ago, compact, duration, percent } from "../lib/format";
import { useTimeRange } from "../lib/time";

export const LLM_TABS = [
  { to: "/llm/overview", label: "Overview" },
  { to: "/llm/traces", label: "Traces" },
  { to: "/llm/experiments", label: "Experiments" },
  { to: "/llm/agents", label: "Agents" },
];

const usd = (v: number) =>
  v === 0 ? "$0" : v < 0.01 ? `$${v.toFixed(4)}` : `$${v.toFixed(2)}`;

const KIND_COLORS: Record<string, string> = {
  workflow: "#9a82c9",
  agent: "#7a5ca8",
  llm: "#dba356",
  embedding: "#cf8f52",
  retrieval: "#5aa6c4",
  tool: "#55ab83",
  task: "#8a8291",
};

export function KindBadge({ kind }: { kind: string }) {
  return (
    <span
      className="llm-kind"
      style={{ background: KIND_COLORS[kind] ?? "#8a8291" }}
    >
      {kind}
    </span>
  );
}

function EvalChips({ evals }: { evals: LlmEval[] }) {
  return (
    <>
      {evals.map((e, i) => (
        <span key={i} className="llm-eval" title={e.type}>
          {e.label}:{" "}
          <b>
            {e.categorical ?? (e.score !== null ? e.score.toFixed(2) : "—")}
          </b>
        </span>
      ))}
    </>
  );
}

function useAppFilter() {
  const [params, setParams] = useSearchParams();
  const app = params.get("ml_app") ?? "";
  const setApp = (v: string) => {
    const p = new URLSearchParams(params);
    if (v) p.set("ml_app", v);
    else p.delete("ml_app");
    setParams(p, { replace: true });
  };
  return [app, setApp] as const;
}

function AppSelect({ apps }: { apps: { ml_app: string; traces: number }[] }) {
  const [app, setApp] = useAppFilter();
  return (
    <label className="muted">
      ML app{" "}
      <select
        className="input"
        value={app}
        onChange={(e) => setApp(e.target.value)}
      >
        <option value="">all</option>
        {apps.map((a) => (
          <option key={a.ml_app} value={a.ml_app}>
            {a.ml_app}
          </option>
        ))}
      </select>
    </label>
  );
}

export function LlmOverview() {
  const time = useTimeRange("1h", 30_000);
  const [app] = useAppFilter();
  const q = useQuery({
    queryKey: ["llm-overview", time.from, time.to, app],
    queryFn: () =>
      api.llmOverview({
        from: time.from,
        to: time.to,
        ml_app: app || undefined,
      }),
    placeholderData: keepPreviousData,
  });
  const d = q.data;
  const times = d?.series.map((p) => p.t) ?? [];
  return (
    <>
      <PageHeader title="LLM Observability" tabs={LLM_TABS}>
        <TimePicker time={time} />
      </PageHeader>
      <div className="filters-row">
        <AppSelect apps={d?.apps ?? []} />
      </div>
      <div className="page-scroll">
        {d && d.traces === 0 && (
          <div className="page-padding">
            <div className="card muted">
              No LLM trace in this period. Enable LLM Observability in ddtrace (
              <code>DD_LLMOBS_ENABLED=1</code>, <code>DD_LLMOBS_ML_APP</code>):
              spans come agentless to <code>/api/v2/llmobs</code> or through the
              Agent's EVP proxy.
            </div>
          </div>
        )}
        {d && d.traces > 0 && (
          <>
            <div className="stat-row">
              <div className="stat">
                <span className="label">Traces</span>
                <span className="value">{compact(d.traces)}</span>
              </div>
              <div className="stat">
                <span className="label">Errors</span>
                <span
                  className="value"
                  style={{ color: d.errors ? "var(--red)" : undefined }}
                >
                  {percent(d.errors / Math.max(1, d.traces))}
                </span>
              </div>
              <div className="stat">
                <span className="label">LLM calls</span>
                <span className="value">{compact(d.llm_calls)}</span>
              </div>
              <div className="stat">
                <span className="label">Tokens in / out</span>
                <span className="value">
                  {compact(d.input_tokens)} / {compact(d.output_tokens)}
                </span>
              </div>
              <div className="stat">
                <span className="label">Estimated cost</span>
                <span className="value">{usd(d.cost)}</span>
              </div>
              <div className="stat">
                <span className="label">Latency p50 · p95</span>
                <span className="value">
                  {duration(d.p50)} · {duration(d.p95)}
                </span>
              </div>
            </div>
            <div className="page-padding">
              <div className="cards">
                <div className="card">
                  <div className="card-title">Traces and errors</div>
                  <TimeChart
                    times={times}
                    interval={d.interval_ms}
                    from={time.from}
                    to={time.to}
                    height={150}
                    series={[
                      {
                        key: "ok",
                        label: "OK",
                        color: "#9a82c9",
                        values: d.series.map((p) => p.traces - p.errors),
                      },
                      {
                        key: "errors",
                        label: "Errors",
                        color: "#d8616a",
                        values: d.series.map((p) => p.errors),
                      },
                    ]}
                    onZoom={time.setAbsolute}
                  />
                </div>
                <div className="card">
                  <div className="card-title">Tokens</div>
                  <TimeChart
                    times={times}
                    interval={d.interval_ms}
                    from={time.from}
                    to={time.to}
                    height={150}
                    series={[
                      {
                        key: "tokens",
                        label: "Tokens",
                        color: "#dba356",
                        values: d.series.map((p) => p.tokens),
                      },
                    ]}
                    onZoom={time.setAbsolute}
                  />
                </div>
                <div className="card">
                  <div className="card-title">Estimated cost</div>
                  <TimeChart
                    kind="lines"
                    times={times}
                    interval={d.interval_ms}
                    from={time.from}
                    to={time.to}
                    height={150}
                    format={usd}
                    series={[
                      {
                        key: "cost",
                        label: "Cost",
                        color: "#55ab83",
                        values: d.series.map((p) => p.cost),
                      },
                    ]}
                    onZoom={time.setAbsolute}
                  />
                </div>
              </div>
              <div className="section-title">Models</div>
              <div className="card" style={{ padding: 0 }}>
                <table className="list">
                  <thead>
                    <tr>
                      <th>Model</th>
                      <th>Provider</th>
                      <th className="num">Calls</th>
                      <th className="num">Errors</th>
                      <th className="num">Input tokens</th>
                      <th className="num">Output tokens</th>
                      <th className="num">P95</th>
                      <th className="num">Estimated cost</th>
                    </tr>
                  </thead>
                  <tbody>
                    {d.models.map((m) => (
                      <tr key={m.provider + m.model} className="llm-model-row">
                        <td className="mono">{m.model}</td>
                        <td>{m.provider}</td>
                        <td className="num">{compact(m.calls)}</td>
                        <td className="num">
                          {m.errors ? percent(m.errors / m.calls) : "—"}
                        </td>
                        <td className="num">{compact(m.input_tokens)}</td>
                        <td className="num">{compact(m.output_tokens)}</td>
                        <td className="num">{duration(m.p95)}</td>
                        <td className="num">{usd(m.cost)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              <div className="section-title">Evaluations</div>
              <div className="card" style={{ padding: 0 }}>
                <table className="list">
                  <thead>
                    <tr>
                      <th>Evaluation</th>
                      <th>Type</th>
                      <th className="num">Count</th>
                      <th className="num">Average score</th>
                      <th>Most frequent value</th>
                      <th className="num">Flagged</th>
                    </tr>
                  </thead>
                  <tbody>
                    {d.evaluations.map((e) => (
                      <tr key={e.label + e.type}>
                        <td>{e.label}</td>
                        <td className="muted">{e.type}</td>
                        <td className="num">{compact(e.count)}</td>
                        <td className="num">
                          {e.avg_score !== null ? e.avg_score.toFixed(2) : "—"}
                        </td>
                        <td>{e.top_value ?? "—"}</td>
                        <td className="num">{e.flagged || "—"}</td>
                      </tr>
                    ))}
                    {!d.evaluations.length && (
                      <tr>
                        <td colSpan={6} className="faint">
                          No evaluation: post them to{" "}
                          <code>/api/intake/llm-obs/v2/eval-metric</code> (
                          <code>LLMObs.submit_evaluation</code>).
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </div>
            </div>
          </>
        )}
      </div>
    </>
  );
}

function IoBlock({ title, value }: { title: string; value: unknown }) {
  const v = value as {
    messages?: { role?: string; content?: string; tool_calls?: unknown }[];
    value?: unknown;
    documents?: { text?: string; name?: string; score?: number }[];
  } | null;
  if (!v || (typeof v === "object" && Object.keys(v).length === 0)) return null;
  return (
    <div className="panel-section">
      <h4>{title}</h4>
      {v.messages?.map((m, i) => (
        <div key={i} className={`llm-message role-${m.role ?? "user"}`}>
          <div className="llm-role">{m.role ?? "message"}</div>
          <div className="llm-content">
            {m.content ??
              (m.tool_calls ? JSON.stringify(m.tool_calls, null, 2) : "")}
          </div>
        </div>
      ))}
      {v.value !== undefined && (
        <div className="llm-message">
          <div className="llm-content">
            {typeof v.value === "string"
              ? v.value
              : JSON.stringify(v.value, null, 2)}
          </div>
        </div>
      )}
      {v.documents?.map((d, i) => (
        <div key={i} className="llm-message">
          <div className="llm-role">
            {d.name ?? "document"}{" "}
            {d.score !== undefined && (
              <span className="faint">score {d.score}</span>
            )}
          </div>
          <div className="llm-content">{d.text}</div>
        </div>
      ))}
    </div>
  );
}

function TracePanel({ id, onClose }: { id: string; onClose: () => void }) {
  const q = useQuery({
    queryKey: ["llm-trace", id],
    queryFn: () => api.llmTrace(id),
  });
  const [selected, setSelected] = useState<string | null>(null);
  const spans = q.data?.spans ?? [];
  // Depth-first order with depths, from parent links.
  const ordered = useMemo(() => {
    const byParent = new Map<string, LlmSpan[]>();
    for (const s of spans) {
      const k = s.parent_id ?? "";
      byParent.set(k, [...(byParent.get(k) ?? []), s]);
    }
    const ids = new Set(spans.map((s) => s.span_id));
    const out: { s: LlmSpan; depth: number }[] = [];
    const walk = (s: LlmSpan, depth: number) => {
      out.push({ s, depth });
      for (const c of byParent.get(s.span_id) ?? []) walk(c, depth + 1);
    };
    for (const s of spans)
      if (!s.parent_id || !ids.has(s.parent_id)) walk(s, 0);
    return out;
  }, [spans]);
  const root = ordered[0]?.s;
  const span = spans.find((s) => s.span_id === selected) ?? root;
  const t0 = root?.start_ns ?? 0;
  const total = Math.max(1, root?.duration ?? 1);
  const meta = (span?.body.meta ?? {}) as Record<string, unknown>;
  const evals = (q.data?.evals ?? []).filter(
    (e) => e.span_id === span?.span_id,
  );
  return (
    <SidePanel
      onClose={onClose}
      storageKey="llm-trace"
      header={
        <>
          <span style={{ fontWeight: 600 }}>{root?.name ?? "Trace"}</span>
          {root && <span className="muted">{root.ml_app}</span>}
          {root?.error && <StatusPill status="error" />}
        </>
      }
    >
      <div className="panel-section">
        <h4>Spans</h4>
        {ordered.map(({ s, depth }) => (
          <div
            key={s.span_id}
            className={`llm-span-row${s.span_id === span?.span_id ? " selected" : ""}`}
            onClick={() => setSelected(s.span_id)}
          >
            <span
              style={{
                paddingLeft: depth * 16,
                minWidth: 0,
                flex: "0 0 42%",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              <KindBadge kind={s.kind} /> {s.name}
              {s.error && <span style={{ color: "var(--red)" }}> ●</span>}
            </span>
            <span className="llm-bar-track">
              <span
                className="llm-bar"
                style={{
                  left: `${((s.start_ns - t0) / total) * 100}%`,
                  width: `${Math.max(0.5, (s.duration / total) * 100)}%`,
                  background: KIND_COLORS[s.kind] ?? "#8a8291",
                }}
              />
            </span>
            <span
              className="mono faint"
              style={{ width: 70, textAlign: "right" }}
            >
              {duration(s.duration)}
            </span>
          </div>
        ))}
      </div>
      {span && (
        <>
          <div className="panel-section">
            <h4>
              <KindBadge kind={span.kind} /> {span.name}
            </h4>
            <dl className="kv-grid">
              <dt>Duration</dt>
              <dd>{duration(span.duration)}</dd>
              {span.model_name && (
                <>
                  <dt>Model</dt>
                  <dd className="mono">
                    {span.model_provider ? `${span.model_provider} / ` : ""}
                    {span.model_name}
                  </dd>
                </>
              )}
              {span.total_tokens !== null && (
                <>
                  <dt>Tokens</dt>
                  <dd>
                    {span.input_tokens ?? 0} in · {span.output_tokens ?? 0} out
                    · {span.total_tokens} total
                  </dd>
                </>
              )}
              {span.cost !== null && (
                <>
                  <dt>Estimated cost</dt>
                  <dd>{usd(span.cost)}</dd>
                </>
              )}
              {span.session_id && (
                <>
                  <dt>Session</dt>
                  <dd className="mono">{span.session_id}</dd>
                </>
              )}
              <dt>Tags</dt>
              <dd style={{ fontSize: 12 }}>{span.tags.join(", ")}</dd>
            </dl>
            {evals.length > 0 && (
              <div style={{ marginTop: 8 }}>
                <EvalChips evals={evals} />
              </div>
            )}
          </div>
          {typeof meta["error.message"] === "string" &&
            meta["error.message"] && (
              <div className="panel-section">
                <h4>Error</h4>
                <div className="error-box">{String(meta["error.message"])}</div>
              </div>
            )}
          <IoBlock title="Input" value={meta.input} />
          <IoBlock title="Output" value={meta.output} />
          {meta.metadata !== undefined && (
            <div className="panel-section">
              <h4>Metadata</h4>
              <pre className="mono" style={{ fontSize: 12, margin: 0 }}>
                {JSON.stringify(meta.metadata, null, 2)}
              </pre>
            </div>
          )}
        </>
      )}
    </SidePanel>
  );
}

export function LlmTraces() {
  const time = useTimeRange("1h", 30_000);
  const [params, setParams] = useSearchParams();
  const [app] = useAppFilter();
  const [query, setQuery] = useState(params.get("query") ?? "");
  const kind = params.get("kind") ?? "";
  const errors = params.get("errors") === "true";
  const open = params.get("trace");
  const set = (k: string, v: string) => {
    const p = new URLSearchParams(params);
    if (v) p.set(k, v);
    else p.delete(k);
    setParams(p, { replace: k !== "trace" });
  };
  const filter = {
    from: time.from,
    to: time.to,
    ml_app: app || undefined,
    query: params.get("query") || undefined,
    kind: kind || undefined,
    errors: errors || undefined,
  };
  const list = useQuery({
    queryKey: ["llm-traces", filter],
    queryFn: () => api.llmTraces(filter),
    placeholderData: keepPreviousData,
  });
  const apps = useQuery({
    queryKey: ["llm-overview-apps", time.from, time.to],
    queryFn: () => api.llmOverview({ from: time.from, to: time.to }),
  });
  return (
    <>
      <PageHeader title="LLM Observability" tabs={LLM_TABS}>
        <TimePicker time={time} />
      </PageHeader>
      <div className="filters-row">
        <AppSelect apps={apps.data?.apps ?? []} />
        <form
          onSubmit={(e) => {
            e.preventDefault();
            set("query", query);
          }}
          style={{ flex: 1, display: "flex" }}
        >
          <input
            className="input"
            style={{ flex: 1 }}
            placeholder="Search prompts, answers and span names"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </form>
        <label className="muted">
          Spans{" "}
          <select
            className="input"
            value={kind}
            onChange={(e) => set("kind", e.target.value)}
          >
            <option value="">traces (root spans)</option>
            {Object.keys(KIND_COLORS).map((k) => (
              <option key={k} value={k}>
                {k}
              </option>
            ))}
          </select>
        </label>
        <label className="form-check">
          <input
            type="checkbox"
            checked={errors}
            onChange={(e) => set("errors", e.target.checked ? "true" : "")}
          />{" "}
          Errors only
        </label>
      </div>
      <div className="page-scroll">
        <table className="list">
          <thead>
            <tr>
              <th style={{ width: 90 }}>Date</th>
              <th>Span</th>
              <th>Input</th>
              <th>Output</th>
              <th className="num">Duration</th>
              <th className="num">Tokens</th>
              <th className="num">Cost</th>
              <th>Evaluations</th>
            </tr>
          </thead>
          <tbody>
            {(list.data ?? []).map((t) => (
              <tr
                key={t.span_id}
                className="llm-trace-row"
                onClick={() => set("trace", t.trace_id)}
              >
                <td className="muted">{ago(t.start_ns / 1e6)}</td>
                <td>
                  <KindBadge kind={t.kind} /> {t.name}
                  {t.error && <StatusPill status="error" />}
                  <div className="faint" style={{ fontSize: 11 }}>
                    {t.ml_app} · {t.spans} spans
                  </div>
                </td>
                <td className="llm-cell">{t.input}</td>
                <td className="llm-cell">{t.output}</td>
                <td className="num mono">{duration(t.duration)}</td>
                <td className="num">{t.tokens ? compact(t.tokens) : "—"}</td>
                <td className="num">{t.cost ? usd(t.cost) : "—"}</td>
                <td>
                  <EvalChips evals={t.evals} />
                </td>
              </tr>
            ))}
            {list.isSuccess && !list.data.length && (
              <tr>
                <td colSpan={8} className="muted">
                  No trace matches.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      {open && <TracePanel id={open} onClose={() => set("trace", "")} />}
    </>
  );
}
