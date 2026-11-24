import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Modal } from "../components/Common";
import { PageHeader } from "../components/Layout";
import { PlusIcon, TrashIcon } from "../components/Icons";
import { api, ApiKey } from "../lib/api";
import { ago } from "../lib/format";

function KeyTable({ kind }: { kind: "api" | "app" }) {
  const qc = useQueryClient();
  const q = useQuery({
    queryKey: ["keys", kind],
    queryFn: () => api.keys(kind),
  });
  const [name, setName] = useState<string | null>(null);
  const [created, setCreated] = useState<string | null>(null);
  const create = useMutation({
    mutationFn: () => api.createKey(kind, name!.trim()),
    onSuccess: (r) => {
      setName(null);
      setCreated(r.data.attributes.key ?? null);
      qc.invalidateQueries({ queryKey: ["keys", kind] });
    },
  });
  const revoke = useMutation({
    mutationFn: (id: string) => api.deleteKey(kind, id),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["keys", kind] }),
  });
  const label = kind === "api" ? "API key" : "Application key";
  return (
    <>
      <div
        className="section-title"
        style={{ display: "flex", alignItems: "center" }}
      >
        {label}s
        <div className="spacer" />
        <button className="btn small" onClick={() => setName("")}>
          <PlusIcon /> New {label.toLowerCase()}
        </button>
      </div>
      <div className="card" style={{ padding: 0 }}>
        <table className="list">
          <thead>
            <tr>
              <th>Name</th>
              <th>Key</th>
              <th>Created</th>
              <th>Last used</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {q.data?.data.map((k: ApiKey) => (
              <tr key={k.id}>
                <td>{k.attributes.name}</td>
                <td className="mono">…{k.attributes.last4}</td>
                <td className="muted">
                  {k.attributes.created_at
                    ? ago(Date.parse(k.attributes.created_at))
                    : ""}
                </td>
                <td className="muted">
                  {k.attributes.last_used_at
                    ? ago(Date.parse(k.attributes.last_used_at))
                    : "never"}
                </td>
                <td>
                  <button
                    className="btn icon ghost"
                    title="Revoke"
                    onClick={() => revoke.mutate(k.id)}
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
      {name != null && (
        <Modal
          title={`New ${label.toLowerCase()}`}
          onClose={() => setName(null)}
        >
          <div className="modal-body">
            <label className="form-field">
              Name
              <input
                className="input"
                autoFocus
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </label>
          </div>
          <div className="modal-footer">
            <button
              className="btn primary"
              disabled={!name.trim()}
              onClick={() => create.mutate()}
            >
              Create
            </button>
          </div>
        </Modal>
      )}
      {created && (
        <Modal
          title={`${label} created`}
          onClose={() => setCreated(null)}
          width={480}
        >
          <div className="modal-body">
            <p>Copy it now: it is not shown again.</p>
            <pre
              className="mono"
              style={{
                userSelect: "all",
                wordBreak: "break-all",
                whiteSpace: "pre-wrap",
              }}
            >
              {created}
            </pre>
          </div>
        </Modal>
      )}
    </>
  );
}

/** API keys and application keys. */
export function ApiKeys() {
  return (
    <>
      <PageHeader title="API Keys" />
      <div className="filters-row">
        <span className="muted">
          API keys are accepted wherever <code>ROBINE_DOG_API_KEY</code> is
          (Agents, tracers, SDKs, the public API), which stays valid.
          Application keys are recorded when a client sends them (
          <code>DD-APPLICATION-KEY</code>).
        </span>
      </div>
      <div className="page-scroll page-padding">
        <KeyTable kind="api" />
        <KeyTable kind="app" />
      </div>
    </>
  );
}

export function Snippet({ title, text }: { title: string; text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="card">
      <div className="card-title">
        {title}
        <div className="spacer" />
        <button
          className="btn small"
          onClick={() => {
            navigator.clipboard?.writeText(text).then(() => setCopied(true));
          }}
        >
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
      <pre
        className="mono"
        style={{ overflow: "auto", fontSize: 12, margin: 0 }}
      >
        {text}
      </pre>
    </div>
  );
}

const origin = () => window.location.origin;
const hostPort = () =>
  window.location.host +
  (window.location.port
    ? ""
    : window.location.protocol === "https:"
      ? ":443"
      : ":80");

/** Install the Datadog Agent against robine-dog. */
export function AgentSetup() {
  const url = origin();
  const logs = hostPort();
  const noSsl = window.location.protocol === "http:";
  const env: [string, string][] = [
    ["DD_SITE", "robine-dog.invalid"],
    ["DD_DD_URL", url],
    ["DD_APM_DD_URL", url],
    ["DD_LOGS_ENABLED", "true"],
    ["DD_LOGS_CONFIG_USE_HTTP", "true"],
    ["DD_LOGS_CONFIG_LOGS_DD_URL", logs],
    ...(noSsl
      ? ([["DD_LOGS_CONFIG_LOGS_NO_SSL", "true"]] as [string, string][])
      : []),
    ["DD_PROCESS_CONFIG_PROCESS_DD_URL", url],
    ["DD_PROCESS_CONFIG_PROCESS_COLLECTION_ENABLED", "true"],
    ["DD_REMOTE_CONFIGURATION_ENABLED", "false"],
  ];
  const docker = `docker run -d --name dd-agent \\
  -e DD_API_KEY=<API key> \\
${env.map(([k, v]) => `  -e ${k}=${v} \\`).join("\n")}
  -v /var/run/docker.sock:/var/run/docker.sock:ro \\
  -v /proc/:/host/proc/:ro -v /sys/fs/cgroup/:/host/sys/fs/cgroup:ro \\
  -p 8126:8126 -p 8125:8125/udp datadog/agent:7`;
  const helm = `# helm repo add datadog https://helm.datadoghq.com
# helm install datadog-agent datadog/datadog -f values.yaml
datadog:
  apiKey: <API key>
  site: robine-dog.invalid
  dd_url: ${url}
  logs:
    enabled: true
    containerCollectAll: true
  apm:
    portEnabled: true
  processAgent:
    processCollection: true
  remoteConfiguration:
    enabled: false
  env:
${env
  .filter(([k]) => !["DD_SITE", "DD_DD_URL", "DD_LOGS_ENABLED"].includes(k))
  .map(([k, v]) => `    - name: ${k}\n      value: "${v}"`)
  .join("\n")}
clusterAgent:
  enabled: true`;
  const host = `# /etc/datadog-agent/datadog.yaml
api_key: <API key>
site: robine-dog.invalid
dd_url: ${url}
apm_config:
  apm_dd_url: ${url}
logs_enabled: true
logs_config:
  use_http: true
  logs_dd_url: ${logs}${noSsl ? "\n  logs_no_ssl: true" : ""}
process_config:
  process_dd_url: ${url}
remote_configuration:
  enabled: false`;
  return (
    <>
      <PageHeader title="Agent" />
      <div className="page-scroll page-padding">
        <p className="muted" style={{ marginTop: 0 }}>
          The Datadog Agent, unchanged, with every endpoint pointed here and a
          site that does not resolve (so nothing can reach Datadog with the
          key). Create a key in{" "}
          <a href="/organization-settings/api-keys">API Keys</a>.
        </p>
        <Snippet title="Docker" text={docker} />
        <Snippet title="Kubernetes (Helm values)" text={helm} />
        <Snippet title="Host (datadog.yaml)" text={host} />
        <p className="muted">
          Optional: <code>network_config.enabled</code> (Cloud Network),{" "}
          <code>runtime_security_config</code> and{" "}
          <code>compliance_config</code> with{" "}
          <code>endpoints.logs_dd_url: {logs}</code> (Workload Protection, Cloud
          Security), the <code>snmp</code> integration (Network Devices).
        </p>
      </div>
    </>
  );
}

/** OpenTelemetry: SDKs and the Collector, over OTLP/HTTP. */
export function OpenTelemetrySetup() {
  const url = origin();
  const sdk = `OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf
OTEL_EXPORTER_OTLP_ENDPOINT=${url}
OTEL_EXPORTER_OTLP_HEADERS="dd-api-key=<API key>"
OTEL_SERVICE_NAME=my-api
OTEL_RESOURCE_ATTRIBUTES=deployment.environment.name=prod,service.version=1.2.3`;
  const collector = `receivers:
  otlp:
    protocols:
      grpc:
      http:
processors:
  batch:
exporters:
  otlphttp/robine-dog:
    endpoint: ${url}
    headers:
      dd-api-key: <API key>
service:
  pipelines:
    traces:
      receivers: [otlp]
      processors: [batch]
      exporters: [otlphttp/robine-dog]
    metrics:
      receivers: [otlp]
      processors: [batch]
      exporters: [otlphttp/robine-dog]
    logs:
      receivers: [otlp]
      processors: [batch]
      exporters: [otlphttp/robine-dog]`;
  return (
    <>
      <PageHeader title="OpenTelemetry" />
      <div className="page-scroll page-padding">
        <p className="muted" style={{ marginTop: 0 }}>
          OTLP/HTTP (protobuf or JSON) on <code>/v1/traces</code>,{" "}
          <code>/v1/metrics</code> and <code>/v1/logs</code>, mapped as Datadog
          maps OTLP (<code>service.name</code>,{" "}
          <code>deployment.environment.name</code>, <code>service.version</code>
          , <code>host.name</code>). gRPC goes through a Collector.
        </p>
        <Snippet title="An SDK, directly" text={sdk} />
        <Snippet title="The OpenTelemetry Collector" text={collector} />
      </div>
    </>
  );
}
