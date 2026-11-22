import {
  keepPreviousData,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { useState } from "react";
import {
  Link,
  useNavigate,
  useParams,
  useSearchParams,
} from "react-router-dom";
import { TimeChart } from "../components/Charts";
import { Modal, ServiceChip } from "../components/Common";
import { PlusIcon, TrashIcon } from "../components/Icons";
import { PageHeader } from "../components/Layout";
import { TimePicker } from "../components/TimePicker";
import {
  api,
  ApiError,
  type RetentionFilter,
  type ServiceDefinition,
  type TeamView,
  type TraceSample,
} from "../lib/api";
import { ago, compact, duration, percent, serviceColor } from "../lib/format";
import { useTimeRange } from "../lib/time";
import { Distribution, EnvSelect, useEnv } from "./Apm";
import { APM_TABS } from "./TraceExplorer";

const errorText = (e: unknown) =>
  e instanceof ApiError ? e.message : e ? String(e) : null;
const bytes = (n: number) =>
  n < 1024
    ? `${n} B`
    : n < 1048576
      ? `${(n / 1024).toFixed(1)} KB`
      : `${(n / 1048576).toFixed(1)} MB`;

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <label className="form-field">
      <span className="form-label">{label}</span>
      {children}
      {hint && <span className="form-hint">{hint}</span>}
    </label>
  );
}

// ---------------------------------------------------------------- Retention filters

function FilterModal({
  initial,
  onClose,
}: {
  initial: RetentionFilter | null;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const a = initial?.attributes;
  const errors = a?.filter_type === "spans-errors-sampling-processor";
  const [name, setName] = useState(a?.name ?? "");
  const [query, setQuery] = useState(a?.filter.query ?? "");
  const [rate, setRate] = useState(
    String(Math.round((a?.rate ?? 1) * 1000) / 10),
  );
  const [enabled, setEnabled] = useState(a?.enabled ?? true);
  const [error, setError] = useState<string | null>(null);
  return (
    <Modal
      title={initial ? `Edit ${a?.name}` : "New retention filter"}
      onClose={onClose}
      width={560}
    >
      <div className="modal-body form">
        <Field label="Name">
          <input
            className="input"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </Field>
        <Field
          label="Keep traces with a span matching"
          hint={
            errors ? "The error filter's query does not change." : undefined
          }
        >
          <input
            className="input mono"
            value={query}
            disabled={errors}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="service:shop-api @http.status_code:5*"
          />
        </Field>
        <Field
          label="Retention rate (%)"
          hint="The share of matching traces kept, decided on the trace id."
        >
          <input
            className="input"
            type="number"
            min={0}
            max={100}
            step={0.1}
            value={rate}
            onChange={(e) => setRate(e.target.value)}
          />
        </Field>
        <label className="form-check">
          <input
            type="checkbox"
            checked={enabled}
            onChange={(e) => setEnabled(e.target.checked)}
          />{" "}
          Enabled
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
              await api.saveRetentionFilter(initial?.id ?? null, {
                name: name.trim(),
                filter: { query: query.trim() || "*" },
                rate: Number(rate) / 100,
                enabled,
              });
              await qc.invalidateQueries({ queryKey: ["retention-filters"] });
              onClose();
            } catch (e) {
              setError(errorText(e));
            }
          }}
        >
          {initial ? "Save" : "Create filter"}
        </button>
      </div>
    </Modal>
  );
}

