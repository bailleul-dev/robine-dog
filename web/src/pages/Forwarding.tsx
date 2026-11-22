import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Modal } from "../components/Common";
import { PageHeader } from "../components/Layout";
import { PlusIcon, TrashIcon } from "../components/Icons";
import { api } from "../lib/api";
import { ago, thousands } from "../lib/format";

/** Metrics without Limits: the tags each metric keeps. */
export function MetricTags() {
  const qc = useQueryClient();
  const q = useQuery({
    queryKey: ["metric-tag-configs"],
    queryFn: api.metricTagConfigs,
  });
  const [edit, setEdit] = useState<{
    metric: string;
    tags: string;
    exclude: boolean;
    type: string;
  } | null>(null);
  const [error, setError] = useState("");
  const save = useMutation({
    mutationFn: () =>
      api.putMetricTags(edit!.metric.trim(), {
        tags: edit!.tags
          .split(/[\s,]+/)
          .map((t) => t.trim())
          .filter(Boolean),
        exclude_tags_mode: edit!.exclude,
        metric_type: edit!.type,
      }),
    onSuccess: () => {
      setEdit(null);
      qc.invalidateQueries({ queryKey: ["metric-tag-configs"] });
    },
    onError: (e: Error) => setError(e.message),
  });
  const remove = useMutation({
    mutationFn: (m: string) => api.deleteMetricTags(m),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["metric-tag-configs"] }),
  });
  return (
    <>
      <PageHeader title="Metrics without Limits">
        <button
          className="btn primary"
          onClick={() =>
            setEdit({ metric: "", tags: "", exclude: false, type: "gauge" })
          }
        >
          <PlusIcon /> Configure a metric
        </button>
      </PageHeader>
      <div className="filters-row">
        <span className="muted">
          The other tags are aggregated away when points are written (counts and
          rates summed, gauges averaged); queries by them no longer split. Also{" "}
          <code>/api/v2/metrics/&#123;name&#125;/tags</code>.
        </span>
      </div>
      <div className="page-scroll page-padding">
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Metric</th>
                <th>Type</th>
                <th>Tags</th>
                <th className="num">Series ingested</th>
                <th className="num">Series stored</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {q.data?.data.map((c) => (
                <tr
                  key={c.metric}
                  onClick={() =>
                    setEdit({
                      metric: c.metric,
                      tags: c.tags.join(", "),
                      exclude: !!c.exclude_tags_mode,
                      type: c.metric_type ?? "gauge",
                    })
                  }
                >
                  <td className="mono">{c.metric}</td>
                  <td>{c.metric_type}</td>
                  <td>
                    {c.exclude_tags_mode ? "all but " : "only "}
                    {c.tags.map((t) => (
                      <span className="tag" key={t}>
                        {t}
                      </span>
                    ))}
                  </td>
                  <td className="num">{thousands(c.ingested_series)}</td>
                  <td className="num">{thousands(c.indexed_series)}</td>
                  <td>
                    <button
                      className="btn icon ghost"
                      title="Keep every tag again"
                      onClick={(e) => {
                        e.stopPropagation();
                        remove.mutate(c.metric);
                      }}
                    >
                      <TrashIcon />
                    </button>
                  </td>
                </tr>
              ))}
              {q.data && !q.data.data.length && (
                <tr>
                  <td colSpan={6} className="muted">
                    Every metric keeps all its tags.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
        <p className="faint">Series counts are for the current hour.</p>
      </div>
      {edit && (
        <Modal
          title="Tags of a metric"
          onClose={() => setEdit(null)}
          width={460}
        >
          <div className="modal-body">
            <label className="form-field">
              Metric
              <input
                className="input mono"
                value={edit.metric}
                onChange={(e) => setEdit({ ...edit, metric: e.target.value })}
              />
            </label>
            <label className="form-field">
              Type
              <select
                className="input"
                value={edit.type}
                onChange={(e) => setEdit({ ...edit, type: e.target.value })}
              >
                {["gauge", "count", "rate", "distribution"].map((t) => (
                  <option key={t}>{t}</option>
                ))}
              </select>
            </label>
            <label className="form-field">
              Tag keys
              <input
                className="input mono"
                placeholder="env, service"
                value={edit.tags}
                onChange={(e) => setEdit({ ...edit, tags: e.target.value })}
              />
            </label>
            <label className="muted">
              <input
                type="checkbox"
                checked={edit.exclude}
                onChange={(e) =>
                  setEdit({ ...edit, exclude: e.target.checked })
                }
              />{" "}
              Drop these tags instead of keeping only them
            </label>
            {error && <div className="form-error">{error}</div>}
          </div>
          <div className="modal-footer">
            <button
              className="btn primary"
              disabled={!edit.metric.trim()}
              onClick={() => save.mutate()}
            >
              Save
            </button>
          </div>
        </Modal>
      )}
    </>
  );
}

type Dest = {
  name: string;
  query: string;
  enabled: boolean;
  type: string;
  endpoint: string;
  secret: string;
  user: string;
  index: string;
};

/** Log Forwarding: custom destinations. */
export function LogForwarding() {
  const qc = useQueryClient();
  const q = useQuery({
    queryKey: ["custom-destinations"],
    queryFn: api.customDestinations,
    refetchInterval: 10_000,
  });
  const [edit, setEdit] = useState<(Dest & { id?: string }) | null>(null);
  const [error, setError] = useState("");
  const save = useMutation({
    mutationFn: () => {
      const e = edit!;
      const d: Record<string, unknown> = { type: e.type, endpoint: e.endpoint };
      if (e.type === "splunk_hec" && e.secret) d.access_token = e.secret;
      if (e.type === "elasticsearch") {
        d.index_name = e.index || "datadog-logs";
        if (e.user)
          d.auth = {
            username: e.user,
            ...(e.secret ? { password: e.secret } : {}),
          };
      }
      if (e.type === "http" && e.user)
        d.auth = {
          type: "basic",
          username: e.user,
          ...(e.secret ? { password: e.secret } : {}),
        };
      if (e.type === "http" && !e.user && e.secret)
        d.auth = {
          type: "custom_header",
          header_name: "Authorization",
          header_value: e.secret,
        };
      const attrs = {
        name: e.name,
        query: e.query,
        enabled: e.enabled,
        forwarder_destination: d,
      };
      return e.id
        ? api.updateCustomDestination(e.id, attrs)
        : api.createCustomDestination(attrs);
    },
    onSuccess: () => {
      setEdit(null);
      qc.invalidateQueries({ queryKey: ["custom-destinations"] });
    },
    onError: (e: Error) => setError(e.message),
  });
  const remove = useMutation({
    mutationFn: (id: string) => api.deleteCustomDestination(id),
    onSuccess: () =>
      qc.invalidateQueries({ queryKey: ["custom-destinations"] }),
  });
  const blank: Dest = {
    name: "",
    query: "",
    enabled: true,
    type: "http",
    endpoint: "",
    secret: "",
    user: "",
    index: "",
  };
  return (
    <>
      <PageHeader title="Log Forwarding">
        <button className="btn primary" onClick={() => setEdit(blank)}>
          <PlusIcon /> New destination
        </button>
      </PageHeader>
      <div className="filters-row">
        <span className="muted">
          Logs matching a destination's query are sent after the pipelines, in
          batches: any HTTP endpoint (JSON array), Splunk HEC or Elasticsearch.
        </span>
      </div>
      <div className="page-scroll page-padding">
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Destination</th>
                <th>Type</th>
                <th>Query</th>
                <th className="num">Sent</th>
                <th className="num">Dropped</th>
                <th>Status</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {q.data?.data.map(({ id, attributes: a }) => (
                <tr
                  key={id}
                  onClick={() =>
                    setEdit({
                      id,
                      name: a.name,
                      query: a.query,
                      enabled: a.enabled,
                      type: a.forwarder_destination.type,
                      endpoint: a.forwarder_destination.endpoint,
                      secret: "",
                      user: a.forwarder_destination.auth?.username ?? "",
                      index: a.forwarder_destination.index_name ?? "",
                    })
                  }
                >
                  <td>
                    <b>{a.name}</b>{" "}
                    <span className="faint mono">
                      {a.forwarder_destination.endpoint}
                    </span>
                  </td>
                  <td>{a.forwarder_destination.type}</td>
                  <td className="mono">{a.query || "*"}</td>
                  <td className="num">{thousands(a.status.sent)}</td>
                  <td className="num">
                    {a.status.dropped ? thousands(a.status.dropped) : ""}
                  </td>
                  <td>
                    {!a.enabled ? (
                      <span className="muted">disabled</span>
                    ) : a.status.last_error ? (
                      <span
                        className="status-pill error"
                        title={a.status.last_error}
                      >
                        failing
                      </span>
                    ) : (
                      <span className="muted">
                        {a.status.last_sent
                          ? `sent ${ago(a.status.last_sent)}`
                          : "waiting for logs"}
                      </span>
                    )}
                  </td>
                  <td>
                    <button
                      className="btn icon ghost"
                      onClick={(e) => {
                        e.stopPropagation();
                        remove.mutate(id);
                      }}
                    >
                      <TrashIcon />
                    </button>
                  </td>
                </tr>
              ))}
              {q.data && !q.data.data.length && (
                <tr>
                  <td colSpan={7} className="muted">
                    No destination.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </div>
      {edit && (
        <Modal
          title={edit.id ? "Edit destination" : "New destination"}
          onClose={() => setEdit(null)}
          width={520}
        >
          <div className="modal-body">
            <label className="form-field">
              Name
              <input
                className="input"
                value={edit.name}
                onChange={(e) => setEdit({ ...edit, name: e.target.value })}
              />
            </label>
            <label className="form-field">
              Logs to forward (query; empty for all)
              <input
                className="input mono"
                value={edit.query}
                onChange={(e) => setEdit({ ...edit, query: e.target.value })}
              />
            </label>
            <label className="form-field">
              Type
              <select
                className="input"
                value={edit.type}
                onChange={(e) => setEdit({ ...edit, type: e.target.value })}
              >
                <option value="http">HTTP</option>
                <option value="splunk_hec">Splunk HEC</option>
                <option value="elasticsearch">Elasticsearch</option>
              </select>
            </label>
            <label className="form-field">
              Endpoint
              <input
                className="input mono"
                placeholder="https://…"
                value={edit.endpoint}
                onChange={(e) => setEdit({ ...edit, endpoint: e.target.value })}
              />
            </label>
            {edit.type !== "splunk_hec" && (
              <label className="form-field">
                Username (basic auth, optional)
                <input
                  className="input"
                  value={edit.user}
                  onChange={(e) => setEdit({ ...edit, user: e.target.value })}
                />
              </label>
            )}
            <label className="form-field">
              {edit.type === "splunk_hec"
                ? "HEC token"
                : edit.type === "http" && !edit.user
                  ? "Authorization header (optional)"
                  : "Password"}
              {edit.id ? " (empty: unchanged)" : ""}
              <input
                className="input"
                type="password"
                value={edit.secret}
                onChange={(e) => setEdit({ ...edit, secret: e.target.value })}
              />
            </label>
            {edit.type === "elasticsearch" && (
              <label className="form-field">
                Index
                <input
                  className="input mono"
                  placeholder="datadog-logs"
                  value={edit.index}
                  onChange={(e) => setEdit({ ...edit, index: e.target.value })}
                />
              </label>
            )}
            <label className="muted">
              <input
                type="checkbox"
                checked={edit.enabled}
                onChange={(e) =>
                  setEdit({ ...edit, enabled: e.target.checked })
                }
              />{" "}
              Enabled
            </label>
            {error && <div className="form-error">{error}</div>}
          </div>
          <div className="modal-footer">
            <button
              className="btn primary"
              disabled={!edit.name || !edit.endpoint}
              onClick={() => save.mutate()}
            >
              Save
            </button>
          </div>
        </Modal>
      )}
    </>
  );
}
