import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link } from "react-router-dom";
import { Modal } from "../components/Common";
import { PlusIcon, TrashIcon } from "../components/Icons";
import { PageHeader } from "../components/Layout";
import {
  api,
  ApiError,
  type GeneratedMetric,
  type GeneratedSource,
} from "../lib/api";
import { RUM_TABS } from "./Events";
import { LOG_TABS } from "./LogsExplorer";
import { APM_TABS } from "./TraceExplorer";

type Source = GeneratedSource;
const RUM_EVENT_TYPES = [
  "session",
  "view",
  "action",
  "error",
  "resource",
  "long_task",
  "vital",
];
const SOURCES: Record<
  Source,
  {
    title: string;
    tabs: { to: string; label: string }[];
    noun: string;
    prefix: string;
    query: string;
    measure: string;
    groupBy: string;
    api: string;
  }
> = {
  logs: {
    title: "Logs",
    tabs: LOG_TABS,
    noun: "logs",
    prefix: "logs.",
    query: "status:error",
    measure: "",
    groupBy: "service, @http.status_code:status_code",
    api: "/api/v2/logs/config/metrics",
  },
  spans: {
    title: "APM",
    tabs: APM_TABS,
    noun: "spans",
    prefix: "spans.",
    query: "service:shop-api",
    measure: "@duration",
    groupBy: "service, env, resource_name",
    api: "/api/v2/apm/config/metrics",
  },
  rum: {
    title: "Real User Monitoring",
    tabs: RUM_TABS,
    noun: "RUM events",
    prefix: "rum.",
    query: "*",
    measure: "@view.loading_time",
    groupBy: "@view.url_path:page, @geo.country:country",
    api: "/api/v2/rum/config/metrics",
  },
};
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
  const src = SOURCES[source];
  const [name, setName] = useState(initial?.id ?? src.prefix);
  const [query, setQuery] = useState(a?.filter.query ?? src.query);
  const [type, setType] = useState(a?.compute.aggregation_type ?? "count");
  const [path, setPath] = useState(a?.compute.path ?? src.measure);
  const [eventType, setEventType] = useState(a?.event_type ?? "view");
  const [when, setWhen] = useState<"match" | "end">(
    a?.uniqueness?.when ?? "match",
  );
  const unique =
    source === "rum" && (eventType === "session" || eventType === "view");
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
    ...(source === "rum"
      ? {
          event_type: eventType,
          ...(unique ? { uniqueness: { when } } : {}),
        }
      : {}),
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
        {source === "rum" && (
          <div className="form-row">
            <label className="form-field" style={{ flex: "0 0 180px" }}>
              <span className="form-label">Event type</span>
              <select
                className="input"
                value={eventType}
                disabled={!!initial}
                onChange={(e) => setEventType(e.target.value)}
              >
                {RUM_EVENT_TYPES.map((t) => (
                  <option key={t} value={t}>
                    {t}
                  </option>
                ))}
              </select>
            </label>
            {unique && (
              <label className="form-field">
                <span className="form-label">Count each {eventType}</span>
                <select
                  className="input"
                  value={when}
                  onChange={(e) => setWhen(e.target.value as "match" | "end")}
                >
                  <option value="match">When it first matches</option>
                  <option value="end">
                    When it ends (with its final values)
                  </option>
                </select>
              </label>
            )}
          </div>
        )}
        <label className="form-field">
          <span className="form-label">
            {source === "rum"
              ? `${eventType === "session" ? "Sessions with events" : `${eventType[0].toUpperCase()}${eventType.slice(1).replace("_", " ")} events`} matching`
              : `${src.noun[0].toUpperCase()}${src.noun.slice(1)} matching`}
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
              <option value="count">
                Count of {source === "rum" ? `${eventType}s` : src.noun}
              </option>
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
                placeholder={src.measure || "@duration"}
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
            placeholder={src.groupBy}
          />
          <span className="form-hint">
            Paths become tags; `path:tag_name` renames one. Missing values are
            tagged `n/a`.
          </span>
        </label>
        <p className="muted" style={{ margin: 0 }}>
          Like Datadog, the metric counts {src.noun} ingested from now on. It is
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
      <PageHeader title={SOURCES[source].title} tabs={SOURCES[source].tabs}>
        <button className="btn primary" onClick={() => setEditing({ m: null })}>
          <PlusIcon /> New Metric
        </button>
      </PageHeader>
      <div className="page-scroll page-padding">
        <p className="muted" style={{ marginTop: 0, maxWidth: 900 }}>
          Metrics generated from {SOURCES[source].noun} as they are ingested:
          counts or distributions, tagged by attributes, kept with the other
          metrics
          {source === "rum" &&
            ", sessions and views counted once, when they match or at their end"}
          . Same API as Datadog: <code>{SOURCES[source].api}</code>.
        </p>
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Metric</th>
                {source === "rum" && <th style={{ width: 120 }}>Event</th>}
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
                    {source === "rum" && (
                      <td>
                        {a.event_type}
                        {a.uniqueness?.when === "end" && (
                          <span className="muted"> · at end</span>
                        )}
                      </td>
                    )}
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
