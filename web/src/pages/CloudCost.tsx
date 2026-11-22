import {
  keepPreviousData,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { useRef, useState } from "react";
import { TimeChart } from "../components/Charts";
import { Modal } from "../components/Common";
import { PlusIcon, TrashIcon } from "../components/Icons";
import { PageHeader } from "../components/Layout";
import { TimePicker } from "../components/TimePicker";
import { api, CostAccount, CostCluster } from "../lib/api";
import { ago, serviceColor } from "../lib/format";
import { useTimeRange } from "../lib/time";

export const COST_TABS = [
  { to: "/cost", label: "Explorer" },
  { to: "/cost/kubernetes", label: "Kubernetes" },
  { to: "/cost/settings", label: "Settings" },
];

const DAY = 86_400_000;

function money(v: number, currency?: string | null) {
  const c = currency && /^[A-Z]{3}$/.test(currency) ? currency : undefined;
  try {
    return new Intl.NumberFormat(undefined, {
      style: c ? "currency" : "decimal",
      currency: c,
      maximumFractionDigits: v < 10 ? 2 : 0,
    }).format(v);
  } catch {
    return v.toFixed(2);
  }
}

function bytes(v: number) {
  const u = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v < 10 ? 1 : 0)} ${u[i]}`;
}

/** Daily costs, grouped. */
export function CostExplorer() {
  const time = useTimeRange("30d");
  const [group, setGroup] = useState("provider");
  const [tag, setTag] = useState("");
  const [provider, setProvider] = useState("");
  const g = group === "tag" ? `tag:${tag.trim()}` : group;
  const q = useQuery({
    queryKey: ["cost", time.from, time.to, g, provider],
    queryFn: () =>
      api.cost({ from: time.from, to: time.to, group: g, provider }),
    placeholderData: keepPreviousData,
    enabled: group !== "tag" || !!tag.trim(),
  });
  const d = q.data;
  const cur = d?.currencies.length === 1 ? d.currencies[0] : null;
  const change =
    d && d.previous_total > 0
      ? (d.total - d.previous_total) / d.previous_total
      : null;
  return (
    <>
      <PageHeader title="Cloud Cost" tabs={COST_TABS}>
        <TimePicker time={time} />
      </PageHeader>
      <div className="filters-row">
        <label className="muted">Group by</label>
        <select
          className="input"
          value={group}
          onChange={(e) => setGroup(e.target.value)}
        >
          <option value="provider">Provider</option>
          <option value="service">Service</option>
          <option value="account">Account</option>
          <option value="resource">Resource</option>
          <option value="workload">Workload</option>
          <option value="tag">Tag…</option>
        </select>
        {group === "tag" && (
          <input
            className="input"
            placeholder="tag key (team, env…)"
            value={tag}
            onChange={(e) => setTag(e.target.value)}
          />
        )}
        <select
          className="input"
          value={provider}
          onChange={(e) => setProvider(e.target.value)}
        >
          <option value="">All providers</option>
          {["aws", "gcp", "azure", "kubernetes", "custom"].map((p) => (
            <option key={p}>{p}</option>
          ))}
        </select>
        {d && d.currencies.length > 1 && (
          <span className="muted">
            Mixed currencies: {d.currencies.join(", ")}
          </span>
        )}
      </div>
      <div className="page-scroll page-padding">
        <div
          className="stat-row"
          style={{ display: "flex", gap: 24, marginBottom: 12 }}
        >
          <div>
            <div className="muted">Total</div>
            <div style={{ fontSize: 24, fontWeight: 600 }}>
              {d ? money(d.total, cur) : "…"}
            </div>
          </div>
          <div>
            <div className="muted">Previous period</div>
            <div style={{ fontSize: 24 }}>
              {d ? money(d.previous_total, cur) : "…"}
              {change != null && (
                <span
                  style={{
                    fontSize: 14,
                    marginLeft: 8,
                    color: change > 0 ? "#d6574f" : "#38a38a",
                  }}
                >
                  {change > 0 ? "+" : ""}
                  {(change * 100).toFixed(1)}%
                </span>
              )}
            </div>
          </div>
        </div>
        {d && d.days.length > 0 && (
          <div className="card">
            <TimeChart
              times={d.days}
              interval={DAY}
              from={d.days[0]}
              to={d.days[d.days.length - 1] + DAY}
              height={180}
              format={(v) => money(v, cur)}
              series={d.groups.map((s) => ({
                key: s.key,
                color: serviceColor(s.key),
                values: s.values,
              }))}
            />
          </div>
        )}
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>
                  {group === "tag"
                    ? tag || "Tag"
                    : group[0].toUpperCase() + group.slice(1)}
                </th>
                <th style={{ textAlign: "right" }}>Cost</th>
                <th style={{ textAlign: "right" }}>Share</th>
              </tr>
            </thead>
            <tbody>
              {d?.groups.map((s) => (
                <tr key={s.key}>
                  <td>
                    <span
                      className="dot"
                      style={{
                        background: serviceColor(s.key),
                        display: "inline-block",
                        width: 8,
                        height: 8,
                        borderRadius: 4,
                        marginRight: 6,
                      }}
                    />
                    {s.key}
                  </td>
                  <td style={{ textAlign: "right" }}>{money(s.total, cur)}</td>
                  <td style={{ textAlign: "right" }} className="muted">
                    {d.total > 0 ? ((s.total / d.total) * 100).toFixed(1) : 0}%
                  </td>
                </tr>
              ))}
              {d && !d.groups.length && (
                <tr>
                  <td colSpan={3} className="muted">
                    No costs over this period. Add a cluster, a cloud account or
                    a Custom Costs file in <a href="/cost/settings">Settings</a>
                    .
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
        <p className="muted">
          Every cost is also the <code>all.cost</code> and{" "}
          <code>&lt;provider&gt;.cost.amortized</code> metrics, daily, for
          dashboards and monitors.
        </p>
      </div>
    </>
  );
}

/** A cluster's price across namespaces and workloads. */
export function CostKubernetes() {
  const time = useTimeRange("30d");
  const q = useQuery({
    queryKey: ["cost-k8s", time.from, time.to],
    queryFn: () => api.costKubernetes({ from: time.from, to: time.to }),
    placeholderData: keepPreviousData,
  });
  const rows = q.data?.data ?? [];
  const total = rows.reduce((a, r) => a + r.cost, 0);
  const idle = rows
    .filter((r) => r.namespace === "__idle__")
    .reduce((a, r) => a + r.cost, 0);
  const cur = rows[0]?.currency;
  return (
    <>
      <PageHeader title="Kubernetes costs" tabs={COST_TABS}>
        <TimePicker time={time} />
      </PageHeader>
      <div className="filters-row">
        <span>
          Total <b>{money(total, cur)}</b>
        </span>
        <span>
          Idle <b>{money(idle, cur)}</b>
        </span>
        <span>
          Efficiency{" "}
          <b>
            {total > 0 ? (((total - idle) / total) * 100).toFixed(1) : "–"}%
          </b>
        </span>
      </div>
      <div className="page-scroll page-padding">
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Cluster</th>
                <th>Namespace</th>
                <th>Workload</th>
                <th style={{ textAlign: "right" }}>Pods</th>
                <th style={{ textAlign: "right" }}>Avg CPU</th>
                <th style={{ textAlign: "right" }}>Avg memory</th>
                <th style={{ textAlign: "right" }}>Cost</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r, i) => (
                <tr key={i}>
                  <td>{r.cluster}</td>
                  <td>
                    {r.namespace === "__idle__" ? (
                      <i className="muted">idle capacity</i>
                    ) : (
                      r.namespace
                    )}
                  </td>
                  <td>{r.workload}</td>
                  <td style={{ textAlign: "right" }}>
                    {r.namespace === "__idle__" ? "" : r.pods}
                  </td>
                  <td style={{ textAlign: "right" }}>
                    {r.cpu_cores != null
                      ? `${r.cpu_cores.toFixed(2)} cores`
                      : ""}
                  </td>
                  <td style={{ textAlign: "right" }}>
                    {r.memory_bytes != null ? bytes(r.memory_bytes) : ""}
                  </td>
                  <td style={{ textAlign: "right" }}>
                    {money(r.cost, r.currency)}
                  </td>
                </tr>
              ))}
              {q.data && !rows.length && (
                <tr>
                  <td colSpan={7} className="muted">
                    No cluster yet: add one in{" "}
                    <a href="/cost/settings">Settings</a> with its monthly
                    price.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
        <p className="muted">
          A cluster's daily price is split across its pods by their CPU (
          <code>kubernetes.cpu.usage.total</code>) and memory (
          <code>kubernetes.memory.usage</code>) against the nodes' allocatable
          capacity; the rest is idle.
        </p>
      </div>
    </>
  );
}

function Clusters() {
  const qc = useQueryClient();
  const q = useQuery({
    queryKey: ["cost-clusters"],
    queryFn: api.costClusters,
  });
  const [edit, setEdit] = useState<Partial<CostCluster> | null>(null);
  const [error, setError] = useState("");
  const save = useMutation({
    mutationFn: () => api.saveCostCluster(edit as CostCluster),
    onSuccess: () => {
      setEdit(null);
      qc.invalidateQueries({ queryKey: ["cost-clusters"] });
    },
    onError: (e: Error) => setError(e.message),
  });
  const del = useMutation({
    mutationFn: (id: string) => api.deleteCostCluster(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["cost-clusters"] }),
  });
  const compute = useMutation({
    mutationFn: (id: string) => api.computeCostCluster(id, 30),
  });
  return (
    <>
      <div
        className="section-title"
        style={{ display: "flex", alignItems: "center" }}
      >
        Kubernetes clusters
        <div className="spacer" />
        <button
          className="btn small"
          onClick={() => {
            setError("");
            setEdit({
              name: "",
              filter: "*",
              monthly_cost: 0,
              currency: "EUR",
              cpu_share: 0.5,
            });
          }}
        >
          <PlusIcon /> New cluster
        </button>
      </div>
      <div className="card" style={{ padding: 0 }}>
        <table className="list">
          <thead>
            <tr>
              <th>Name</th>
              <th>Metrics filter</th>
              <th style={{ textAlign: "right" }}>Monthly price</th>
              <th>CPU share</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {q.data?.data.map((c) => (
              <tr key={c.id}>
                <td>
                  <a
                    href="#"
                    onClick={(e) => {
                      e.preventDefault();
                      setError("");
                      setEdit(c);
                    }}
                  >
                    {c.name}
                  </a>
                </td>
                <td className="mono">{c.filter}</td>
                <td style={{ textAlign: "right" }}>
                  {money(c.monthly_cost, c.currency)}
                </td>
                <td>{Math.round(c.cpu_share * 100)}%</td>
                <td style={{ whiteSpace: "nowrap" }}>
                  <button
                    className="btn small"
                    disabled={compute.isPending}
                    onClick={() => compute.mutate(c.id!)}
                  >
                    Recompute 30 days
                  </button>{" "}
                  <button
                    className="btn icon ghost"
                    title="Delete"
                    onClick={() => del.mutate(c.id!)}
                  >
                    <TrashIcon />
                  </button>
                </td>
              </tr>
            ))}
            {q.data && !q.data.data.length && (
              <tr>
                <td colSpan={5} className="muted">
                  None.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      {compute.data && (
        <p className="muted">{compute.data.rows} cost rows computed.</p>
      )}
      {edit && (
        <Modal
          title={edit.id ? "Edit cluster" : "New cluster"}
          onClose={() => setEdit(null)}
        >
          <label className="form-field">
            Name
            <input
              className="input"
              value={edit.name}
              onChange={(e) => setEdit({ ...edit, name: e.target.value })}
            />
          </label>
          <label className="form-field">
            Metrics filter (its nodes and pods)
            <input
              className="input mono"
              placeholder="kube_cluster_name:silo01"
              value={edit.filter}
              onChange={(e) => setEdit({ ...edit, filter: e.target.value })}
            />
          </label>
          <div style={{ display: "flex", gap: 8 }}>
            <label className="form-field" style={{ flex: 1 }}>
              Monthly price
              <input
                className="input"
                type="number"
                min={0}
                step="0.01"
                value={edit.monthly_cost}
                onChange={(e) =>
                  setEdit({ ...edit, monthly_cost: Number(e.target.value) })
                }
              />
            </label>
            <label className="form-field" style={{ width: 90 }}>
              Currency
              <input
                className="input"
                value={edit.currency}
                onChange={(e) =>
                  setEdit({ ...edit, currency: e.target.value.toUpperCase() })
                }
              />
            </label>
            <label className="form-field" style={{ width: 110 }}>
              CPU share %
              <input
                className="input"
                type="number"
                min={0}
                max={100}
                value={Math.round((edit.cpu_share ?? 0.5) * 100)}
                onChange={(e) =>
                  setEdit({ ...edit, cpu_share: Number(e.target.value) / 100 })
                }
              />
            </label>
          </div>
          {error && <div className="error-text">{error}</div>}
          <div className="toolbar">
            <div className="spacer" />
            <button
              className="btn primary"
              disabled={!edit.name?.trim()}
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

function CustomCosts() {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ["custom-costs"], queryFn: api.customCosts });
  const input = useRef<HTMLInputElement>(null);
  const [error, setError] = useState("");
  const upload = useMutation({
    mutationFn: (f: File) => api.uploadCustomCosts(f),
    onSuccess: () => {
      setError("");
      qc.invalidateQueries({ queryKey: ["custom-costs"] });
    },
    onError: (e: Error) => setError(e.message),
  });
  const del = useMutation({
    mutationFn: (id: string) => api.deleteCustomCosts(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["custom-costs"] }),
  });
  return (
    <>
      <div
        className="section-title"
        style={{ display: "flex", alignItems: "center" }}
      >
        Custom Costs
        <div className="spacer" />
        <input
          ref={input}
          type="file"
          accept=".csv,.json,.gz"
          style={{ display: "none" }}
          onChange={(e) => {
            const f = e.target.files?.[0];
            if (f) upload.mutate(f);
            e.target.value = "";
          }}
        />
        <button
          className="btn small"
          disabled={upload.isPending}
          onClick={() => input.current?.click()}
        >
          <PlusIcon /> Upload a file
        </button>
      </div>
      {error && <div className="error-text">{error}</div>}
      <div className="card" style={{ padding: 0 }}>
        <table className="list">
          <thead>
            <tr>
              <th>File</th>
              <th>Providers</th>
              <th>Period</th>
              <th style={{ textAlign: "right" }}>Rows</th>
              <th style={{ textAlign: "right" }}>Cost</th>
              <th>Uploaded</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {q.data?.data.map((f) => (
              <tr key={f.id}>
                <td>{f.attributes.name}</td>
                <td>{f.attributes.providers?.join(", ")}</td>
                <td className="muted">
                  {f.attributes.charge_period.start?.slice(0, 10)} →{" "}
                  {f.attributes.charge_period.end?.slice(0, 10)}
                </td>
                <td style={{ textAlign: "right" }}>{f.attributes.rows}</td>
                <td style={{ textAlign: "right" }}>
                  {money(f.attributes.billed_cost, f.attributes.currency)}
                </td>
                <td className="muted">
                  {f.attributes.uploaded_at
                    ? ago(Date.parse(f.attributes.uploaded_at))
                    : ""}
                </td>
                <td>
                  <button
                    className="btn icon ghost"
                    title="Delete the file and its costs"
                    onClick={() => del.mutate(f.id)}
                  >
                    <TrashIcon />
                  </button>
                </td>
              </tr>
            ))}
            {q.data && !q.data.data.length && (
              <tr>
                <td colSpan={7} className="muted">
                  None.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      <p className="muted">
        FOCUS files (CSV or JSON, gzip or not): <code>ChargePeriodStart</code>,{" "}
        <code>BilledCost</code>, <code>BillingCurrency</code>,{" "}
        <code>ProviderName</code>, <code>ServiceName</code> or{" "}
        <code>ChargeDescription</code>, <code>Tags</code>. AWS CUR columns work
        too. Also <code>PUT /api/v2/cost/custom_costs</code>.
      </p>
    </>
  );
}

const ACCOUNT_HELP: Record<string, string> = {
  s3: "AWS Data Exports (FOCUS 1.0, CSV) or the CUR in S3; GCS with HMAC keys (endpoint https://storage.googleapis.com); OVH, MinIO, any S3 API.",
  azure:
    "A Cost Management export (FOCUS, CSV) to a Storage container: its SAS URL, with read and list rights.",
  http: "One FOCUS file at a URL, read again when its ETag changes (or daily).",
};

function CloudAccounts() {
  const qc = useQueryClient();
  const q = useQuery({
    queryKey: ["cost-accounts"],
    queryFn: api.costAccounts,
  });
  const [edit, setEdit] = useState<Partial<CostAccount> | null>(null);
  const [error, setError] = useState("");
  const save = useMutation({
    mutationFn: () => api.saveCostAccount(edit as CostAccount),
    onSuccess: () => {
      setEdit(null);
      qc.invalidateQueries({ queryKey: ["cost-accounts"] });
    },
    onError: (e: Error) => setError(e.message),
  });
  const del = useMutation({
    mutationFn: (id: string) => api.deleteCostAccount(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["cost-accounts"] }),
  });
  const run = useMutation({
    mutationFn: (id: string) => api.importCostAccount(id),
    onSettled: () => qc.invalidateQueries({ queryKey: ["cost-accounts"] }),
  });
  const field = (
    k: keyof CostAccount,
    label: string,
    placeholder = "",
    type = "text",
  ) => (
    <label className="form-field">
      {label}
      <input
        className="input"
        type={type}
        placeholder={placeholder}
        value={(edit?.[k] as string) ?? ""}
        onChange={(e) => setEdit({ ...edit, [k]: e.target.value })}
      />
    </label>
  );
  return (
    <>
      <div
        className="section-title"
        style={{ display: "flex", alignItems: "center" }}
      >
        Cloud accounts
        <div className="spacer" />
        <button
          className="btn small"
          onClick={() => {
            setError("");
            setEdit({ type: "s3", name: "", provider: "AWS" });
          }}
        >
          <PlusIcon /> New account
        </button>
      </div>
      <div className="card" style={{ padding: 0 }}>
        <table className="list">
          <thead>
            <tr>
              <th>Name</th>
              <th>Source</th>
              <th style={{ textAlign: "right" }}>Files</th>
              <th>Last import</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {q.data?.data.map((a) => (
              <tr key={a.id}>
                <td>
                  <a
                    href="#"
                    onClick={(e) => {
                      e.preventDefault();
                      setError("");
                      setEdit(a);
                    }}
                  >
                    {a.name}
                  </a>
                </td>
                <td className="mono">
                  {a.type === "s3"
                    ? `s3://${a.bucket}/${a.prefix ?? ""}`
                    : a.type === "azure"
                      ? a.sas_url?.split("?")[0]
                      : a.url}
                </td>
                <td style={{ textAlign: "right" }}>{a.files}</td>
                <td>
                  {a.last_import ? (
                    <span className="muted">{ago(a.last_import)}</span>
                  ) : (
                    "never"
                  )}
                  {a.last_error && (
                    <div className="error-text">{a.last_error}</div>
                  )}
                </td>
                <td style={{ whiteSpace: "nowrap" }}>
                  <button
                    className="btn small"
                    disabled={run.isPending}
                    onClick={() => run.mutate(a.id!)}
                  >
                    Import now
                  </button>{" "}
                  <button
                    className="btn icon ghost"
                    title="Delete the account and its costs"
                    onClick={() => del.mutate(a.id!)}
                  >
                    <TrashIcon />
                  </button>
                </td>
              </tr>
            ))}
            {q.data && !q.data.data.length && (
              <tr>
                <td colSpan={5} className="muted">
                  None.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      {run.data && (
        <p className="muted">
          {run.data.files} file(s) imported, {run.data.rows} cost rows.
        </p>
      )}
      {run.error && (
        <div className="error-text">{(run.error as Error).message}</div>
      )}
      {edit && (
        <Modal
          title={edit.id ? "Edit account" : "New cloud account"}
          onClose={() => setEdit(null)}
          width={560}
        >
          <div style={{ display: "flex", gap: 8 }}>
            <label className="form-field" style={{ flex: 1 }}>
              Source
              <select
                className="input"
                value={edit.type}
                onChange={(e) =>
                  setEdit({
                    ...edit,
                    type: e.target.value as CostAccount["type"],
                  })
                }
              >
                <option value="s3">S3 bucket (AWS, GCS, OVH, MinIO…)</option>
                <option value="azure">Azure Storage container</option>
                <option value="http">HTTP URL</option>
              </select>
            </label>
            <label className="form-field" style={{ width: 160 }}>
              Provider (if the file has none)
              <input
                className="input"
                value={edit.provider ?? ""}
                onChange={(e) => setEdit({ ...edit, provider: e.target.value })}
              />
            </label>
          </div>
          <p className="muted" style={{ marginTop: 0 }}>
            {ACCOUNT_HELP[edit.type ?? "s3"]}
          </p>
          {field("name", "Name")}
          {edit.type === "s3" && (
            <>
              {field("bucket", "Bucket")}
              {field("prefix", "Prefix", "exports/focus/")}
              {field("region", "Region", "eu-west-3")}
              {field(
                "endpoint",
                "Endpoint (not AWS)",
                "https://storage.googleapis.com",
              )}
              {field("access_key_id", "Access key ID")}
              {field("secret_access_key", "Secret access key", "", "password")}
            </>
          )}
          {edit.type === "azure" && (
            <>
              {field(
                "sas_url",
                "Container SAS URL",
                "https://account.blob.core.windows.net/exports?sv=…&sig=…",
                "password",
              )}
              {field("prefix", "Prefix", "focus/")}
            </>
          )}
          {edit.type === "http" && (
            <>
              {field("url", "URL")}
              {field("header_name", "Header (optional)", "Authorization")}
              {field("header_value", "Header value", "", "password")}
            </>
          )}
          {error && <div className="error-text">{error}</div>}
          <div className="toolbar">
            <div className="spacer" />
            <button
              className="btn primary"
              disabled={!edit.name?.trim()}
              onClick={() => save.mutate()}
            >
              Save and import
            </button>
          </div>
        </Modal>
      )}
    </>
  );
}

/** Where costs come from. */
export function CostSettings() {
  return (
    <>
      <PageHeader title="Cloud Cost settings" tabs={COST_TABS} />
      <div className="page-scroll page-padding">
        <CloudAccounts />
        <Clusters />
        <CustomCosts />
      </div>
    </>
  );
}
