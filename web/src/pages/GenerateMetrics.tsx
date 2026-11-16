import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link } from "react-router-dom";
import { Modal } from "../components/Common";
import { PlusIcon, TrashIcon } from "../components/Icons";
import { PageHeader } from "../components/Layout";
import { api, ApiError, type GeneratedMetric } from "../lib/api";
import { LOG_TABS } from "./LogsExplorer";
import { APM_TABS } from "./TraceExplorer";

type Source = "logs" | "spans";
const errorText = (e: unknown) =>
  e instanceof ApiError ? e.message : e ? String(e) : null;

function MetricModal({
  source,
  initial,
  onClose,
}: {
  source: Source;
  initial: GeneratedMetric | null;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const a = initial?.attributes;
  const [name, setName] = useState(
    initial?.id ?? (source === "logs" ? "logs." : "spans."),
  );
  const [query, setQuery] = useState(
    a?.filter.query ??
      (source === "logs" ? "status:error" : "service:shop-api"),
  );
  const [type, setType] = useState(a?.compute.aggregation_type ?? "count");
  const [path, setPath] = useState(
    a?.compute.path ?? (source === "spans" ? "@duration" : ""),
  );
  const [percentiles, setPercentiles] = useState(
    a?.compute.include_percentiles ?? false,
  );
  const [groupBy, setGroupBy] = useState(
    (a?.group_by ?? [])
      .map((g) =>
        g.tag_name && g.tag_name !== g.path.replace(/^@/, "")
          ? `${g.path}:${g.tag_name}`
          : g.path,
      )
      .join(", "),
  );
  const [error, setError] = useState<string | null>(null);
  const attributes = {
    compute: {
      aggregation_type: type,
      ...(type === "distribution"
        ? { path, include_percentiles: percentiles }
        : {}),
    },
    filter: { query: query.trim() || "*" },
    group_by: groupBy
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean)
      .map((g) => {
        const [p, tag] = g.split(/:(?=[^:]*$)/);
        return tag
          ? { path: p, tag_name: tag }
          : { path: p, tag_name: p.replace(/^@/, "") };
      }),
  };
  return (
    <Modal
      title={initial ? `Edit ${initial.id}` : "Generate a metric"}
      onClose={onClose}
      width={640}
    >
      <div className="modal-body form">
        <label className="form-field">
          <span className="form-label">Metric name</span>
          <input
            className="input mono"
            value={name}
            disabled={!!initial}
            onChange={(e) => setName(e.target.value)}
          />
        </label>
        <label className="form-field">
          <span className="form-label">
            {source === "logs" ? "Logs matching" : "Spans matching"}
          </span>
          <input
            className="input mono"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="*"
          />
        </label>
        <div className="form-row">
          <label className="form-field" style={{ flex: "0 0 180px" }}>
            <span className="form-label">Compute</span>
            <select
              className="input"
              value={type}
              disabled={!!initial}
              onChange={(e) => setType(e.target.value)}
            >
              <option value="count">Count of {source}</option>
              <option value="distribution">Distribution of</option>
            </select>
          </label>
          {type === "distribution" && (
            <label className="form-field">
              <span className="form-label">Measure</span>
              <input
                className="input mono"
                value={path}
                disabled={!!initial}
                onChange={(e) => setPath(e.target.value)}
                placeholder="@duration"
              />
            </label>
          )}
        </div>
        {type === "distribution" && (
          <label className="form-check">
            <input
              type="checkbox"
              checked={percentiles}
              onChange={(e) => setPercentiles(e.target.checked)}
            />{" "}
            Include percentiles
          </label>
        )}
        <label className="form-field">
          <span className="form-label">Group by</span>
          <input
            className="input mono"
            value={groupBy}
            onChange={(e) => setGroupBy(e.target.value)}
            placeholder={
              source === "logs"
                ? "service, @http.status_code:status_code"
                : "service, env, resource_name"
            }
          />
          <span className="form-hint">
            Paths become tags; `path:tag_name` renames one. Missing values are
            tagged `n/a`.
          </span>
        </label>
        <p className="muted" style={{ margin: 0 }}>
          Like Datadog, the metric counts {source} ingested from now on. It is
          queried like any metric:{" "}
          <code>
            {type === "count" ? `sum:${name}{*}.as_count()` : `avg:${name}{*}`}
          </code>
          .
        </p>
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
              if (initial)
                await api.updateGeneratedMetric(source, initial.id, attributes);
              else
                await api.createGeneratedMetric(
                  source,
                  name.trim(),
                  attributes,
                );
              await qc.invalidateQueries({ queryKey: ["generated", source] });
              onClose();
            } catch (e) {
              setError(errorText(e));
            }
          }}
        >
          {initial ? "Save" : "Create metric"}
        </button>
      </div>
    </Modal>
  );
}