export function RetentionFilters() {
  const qc = useQueryClient();
  const list = useQuery({
    queryKey: ["retention-filters"],
    queryFn: api.retentionFilters,
  });
  const [editing, setEditing] = useState<{ f: RetentionFilter | null } | null>(
    null,
  );
  const [error, setError] = useState<string | null>(null);
  const filters = list.data ?? [];
  const run = async (f: () => Promise<unknown>) => {
    setError(null);
    try {
      await f();
      await qc.invalidateQueries({ queryKey: ["retention-filters"] });
    } catch (e) {
      setError(errorText(e));
    }
  };
  const move = (i: number, d: number) => {
    const ids = filters.map((f) => f.id);
    const j = i + d;
    [ids[i], ids[j]] = [ids[j], ids[i]];
    run(() => api.reorderRetentionFilters(ids));
  };
  return (
    <>
      <PageHeader title="APM" tabs={APM_TABS}>
        <button className="btn primary" onClick={() => setEditing({ f: null })}>
          <PlusIcon /> New Retention Filter
        </button>
      </PageHeader>
      <div className="page-scroll page-padding">
        <p className="muted" style={{ marginTop: 0, maxWidth: 900 }}>
          Every ingested span feeds trace metrics and span-based metrics;
          retention filters then decide which traces are stored and searchable.
          A trace is kept when one of its spans matches an enabled filter, at
          that filter's rate. Traces matching no filter are dropped. Same API as
          Datadog: <code>/api/v2/apm/config/retention-filters</code>.
        </p>
        {error && <div className="error-box">{error}</div>}
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th style={{ width: 70 }}>Order</th>
                <th>Filter</th>
                <th>Query</th>
                <th style={{ width: 110 }}>Retention</th>
                <th style={{ width: 100 }}>State</th>
                <th style={{ width: 130 }} />
              </tr>
            </thead>
            <tbody>
              {filters.map((f, i) => {
                const a = f.attributes;
                return (
                  <tr key={f.id} className="retention-filter-row">
                    <td>
                      <button
                        className="btn icon ghost"
                        title="Move up"
                        disabled={i === 0}
                        onClick={() => move(i, -1)}
                      >
                        ↑
                      </button>
                      <button
                        className="btn icon ghost"
                        title="Move down"
                        disabled={i === filters.length - 1}
                        onClick={() => move(i, 1)}
                      >
                        ↓
                      </button>
                    </td>
                    <td>{a.name}</td>
                    <td className="mono">{a.filter.query || "*"}</td>
                    <td>{Math.round(a.rate * 1000) / 10}%</td>
                    <td>
                      <span className={`pill ${a.enabled ? "ok" : ""}`}>
                        {a.enabled ? "active" : "disabled"}
                      </span>
                    </td>
                    <td style={{ textAlign: "right" }}>
                      <button className="btn" onClick={() => setEditing({ f })}>
                        Edit
                      </button>{" "}
                      {a.filter_type !== "spans-errors-sampling-processor" && (
                        <button
                          className="btn icon ghost"
                          title="Delete"
                          onClick={() =>
                            run(() => api.deleteRetentionFilter(f.id))
                          }
                        >
                          <TrashIcon />
                        </button>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      </div>
      {editing && (
        <FilterModal initial={editing.f} onClose={() => setEditing(null)} />
      )}
    </>
  );
}

// ---------------------------------------------------------------- Ingestion control

export function IngestionControl() {
  const list = useQuery({
    queryKey: ["apm-ingestion"],
    queryFn: api.apmIngestion,
    refetchInterval: 15000,
  });
  const rows = list.data ?? [];
  const total = rows.reduce(
    (t, s) => ({
      spans: t.spans + s.last_hour.spans,
      kept: t.kept + s.last_hour.kept_spans,
      bytes: t.bytes + s.last_hour.bytes,
    }),
    { spans: 0, kept: 0, bytes: 0 },
  );
  return (
    <>
      <PageHeader title="APM" tabs={APM_TABS} />
      <div className="page-scroll page-padding">
        <p className="muted" style={{ marginTop: 0, maxWidth: 900 }}>
          Spans received over the last hour, per service, and how the tracers
          decided to send them (<code>_dd.p.dm</code>, sampling priority).
          Stored spans are those the{" "}
          <Link to="/apm/retention-filters">retention filters</Link> keep. Also
          the metrics <code>datadog.estimated_usage.apm.ingested_spans</code>,{" "}
          <code>ingested_bytes</code> and <code>indexed_spans</code>.
        </p>
        <div className="stat-row" style={{ padding: 0, marginBottom: 12 }}>
          <div className="stat">
            <span className="label">Ingested spans (1h)</span>
            <span className="value">{compact(total.spans)}</span>
          </div>
          <div className="stat">
            <span className="label">Ingested volume (1h)</span>
            <span className="value">{bytes(total.bytes)}</span>
          </div>
          <div className="stat">
            <span className="label">Stored</span>
            <span className="value">
              {total.spans ? percent(total.kept / total.spans) : "—"}
            </span>
          </div>
        </div>
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Service</th>
                <th className="num">Spans/s</th>
                <th className="num">Spans (1h)</th>
                <th className="num">Volume (1h)</th>
                <th className="num">Stored</th>
                <th>Sampling decisions (root spans)</th>
                <th>Tracer</th>
                <th style={{ width: 110 }}>Last seen</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((s) => (
                <tr key={s.service} className="ingestion-row">
                  <td>
                    <Link to={`/apm/services/${encodeURIComponent(s.service)}`}>
                      <ServiceChip name={s.service} />
                    </Link>
                  </td>
                  <td className="num">{s.spans_per_second.toFixed(2)}</td>
                  <td className="num">{compact(s.last_hour.spans)}</td>
                  <td className="num">{bytes(s.last_hour.bytes)}</td>
                  <td className="num">
                    {s.last_hour.spans
                      ? percent(s.last_hour.kept_spans / s.last_hour.spans)
                      : "—"}
                  </td>
                  <td style={{ fontSize: 12 }}>
                    {Object.entries(s.mechanisms).map(([k, v]) => (
                      <div key={k}>
                        {k} <span className="faint">{compact(v)}</span>
                      </div>
                    ))}
                    {Object.keys(s.mechanisms).length === 0 && (
                      <span className="faint">not a trace root</span>
                    )}
                  </td>
                  <td className="mono" style={{ fontSize: 12 }}>
                    {Object.keys(s.tracers).join(", ")}
                  </td>
                  <td className="muted">{ago(s.last_seen)}</td>
                </tr>
              ))}
              {rows.length === 0 && (
                <tr>
                  <td colSpan={8} className="muted">
                    No spans received since robine-dog started.
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

// ---------------------------------------------------------------- Resource page

function Samples({ rows, empty }: { rows: TraceSample[]; empty: string }) {
  if (!rows.length) return <div className="faint">{empty}</div>;
  return (
    <table className="list">
      <thead>
        <tr>
          <th>Date</th>
          <th className="num">Duration</th>
          <th>Status</th>
          <th>Trace</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((t) => (
          <tr key={`${t.trace_id}/${t.span_id}`}>
            <td className="muted">{new Date(t.start).toLocaleString()}</td>
            <td className="num mono">{duration(t.duration)}</td>
            <td style={{ color: t.error ? "var(--red)" : undefined }}>
              {t.status_code ?? (t.error ? "error" : "ok")}
            </td>
            <td>
              <Link
                className="mono"
                to={`/apm/traces?${new URLSearchParams({ query: "", spans: "all", trace: t.trace_id, span: t.span_id })}`}
              >
                {t.trace_id.slice(0, 16)}
              </Link>
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export function ResourcePage() {
  const { name = "" } = useParams();
  const [params] = useSearchParams();
  const resource = params.get("resource") ?? "";
  const time = useTimeRange("1h", 30_000);
  const [env] = useEnv();
  const detail = useQuery({
    queryKey: ["apm-resource", name, resource, time.from, time.to, env],
    queryFn: () =>
      api.apmResource(name, resource, time.from, time.to, env || undefined),
    placeholderData: keepPreviousData,
  });
  const d = detail.data;
  const times = d?.timeseries.map((p) => p.t) ?? [];
  const seconds = Math.max(1, (time.to - time.from) / 1000);
  const back = new URLSearchParams(params);
  back.delete("resource");
  return (
    <>
      <PageHeader
        title={
          <>
            <Link
              to={`/apm/services?${back}`}
              className="muted"
              style={{ fontWeight: 400 }}
            >
              Services
            </Link>
            <span className="faint">/</span>
            <Link
              to={`/apm/services/${encodeURIComponent(name)}?${back}`}
              className="service-chip"
            >
              <i
                style={{
                  background: serviceColor(name),
                  width: 12,
                  height: 12,
                }}
              />
              {name}
            </Link>
            <span className="faint">/</span>
            <span className="mono" style={{ fontSize: 15 }}>
              {resource}
            </span>
          </>
        }
      >
        <TimePicker time={time} />
      </PageHeader>
      <div className="filters-row">
        <EnvSelect />
        {d?.operation && (
          <span className="muted">
            Operation: <span className="mono">{d.operation}</span>
          </span>
        )}
        <div className="spacer" />
        <Link
          className="btn"
          to={`/apm/traces?${new URLSearchParams({ query: `service:${name} resource_name:"${resource.replace(/"/g, '\\"')}"` })}`}
        >
          View Traces
        </Link>
      </div>
      <div className="page-scroll">
        {d && (
          <>
            <div className="stat-row">
              <div className="stat">
                <span className="label">Requests</span>
                <span className="value">{compact(d.summary.hits)}</span>
                <span className="faint">
                  {(d.summary.hits / seconds).toFixed(2)} req/s
                </span>
              </div>
              <div className="stat">
                <span className="label">Errors</span>
                <span
                  className="value"
                  style={{ color: d.summary.errors ? "var(--red)" : undefined }}
                >
                  {compact(d.summary.errors)}
                </span>
                <span className="faint">
                  {percent(d.summary.error_rate)} error rate
                </span>
              </div>
              <div className="stat">
                <span className="label">P50</span>
                <span className="value">{duration(d.summary.p50)}</span>
              </div>
              <div className="stat">
                <span className="label">P95</span>
                <span className="value">{duration(d.summary.p95)}</span>
              </div>
              <div className="stat">
                <span className="label">P99</span>
                <span className="value">{duration(d.summary.p99)}</span>
              </div>
              <div className="stat">
                <span className="label">Status codes</span>
                <span className="value" style={{ fontSize: 13 }}>
                  {d.status_codes
                    .map(([c, n]) => `${c} ×${compact(n)}`)
                    .join("  ") || "—"}
                </span>
              </div>
            </div>
            <div className="page-padding">
              <div className="cards">
                <div className="card">
                  <div className="card-title">Requests and errors</div>
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
                        values: d.timeseries.map((p) => p.hits - p.errors),
                      },
                      {
                        key: "errors",
                        label: "Errors",
                        color: "#d8616a",
                        values: d.timeseries.map((p) => p.errors),
                      },
                    ]}
                    onZoom={time.setAbsolute}
                  />
                </div>
                <div className="card">
                  <div className="card-title">
                    Latency <span className="muted">p50 · p95 · p99</span>
                  </div>
                  <TimeChart
                    kind="lines"
                    times={times}
                    interval={d.interval_ms}
                    from={time.from}
                    to={time.to}
                    height={150}
                    format={duration}
                    series={[
                      {
                        key: "p50",
                        color: "#6b8fd6",
                        values: d.timeseries.map((p) => p.p50),
                      },
                      {
                        key: "p95",
                        color: "#9a82c9",
                        values: d.timeseries.map((p) => p.p95),
                      },
                      {
                        key: "p99",
                        color: "#dba356",
                        values: d.timeseries.map((p) => p.p99),
                      },
                    ]}
                    onZoom={time.setAbsolute}
                  />
                </div>
                <div className="card">
                  <div className="card-title">Latency Distribution</div>
                  <Distribution
                    data={d.distribution}
                    p50={d.summary.p50}
                    p95={d.summary.p95}
                    p99={d.summary.p99}
                  />
                </div>
              </div>
              <div className="section-title">Span Summary</div>
              <table
                className="list"
                style={{ border: "1px solid var(--border)", borderRadius: 6 }}
              >
                <thead>
                  <tr>
                    <th>Span</th>
                    <th className="num">Traces</th>
                    <th className="num">Spans per trace</th>
                    <th className="num">Avg duration</th>
                    <th className="num">P95</th>
                    <th className="num">% of exec time</th>
                  </tr>
                </thead>
                <tbody>
                  {d.spans.map((s) => (
                    <tr
                      key={`${s.service}/${s.name}`}
                      className="span-summary-row"
                    >
                      <td>
                        <ServiceChip name={s.service} />{" "}
                        <span className="mono">{s.name}</span>
                      </td>
                      <td className="num">{compact(s.traces)}</td>
                      <td className="num">{s.spans_per_trace.toFixed(2)}</td>
                      <td className="num mono">{duration(s.avg)}</td>
                      <td className="num mono">{duration(s.p95)}</td>
                      <td className="num">
                        <span
                          className="errbar"
                          style={{
                            width: Math.max(2, s.time_share * 60),
                            background: "var(--purple)",
                          }}
                        />
                        {percent(s.time_share)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              <div className="cards" style={{ marginTop: 12 }}>
                <div className="card">
                  <div className="card-title">Slowest requests</div>
                  <Samples rows={d.slowest} empty="No requests." />
                </div>
                <div className="card">
                  <div className="card-title">Latest errors</div>
                  <Samples rows={d.errors} empty="No errors." />
                </div>
              </div>
            </div>
          </>
        )}
        {detail.isLoading && <div className="loading">Loading…</div>}
      </div>
    </>
  );
}

// ---------------------------------------------------------------- Service definitions

export function DefinitionModal({
  service,
  onClose,
}: {
  service: string;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const tpl = useQuery({
    queryKey: ["definition-yaml", service],
    queryFn: () => api.definitionYaml(service),
  });
  const [yaml, setYaml] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const text = yaml ?? tpl.data?.yaml ?? "";
  const done = async () => {
    await qc.invalidateQueries({ queryKey: ["service-definitions"] });
    await qc.invalidateQueries({ queryKey: ["scorecards"] });
    await qc.invalidateQueries({ queryKey: ["definition-yaml", service] });
    onClose();
  };
  return (
    <Modal
      title={`Service definition: ${service}`}
      onClose={onClose}
      width={720}
    >
      <div className="modal-body form">
        <p className="muted" style={{ margin: 0 }}>
          Schema v2.2 in YAML or JSON, as Datadog's{" "}
          <code>/api/v2/services/definitions</code> takes it (also from
          <code> datadog-ci</code> or Terraform).
        </p>
        <textarea
          className="input mono"
          rows={22}
          value={text}
          onChange={(e) => setYaml(e.target.value)}
          spellCheck={false}
        />
        {error && <div className="error-box">{error}</div>}
      </div>
      <div className="modal-footer">
        {tpl.data?.exists && (
          <button
            className="btn"
            onClick={async () => {
              setError(null);
              try {
                await api.deleteDefinition(service);
                await done();
              } catch (e) {
                setError(errorText(e));
              }
            }}
          >
            <TrashIcon /> Delete
          </button>
        )}
        <div className="spacer" />
        <button className="btn" onClick={onClose}>
          Cancel
        </button>
        <button
          className="btn primary"
          onClick={async () => {
            setError(null);
            try {
              await api.saveDefinition(text);
              await done();
            } catch (e) {
              setError(errorText(e));
            }
          }}
        >
          Save definition
        </button>
      </div>
    </Modal>
  );
}

/** A service's owner, contacts, links and scores, on its page. */
export function ServiceOwnership({ service }: { service: string }) {
  const defs = useQuery({
    queryKey: ["service-definitions"],
    queryFn: api.serviceDefinitions,
  });
  const cards = useQuery({ queryKey: ["scorecards"], queryFn: api.scorecards });
  const [editing, setEditing] = useState(false);
  const def: ServiceDefinition["attributes"]["schema"] | undefined = (
    defs.data ?? []
  ).find((d) => d.attributes.schema["dd-service"] === service)?.attributes
    .schema;
  const score = cards.data?.services.find((s) => s.service === service);
  const links = [
    ...(def?.links ?? []),
    ...(def?.repos ?? []).map((r) => ({ ...r, type: "repo" })),
    ...(def?.docs ?? []).map((r) => ({ ...r, type: "doc" })),
  ];
  return (
    <div className="card service-ownership" style={{ marginBottom: 12 }}>
      <div className="card-title">
        Ownership
        <div className="spacer" />
        <button className="btn" onClick={() => setEditing(true)}>
          {def ? "Edit definition" : "Add a service definition"}
        </button>
      </div>
      {!def && (
        <div className="faint">
          No service definition: set its team, contacts, links and tier, as
          Datadog's Software Catalog.
        </div>
      )}
      {def && (
        <div className="ownership-grid">
          <div>
            <span className="label">Team</span>
            {def.team ? (
              <Link to={`/teams?team=${encodeURIComponent(def.team)}`}>
                {def.team}
              </Link>
            ) : (
              "—"
            )}
          </div>
          <div>
            <span className="label">Tier</span>
            {def.tier ?? "—"}
          </div>
          <div>
            <span className="label">Lifecycle</span>
            {def.lifecycle ?? "—"}
          </div>
          <div>
            <span className="label">Languages</span>
            {(def.languages ?? []).join(", ") || "—"}
          </div>
          <div>
            <span className="label">Contacts</span>
            {(def.contacts ?? []).map((c) => (
              <div key={c.contact}>
                {c.type === "email" ? (
                  <a href={`mailto:${c.contact}`}>{c.name || c.contact}</a>
                ) : (
                  <a href={c.contact}>{c.name || c.contact}</a>
                )}{" "}
                <span className="faint">{c.type}</span>
              </div>
            ))}
            {!(def.contacts ?? []).length && "—"}
          </div>
          <div>
            <span className="label">Links</span>
            {links.map((l) => (
              <div key={l.url}>
                <a href={l.url} target="_blank" rel="noreferrer">
                  {l.name}
                </a>{" "}
                <span className="faint">{l.type}</span>
              </div>
            ))}
            {!links.length && "—"}
          </div>
          {def.description && (
            <div style={{ gridColumn: "1 / -1" }}>
              <span className="label">Description</span>
              {def.description}
            </div>
          )}
        </div>
      )}
      {score && (
        <div className="ownership-scores">
          {Object.entries(score.scorecards).map(([card, v]) => (
            <Link key={card} to="/apm/scorecards" className="score-chip">
              {card} <b>{v === null ? "—" : `${Math.round(v * 100)}%`}</b>
            </Link>
          ))}
        </div>
      )}
      {editing && (
        <DefinitionModal service={service} onClose={() => setEditing(false)} />
      )}
    </div>
  );
}

// ---------------------------------------------------------------- Teams

function TeamModal({
  initial,
  onClose,
}: {
  initial: TeamView | null;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const [name, setName] = useState(initial?.name ?? "");
  const [handle, setHandle] = useState(initial?.handle ?? "");
  const [description, setDescription] = useState(initial?.description ?? "");
  const [error, setError] = useState<string | null>(null);
  return (
    <Modal
      title={initial ? `Edit ${initial.name}` : "New team"}
      onClose={onClose}
      width={520}
    >
      <div className="modal-body form">
        <Field label="Name">
          <input
            className="input"
            value={name}
            onChange={(e) => {
              setName(e.target.value);
              if (!initial)
                setHandle(
                  e.target.value
                    .toLowerCase()
                    .replace(/[^a-z0-9_.-]+/g, "-")
                    .replace(/^-|-$/g, ""),
                );
            }}
          />
        </Field>
        <Field label="Handle" hint="What service definitions name in team:">
          <input
            className="input mono"
            value={handle}
            onChange={(e) => setHandle(e.target.value)}
          />
        </Field>
        <Field label="Description">
          <textarea
            className="input"
            rows={3}
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />
        </Field>
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
              await api.saveTeam(initial?.id ?? null, {
                name: name.trim(),
                handle: handle.trim(),
                description,
              });
              await qc.invalidateQueries({ queryKey: ["teams"] });
              onClose();
            } catch (e) {
              setError(errorText(e));
            }
          }}
        >
          {initial ? "Save" : "Create team"}
        </button>
      </div>
    </Modal>
  );
}

function TeamCard({ t, open }: { t: TeamView; open: boolean }) {
  const qc = useQueryClient();
  const [editing, setEditing] = useState(false);
  const [user, setUser] = useState("");
  const [label, setLabel] = useState("");
  const [url, setUrl] = useState("");
  const [error, setError] = useState<string | null>(null);
  const run = async (f: () => Promise<unknown>) => {
    setError(null);
    try {
      await f();
      await qc.invalidateQueries({ queryKey: ["teams"] });
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <div
      className={`card team-card${open ? " selected" : ""}`}
      id={`team-${t.handle}`}
    >
      <div className="card-title">
        {t.name} <span className="faint mono">@{t.handle}</span>
        <div className="spacer" />
        <button className="btn" onClick={() => setEditing(true)}>
          Edit
        </button>
        <button
          className="btn icon ghost"
          title="Delete"
          onClick={() => run(() => api.deleteTeam(t.id))}
        >
          <TrashIcon />
        </button>
      </div>
      {t.description && (
        <p className="muted" style={{ marginTop: 0 }}>
          {t.description}
        </p>
      )}
      <div className="team-grid">
        <div>
          <div className="label">Members ({t.members.length})</div>
          {t.members.map((m) => (
            <div key={m.user} className="team-item">
              {m.user}{" "}
              {m.role === "admin" && <span className="pill ok">admin</span>}
              <button
                className="btn icon ghost"
                title="Remove"
                onClick={() => run(() => api.removeTeamMember(t.id, m.user))}
              >
                ×
              </button>
            </div>
          ))}
          <form
            className="team-add"
            onSubmit={(e) => {
              e.preventDefault();
              if (user.trim())
                run(() => api.addTeamMember(t.id, user.trim(), false)).then(
                  () => setUser(""),
                );
            }}
          >
            <input
              className="input"
              placeholder="alice@example.com"
              value={user}
              onChange={(e) => setUser(e.target.value)}
            />
            <button className="btn">Add</button>
          </form>
        </div>
        <div>
          <div className="label">Services ({t.services.length})</div>
          {t.services.map((s) => (
            <div key={s} className="team-item">
              <Link to={`/apm/services/${encodeURIComponent(s)}`}>
                <ServiceChip name={s} />
              </Link>
            </div>
          ))}
          {t.services.length === 0 && (
            <div className="faint">
              No service definition names team: {t.handle}.
            </div>
          )}
        </div>
        <div>
          <div className="label">Links</div>
          {t.links.map((l) => (
            <div key={l.id} className="team-item">
              <a href={l.url} target="_blank" rel="noreferrer">
                {l.label}
              </a>
              <button
                className="btn icon ghost"
                title="Remove"
                onClick={() => run(() => api.removeTeamLink(t.id, l.id))}
              >
                ×
              </button>
            </div>
          ))}
          <form
            className="team-add"
            onSubmit={(e) => {
              e.preventDefault();
              run(() => api.addTeamLink(t.id, label.trim(), url.trim())).then(
                () => {
                  setLabel("");
                  setUrl("");
                },
              );
            }}
          >
            <input
              className="input"
              placeholder="Label"
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              style={{ width: 90 }}
            />
            <input
              className="input"
              placeholder="https://"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
            />
            <button className="btn">Add</button>
          </form>
        </div>
      </div>
      {error && <div className="error-box">{error}</div>}
      {editing && <TeamModal initial={t} onClose={() => setEditing(false)} />}
    </div>
  );
}

export function Teams() {
  const list = useQuery({ queryKey: ["teams"], queryFn: api.teams });
  const [params] = useSearchParams();
  const [creating, setCreating] = useState(false);
  const [filter, setFilter] = useState("");
  const selected = params.get("team");
  const teams = (list.data ?? []).filter(
    (t) =>
      !filter ||
      t.name.toLowerCase().includes(filter.toLowerCase()) ||
      t.handle.includes(filter.toLowerCase()),
  );
  return (
    <>
      <PageHeader title="Teams">
        <button className="btn primary" onClick={() => setCreating(true)}>
          <PlusIcon /> New Team
        </button>
      </PageHeader>
      <div className="filters-row">
        <input
          className="input"
          placeholder="Filter teams"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          style={{ width: 240 }}
        />
        <div className="spacer" />
        <span className="muted">
          {teams.length} teams · same API as Datadog: <code>/api/v2/team</code>
        </span>
      </div>
      <div className="page-scroll page-padding">
        {teams.map((t) => (
          <TeamCard key={t.id} t={t} open={selected === t.handle} />
        ))}
        {list.isSuccess && teams.length === 0 && (
          <div className="card muted">No team yet.</div>
        )}
      </div>
      {creating && (
        <TeamModal initial={null} onClose={() => setCreating(false)} />
      )}
    </>
  );
}

// ---------------------------------------------------------------- Scorecards

function RuleModal({
  cards,
  onClose,
}: {
  cards: string[];
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const [name, setName] = useState("");
  const [card, setCard] = useState(cards[0] ?? "Production Readiness");
  const [description, setDescription] = useState("");
  const [error, setError] = useState<string | null>(null);
  return (
    <Modal title="New custom rule" onClose={onClose} width={560}>
      <div className="modal-body form">
        <Field label="Name">
          <input
            className="input"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </Field>
        <Field label="Scorecard" hint="An existing one, or a new name.">
          <input
            className="input"
            list="scorecard-names"
            value={card}
            onChange={(e) => setCard(e.target.value)}
          />
          <datalist id="scorecard-names">
            {cards.map((c) => (
              <option key={c} value={c} />
            ))}
          </datalist>
        </Field>
        <Field label="Description">
          <input
            className="input"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />
        </Field>
        <p className="muted" style={{ margin: 0 }}>
          Custom rules are evaluated outside robine-dog, which posts outcomes to{" "}
          <code>/api/v2/scorecard/outcomes/batch</code>, as in Datadog.
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
              await api.createScorecardRule({
                name: name.trim(),
                scorecard_name: card.trim(),
                description,
              });
              await qc.invalidateQueries({ queryKey: ["scorecards"] });
              onClose();
            } catch (e) {
              setError(errorText(e));
            }
          }}
        >
          Create rule
        </button>
      </div>
    </Modal>
  );
}

const STATE_ICON: Record<string, string> = { pass: "✓", fail: "✗", skip: "–" };

export function Scorecards() {
  const qc = useQueryClient();
  const data = useQuery({
    queryKey: ["scorecards"],
    queryFn: api.scorecards,
    refetchInterval: 60000,
  });
  const navigate = useNavigate();
  const [creating, setCreating] = useState(false);
  const [definition, setDefinition] = useState<string | null>(null);
  const rules = data.data?.rules ?? [];
  const cards = [...new Set(rules.map((r) => r.attributes.scorecard_name))];
  const services = data.data?.services ?? [];
  return (
    <>
      <PageHeader title="APM" tabs={APM_TABS}>
        <button className="btn primary" onClick={() => setCreating(true)}>
          <PlusIcon /> New Rule
        </button>
      </PageHeader>
      <div className="page-scroll page-padding">
        <p className="muted" style={{ marginTop: 0, maxWidth: 900 }}>
          How each service meets its scorecards' rules. Out-of-the-box rules are
          evaluated from service definitions, monitors, SLOs, spans and logs of
          the last day; custom rules get outcomes through{" "}
          <code>/api/v2/scorecard/outcomes</code>.
        </p>
        <div className="card" style={{ padding: 0, overflowX: "auto" }}>
          <table className="list scorecard-table">
            <thead>
              <tr>
                <th rowSpan={2}>Service</th>
                <th rowSpan={2} className="num">
                  Score
                </th>
                {cards.map((c) => (
                  <th
                    key={c}
                    colSpan={
                      rules.filter((r) => r.attributes.scorecard_name === c)
                        .length
                    }
                    className="scorecard-group"
                  >
                    {c}
                  </th>
                ))}
              </tr>
              <tr>
                {cards.flatMap((c) =>
                  rules
                    .filter((r) => r.attributes.scorecard_name === c)
                    .map((r) => (
                      <th
                        key={r.id}
                        title={r.attributes.description}
                        className="scorecard-rule"
                      >
                        {r.attributes.name}
                        {r.attributes.custom && (
                          <button
                            className="btn icon ghost"
                            title="Delete rule"
                            onClick={async () => {
                              await api.deleteScorecardRule(r.id);
                              await qc.invalidateQueries({
                                queryKey: ["scorecards"],
                              });
                            }}
                          >
                            ×
                          </button>
                        )}
                      </th>
                    )),
                )}
              </tr>
            </thead>
            <tbody>
              {services.map((s) => (
                <tr key={s.service} className="scorecard-row">
                  <td>
                    <span
                      className="link-button"
                      onClick={() =>
                        navigate(
                          `/apm/services/${encodeURIComponent(s.service)}`,
                        )
                      }
                    >
                      <ServiceChip name={s.service} />
                    </span>
                    {s.team && (
                      <div
                        className="faint"
                        style={{ fontSize: 11, paddingLeft: 15 }}
                      >
                        @{s.team}
                      </div>
                    )}
                    {!s.has_definition && (
                      <button
                        className="link-button"
                        style={{ fontSize: 11, paddingLeft: 15 }}
                        onClick={() => setDefinition(s.service)}
                      >
                        add a definition
                      </button>
                    )}
                  </td>
                  <td className="num">
                    <b>
                      {s.score === null ? "—" : `${Math.round(s.score * 100)}%`}
                    </b>
                  </td>
                  {cards.flatMap((c) =>
                    rules
                      .filter((r) => r.attributes.scorecard_name === c)
                      .map((r) => {
                        const o = s.outcomes[r.id];
                        return (
                          <td
                            key={r.id}
                            className={`score-${o?.state ?? "none"}`}
                            title={o?.remarks}
                          >
                            {o ? STATE_ICON[o.state] : ""}
                          </td>
                        );
                      }),
                  )}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
      {creating && (
        <RuleModal cards={cards} onClose={() => setCreating(false)} />
      )}
      {definition && (
        <DefinitionModal
          service={definition}
          onClose={() => setDefinition(null)}
        />
      )}
    </>
  );
}
