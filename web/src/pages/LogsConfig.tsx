import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link } from "react-router-dom";
import { Modal } from "../components/Common";
import { PlusIcon, TrashIcon } from "../components/Icons";
import { PageHeader } from "../components/Layout";
import {
  api,
  ApiError,
  type ArchiveAttributes,
  type HistoricalView,
  type LogArchive,
  type LogIndex,
  type SdsJson,
  type SdsReplacement,
  type ReferenceTable,
} from "../lib/api";
import { LOG_TABS } from "./LogsExplorer";

const errorText = (e: unknown) =>
  e instanceof ApiError ? e.message : e ? String(e) : null;
const fmtNum = (n: number) => n.toLocaleString("en-US");
const fmtBytes = (n: number) =>
  n < 1024
    ? `${n} B`
    : n < 1024 * 1024
      ? `${(n / 1024).toFixed(1)} KB`
      : `${(n / 1024 / 1024).toFixed(1)} MB`;
const fmtTime = (ms: number | null | undefined) =>
  ms ? new Date(ms).toLocaleString() : "—";

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

/** Moves item `i` of a list by `d` places. */
function move<T>(list: T[], i: number, d: number): T[] {
  const j = i + d;
  if (j < 0 || j >= list.length) return list;
  const out = [...list];
  [out[i], out[j]] = [out[j], out[i]];
  return out;
}

function OrderButtons({
  i,
  n,
  onMove,
}: {
  i: number;
  n: number;
  onMove: (d: number) => void;
}) {
  return (
    <span style={{ display: "inline-flex", gap: 2 }}>
      <button
        className="btn icon ghost"
        disabled={i === 0}
        onClick={() => onMove(-1)}
        title="Move up"
      >
        ↑
      </button>
      <button
        className="btn icon ghost"
        disabled={i === n - 1}
        onClick={() => onMove(1)}
        title="Move down"
      >
        ↓
      </button>
    </span>
  );
}

// ---------------------------------------------------------------- Indexes

function IndexModal({
  initial,
  onClose,
}: {
  initial: LogIndex | null;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const [name, setName] = useState(initial?.name ?? "");
  const [query, setQuery] = useState(initial?.filter.query ?? "");
  const [retention, setRetention] = useState(
    initial?.num_retention_days?.toString() ?? "",
  );
  const [limit, setLimit] = useState(initial?.daily_limit?.toString() ?? "");
  const [resetTime, setResetTime] = useState(
    initial?.daily_limit_reset?.reset_time ?? "00:00",
  );
  const [resetOffset, setResetOffset] = useState(
    initial?.daily_limit_reset?.reset_utc_offset ?? "+00:00",
  );
  const [warn, setWarn] = useState(
    initial?.daily_limit_warning_threshold_percentage?.toString() ?? "",
  );
  const [exclusions, setExclusions] = useState(
    initial?.exclusion_filters ?? [],
  );
  const [error, setError] = useState<string | null>(null);
  const num = (s: string) => (s.trim() ? Number(s) : null);
  const body: LogIndex = {
    name: name.trim(),
    filter: { query: query.trim() },
    exclusion_filters: exclusions,
    daily_limit: num(limit),
    daily_limit_reset: num(limit)
      ? { reset_time: resetTime, reset_utc_offset: resetOffset }
      : null,
    daily_limit_warning_threshold_percentage: num(warn),
    num_retention_days: num(retention),
  };
  const setEx = (i: number, patch: Partial<LogIndex["exclusion_filters"][0]>) =>
    setExclusions(exclusions.map((e, j) => (j === i ? { ...e, ...patch } : e)));
  return (
    <Modal
      title={initial ? `Edit index ${initial.name}` : "New index"}
      onClose={onClose}
      width={720}
    >
      <div className="modal-body form">
        <div className="form-row">
          <Field label="Name">
            <input
              className="input mono"
              value={name}
              disabled={!!initial}
              onChange={(e) => setName(e.target.value)}
              placeholder="payments"
            />
          </Field>
          <Field label="Retention (days)" hint="Default: the logs table's">
            <input
              className="input"
              type="number"
              min={1}
              value={retention}
              onChange={(e) => setRetention(e.target.value)}
            />
          </Field>
        </div>
        <Field
          label="Filter"
          hint="Logs go to the first index, in order, whose filter matches; new indexes come last. Empty: every log."
        >
          <input
            className="input mono"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="service:payment-service"
          />
        </Field>
        <div className="form-label">Exclusion filters</div>
        {exclusions.map((x, i) => (
          <div className="form-row" key={i} style={{ alignItems: "flex-end" }}>
            <label
              className="form-check"
              style={{ flex: "0 0 auto", paddingBottom: 8 }}
            >
              <input
                type="checkbox"
                checked={x.is_enabled}
                onChange={(e) => setEx(i, { is_enabled: e.target.checked })}
              />
            </label>
            <Field label="Name">
              <input
                className="input"
                value={x.name}
                onChange={(e) => setEx(i, { name: e.target.value })}
              />
            </Field>
            <Field label="Exclude logs matching">
              <input
                className="input mono"
                value={x.filter.query}
                onChange={(e) =>
                  setEx(i, { filter: { ...x.filter, query: e.target.value } })
                }
              />
            </Field>
            <Field label="Exclude %">
              <input
                className="input"
                type="number"
                min={0}
                max={100}
                style={{ width: 80 }}
                value={Math.round((x.filter.sample_rate ?? 1) * 100)}
                onChange={(e) =>
                  setEx(i, {
                    filter: {
                      ...x.filter,
                      sample_rate: Number(e.target.value) / 100,
                    },
                  })
                }
              />
            </Field>
            <button
              className="btn icon ghost"
              style={{ marginBottom: 4 }}
              onClick={() =>
                setExclusions(exclusions.filter((_, j) => j !== i))
              }
            >
              <TrashIcon />
            </button>
          </div>
        ))}
        <div>
          <button
            className="btn"
            onClick={() =>
              setExclusions([
                ...exclusions,
                {
                  name: "",
                  is_enabled: true,
                  filter: { query: "", sample_rate: 1 },
                },
              ])
            }
          >
            <PlusIcon /> Add an exclusion filter
          </button>
        </div>
        <div className="form-row">
          <Field label="Daily quota (logs)" hint="Empty: no limit">
            <input
              className="input"
              type="number"
              min={1}
              value={limit}
              onChange={(e) => setLimit(e.target.value)}
            />
          </Field>
          <Field label="Reset at">
            <input
              className="input mono"
              value={resetTime}
              disabled={!limit}
              onChange={(e) => setResetTime(e.target.value)}
            />
          </Field>
          <Field label="UTC offset">
            <input
              className="input mono"
              value={resetOffset}
              disabled={!limit}
              onChange={(e) => setResetOffset(e.target.value)}
            />
          </Field>
          <Field label="Warn at %">
            <input
              className="input"
              type="number"
              min={50}
              max={99}
              value={warn}
              disabled={!limit}
              onChange={(e) => setWarn(e.target.value)}
            />
          </Field>
        </div>
        <p className="muted" style={{ margin: 0 }}>
          Excluded logs are not stored, but still count in log-based metrics and
          go to archives, as in Datadog.
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
              if (initial) await api.updateLogIndex(initial.name, body);
              else await api.createLogIndex(body);
              await qc.invalidateQueries({ queryKey: ["log-indexes"] });
              onClose();
            } catch (e) {
              setError(errorText(e));
            }
          }}
        >
          {initial ? "Save" : "Create index"}
        </button>
      </div>
    </Modal>
  );
}