export function GenerateMetrics({ source }: { source: Source }) {
  const qc = useQueryClient();
  const list = useQuery({
    queryKey: ["generated", source],
    queryFn: () => api.generatedMetrics(source),
  });
  const [editing, setEditing] = useState<{ m: GeneratedMetric | null } | null>(
    null,
  );
  return (
    <>
      <PageHeader
        title={source === "logs" ? "Logs" : "APM"}
        tabs={source === "logs" ? LOG_TABS : APM_TABS}
      >
        <button className="btn primary" onClick={() => setEditing({ m: null })}>
          <PlusIcon /> New Metric
        </button>
      </PageHeader>
      <div className="page-scroll page-padding">
        <p className="muted" style={{ marginTop: 0, maxWidth: 900 }}>
          Metrics generated from {source === "logs" ? "logs" : "spans"} as they
          are ingested: counts or distributions, tagged by attributes, kept with
          the other metrics. Same API as Datadog:{" "}
          <code>
            {source === "logs"
              ? "/api/v2/logs/config/metrics"
              : "/api/v2/apm/config/metrics"}
          </code>
          .
        </p>
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Metric</th>
                <th>Filter</th>
                <th style={{ width: 200 }}>Compute</th>
                <th>Group by</th>
                <th style={{ width: 200 }} />
              </tr>
            </thead>
            <tbody>
              {(list.data ?? []).map((m) => {
                const a = m.attributes;
                const q =
                  a.compute.aggregation_type === "count"
                    ? `sum:${m.id}{*}.as_count()`
                    : `avg:${m.id}{*}`;
                return (
                  <tr key={m.id}>
                    <td className="mono">
                      <Link
                        to={`/metrics/explorer?${new URLSearchParams({ q })}`}
                      >
                        {m.id}
                      </Link>
                    </td>
                    <td className="mono ellipsis">{a.filter.query}</td>
                    <td>
                      {a.compute.aggregation_type === "count"
                        ? "Count"
                        : `Distribution of ${a.compute.path}`}
                    </td>
                    <td className="mono ellipsis">
                      {a.group_by.map((g) => g.tag_name ?? g.path).join(", ") ||
                        "—"}
                    </td>
                    <td>
                      <span
                        className="row-actions"
                        style={{ justifyContent: "flex-end" }}
                      >
                        <button
                          className="btn ghost small"
                          onClick={() => setEditing({ m })}
                        >
                          Edit
                        </button>
                        <button
                          className="btn ghost small"
                          title="Delete"
                          onClick={async () => {
                            if (
                              !window.confirm(
                                `Delete the metric ${m.id}? Its points stay.`,
                              )
                            )
                              return;
                            await api.deleteGeneratedMetric(source, m.id);
                            await qc.invalidateQueries({
                              queryKey: ["generated", source],
                            });
                          }}
                        >
                          <TrashIcon />
                        </button>
                      </span>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          {list.data && !list.data.length && (
            <div className="empty-state">No generated metric yet.</div>
          )}
        </div>
      </div>
      {editing && (
        <MetricModal
          source={source}
          initial={editing.m}
          onClose={() => setEditing(null)}
        />
      )}
    </>
  );
}
