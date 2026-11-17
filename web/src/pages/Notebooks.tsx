import { keepPreviousData, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { TimeChart, type Series } from "../components/Charts";
import { PlusIcon, TrashIcon } from "../components/Icons";
import { PageHeader } from "../components/Layout";
import { TimePicker } from "../components/TimePicker";
import { api, type Notebook, type NotebookCell } from "../lib/api";
import { ago, statusColor, STATUS_ORDER } from "../lib/format";
import { formatValue } from "../lib/metrics";
import { useTimeRange, type TimeRange } from "../lib/time";
import { Markdown } from "./Dashboards";
import { lines } from "./Metrics";

const TABS = [
  { to: "/dashboard/lists", label: "Dashboards" },
  { to: "/notebook/list", label: "Notebooks" },
];
const HINTS: Record<NotebookCell["type"], string> = {
  markdown: "Markdown: **bold**, `code`, [links](https://…), # titles, - lists",
  metric: "A metric query: avg:system.cpu.idle{*} by {host}",
  logs: "A log search: service:shop status:error",
};
const newId = () => Math.random().toString(36).slice(2, 10);

export function NotebookList() {
  const navigate = useNavigate();
  const list = useQuery({ queryKey: ["notebooks"], queryFn: api.notebooks });
  const rows = list.data?.data ?? [];
  return (
    <>
      <PageHeader title="Dashboards" tabs={TABS}>
        <button
          className="btn primary"
          onClick={async () => {
            const n = await api.createNotebook({
              name: `Notebook of ${new Date().toLocaleDateString()}`,
              cells: [{ id: newId(), type: "markdown", text: "# What happened\n\nNotes go here." }],
            });
            navigate(`/notebook/${n.id}`);
          }}
        >
          <PlusIcon /> New Notebook
        </button>
      </PageHeader>
      <div className="page-scroll page-padding">
        <div className="card" style={{ padding: 0 }}>
          <table className="list infra-table">
            <thead>
              <tr>
                <th>Name</th>
                <th style={{ width: 100 }}>Cells</th>
                <th style={{ width: 160 }}>Modified</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((n) => (
                <tr key={n.id} onClick={() => navigate(`/notebook/${n.id}`)}>
                  <td className="infra-name">
                    <Link to={`/notebook/${n.id}`}>{n.name}</Link>
                  </td>
                  <td className="muted">{n.cells}</td>
                  <td className="muted">{ago(n.modified_at)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {list.data && rows.length === 0 && (
            <div className="empty-state">
              <h3>No notebook yet</h3>
              A notebook mixes text with metric and log graphs: for an investigation, a postmortem, a runbook.
            </div>
          )}
        </div>
      </div>
    </>
  );
}

function MetricGraph({ query, time }: { query: string; time: TimeRange }) {
  const q = useQuery({
    queryKey: ["notebook-metric", query, time.from, time.to],
    queryFn: ({ signal }) => api.queryMetrics([query], time.from, time.to, signal),
    placeholderData: keepPreviousData,
  });
  const built = useMemo(() => (q.data ? lines(q.data, time.from, time.to) : null), [q.data, time.from, time.to]);
  const error = q.data?.results[0]?.error;
  if (error) return <div className="form-error">{error}</div>;
  if (!built || built.lines.length === 0) return <div className="muted infra-nodata">{q.isLoading ? "Loading…" : "No data"}</div>;
  const unit = built.lines[0].unit;
  return (
    <TimeChart
      times={built.times}
      interval={q.data!.interval_ms}
      from={time.from}
      to={time.to}
      kind="lines"
      height={200}
      series={built.lines.map((l): Series => ({ key: l.key, label: l.label.replace(/^a: /, ""), color: l.color, values: l.values }))}
      format={(v) => formatValue(v, unit)}
      onZoom={(a, b) => time.setAbsolute(Math.round(a), Math.round(b))}
    />
  );
}

function LogsGraph({ query, time }: { query: string; time: TimeRange }) {
  const q = useQuery({
    queryKey: ["notebook-logs", query, time.from, time.to],
    queryFn: ({ signal }) => api.logsHistogram({ query, from: time.from, to: time.to }, signal),
    placeholderData: keepPreviousData,
    retry: false,
  });
  if (q.error) return <div className="form-error">{String((q.error as Error).message)}</div>;
  if (!q.data) return <div className="muted infra-nodata">Loading…</div>;
  const statuses = STATUS_ORDER.filter((s) => q.data.buckets.some((b) => b.counts[s]));
  return (
    <>
      <TimeChart
        times={q.data.buckets.map((b) => b.t)}
        interval={q.data.interval_ms}
        from={time.from}
        to={time.to}
        height={160}
        series={statuses.map((s) => ({ key: s, color: statusColor(s), values: q.data.buckets.map((b) => b.counts[s] ?? 0) }))}
        onZoom={(a, b) => time.setAbsolute(Math.round(a), Math.round(b))}
      />
      <Link className="faint" to={`/logs?query=${encodeURIComponent(query)}&from_ts=${time.from}&to_ts=${time.to}&live=false`}>
        Open in Logs
      </Link>
    </>
  );
}

function Cell({
  cell,
  time,
  onChange,
  onDelete,
  onMove,
}: {
  cell: NotebookCell;
  time: TimeRange;
  onChange: (text: string) => void;
  onDelete: () => void;
  onMove: (by: number) => void;
}) {
  const [editing, setEditing] = useState(cell.text === "");
  const [draft, setDraft] = useState(cell.text);
  useEffect(() => setDraft(cell.text), [cell.text]);
  const commit = () => {
    setEditing(false);
    if (draft !== cell.text) onChange(draft);
  };
  return (
    <div className="card notebook-cell">
      <div className="notebook-tools">
        <button className="btn ghost small" onClick={() => onMove(-1)} title="Move up">
          ↑
        </button>
        <button className="btn ghost small" onClick={() => onMove(1)} title="Move down">
          ↓
        </button>
        <button className="btn ghost small" onClick={() => (editing ? commit() : setEditing(true))}>
          {editing ? "Done" : "Edit"}
        </button>
        <button className="btn ghost small icon" onClick={onDelete} title="Delete the cell">
          <TrashIcon />
        </button>
      </div>
      {editing && (
        <>
          {cell.type === "markdown" ? (
            <textarea className="input mono" rows={Math.max(4, draft.split("\n").length + 1)} autoFocus value={draft} onChange={(e) => setDraft(e.target.value)} onBlur={commit} />
          ) : (
            <input
              className="input mono"
              autoFocus
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onBlur={commit}
              onKeyDown={(e) => e.key === "Enter" && commit()}
            />
          )}
          <div className="form-hint">{HINTS[cell.type]}</div>
        </>
      )}
      {cell.type === "markdown" && !editing && (
        <div onDoubleClick={() => setEditing(true)}>
          <Markdown text={cell.text || "_Empty_"} />
        </div>
      )}
      {cell.type !== "markdown" && cell.text && (
        <>
          {!editing && <div className="mono muted notebook-query">{cell.text}</div>}
          {cell.type === "metric" ? <MetricGraph query={cell.text} time={time} /> : <LogsGraph query={cell.text} time={time} />}
        </>
      )}
    </div>
  );
}

export function NotebookPage() {
  const { id = "" } = useParams();
  const navigate = useNavigate();
  const qc = useQueryClient();
  const time = useTimeRange("1h");
  const q = useQuery({ queryKey: ["notebook", id], queryFn: () => api.notebook(id) });
  const [nb, setNb] = useState<Notebook | null>(null);
  useEffect(() => {
    if (q.data) setNb(q.data);
  }, [q.data]);
  if (q.error) return <div className="empty-state">This notebook does not exist.</div>;
  if (!nb) return <div className="loading">Loading…</div>;

  // Every change is saved at once: a notebook has no draft state.
  const save = (next: Notebook) => {
    setNb(next);
    void api.updateNotebook(id, next).then(() => qc.invalidateQueries({ queryKey: ["notebooks"] }));
  };
  const cells = (f: (c: NotebookCell[]) => NotebookCell[]) => save({ ...nb, cells: f([...nb.cells]) });
  return (
    <>
      <PageHeader
        title={
          <input
            className="notebook-title"
            value={nb.name}
            onChange={(e) => setNb({ ...nb, name: e.target.value })}
            onBlur={() => nb.name.trim() && save(nb)}
            aria-label="Notebook name"
          />
        }
      >
        <Link className="btn ghost" to="/notebook/list">
          All notebooks
        </Link>
        <button
          className="btn ghost"
          onClick={async () => {
            await api.deleteNotebook(id);
            await qc.invalidateQueries({ queryKey: ["notebooks"] });
            navigate("/notebook/list");
          }}
        >
          Delete
        </button>
        <TimePicker time={time} />
      </PageHeader>
      <div className="page-scroll page-padding">
        <div className="notebook">
          {nb.cells.map((c, i) => (
            <Cell
              key={c.id}
              cell={c}
              time={time}
              onChange={(text) => cells((cs) => cs.map((x) => (x.id === c.id ? { ...x, text } : x)))}
              onDelete={() => cells((cs) => cs.filter((x) => x.id !== c.id))}
              onMove={(by) =>
                cells((cs) => {
                  const j = i + by;
                  if (j < 0 || j >= cs.length) return cs;
                  [cs[i], cs[j]] = [cs[j], cs[i]];
                  return cs;
                })
              }
            />
          ))}
          <div className="toolbar notebook-add">
            {(["markdown", "metric", "logs"] as const).map((t) => (
              <button key={t} className="btn small" onClick={() => cells((cs) => [...cs, { id: newId(), type: t, text: "" }])}>
                <PlusIcon /> {t === "markdown" ? "Text" : t === "metric" ? "Metric graph" : "Log graph"}
              </button>
            ))}
          </div>
        </div>
      </div>
    </>
  );
}