export function LogIndexes() {
  const qc = useQueryClient();
  const list = useQuery({
    queryKey: ["log-indexes"],
    queryFn: api.logIndexes,
    refetchInterval: 15000,
  });
  const [editing, setEditing] = useState<{ i: LogIndex | null } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const indexes = list.data ?? [];
  const reorder = async (names: string[]) => {
    setError(null);
    try {
      await api.setLogIndexOrder(names);
      await qc.invalidateQueries({ queryKey: ["log-indexes"] });
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <>
      <PageHeader title="Logs" tabs={LOG_TABS}>
        <button className="btn primary" onClick={() => setEditing({ i: null })}>
          <PlusIcon /> New Index
        </button>
      </PageHeader>
      <div className="page-scroll page-padding">
        <p className="muted" style={{ marginTop: 0, maxWidth: 900 }}>
          Each log goes to the first index whose filter matches. Exclusion
          filters and daily quotas decide what is stored; retention is per
          index. Search one with <code>index:name</code>. Same API as Datadog:{" "}
          <code>/api/v1/logs/config/indexes</code>.
        </p>
        {error && <div className="error-box">{error}</div>}
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th style={{ width: 70 }}>Order</th>
                <th>Index</th>
                <th>Filter</th>
                <th>Exclusions</th>
                <th style={{ width: 90 }}>Retention</th>
                <th style={{ width: 220 }}>Today</th>
                <th style={{ width: 130 }} />
              </tr>
            </thead>
            <tbody>
              {indexes.map((x, i) => {
                const u = x.usage;
                const pct =
                  x.daily_limit && u
                    ? Math.min(100, (u.indexed / x.daily_limit) * 100)
                    : null;
                return (
                  <tr key={x.name} className="log-index-row">
                    <td>
                      <OrderButtons
                        i={i}
                        n={indexes.length}
                        onMove={(d) =>
                          reorder(move(indexes, i, d).map((y) => y.name))
                        }
                      />
                    </td>
                    <td>
                      <Link
                        className="mono"
                        to={`/logs?query=${encodeURIComponent(`index:${x.name}`)}`}
                      >
                        {x.name}
                      </Link>
                      {x.is_rate_limited && (
                        <span className="pill error" style={{ marginLeft: 6 }}>
                          quota reached
                        </span>
                      )}
                    </td>
                    <td className="mono">{x.filter.query || "*"}</td>
                    <td>
                      {x.exclusion_filters.length === 0 ? (
                        <span className="muted">none</span>
                      ) : (
                        x.exclusion_filters.map((e) => (
                          <div
                            key={e.name}
                            className={e.is_enabled ? "" : "muted"}
                          >
                            <span className="mono">
                              {e.filter.query || "*"}
                            </span>{" "}
                            <span className="muted">
                              ({Math.round((e.filter.sample_rate ?? 1) * 100)}%
                              {e.is_enabled ? "" : ", off"})
                            </span>
                          </div>
                        ))
                      )}
                    </td>
                    <td>
                      {x.num_retention_days
                        ? `${x.num_retention_days} d`
                        : "default"}
                    </td>
                    <td>
                      {u ? (
                        <>
                          <div>
                            {fmtNum(u.indexed)} indexed
                            {x.daily_limit ? ` / ${fmtNum(x.daily_limit)}` : ""}
                          </div>
                          <div className="muted" style={{ fontSize: 12 }}>
                            {fmtNum(u.excluded)} excluded
                            {u.over_quota
                              ? `, ${fmtNum(u.over_quota)} over quota`
                              : ""}
                          </div>
                          {pct !== null && (
                            <div className="quota-bar">
                              <div
                                style={{
                                  width: `${pct}%`,
                                  background:
                                    pct >= 100 ? "var(--red)" : "var(--purple)",
                                }}
                              />
                            </div>
                          )}
                        </>
                      ) : (
                        "—"
                      )}
                    </td>
                    <td style={{ textAlign: "right" }}>
                      <button
                        className="btn"
                        onClick={() => setEditing({ i: x })}
                      >
                        Edit
                      </button>{" "}
                      <button
                        className="btn icon ghost"
                        title="Delete"
                        onClick={async () => {
                          setError(null);
                          try {
                            await api.deleteLogIndex(x.name);
                            await qc.invalidateQueries({
                              queryKey: ["log-indexes"],
                            });
                          } catch (e) {
                            setError(errorText(e));
                          }
                        }}
                      >
                        <TrashIcon />
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      </div>
      {editing && (
        <IndexModal initial={editing.i} onClose={() => setEditing(null)} />
      )}
    </>
  );
}

// ---------------------------------------------------------------- Archives

function ArchiveModal({
  initial,
  onClose,
}: {
  initial: LogArchive | null;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const a = initial?.attributes;
  const [name, setName] = useState(a?.name ?? "");
  const [query, setQuery] = useState(a?.query ?? "*");
  const [kind, setKind] = useState(a?.destination.type ?? "s3");
  const [bucket, setBucket] = useState(a?.destination.bucket ?? "");
  const [path, setPath] = useState(a?.destination.path ?? "");
  const [region, setRegion] = useState(a?.destination.region ?? "");
  const [endpoint, setEndpoint] = useState(a?.destination.endpoint ?? "");
  const [includeTags, setIncludeTags] = useState(a?.include_tags ?? true);
  const [tags, setTags] = useState((a?.rehydration_tags ?? []).join(", "));
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const body: ArchiveAttributes = {
    name: name.trim(),
    query: query.trim() || "*",
    destination:
      kind === "s3"
        ? {
            type: "s3",
            bucket: bucket.trim(),
            path: path.trim(),
            ...(region.trim() ? { region: region.trim() } : {}),
            ...(endpoint.trim() ? { endpoint: endpoint.trim() } : {}),
          }
        : { type: "local", path: path.trim() },
    include_tags: includeTags,
    rehydration_tags: tags
      .split(",")
      .map((t) => t.trim())
      .filter(Boolean),
  };
  return (
    <Modal
      title={initial ? `Edit archive ${a?.name}` : "New archive"}
      onClose={onClose}
      width={640}
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
          label="Archive logs matching"
          hint="Every ingested log, excluded from indexes or not."
        >
          <input
            className="input mono"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </Field>
        <div className="form-row">
          <Field label="Destination">
            <select
              className="input"
              value={kind}
              onChange={(e) => setKind(e.target.value)}
            >
              <option value="s3">S3-compatible bucket</option>
              <option value="local">Local directory</option>
            </select>
          </Field>
          {kind === "s3" && (
            <Field label="Bucket">
              <input
                className="input mono"
                value={bucket}
                onChange={(e) => setBucket(e.target.value)}
              />
            </Field>
          )}
        </div>
        <Field
          label={kind === "s3" ? "Path prefix" : "Directory"}
          hint={kind === "s3" ? "Optional" : "On the robine-dog server"}
        >
          <input
            className="input mono"
            value={path}
            onChange={(e) => setPath(e.target.value)}
          />
        </Field>
        {kind === "s3" && (
          <div className="form-row">
            <Field label="Region">
              <input
                className="input mono"
                value={region}
                onChange={(e) => setRegion(e.target.value)}
                placeholder="eu-west-1"
              />
            </Field>
            <Field label="Endpoint" hint="For MinIO, Garage, R2…">
              <input
                className="input mono"
                value={endpoint}
                onChange={(e) => setEndpoint(e.target.value)}
                placeholder="https://s3.example.com"
              />
            </Field>
          </div>
        )}
        {kind === "s3" && (
          <p className="muted" style={{ margin: 0 }}>
            Credentials come from the server's environment:{" "}
            <code>AWS_ACCESS_KEY_ID</code>, <code>AWS_SECRET_ACCESS_KEY</code>.
          </p>
        )}
        <label className="form-check">
          <input
            type="checkbox"
            checked={includeTags}
            onChange={(e) => setIncludeTags(e.target.checked)}
          />{" "}
          Include tags
        </label>
        <Field label="Tags added to rehydrated logs" hint="Comma-separated">
          <input
            className="input mono"
            value={tags}
            onChange={(e) => setTags(e.target.value)}
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
          disabled={saving}
          onClick={async () => {
            setError(null);
            setSaving(true);
            try {
              if (initial) await api.updateLogArchive(initial.id, body);
              else await api.createLogArchive(body);
              await qc.invalidateQueries({ queryKey: ["log-archives"] });
              onClose();
            } catch (e) {
              setError(errorText(e));
            } finally {
              setSaving(false);
            }
          }}
        >
          {saving
            ? "Checking the destination…"
            : initial
              ? "Save"
              : "Create archive"}
        </button>
      </div>
    </Modal>
  );
}

function RehydrateModal({
  archives,
  onClose,
}: {
  archives: LogArchive[];
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const now = Date.now();
  const local = (ms: number) => {
    const d = new Date(ms - new Date(ms).getTimezoneOffset() * 60000);
    return d.toISOString().slice(0, 16);
  };
  const [name, setName] = useState("");
  const [archive, setArchive] = useState(archives[0]?.id ?? "");
  const [query, setQuery] = useState("");
  const [from, setFrom] = useState(local(now - 86400000));
  const [to, setTo] = useState(local(now));
  const [max, setMax] = useState("");
  const [error, setError] = useState<string | null>(null);
  return (
    <Modal title="Rehydrate from an archive" onClose={onClose} width={600}>
      <div className="modal-body form">
        <Field label="Historical view name" hint="Searched with index:name">
          <input
            className="input mono"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="incident-2026-11-20"
          />
        </Field>
        <Field label="Archive">
          <select
            className="input"
            value={archive}
            onChange={(e) => setArchive(e.target.value)}
          >
            {archives.map((a) => (
              <option key={a.id} value={a.id}>
                {a.attributes.name}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Logs matching" hint="Empty: everything in the period">
          <input
            className="input mono"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </Field>
        <div className="form-row">
          <Field label="From">
            <input
              className="input"
              type="datetime-local"
              value={from}
              onChange={(e) => setFrom(e.target.value)}
            />
          </Field>
          <Field label="To">
            <input
              className="input"
              type="datetime-local"
              value={to}
              onChange={(e) => setTo(e.target.value)}
            />
          </Field>
          <Field label="Max logs">
            <input
              className="input"
              type="number"
              min={1}
              value={max}
              onChange={(e) => setMax(e.target.value)}
            />
          </Field>
        </div>
        {error && <div className="error-box">{error}</div>}
      </div>
      <div className="modal-footer">
        <button className="btn" onClick={onClose}>
          Cancel
        </button>
        <button
          className="btn primary"
          disabled={!archive}
          onClick={async () => {
            setError(null);
            try {
              await api.rehydrate({
                name: name.trim(),
                archive_id: archive,
                query: query.trim(),
                from: new Date(from).getTime(),
                to: new Date(to).getTime(),
                ...(max ? { max_logs: Number(max) } : {}),
              });
              await qc.invalidateQueries({ queryKey: ["historical-views"] });
              onClose();
            } catch (e) {
              setError(errorText(e));
            }
          }}
        >
          Rehydrate
        </button>
      </div>
    </Modal>
  );
}

function ViewStatus({ v }: { v: HistoricalView }) {
  const cls =
    v.status === "done" ? "ok" : v.status === "failed" ? "error" : "warn";
  return <span className={`pill ${cls}`}>{v.status}</span>;
}

export function LogArchives() {
  const qc = useQueryClient();
  const archives = useQuery({
    queryKey: ["log-archives"],
    queryFn: api.logArchives,
    refetchInterval: 15000,
  });
  const views = useQuery({
    queryKey: ["historical-views"],
    queryFn: api.historicalViews,
    refetchInterval: (q) =>
      (q.state.data ?? []).some((v) => v.status === "running") ? 2000 : 15000,
  });
  const [editing, setEditing] = useState<{ a: LogArchive | null } | null>(null);
  const [rehydrating, setRehydrating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const list = archives.data ?? [];
  const names = Object.fromEntries(list.map((a) => [a.id, a.attributes.name]));
  const run = async (f: () => Promise<unknown>, key: string) => {
    setError(null);
    try {
      await f();
      await qc.invalidateQueries({ queryKey: [key] });
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <>
      <PageHeader title="Logs" tabs={LOG_TABS}>
        <button
          className="btn"
          disabled={list.length === 0}
          onClick={() => setRehydrating(true)}
        >
          Rehydrate
        </button>
        <button className="btn primary" onClick={() => setEditing({ a: null })}>
          <PlusIcon /> New Archive
        </button>
      </PageHeader>
      <div className="page-scroll page-padding">
        <p className="muted" style={{ marginTop: 0, maxWidth: 900 }}>
          Ingested logs, before index exclusions, written to the first archive
          whose filter matches, in Datadog's format (gzipped JSON lines under{" "}
          <code>dt=YYYYMMDD/hour=HH/</code>). Rehydrate them into a historical
          view to search them again. Same API as Datadog:{" "}
          <code>/api/v2/logs/config/archives</code>.
        </p>
        {error && <div className="error-box">{error}</div>}
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th style={{ width: 70 }}>Order</th>
                <th>Archive</th>
                <th>Filter</th>
                <th>Destination</th>
                <th style={{ width: 240 }}>Written</th>
                <th style={{ width: 130 }} />
              </tr>
            </thead>
            <tbody>
              {list.map((x, i) => {
                const a = x.attributes;
                const s = a.status;
                const d = a.destination;
                return (
                  <tr key={x.id}>
                    <td>
                      <OrderButtons
                        i={i}
                        n={list.length}
                        onMove={(dd) =>
                          run(
                            () =>
                              api.setLogArchiveOrder(
                                move(list, i, dd).map((y) => y.id),
                              ),
                            "log-archives",
                          )
                        }
                      />
                    </td>
                    <td>
                      {a.name}
                      {s?.last_error && (
                        <div className="error-text" style={{ fontSize: 12 }}>
                          {s.last_error}
                        </div>
                      )}
                    </td>
                    <td className="mono">{a.query}</td>
                    <td className="mono">
                      {d.type === "s3"
                        ? `s3://${d.bucket}${d.path ? `/${d.path.replace(/^\//, "")}` : ""}`
                        : d.path}
                      {d.endpoint && <div className="muted">{d.endpoint}</div>}
                    </td>
                    <td>
                      {s ? (
                        <>
                          <div>
                            {fmtNum(s.archived)} logs, {s.files} file
                            {s.files === 1 ? "" : "s"}, {fmtBytes(s.bytes)}
                          </div>
                          <div className="muted" style={{ fontSize: 12 }}>
                            last flush {fmtTime(s.last_flush)}
                          </div>
                        </>
                      ) : (
                        "—"
                      )}
                    </td>
                    <td style={{ textAlign: "right" }}>
                      <button
                        className="btn"
                        onClick={() => setEditing({ a: x })}
                      >
                        Edit
                      </button>{" "}
                      <button
                        className="btn icon ghost"
                        title="Delete"
                        onClick={() =>
                          run(() => api.deleteLogArchive(x.id), "log-archives")
                        }
                      >
                        <TrashIcon />
                      </button>
                    </td>
                  </tr>
                );
              })}
              {list.length === 0 && (
                <tr>
                  <td colSpan={6} className="muted">
                    No archive yet.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
        <h3 style={{ marginTop: 24 }}>Historical views</h3>
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>View</th>
                <th>Archive</th>
                <th>Query</th>
                <th>Period</th>
                <th style={{ width: 100 }}>Status</th>
                <th style={{ width: 220 }}>Scanned</th>
                <th style={{ width: 60 }} />
              </tr>
            </thead>
            <tbody>
              {(views.data ?? []).map((v) => (
                <tr key={v.id} className="historical-view-row">
                  <td>
                    <Link
                      className="mono"
                      to={`/logs?query=${encodeURIComponent(`index:${v.name}`)}&from_ts=${v.from}&to_ts=${v.to}&live=false`}
                    >
                      {v.name}
                    </Link>
                  </td>
                  <td>{names[v.archive_id] ?? v.archive_id}</td>
                  <td className="mono">{v.query || "*"}</td>
                  <td style={{ fontSize: 12 }}>
                    {fmtTime(v.from)} → {fmtTime(v.to)}
                  </td>
                  <td>
                    <ViewStatus v={v} />
                    {v.error && (
                      <div className="error-text" style={{ fontSize: 12 }}>
                        {v.error}
                      </div>
                    )}
                  </td>
                  <td>
                    <div>{fmtNum(v.rehydrated)} logs rehydrated</div>
                    <div className="muted" style={{ fontSize: 12 }}>
                      {fmtNum(v.scanned_logs)} scanned in {v.files} file
                      {v.files === 1 ? "" : "s"} ({fmtBytes(v.scanned_bytes)})
                    </div>
                  </td>
                  <td style={{ textAlign: "right" }}>
                    <button
                      className="btn icon ghost"
                      title="Delete"
                      onClick={() =>
                        run(
                          () => api.deleteHistoricalView(v.id),
                          "historical-views",
                        )
                      }
                    >
                      <TrashIcon />
                    </button>
                  </td>
                </tr>
              ))}
              {(views.data ?? []).length === 0 && (
                <tr>
                  <td colSpan={7} className="muted">
                    No historical view.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </div>
      {editing && (
        <ArchiveModal initial={editing.a} onClose={() => setEditing(null)} />
      )}
      {rehydrating && (
        <RehydrateModal archives={list} onClose={() => setRehydrating(false)} />
      )}
    </>
  );
}

// ---------------------------------------------------------------- Sensitive Data Scanner

type Rel = { id: string; type: string };
const relOne = (j: SdsJson, k: string) => {
  const d = j.relationships?.[k]?.data;
  return d && !Array.isArray(d) ? d.id : null;
};
const relMany = (j: SdsJson, k: string): string[] => {
  const d = j.relationships?.[k]?.data;
  return Array.isArray(d) ? d.map((r: Rel) => r.id) : [];
};
const REPLACEMENTS: Record<string, string> = {
  none: "Tag only",
  replacement_string: "Replace with a string",
  hash: "Hash",
  partial_replacement_from_beginning: "Mask the first characters",
  partial_replacement_from_end: "Mask the last characters",
};

function GroupModal({
  initial,
  onClose,
}: {
  initial: SdsJson | null;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const a = initial?.attributes ?? {};
  const [name, setName] = useState(String(a.name ?? ""));
  const [description, setDescription] = useState(String(a.description ?? ""));
  const [query, setQuery] = useState(
    String((a.filter as { query?: string } | undefined)?.query ?? "*"),
  );
  const [enabled, setEnabled] = useState(a.is_enabled !== false);
  const [error, setError] = useState<string | null>(null);
  return (
    <Modal
      title={initial ? `Edit group ${name}` : "New scanning group"}
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
        <Field label="Description">
          <input
            className="input"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />
        </Field>
        <Field label="Scan logs matching">
          <input
            className="input mono"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
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
              await api.sdsSaveGroup(initial?.id ?? null, {
                name: name.trim(),
                description,
                filter: { query: query.trim() || "*" },
                is_enabled: enabled,
                product_list: ["logs"],
              });
              await qc.invalidateQueries({ queryKey: ["sds"] });
              onClose();
            } catch (e) {
              setError(errorText(e));
            }
          }}
        >
          {initial ? "Save" : "Create group"}
        </button>
      </div>
    </Modal>
  );
}

function RuleModal({
  groupId,
  initial,
  onClose,
}: {
  groupId: string;
  initial: SdsJson | null;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const patterns = useQuery({
    queryKey: ["sds-patterns"],
    queryFn: api.sdsStandardPatterns,
  });
  const a = initial?.attributes ?? {};
  const [name, setName] = useState(String(a.name ?? ""));
  const [standard, setStandard] = useState(
    initial ? (relOne(initial, "standard_pattern") ?? "") : "credit_card",
  );
  const [pattern, setPattern] = useState(String(a.pattern ?? ""));
  const [namespaces, setNamespaces] = useState(
    ((a.namespaces as string[]) ?? []).join(", "),
  );
  const [excluded, setExcluded] = useState(
    ((a.excluded_namespaces as string[]) ?? []).join(", "),
  );
  const [tags, setTags] = useState(((a.tags as string[]) ?? []).join(", "));
  const rep0 = (a.text_replacement as SdsReplacement | undefined) ?? {
    type: "replacement_string",
    replacement_string: "[redacted]",
  };
  const [repType, setRepType] = useState(rep0.type);
  const [repString, setRepString] = useState(
    rep0.replacement_string ?? "[redacted]",
  );
  const [repChars, setRepChars] = useState(String(rep0.number_of_chars ?? 12));
  const [enabled, setEnabled] = useState(a.is_enabled !== false);
  const [sample, setSample] = useState(
    "card 4111 1111 1111 1111 for bob@example.com",
  );
  const [result, setResult] = useState<{ n: number; text: string } | null>(
    null,
  );
  const [error, setError] = useState<string | null>(null);
  const split = (s: string) =>
    s
      .split(",")
      .map((x) => x.trim())
      .filter(Boolean);
  const replacement: SdsReplacement =
    repType === "replacement_string"
      ? { type: repType, replacement_string: repString }
      : repType.startsWith("partial")
        ? { type: repType, number_of_chars: Number(repChars) }
        : { type: repType };
  const std = (patterns.data ?? []).find((p) => p.id === standard);
  return (
    <Modal
      title={initial ? `Edit rule ${name}` : "New scanning rule"}
      onClose={onClose}
      width={680}
    >
      <div className="modal-body form">
        <Field label="Name">
          <input
            className="input"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </Field>
        <Field label="Pattern">
          <select
            className="input"
            value={standard}
            onChange={(e) => setStandard(e.target.value)}
          >
            <option value="">Custom regex</option>
            {(patterns.data ?? []).map((p) => (
              <option key={p.id} value={p.id}>
                {p.attributes.name}
              </option>
            ))}
          </select>
        </Field>
        {standard ? (
          <p className="muted" style={{ margin: 0 }}>
            {std?.attributes.description} <code>{std?.attributes.pattern}</code>
          </p>
        ) : (
          <Field label="Regex">
            <input
              className="input mono"
              value={pattern}
              onChange={(e) => setPattern(e.target.value)}
            />
          </Field>
        )}
        <div className="form-row">
          <Field
            label="Scan only these attributes"
            hint="Empty: the whole event"
          >
            <input
              className="input mono"
              value={namespaces}
              onChange={(e) => setNamespaces(e.target.value)}
              placeholder="message, usr.email"
            />
          </Field>
          <Field label="Except">
            <input
              className="input mono"
              value={excluded}
              onChange={(e) => setExcluded(e.target.value)}
            />
          </Field>
        </div>
        <div className="form-row">
          <Field label="Action">
            <select
              className="input"
              value={repType}
              onChange={(e) => setRepType(e.target.value)}
            >
              {Object.entries(REPLACEMENTS).map(([k, v]) => (
                <option key={k} value={k}>
                  {v}
                </option>
              ))}
            </select>
          </Field>
          {repType === "replacement_string" && (
            <Field label="Replacement">
              <input
                className="input mono"
                value={repString}
                onChange={(e) => setRepString(e.target.value)}
              />
            </Field>
          )}
          {repType.startsWith("partial") && (
            <Field label="Characters">
              <input
                className="input"
                type="number"
                min={1}
                value={repChars}
                onChange={(e) => setRepChars(e.target.value)}
              />
            </Field>
          )}
        </div>
        <Field
          label="Tags added to matching logs"
          hint="Default: sensitive_data:<rule name>"
        >
          <input
            className="input mono"
            value={tags}
            onChange={(e) => setTags(e.target.value)}
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
        <div className="form-label">Try it</div>
        <div className="form-row">
          <input
            className="input mono"
            style={{ flex: 1 }}
            value={sample}
            onChange={(e) => setSample(e.target.value)}
          />
          <button
            className="btn"
            style={{ flex: "0 0 auto" }}
            onClick={async () => {
              setError(null);
              try {
                const r = await api.sdsTest({
                  pattern,
                  standard_pattern_id: standard || null,
                  text_replacement: replacement,
                  text: sample,
                });
                setResult({ n: r.matches.length, text: r.result });
              } catch (e) {
                setError(errorText(e));
              }
            }}
          >
            Test
          </button>
        </div>
        {result && (
          <div className="sds-test-result">
            <span className="muted">
              {result.n} match{result.n === 1 ? "" : "es"}:
            </span>{" "}
            <span className="mono">{result.text}</span>
          </div>
        )}
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
              await api.sdsSaveRule(
                initial?.id ?? null,
                groupId,
                {
                  name: name.trim(),
                  description: "",
                  ...(standard ? {} : { pattern }),
                  namespaces: split(namespaces),
                  excluded_namespaces: split(excluded),
                  tags: split(tags),
                  text_replacement: replacement,
                  is_enabled: enabled,
                },
                standard || null,
              );
              await qc.invalidateQueries({ queryKey: ["sds"] });
              onClose();
            } catch (e) {
              setError(errorText(e));
            }
          }}
        >
          {initial ? "Save" : "Create rule"}
        </button>
      </div>
    </Modal>
  );
}

export function SensitiveDataScanner() {
  const qc = useQueryClient();
  const cfg = useQuery({ queryKey: ["sds"], queryFn: api.sdsConfig });
  const [group, setGroup] = useState<{ g: SdsJson | null } | null>(null);
  const [rule, setRule] = useState<{ group: string; r: SdsJson | null } | null>(
    null,
  );
  const [error, setError] = useState<string | null>(null);
  const included = cfg.data?.included ?? [];
  const byId = Object.fromEntries(included.map((x) => [x.id, x]));
  const groupIds = cfg.data ? relMany(cfg.data.data, "groups") : [];
  const groups = groupIds.map((id) => byId[id]).filter(Boolean);
  const run = async (f: () => Promise<unknown>) => {
    setError(null);
    try {
      await f();
      await qc.invalidateQueries({ queryKey: ["sds"] });
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <>
      <PageHeader title="Logs" tabs={LOG_TABS}>
        <button className="btn primary" onClick={() => setGroup({ g: null })}>
          <PlusIcon /> New Scanning Group
        </button>
      </PageHeader>
      <div className="page-scroll page-padding">
        <p className="muted" style={{ marginTop: 0, maxWidth: 900 }}>
          Logs are scanned at intake, before they are stored or archived:
          matches are redacted, hashed or masked, and the log is tagged{" "}
          <code>sensitive_data:&lt;rule&gt;</code>. Same API as Datadog:{" "}
          <code>/api/v2/sensitive-data-scanner/config</code>.
        </p>
        {error && <div className="error-box">{error}</div>}
        {groups.length === 0 && cfg.isSuccess && (
          <div className="card muted">No scanning group yet.</div>
        )}
        {groups.map((g, gi) => {
          const ga = g.attributes;
          const rules = relMany(g, "rules")
            .map((id) => byId[id])
            .filter(Boolean);
          return (
            <div
              key={g.id}
              className="card sds-group"
              style={{ padding: 0, marginBottom: 16 }}
            >
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 8,
                  padding: "10px 12px",
                  borderBottom: "1px solid var(--border)",
                }}
              >
                <OrderButtons
                  i={gi}
                  n={groups.length}
                  onMove={(d) =>
                    run(() => api.sdsReorderGroups(move(groupIds, gi, d)))
                  }
                />
                <strong>{String(ga.name)}</strong>
                {ga.is_enabled === false && (
                  <span className="pill">disabled</span>
                )}
                <span className="mono muted">
                  {String((ga.filter as { query?: string })?.query ?? "*")}
                </span>
                <div className="spacer" />
                <button
                  className="btn"
                  onClick={() => setRule({ group: g.id, r: null })}
                >
                  <PlusIcon /> Add rule
                </button>
                <button className="btn" onClick={() => setGroup({ g })}>
                  Edit
                </button>
                <button
                  className="btn icon ghost"
                  title="Delete"
                  onClick={() => run(() => api.sdsDeleteGroup(g.id))}
                >
                  <TrashIcon />
                </button>
              </div>
              <table className="list">
                <thead>
                  <tr>
                    <th>Rule</th>
                    <th>Pattern</th>
                    <th>Scope</th>
                    <th>Action</th>
                    <th>Tags</th>
                    <th style={{ width: 120 }} />
                  </tr>
                </thead>
                <tbody>
                  {rules.map((r) => {
                    const ra = r.attributes;
                    const rep = ra.text_replacement as SdsReplacement;
                    const sp = relOne(r, "standard_pattern");
                    return (
                      <tr
                        key={r.id}
                        className={ra.is_enabled === false ? "muted" : ""}
                      >
                        <td>{String(ra.name)}</td>
                        <td
                          className="mono"
                          style={{
                            maxWidth: 280,
                            overflow: "hidden",
                            textOverflow: "ellipsis",
                          }}
                        >
                          {sp ? (
                            <span className="pill">{sp}</span>
                          ) : (
                            String(ra.pattern)
                          )}
                        </td>
                        <td className="mono">
                          {((ra.namespaces as string[]) ?? []).join(", ") ||
                            "whole event"}
                        </td>
                        <td>
                          {REPLACEMENTS[rep?.type] ?? rep?.type}
                          {rep?.replacement_string
                            ? ` (${rep.replacement_string})`
                            : ""}
                          {rep?.number_of_chars
                            ? ` (${rep.number_of_chars})`
                            : ""}
                        </td>
                        <td className="mono">
                          {((ra.tags as string[]) ?? []).join(", ")}
                        </td>
                        <td style={{ textAlign: "right" }}>
                          <button
                            className="btn"
                            onClick={() => setRule({ group: g.id, r })}
                          >
                            Edit
                          </button>{" "}
                          <button
                            className="btn icon ghost"
                            title="Delete"
                            onClick={() => run(() => api.sdsDeleteRule(r.id))}
                          >
                            <TrashIcon />
                          </button>
                        </td>
                      </tr>
                    );
                  })}
                  {rules.length === 0 && (
                    <tr>
                      <td colSpan={6} className="muted">
                        No rule in this group.
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
          );
        })}
      </div>
      {group && <GroupModal initial={group.g} onClose={() => setGroup(null)} />}
      {rule && (
        <RuleModal
          groupId={rule.group}
          initial={rule.r}
          onClose={() => setRule(null)}
        />
      )}
    </>
  );
}

// ---------------------------------------------------------------- Reference Tables

function TableModal({
  initial,
  onClose,
}: {
  initial: ReferenceTable | null;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const a = initial?.attributes;
  const [name, setName] = useState(a?.table_name ?? "");
  const [description, setDescription] = useState(a?.description ?? "");
  const [key, setKey] = useState(a?.schema.primary_keys[0] ?? "");
  const [csv, setCsv] = useState("");
  const [error, setError] = useState<string | null>(null);
  const header =
    csv
      .split("\n")[0]
      ?.split(",")
      .map((h) => h.trim())
      .filter(Boolean) ?? [];
  return (
    <Modal
      title={initial ? `Replace ${a?.table_name}` : "New reference table"}
      onClose={onClose}
      width={680}
    >
      <div className="modal-body form">
        <div className="form-row">
          <Field label="Name">
            <input
              className="input mono"
              value={name}
              disabled={!!initial}
              onChange={(e) => setName(e.target.value)}
              placeholder="service_owners"
            />
          </Field>
          <Field
            label="Primary key"
            hint={initial ? undefined : "Default: the first column"}
          >
            {initial ? (
              <input className="input mono" value={key} disabled />
            ) : (
              <select
                className="input"
                value={key}
                onChange={(e) => setKey(e.target.value)}
              >
                <option value="">{header[0] ?? "—"}</option>
                {header.slice(1).map((h) => (
                  <option key={h} value={h}>
                    {h}
                  </option>
                ))}
              </select>
            )}
          </Field>
        </div>
        <Field label="Description">
          <input
            className="input"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />
        </Field>
        <Field label="CSV" hint="With a header row. Up to 1,000,000 rows.">
          <textarea
            className="input mono"
            rows={10}
            value={csv}
            onChange={(e) => setCsv(e.target.value)}
            placeholder={"service,team,tier\npayment-service,payments,1"}
          />
        </Field>
        <input
          type="file"
          accept=".csv,text/csv"
          onChange={async (e) => {
            const f = e.target.files?.[0];
            if (f) setCsv(await f.text());
          }}
        />
        {error && <div className="error-box">{error}</div>}
      </div>
      <div className="modal-footer">
        <button className="btn" onClick={onClose}>
          Cancel
        </button>
        <button
          className="btn primary"
          disabled={!csv.trim() && !initial}
          onClick={async () => {
            setError(null);
            try {
              if (initial)
                await api.replaceReferenceTable(initial.id, csv, description);
              else
                await api.createReferenceTable({
                  table_name: name.trim(),
                  description,
                  primary_key: key || header[0] || "",
                  csv,
                });
              await qc.invalidateQueries({ queryKey: ["reference-tables"] });
              onClose();
            } catch (e) {
              setError(errorText(e));
            }
          }}
        >
          {initial ? "Save" : "Create table"}
        </button>
      </div>
    </Modal>
  );
}

function TableRows({ t }: { t: ReferenceTable }) {
  const [search, setSearch] = useState("");
  const rows = useQuery({
    queryKey: ["reference-table-rows", t.id, search],
    queryFn: () => api.referenceTableRows(t.id, search),
  });
  const fields = t.attributes.schema.fields;
  return (
    <div style={{ padding: 12 }}>
      <input
        className="input"
        style={{ maxWidth: 320, marginBottom: 8 }}
        placeholder="Search rows"
        value={search}
        onChange={(e) => setSearch(e.target.value)}
      />
      <div style={{ overflowX: "auto" }}>
        <table className="list">
          <thead>
            <tr>
              {fields.map((f) => (
                <th key={f.name}>
                  {f.name} <span className="muted">{f.type.toLowerCase()}</span>
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {(rows.data?.data ?? []).map((r) => (
              <tr key={r.id}>
                {fields.map((f) => (
                  <td key={f.name} className="mono">
                    {String(r.attributes.values[f.name] ?? "")}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {rows.data && (
        <div className="muted" style={{ fontSize: 12, marginTop: 6 }}>
          {rows.data.data.length} of {fmtNum(rows.data.meta.total)} rows
        </div>
      )}
    </div>
  );
}

export function ReferenceTables() {
  const qc = useQueryClient();
  const list = useQuery({
    queryKey: ["reference-tables"],
    queryFn: api.referenceTables,
  });
  const [editing, setEditing] = useState<{ t: ReferenceTable | null } | null>(
    null,
  );
  const [open, setOpen] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  return (
    <>
      <PageHeader title="Logs" tabs={LOG_TABS}>
        <button className="btn primary" onClick={() => setEditing({ t: null })}>
          <PlusIcon /> New Reference Table
        </button>
      </PageHeader>
      <div className="page-scroll page-padding">
        <p className="muted" style={{ marginTop: 0, maxWidth: 900 }}>
          CSV tables that enrich logs with a lookup processor (
          <code>lookup_enrichment_table</code>) and filter searches:{" "}
          <code>@usr.id:reference_table(customers, tier:gold)</code>. Same API
          as Datadog: <code>/api/v2/reference-tables/tables</code>.
        </p>
        {error && <div className="error-box">{error}</div>}
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Table</th>
                <th>Description</th>
                <th>Primary key</th>
                <th>Columns</th>
                <th style={{ width: 90 }}>Rows</th>
                <th style={{ width: 170 }}>Updated</th>
                <th style={{ width: 190 }} />
              </tr>
            </thead>
            <tbody>
              {(list.data ?? []).map((t) => {
                const a = t.attributes;
                return [
                  <tr key={t.id} className="reference-table-row">
                    <td>
                      <button
                        className="link-button mono"
                        onClick={() => setOpen(open === t.id ? null : t.id)}
                      >
                        {open === t.id ? "▾" : "▸"} {a.table_name}
                      </button>
                    </td>
                    <td>{a.description}</td>
                    <td className="mono">{a.schema.primary_keys.join(", ")}</td>
                    <td className="mono">
                      {a.schema.fields.map((f) => f.name).join(", ")}
                    </td>
                    <td>{fmtNum(a.row_count)}</td>
                    <td>{new Date(a.updated_at).toLocaleString()}</td>
                    <td style={{ textAlign: "right" }}>
                      <button className="btn" onClick={() => setEditing({ t })}>
                        Replace CSV
                      </button>{" "}
                      <button
                        className="btn icon ghost"
                        title="Delete"
                        onClick={async () => {
                          setError(null);
                          try {
                            await api.deleteReferenceTable(t.id);
                            await qc.invalidateQueries({
                              queryKey: ["reference-tables"],
                            });
                          } catch (e) {
                            setError(errorText(e));
                          }
                        }}
                      >
                        <TrashIcon />
                      </button>
                    </td>
                  </tr>,
                  open === t.id && (
                    <tr key={`${t.id}-rows`}>
                      <td colSpan={7} style={{ padding: 0 }}>
                        <TableRows t={t} />
                      </td>
                    </tr>
                  ),
                ];
              })}
              {(list.data ?? []).length === 0 && (
                <tr>
                  <td colSpan={7} className="muted">
                    No reference table yet.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </div>
      {editing && (
        <TableModal initial={editing.t} onClose={() => setEditing(null)} />
      )}
    </>
  );
}
