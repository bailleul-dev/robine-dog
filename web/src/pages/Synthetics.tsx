import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { TimeChart, type Series } from "../components/Charts";
import { Modal, SidePanel } from "../components/Common";
import { PlusIcon } from "../components/Icons";
import { PageHeader } from "../components/Layout";
import {
  api,
  ApiError,
  type SyntheticApiStep,
  type SyntheticAssertion,
  type SyntheticBrowserStep,
  type SyntheticConfigVariable,
  type SyntheticExtract,
  type SyntheticRequest,
  type SyntheticResult,
  type SyntheticTest,
  type SyntheticVariable,
} from "../lib/api";
import { ago } from "../lib/format";
import { lines } from "./Metrics";

export const SYNTH_TABS = [
  { to: "/synthetics/tests", label: "Tests", end: true },
  { to: "/synthetics/variables", label: "Global Variables" },
  { to: "/synthetics/ci", label: "Continuous Testing" },
];

const FREQUENCIES: [string, number][] = [
  ["30 seconds", 30],
  ["1 minute", 60],
  ["5 minutes", 300],
  ["15 minutes", 900],
  ["30 minutes", 1800],
  ["1 hour", 3600],
  ["6 hours", 21600],
  ["1 day", 86400],
];

const SUBTYPES: [string, string][] = [
  ["http", "HTTP"],
  ["ssl", "SSL"],
  ["tcp", "TCP"],
  ["dns", "DNS"],
  ["udp", "UDP"],
  ["websocket", "WebSocket"],
  ["grpc", "gRPC"],
  ["icmp", "ICMP"],
];

const DEVICES: [string, string][] = [
  ["chrome.laptop_large", "Laptop large (1440×1100)"],
  ["chrome.tablet", "Tablet (1024×1366)"],
  ["chrome.mobile_small", "Mobile small (390×844)"],
];

const NUMBER_OPS = ["is", "isNot", "lessThan", "lessThanOrEqual", "moreThan", "moreThanOrEqual"];
const TEXT_OPS = ["contains", "doesNotContain", "is", "isNot", "matches", "doesNotMatch", "startsWith"];
const OP_LABEL: Record<string, string> = {
  is: "is",
  isNot: "is not",
  lessThan: "<",
  lessThanOrEqual: "<=",
  moreThan: ">",
  moreThanOrEqual: ">=",
  contains: "contains",
  doesNotContain: "does not contain",
  matches: "matches",
  doesNotMatch: "does not match",
  startsWith: "starts with",
  validatesJSONPath: "JSONPath",
  isInMoreThan: "expires in more than",
  isInLessThan: "expires in less than",
  equals: "equals",
};

// What can be asserted on each protocol: [type, label, operators, property placeholder].
type AssertionKind = { type: string; label: string; ops: string[]; property?: string; target?: string };
const RESPONSE_TIME: AssertionKind = { type: "responseTime", label: "response time (ms)", ops: ["lessThan", "moreThan"] };
const ASSERTIONS: Record<string, AssertionKind[]> = {
  http: [
    { type: "statusCode", label: "status code", ops: NUMBER_OPS, target: "200" },
    RESPONSE_TIME,
    { type: "header", label: "header", ops: TEXT_OPS, property: "content-type" },
    { type: "body", label: "body", ops: [...TEXT_OPS, "validatesJSONPath"] },
  ],
  ssl: [
    { type: "certificate", label: "certificate", ops: ["isInMoreThan", "isInLessThan"], target: "days" },
    { type: "tlsVersion", label: "TLS version", ops: NUMBER_OPS, target: "1.2" },
    { type: "property", label: "property", ops: TEXT_OPS, property: "issuer" },
    RESPONSE_TIME,
  ],
  tcp: [{ type: "connection", label: "connection", ops: ["is", "isNot"], target: "established" }, RESPONSE_TIME],
  dns: [
    { type: "recordSome", label: "at least one record", ops: TEXT_OPS, property: "A" },
    { type: "recordEvery", label: "every record", ops: TEXT_OPS, property: "A" },
    RESPONSE_TIME,
  ],
  udp: [{ type: "receivedMessage", label: "received message", ops: TEXT_OPS }, RESPONSE_TIME],
  websocket: [
    { type: "receivedMessage", label: "received message", ops: TEXT_OPS },
    { type: "header", label: "header", ops: TEXT_OPS, property: "server" },
    RESPONSE_TIME,
  ],
  grpc: [
    { type: "grpcHealthcheckStatus", label: "health status", ops: ["is", "isNot"], target: "SERVING" },
    { type: "grpcMetadata", label: "metadata", ops: TEXT_OPS, property: "key" },
    RESPONSE_TIME,
  ],
  icmp: [
    { type: "latency", label: "latency (ms)", ops: ["lessThan", "moreThan"], property: "avg" },
    { type: "packetLossPercentage", label: "packet loss (%)", ops: ["lessThan", "is"] },
    { type: "packetsReceived", label: "packets received", ops: NUMBER_OPS },
    RESPONSE_TIME,
  ],
};

const DEFAULT_ASSERTIONS: Record<string, SyntheticAssertion[]> = {
  http: [{ type: "statusCode", operator: "is", target: 200 }],
  ssl: [{ type: "certificate", operator: "isInMoreThan", target: 10 }],
  tcp: [{ type: "connection", operator: "is", target: "established" }],
  dns: [{ type: "recordSome", operator: "contains", property: "A", target: "" }],
  udp: [{ type: "receivedMessage", operator: "contains", target: "" }],
  websocket: [{ type: "receivedMessage", operator: "contains", target: "" }],
  grpc: [{ type: "grpcHealthcheckStatus", operator: "is", target: 1 }],
  icmp: [{ type: "packetLossPercentage", operator: "lessThan", target: 50 }],
};

const DEFAULT_REQUEST: Record<string, SyntheticRequest> = {
  http: { method: "GET", url: "", headers: {}, timeout: 10 },
  ssl: { host: "", port: 443 },
  tcp: { host: "", port: 80 },
  dns: { host: "" },
  udp: { host: "", port: 53, message: "" },
  websocket: { url: "wss://", message: "ping" },
  grpc: { host: "", port: 50051, service: "" },
  icmp: { host: "", numberOfPackets: 4 },
};

// Browser steps: what each one needs.
type StepKind = { type: string; label: string; element?: boolean; value?: string; check?: boolean; attribute?: boolean; code?: boolean; variable?: boolean };
const BROWSER_STEPS: StepKind[] = [
  { type: "goToUrl", label: "Go to URL", value: "https://…" },
  { type: "click", label: "Click", element: true },
  { type: "hover", label: "Hover", element: true },
  { type: "typeText", label: "Type text", element: true, value: "text or {{ VARIABLE }}" },
  { type: "pressKey", label: "Press key", value: "Enter" },
  { type: "selectOption", label: "Select option", element: true, value: "option value or text" },
  { type: "scroll", label: "Scroll to", element: true },
  { type: "wait", label: "Wait (seconds)", value: "2" },
  { type: "refresh", label: "Refresh" },
  { type: "assertElementPresent", label: "Assert element is present", element: true },
  { type: "assertElementContent", label: "Assert element content", element: true, check: true, value: "expected" },
  { type: "assertElementAttribute", label: "Assert element attribute", element: true, attribute: true, check: true, value: "expected" },
  { type: "assertPageContains", label: "Assert page contains", value: "text" },
  { type: "assertPageDoesNotContain", label: "Assert page does not contain", value: "text" },
  { type: "assertCurrentUrl", label: "Assert current URL", check: true, value: "/checkout" },
  { type: "assertFromJavascript", label: "Assert with JavaScript", code: true },
  { type: "extractVariable", label: "Extract element content", element: true, variable: true },
  { type: "extractFromJavascript", label: "Extract with JavaScript", code: true, variable: true },
];

type Kind = "api" | "multi" | "browser";
const kindOf = (t: SyntheticTest): Kind => (t.type === "browser" ? "browser" : t.subtype === "multi" ? "multi" : "api");

const blank = (kind: Kind): SyntheticTest => ({
  name: "",
  type: kind === "browser" ? "browser" : "api",
  subtype: kind === "browser" ? undefined : kind === "multi" ? "multi" : "http",
  status: "live",
  config:
    kind === "multi"
      ? {
          request: {},
          assertions: [],
          steps: [{ name: "", subtype: "http", request: { ...DEFAULT_REQUEST.http }, assertions: [...DEFAULT_ASSERTIONS.http] }],
        }
      : kind === "browser"
        ? { request: { method: "GET", url: "https://" }, assertions: [] }
        : { request: { ...DEFAULT_REQUEST.http }, assertions: [...DEFAULT_ASSERTIONS.http] },
  options: { tick_every: kind === "browser" ? 900 : 60, device_ids: kind === "browser" ? ["chrome.laptop_large"] : undefined },
  steps: kind === "browser" ? [{ type: "assertPageContains", params: { value: "" } }] : undefined,
  message: "",
  tags: [],
});

const typeLabel = (t: SyntheticTest) =>
  t.type === "browser" ? "Browser" : t.subtype === "multi" ? "Multistep" : (SUBTYPES.find(([s]) => s === (t.subtype || "http"))?.[1] ?? t.subtype);

export function describeTarget(t: SyntheticTest): string {
  const r = t.config.request;
  if (t.type === "browser") return r.url ?? "";
  if (t.subtype === "multi") return `${t.config.steps?.length ?? 0} steps: ${(t.config.steps ?? []).map((s) => s.name || s.subtype).join(" → ")}`;
  if (!t.subtype || t.subtype === "http") return `${r.method ?? "GET"} ${r.url ?? ""}`;
  if (t.subtype === "websocket") return r.url ?? "";
  return r.port != null && r.port !== "" ? `${r.host}:${r.port}` : (r.host ?? "");
}

const targetText = (v: unknown) => (v == null ? "" : typeof v === "object" ? JSON.stringify(v) : String(v));

const parseTarget = (s: string): string | number => (s.trim() !== "" && !Number.isNaN(Number(s)) ? Number(s) : s);

const updateAt = <T,>(list: T[], i: number, v: T) => list.map((x, j) => (j === i ? v : x));
const move = <T,>(list: T[], i: number, d: number) => {
  const j = i + d;
  if (j < 0 || j >= list.length) return list;
  const next = [...list];
  [next[i], next[j]] = [next[j], next[i]];
  return next;
};

// --- editors -----------------------------------------------------------------------------------------------------

function AssertionsEditor({
  subtype,
  value,
  onChange,
}: {
  subtype: string;
  value: SyntheticAssertion[];
  onChange: (v: SyntheticAssertion[]) => void;
}) {
  const kinds = ASSERTIONS[subtype] ?? ASSERTIONS.http;
  return (
    <div className="synth-rows">
      {value.map((a, i) => {
        const kind = kinds.find((k) => k.type === a.type) ?? { type: a.type, label: a.type, ops: [a.operator] };
        const jp = a.operator === "validatesJSONPath" ? ((a.target ?? {}) as { jsonPath?: string; operator?: string; targetValue?: unknown }) : null;
        const set = (patch: Partial<SyntheticAssertion>) => onChange(updateAt(value, i, { ...a, ...patch }));
        return (
          <div key={i} className="synth-row">
            <select
              className="input small"
              value={a.type}
              aria-label="Assertion"
              onChange={(e) => {
                const k = kinds.find((x) => x.type === e.target.value)!;
                set({ type: k.type, operator: k.ops[0], property: k.property ? k.property : undefined, target: "" });
              }}
            >
              {kinds.map((k) => (
                <option key={k.type} value={k.type}>
                  {k.label}
                </option>
              ))}
              {!kinds.some((k) => k.type === a.type) && <option value={a.type}>{a.type}</option>}
            </select>
            {kind.property !== undefined && (
              <input
                className="input small mono"
                style={{ width: 110 }}
                value={a.property ?? ""}
                placeholder={kind.property}
                aria-label="Property"
                onChange={(e) => set({ property: e.target.value })}
              />
            )}
            <select
              className="input small"
              value={a.operator}
              aria-label="Operator"
              onChange={(e) =>
                set({
                  operator: e.target.value,
                  target: e.target.value === "validatesJSONPath" ? { jsonPath: "$.", operator: "is", targetValue: "" } : jp ? "" : a.target,
                })
              }
            >
              {kind.ops.map((o) => (
                <option key={o} value={o}>
                  {OP_LABEL[o] ?? o}
                </option>
              ))}
            </select>
            {jp ? (
              <>
                <input
                  className="input small mono"
                  style={{ width: 120 }}
                  value={jp.jsonPath ?? ""}
                  aria-label="JSONPath"
                  onChange={(e) => set({ target: { ...jp, jsonPath: e.target.value } })}
                />
                <select className="input small" value={jp.operator ?? "is"} aria-label="JSONPath operator" onChange={(e) => set({ target: { ...jp, operator: e.target.value } })}>
                  {[...new Set([...TEXT_OPS, ...NUMBER_OPS])].map((o) => (
                    <option key={o} value={o}>
                      {OP_LABEL[o] ?? o}
                    </option>
                  ))}
                </select>
                <input
                  className="input small mono"
                  style={{ flex: 1 }}
                  value={targetText(jp.targetValue)}
                  aria-label="Expected"
                  onChange={(e) => set({ target: { ...jp, targetValue: parseTarget(e.target.value) } })}
                />
              </>
            ) : (
              <input
                className="input small mono"
                style={{ flex: 1 }}
                value={targetText(a.target)}
                placeholder={kind.target ?? ""}
                aria-label="Target"
                onChange={(e) => set({ target: parseTarget(e.target.value) })}
              />
            )}
            <button className="btn small ghost" aria-label="Remove the assertion" onClick={() => onChange(value.filter((_, j) => j !== i))}>
              ×
            </button>
          </div>
        );
      })}
      <button
        className="btn small"
        onClick={() => onChange([...value, { type: kinds[0].type, operator: kinds[0].ops[0], property: kinds[0].property, target: "" }])}
      >
        <PlusIcon /> Assertion
      </button>
    </div>
  );
}

function Field({ label, children, width }: { label: string; children: React.ReactNode; width?: number | string }) {
  return (
    <label className="form-field" style={{ width, flex: width ? undefined : 1 }}>
      <span className="form-label">{label}</span>
      {children}
    </label>
  );
}

const headerText = (h?: Record<string, string>) =>
  Object.entries(h ?? {})
    .map(([k, v]) => `${k}: ${v}`)
    .join("\n");
const parseHeaders = (s: string) =>
  Object.fromEntries(
    s
      .split("\n")
      .map((l) => l.split(/:(.*)/s))
      .filter((p) => p[0].trim() && p[1] !== undefined)
      .map((p) => [p[0].trim(), p[1].trim()]),
  );

function HeadersInput({ value, onChange }: { value?: Record<string, string>; onChange: (h: Record<string, string>) => void }) {
  const [text, setText] = useState(headerText(value));
  return (
    <textarea
      className="input mono"
      rows={2}
      value={text}
      placeholder="Authorization: Bearer {{ API_TOKEN }}"
      onChange={(e) => {
        setText(e.target.value);
        onChange(parseHeaders(e.target.value));
      }}
    />
  );
}

function RequestEditor({ subtype, value, onChange }: { subtype: string; value: SyntheticRequest; onChange: (r: SyntheticRequest) => void }) {
  const set = (patch: Partial<SyntheticRequest>) => onChange({ ...value, ...patch });
  const text = (k: keyof SyntheticRequest, placeholder = "") => (
    <input className="input mono" value={(value[k] as string | undefined) ?? ""} placeholder={placeholder} onChange={(e) => set({ [k]: e.target.value })} />
  );
  const num = (k: keyof SyntheticRequest, placeholder = "") => (
    <input
      className="input mono"
      value={(value[k] as number | string | undefined) ?? ""}
      placeholder={placeholder}
      onChange={(e) => set({ [k]: e.target.value === "" ? undefined : Number.isNaN(Number(e.target.value)) ? e.target.value : Number(e.target.value) })}
    />
  );
  if (subtype === "http") {
    return (
      <>
        <div style={{ display: "flex", gap: 8 }}>
          <Field label="Method" width={110}>
            <select className="input" value={value.method ?? "GET"} onChange={(e) => set({ method: e.target.value })}>
              {["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"].map((m) => (
                <option key={m}>{m}</option>
              ))}
            </select>
          </Field>
          <Field label="URL">{text("url", "https://example.com/health")}</Field>
          <Field label="Timeout (s)" width={90}>
            {num("timeout", "60")}
          </Field>
        </div>
        <Field label="Headers">
          <HeadersInput value={value.headers} onChange={(headers) => set({ headers })} />
        </Field>
        {!["GET", "HEAD"].includes(value.method ?? "GET") && (
          <Field label="Body">
            <textarea className="input mono" rows={3} value={value.body ?? ""} onChange={(e) => set({ body: e.target.value || undefined })} />
          </Field>
        )}
      </>
    );
  }
  if (subtype === "websocket") {
    return (
      <>
        <Field label="URL">{text("url", "wss://example.com/socket")}</Field>
        <Field label="Message sent">{text("message", "ping")}</Field>
        <Field label="Headers">
          <HeadersInput value={value.headers} onChange={(headers) => set({ headers })} />
        </Field>
      </>
    );
  }
  return (
    <>
      <div style={{ display: "flex", gap: 8 }}>
        <Field label="Host">{text("host", subtype === "dns" ? "example.com" : "db.internal")}</Field>
        {["ssl", "tcp", "udp", "grpc"].includes(subtype) && (
          <Field label="Port" width={90}>
            {num("port")}
          </Field>
        )}
        {subtype === "icmp" && (
          <Field label="Pings" width={90}>
            {num("numberOfPackets", "4")}
          </Field>
        )}
        <Field label="Timeout (s)" width={90}>
          {num("timeout", "60")}
        </Field>
      </div>
      {subtype === "ssl" && <Field label="Server name (SNI)">{text("servername", "the host")}</Field>}
      {subtype === "dns" && (
        <div style={{ display: "flex", gap: 8 }}>
          <Field label="DNS server">{text("dnsServer", "the system's resolver")}</Field>
          <Field label="DNS port" width={90}>
            {num("dnsServerPort", "53")}
          </Field>
        </div>
      )}
      {subtype === "udp" && <Field label="Message sent">{text("message")}</Field>}
      {subtype === "grpc" && (
        <div style={{ display: "flex", gap: 8, alignItems: "flex-end" }}>
          <Field label="Service (empty: the server's health)">{text("service")}</Field>
          <label className="checkbox" style={{ paddingBottom: 8 }}>
            <input type="checkbox" checked={value.useTls === true} onChange={(e) => set({ useTls: e.target.checked })} /> TLS
          </label>
        </div>
      )}
    </>
  );
}

function VariablesEditor({ value, onChange }: { value: SyntheticConfigVariable[]; onChange: (v: SyntheticConfigVariable[]) => void }) {
  const globals = useQuery({ queryKey: ["synthetic-variables"], queryFn: api.syntheticVariables });
  return (
    <div className="synth-rows">
      {value.map((v, i) => (
        <div key={i} className="synth-row">
          <input
            className="input small mono"
            style={{ width: 140 }}
            value={v.name}
            placeholder="NAME"
            aria-label="Variable name"
            onChange={(e) => onChange(updateAt(value, i, { ...v, name: e.target.value.toUpperCase() }))}
          />
          <select
            className="input small"
            value={v.type}
            aria-label="Variable type"
            onChange={(e) => onChange(updateAt(value, i, { name: v.name, type: e.target.value, pattern: e.target.value === "text" ? "" : undefined }))}
          >
            <option value="text">pattern</option>
            <option value="global">global variable</option>
          </select>
          {v.type === "global" ? (
            <select className="input small" style={{ flex: 1 }} value={v.id ?? ""} aria-label="Global variable" onChange={(e) => onChange(updateAt(value, i, { ...v, id: e.target.value }))}>
              <option value="">Pick a global variable…</option>
              {(globals.data?.variables ?? []).map((g) => (
                <option key={g.id} value={g.id}>
                  {g.name}
                </option>
              ))}
            </select>
          ) : (
            <input
              className="input small mono"
              style={{ flex: 1 }}
              value={v.pattern ?? ""}
              placeholder="order-{{ numeric(6) }}"
              aria-label="Pattern"
              onChange={(e) => onChange(updateAt(value, i, { ...v, pattern: e.target.value }))}
            />
          )}
          <button className="btn small ghost" aria-label="Remove the variable" onClick={() => onChange(value.filter((_, j) => j !== i))}>
            ×
          </button>
        </div>
      ))}
      <button className="btn small" onClick={() => onChange([...value, { name: "", type: "text", pattern: "" }])}>
        <PlusIcon /> Variable
      </button>
      <div className="form-hint">
        Use them as <code>{"{{ NAME }}"}</code>. Patterns: <code>{"{{ numeric(6) }}"}</code>, <code>{"{{ alphanumeric(8) }}"}</code>,{" "}
        <code>{"{{ uuid }}"}</code>, <code>{"{{ timestamp(0, ms) }}"}</code>, <code>{"{{ date(1d, YYYY-MM-DD) }}"}</code>. Global variables are
        available by their own name too.
      </div>
    </div>
  );
}

function ExtractEditor({ value, onChange }: { value: SyntheticExtract[]; onChange: (v: SyntheticExtract[]) => void }) {
  return (
    <div className="synth-rows">
      {value.map((x, i) => {
        const set = (patch: Partial<SyntheticExtract>) => onChange(updateAt(value, i, { ...x, ...patch }));
        return (
          <div key={i} className="synth-row">
            <input
              className="input small mono"
              style={{ width: 120 }}
              value={x.name}
              placeholder="NAME"
              aria-label="Variable name"
              onChange={(e) => set({ name: e.target.value.toUpperCase() })}
            />
            <select className="input small" value={x.type} aria-label="From" onChange={(e) => set({ type: e.target.value })}>
              <option value="http_body">from the body</option>
              <option value="http_header">from a header</option>
              <option value="http_status">the status code</option>
            </select>
            {x.type === "http_header" && (
              <input className="input small mono" style={{ width: 110 }} value={x.field ?? ""} placeholder="location" aria-label="Header" onChange={(e) => set({ field: e.target.value })} />
            )}
            <select className="input small" value={x.parser.type} aria-label="Parser" onChange={(e) => set({ parser: { ...x.parser, type: e.target.value } })}>
              <option value="raw">whole</option>
              <option value="json_path">JSONPath</option>
              <option value="regex">regex</option>
            </select>
            {x.parser.type !== "raw" && (
              <input
                className="input small mono"
                style={{ flex: 1 }}
                value={x.parser.value ?? ""}
                placeholder={x.parser.type === "json_path" ? "$.token" : "id=(\\d+)"}
                aria-label="Expression"
                onChange={(e) => set({ parser: { ...x.parser, value: e.target.value } })}
              />
            )}
            <label className="checkbox">
              <input type="checkbox" checked={x.secure === true} onChange={(e) => set({ secure: e.target.checked })} /> secure
            </label>
            <button className="btn small ghost" aria-label="Remove" onClick={() => onChange(value.filter((_, j) => j !== i))}>
              ×
            </button>
          </div>
        );
      })}
      <button className="btn small" onClick={() => onChange([...value, { name: "", type: "http_body", parser: { type: "json_path", value: "$." } }])}>
        <PlusIcon /> Extract a variable
      </button>
    </div>
  );
}

function StepCard({
  index,
  count,
  title,
  onMove,
  onRemove,
  children,
}: {
  index: number;
  count: number;
  title: React.ReactNode;
  onMove: (d: number) => void;
  onRemove: () => void;
  children: React.ReactNode;
}) {
  return (
    <div className="synth-step">
      <div className="synth-step-head">
        <span className="synth-step-n">{index + 1}</span>
        <div style={{ flex: 1, display: "flex", gap: 8, alignItems: "center" }}>{title}</div>
        <button className="btn small ghost" disabled={index === 0} aria-label="Move up" onClick={() => onMove(-1)}>
          ↑
        </button>
        <button className="btn small ghost" disabled={index === count - 1} aria-label="Move down" onClick={() => onMove(1)}>
          ↓
        </button>
        <button className="btn small ghost" aria-label="Remove the step" onClick={onRemove}>
          ×
        </button>
      </div>
      {children}
    </div>
  );
}

function ApiStepsEditor({ value, onChange }: { value: SyntheticApiStep[]; onChange: (v: SyntheticApiStep[]) => void }) {
  return (
    <div className="synth-steps">
      {value.map((s, i) => {
        const set = (patch: Partial<SyntheticApiStep>) => onChange(updateAt(value, i, { ...s, ...patch }));
        return (
          <StepCard
            key={i}
            index={i}
            count={value.length}
            onMove={(d) => onChange(move(value, i, d))}
            onRemove={() => onChange(value.filter((_, j) => j !== i))}
            title={
              <>
                <select
                  className="input small"
                  value={s.subtype}
                  aria-label="Step type"
                  onChange={(e) =>
                    set(
                      e.target.value === "wait"
                        ? { subtype: "wait", value: 1, request: {}, assertions: [] }
                        : { subtype: e.target.value, request: { ...DEFAULT_REQUEST[e.target.value] }, assertions: [...DEFAULT_ASSERTIONS[e.target.value]] },
                    )
                  }
                >
                  {SUBTYPES.map(([v, l]) => (
                    <option key={v} value={v}>
                      {l}
                    </option>
                  ))}
                  <option value="wait">Wait</option>
                </select>
                <input className="input small" style={{ flex: 1 }} value={s.name} placeholder="Step name" aria-label="Step name" onChange={(e) => set({ name: e.target.value })} />
              </>
            }
          >
            {s.subtype === "wait" ? (
              <Field label="Seconds" width={120}>
                <input className="input" type="number" min={0} max={180} value={s.value ?? 1} onChange={(e) => set({ value: Number(e.target.value) })} />
              </Field>
            ) : (
              <>
                <RequestEditor subtype={s.subtype} value={s.request} onChange={(request) => set({ request })} />
                <div className="form-field">
                  <span className="form-label">Assertions</span>
                  <AssertionsEditor subtype={s.subtype} value={s.assertions} onChange={(assertions) => set({ assertions })} />
                </div>
                {s.subtype === "http" && (
                  <div className="form-field">
                    <span className="form-label">Extracted variables, for the next steps</span>
                    <ExtractEditor value={s.extractedValues ?? []} onChange={(extractedValues) => set({ extractedValues })} />
                  </div>
                )}
                <div style={{ display: "flex", gap: 16 }}>
                  <label className="checkbox">
                    <input type="checkbox" checked={s.allowFailure === true} onChange={(e) => set({ allowFailure: e.target.checked })} /> Continue if this step fails
                  </label>
                  {s.allowFailure && (
                    <label className="checkbox">
                      <input type="checkbox" checked={s.isCritical === true} onChange={(e) => set({ isCritical: e.target.checked })} /> …but still fail the test
                    </label>
                  )}
                </div>
              </>
            )}
          </StepCard>
        );
      })}
      <button className="btn small" onClick={() => onChange([...value, { name: "", subtype: "http", request: { ...DEFAULT_REQUEST.http }, assertions: [...DEFAULT_ASSERTIONS.http] }])}>
        <PlusIcon /> Step
      </button>
    </div>
  );
}

type Locator = { type: string; value: string };
const locatorOf = (p: Record<string, unknown>): Locator => {
  const el = (p.element ?? {}) as { userLocator?: { values?: Locator[] }; selector?: string };
  return el.userLocator?.values?.[0] ?? { type: "css", value: el.selector ?? "" };
};
const withLocator = (p: Record<string, unknown>, l: Locator) => ({ ...p, element: { userLocator: { failTestOnCannotLocate: true, values: [l] } } });

function BrowserStepsEditor({ value, onChange }: { value: SyntheticBrowserStep[]; onChange: (v: SyntheticBrowserStep[]) => void }) {
  return (
    <div className="synth-steps">
      {value.map((s, i) => {
        const kind = BROWSER_STEPS.find((k) => k.type === s.type) ?? { type: s.type, label: s.type };
        const set = (patch: Partial<SyntheticBrowserStep>) => onChange(updateAt(value, i, { ...s, ...patch }));
        const setParam = (k: string, v: unknown) => set({ params: { ...s.params, [k]: v } });
        const loc = locatorOf(s.params);
        return (
          <StepCard
            key={i}
            index={i}
            count={value.length}
            onMove={(d) => onChange(move(value, i, d))}
            onRemove={() => onChange(value.filter((_, j) => j !== i))}
            title={
              <>
                <select className="input small" value={s.type} aria-label="Step type" onChange={(e) => set({ type: e.target.value, params: {} })}>
                  {BROWSER_STEPS.map((k) => (
                    <option key={k.type} value={k.type}>
                      {k.label}
                    </option>
                  ))}
                </select>
                <input className="input small" style={{ flex: 1 }} value={s.name ?? ""} placeholder="Name (optional)" aria-label="Step name" onChange={(e) => set({ name: e.target.value || undefined })} />
              </>
            }
          >
            <div className="synth-row" style={{ flexWrap: "wrap" }}>
              {kind.element && (
                <>
                  <select className="input small" value={loc.type} aria-label="Locator" onChange={(e) => set({ params: withLocator(s.params, { ...loc, type: e.target.value }) })}>
                    <option value="css">CSS</option>
                    <option value="xpath">XPath</option>
                  </select>
                  <input
                    className="input small mono"
                    style={{ flex: 1, minWidth: 160 }}
                    value={loc.value}
                    placeholder={loc.type === "css" ? "#checkout button.primary" : "//button[text()='Buy']"}
                    aria-label="Element"
                    onChange={(e) => set({ params: withLocator(s.params, { ...loc, value: e.target.value }) })}
                  />
                </>
              )}
              {kind.attribute && (
                <input className="input small mono" style={{ width: 110 }} value={s.params.attribute ?? ""} placeholder="attribute" aria-label="Attribute" onChange={(e) => setParam("attribute", e.target.value)} />
              )}
              {kind.check && (
                <select className="input small" value={s.params.check ?? "contains"} aria-label="Check" onChange={(e) => setParam("check", e.target.value)}>
                  {[...TEXT_OPS, "equals"].map((o) => (
                    <option key={o} value={o}>
                      {OP_LABEL[o] ?? o}
                    </option>
                  ))}
                </select>
              )}
              {kind.value && (
                <input className="input small mono" style={{ flex: 1, minWidth: 140 }} value={String(s.params.value ?? "")} placeholder={kind.value} aria-label="Value" onChange={(e) => setParam("value", e.target.value)} />
              )}
              {s.type === "click" && (
                <select className="input small" value={s.params.clickType ?? "primary"} aria-label="Click type" onChange={(e) => setParam("clickType", e.target.value)}>
                  <option value="primary">click</option>
                  <option value="double">double click</option>
                  <option value="contextual">right click</option>
                </select>
              )}
              {kind.variable && (
                <input
                  className="input small mono"
                  style={{ width: 140 }}
                  value={s.params.variable?.name ?? ""}
                  placeholder="VARIABLE"
                  aria-label="Variable"
                  onChange={(e) => setParam("variable", { name: e.target.value.toUpperCase() })}
                />
              )}
            </div>
            {kind.code && (
              <textarea
                className="input mono"
                rows={3}
                value={s.params.code ?? ""}
                placeholder={kind.variable ? "return document.title" : "return document.querySelectorAll('.item').length > 0"}
                aria-label="JavaScript"
                onChange={(e) => setParam("code", e.target.value)}
              />
            )}
            <div style={{ display: "flex", gap: 16, alignItems: "center" }}>
              <label className="checkbox">
                <input type="checkbox" checked={s.allowFailure === true} onChange={(e) => set({ allowFailure: e.target.checked })} /> Continue if this step fails
              </label>
              {s.allowFailure && (
                <label className="checkbox">
                  <input type="checkbox" checked={s.isCritical === true} onChange={(e) => set({ isCritical: e.target.checked })} /> …but still fail the test
                </label>
              )}
              <label className="checkbox">
                Timeout
                <input
                  className="input small"
                  style={{ width: 60 }}
                  value={s.timeout ?? ""}
                  placeholder="20"
                  aria-label="Step timeout"
                  onChange={(e) => set({ timeout: e.target.value ? Number(e.target.value) : undefined })}
                />
                s
              </label>
            </div>
          </StepCard>
        );
      })}
      <button className="btn small" onClick={() => onChange([...value, { type: "click", params: {} }])}>
        <PlusIcon /> Step
      </button>
    </div>
  );
}

function TestModal({ test, onClose }: { test: SyntheticTest; onClose: () => void }) {
  const qc = useQueryClient();
  const kind = kindOf(test);
  const [t, setT] = useState<SyntheticTest>(test);
  const [tags, setTags] = useState(test.tags.join(", "));
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const subtype = t.subtype || "http";
  const set = (patch: Partial<SyntheticTest>) => setT((x) => ({ ...x, ...patch }));
  const setConfig = (patch: Partial<SyntheticTest["config"]>) => setT((x) => ({ ...x, config: { ...x.config, ...patch } }));
  const setOptions = (patch: Partial<SyntheticTest["options"]>) => setT((x) => ({ ...x, options: { ...x.options, ...patch } }));
  const varsKey = kind === "browser" ? "variables" : "configVariables";

  const save = async () => {
    const next: SyntheticTest = {
      ...t,
      tags: tags
        .split(",")
        .map((x) => x.trim())
        .filter(Boolean),
    };
    setSaving(true);
    try {
      if (test.public_id) await api.updateSynthetic(test.public_id, next);
      else await api.createSynthetic(next);
      await qc.invalidateQueries({ queryKey: ["synthetics"] });
      onClose();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : String(e));
      setSaving(false);
    }
  };

  const title = `${test.public_id ? "Edit" : "New"} ${kind === "browser" ? "Browser Test" : kind === "multi" ? "Multistep API Test" : "API Test"}`;
  const frequencies = FREQUENCIES.filter(([, s]) => kind !== "browser" || s >= 300);
  return (
    <Modal title={title} onClose={onClose} width={760}>
      <div className="modal-body form synth-form">
        <Field label="Name">
          <input className="input" autoFocus value={t.name} onChange={(e) => set({ name: e.target.value })} placeholder="Checkout works" />
        </Field>
        {kind === "api" && (
          <>
            <div className="synth-subtypes" role="tablist">
              {SUBTYPES.map(([v, l]) => (
                <button
                  key={v}
                  className={`btn small ${subtype === v ? "primary" : ""}`}
                  onClick={() =>
                    v !== subtype &&
                    setT((x) => ({ ...x, subtype: v, config: { ...x.config, request: { ...DEFAULT_REQUEST[v] }, assertions: [...DEFAULT_ASSERTIONS[v]] } }))
                  }
                >
                  {l}
                </button>
              ))}
            </div>
            <RequestEditor subtype={subtype} value={t.config.request} onChange={(request) => setConfig({ request })} />
            <div className="form-field">
              <span className="form-label">Assertions</span>
              <AssertionsEditor subtype={subtype} value={t.config.assertions} onChange={(assertions) => setConfig({ assertions })} />
            </div>
            {subtype === "http" && (
              <label className="checkbox">
                <input type="checkbox" checked={t.options.follow_redirects !== false} onChange={(e) => setOptions({ follow_redirects: e.target.checked })} /> Follow
                redirects
              </label>
            )}
            {subtype === "ssl" && (
              <label className="checkbox">
                <input type="checkbox" checked={t.options.accept_self_signed === true} onChange={(e) => setOptions({ accept_self_signed: e.target.checked })} />{" "}
                Accept self-signed certificates
              </label>
            )}
            {subtype === "icmp" && (
              <div className="form-hint">
                ICMP needs the <code>NET_RAW</code> capability, or a <code>net.ipv4.ping_group_range</code> that includes robine-dog's group.
              </div>
            )}
          </>
        )}
        {kind === "multi" && (
          <div className="form-field">
            <span className="form-label">Steps</span>
            <ApiStepsEditor value={t.config.steps ?? []} onChange={(steps) => setConfig({ steps })} />
          </div>
        )}
        {kind === "browser" && (
          <>
            <div style={{ display: "flex", gap: 8 }}>
              <Field label="Starting URL">
                <input
                  className="input mono"
                  value={t.config.request.url ?? ""}
                  onChange={(e) => setConfig({ request: { ...t.config.request, url: e.target.value } })}
                  placeholder="https://shop.example.com"
                />
              </Field>
              <Field label="Device" width={230}>
                <select className="input" value={t.options.device_ids?.[0] ?? "chrome.laptop_large"} onChange={(e) => setOptions({ device_ids: [e.target.value] })}>
                  {DEVICES.map(([v, l]) => (
                    <option key={v} value={v}>
                      {l}
                    </option>
                  ))}
                </select>
              </Field>
            </div>
            <div className="form-field">
              <span className="form-label">Steps (after loading the starting URL)</span>
              <BrowserStepsEditor value={t.steps ?? []} onChange={(steps) => set({ steps })} />
            </div>
          </>
        )}
        <div className="form-field">
          <span className="form-label">Variables</span>
          <VariablesEditor value={t.config[varsKey] ?? []} onChange={(v) => setConfig({ [varsKey]: v })} />
        </div>
        <div style={{ display: "flex", gap: 12 }}>
          <Field label="Run every" width={150}>
            <select className="input" value={t.options.tick_every ?? 60} onChange={(e) => setOptions({ tick_every: Number(e.target.value) })}>
              {frequencies.map(([label, s]) => (
                <option key={s} value={s}>
                  {label}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Retry a failed run" width={150}>
            <select
              className="input"
              value={t.options.retry?.count ?? 0}
              onChange={(e) => setOptions({ retry: Number(e.target.value) ? { count: Number(e.target.value), interval: 300 } : undefined })}
            >
              {[0, 1, 2, 3].map((n) => (
                <option key={n} value={n}>
                  {n === 0 ? "Never" : `${n} time${n > 1 ? "s" : ""}`}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Tags">
            <input className="input mono" value={tags} onChange={(e) => setTags(e.target.value)} placeholder="env:prod, team:shop" />
          </Field>
        </div>
        <div style={{ display: "flex", gap: 12 }}>
          <Field label="Alert after failing for" width={180}>
            <select className="input" value={t.options.min_failure_duration ?? 0} onChange={(e) => setOptions({ min_failure_duration: Number(e.target.value) })}>
              {(
                [
                  ["The first failure", 0],
                  ["1 minute", 60],
                  ["5 minutes", 300],
                  ["15 minutes", 900],
                  ["1 hour", 3600],
                ] as [string, number][]
              ).map(([label, v]) => (
                <option key={v} value={v}>
                  {label}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Notify">
            <input className="input" value={t.message} onChange={(e) => set({ message: e.target.value })} placeholder="The shop is down @webhook-zulip @ops@example.com" />
          </Field>
        </div>
        <div className="form-hint">
          When the test starts failing and when it recovers, an event is posted and this message is sent to the <code>@webhook-…</code> and e-mail
          addresses it names.
        </div>
        {error && <div className="form-error">{error}</div>}
      </div>
      <div className="modal-footer">
        <button className="btn" onClick={onClose}>
          Cancel
        </button>
        <button className="btn primary" onClick={save} disabled={saving}>
          {test.public_id ? "Save" : "Create & run"}
        </button>
      </div>
    </Modal>
  );
}

// --- results -----------------------------------------------------------------------------------------------------

function Graph({ title, query, unit }: { title: string; query: string; unit: string }) {
  const q = useQuery({
    queryKey: ["synth-graph", query],
    refetchInterval: 60_000,
    queryFn: async ({ signal }) => {
      const to = Date.now();
      const from = to - 4 * 3_600_000;
      return { from, to, data: await api.queryMetrics([query], from, to, signal) };
    },
  });
  const built = useMemo(() => (q.data ? lines(q.data.data, q.data.from, q.data.to) : null), [q.data]);
  return (
    <div className="card">
      <div className="card-title">
        {title} <span className="muted">past 4 hours</span>
      </div>
      {built && built.lines.length > 0 ? (
        <TimeChart
          times={built.times}
          interval={q.data!.data.interval_ms}
          from={q.data!.from}
          to={q.data!.to}
          kind={unit ? "lines" : "bars"}
          height={130}
          series={built.lines.map((l): Series => {
            const label = l.label.replace(/^a: /, "").replace("status:", "");
            const color = label === "failed" ? "var(--red)" : label === "passed" ? "var(--green)" : "#8b6fd0";
            return { key: l.key, label, color, values: unit ? l.values : l.values.map((v) => (Number.isNaN(v) ? 0 : v)) };
          })}
          format={(v) => (unit ? `${Math.round(v)} ${unit}` : String(Math.round(v)))}
        />
      ) : (
        <div className="muted infra-nodata">{q.isLoading ? "Loading…" : "No data yet"}</div>
      )}
    </div>
  );
}

function Outcome({ t }: { t: SyntheticTest }) {
  if (t.status === "paused") return <span className="synth-state paused">Paused</span>;
  if (!t.last_result) return <span className="synth-state paused">No data</span>;
  return t.last_result.passed ? <span className="synth-state ok">OK</span> : <span className="synth-state alert">Alert</span>;
}

const ms = (v: number) => (v >= 10_000 ? `${(v / 1000).toFixed(1)} s` : `${Math.round(v)} ms`);

function ResultView({ r }: { r: SyntheticResult }) {
  const [zoom, setZoom] = useState<number | null>(null);
  return (
    <div>
      <div className={`synth-last ${r.passed ? "ok" : "alert"}`}>
        <strong>{r.passed ? "Passed" : "Failed"}</strong> {ago(r.ts)} · {new Date(r.ts).toLocaleString()}
        {r.status_code ? ` · HTTP ${r.status_code}` : ""} · {ms(r.duration_ms)}
        {r.retries ? ` · after ${r.retries} retr${r.retries > 1 ? "ies" : "y"}` : ""}
        {r.batch_id ? " · from CI" : ""}
        {r.failure && <div className="mono">{r.failure}</div>}
      </div>
      {r.steps && r.steps.length > 0 && (
        <table className="list synth-steps-table">
          <thead>
            <tr>
              <th style={{ width: 30 }}>#</th>
              <th>Step</th>
              <th className="num" style={{ width: 80 }}>
                Duration
              </th>
              {r.steps.some((s) => s.screenshot) && <th style={{ width: 130 }}>Screenshot</th>}
            </tr>
          </thead>
          <tbody>
            {r.steps.map((s, i) => (
              <tr key={i}>
                <td>
                  <span className={`synth-dot ${s.passed ? "ok" : s.allow_failure ? "warn" : "alert"}`} title={s.passed ? "passed" : "failed"} />
                </td>
                <td>
                  <div>{s.name}</div>
                  {s.url && <div className="muted mono ellipsis synth-small">{s.url}</div>}
                  {s.failure && <div className="mono synth-failure">{s.failure}</div>}
                  {s.extracted && Object.keys(s.extracted).length > 0 && (
                    <div className="muted mono synth-small">
                      {Object.entries(s.extracted)
                        .map(([k, v]) => `${k} = ${String(v)}`)
                        .join(", ")}
                    </div>
                  )}
                </td>
                <td className="num">{ms(s.duration_ms)}</td>
                {r.steps!.some((x) => x.screenshot) && (
                  <td>
                    {s.screenshot && (
                      <img
                        className="synth-shot"
                        loading="lazy"
                        alt={`Step ${i}`}
                        src={api.syntheticScreenshot(r.result_id, i)}
                        onClick={() => setZoom(i)}
                      />
                    )}
                  </td>
                )}
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {r.details && Object.keys(r.details).length > 0 && (
        <details className="synth-details">
          <summary>Details</summary>
          <pre className="mono">{JSON.stringify(r.details, null, 2)}</pre>
        </details>
      )}
      {zoom !== null && (
        <Modal title={`Step ${zoom}: ${r.steps?.[zoom]?.name ?? ""}`} onClose={() => setZoom(null)} width={1000}>
          <div className="modal-body">
            <img style={{ width: "100%" }} alt={`Step ${zoom}`} src={api.syntheticScreenshot(r.result_id, zoom)} />
          </div>
        </Modal>
      )}
    </div>
  );
}

function History({ id, onPick, picked }: { id: string; onPick: (r: SyntheticResult) => void; picked?: string }) {
  const q = useQuery({
    queryKey: ["synthetics", "results", id],
    refetchInterval: 30_000,
    queryFn: () => {
      const to = Date.now();
      return api.syntheticResults(id, to - 24 * 3_600_000, to);
    },
  });
  const results = q.data?.results ?? [];
  const passed = results.filter((r) => r.passed).length;
  return (
    <>
      <div className="section-title">
        Runs, past day{" "}
        {results.length > 0 && (
          <span className="muted">
            · {((passed / results.length) * 100).toFixed(1)}% passed of {results.length}
          </span>
        )}
      </div>
      <div className="synth-history">
        {[...results].reverse().map((r) => (
          <button
            key={r.result_id}
            className={`synth-bar ${r.passed ? "ok" : "alert"} ${r.result_id === picked ? "picked" : ""}`}
            title={`${new Date(r.ts).toLocaleString()} · ${r.passed ? "passed" : "failed"} · ${ms(r.duration_ms)}${r.batch_id ? " · CI" : ""}`}
            onClick={() => onPick(r)}
          />
        ))}
        {q.data && results.length === 0 && <span className="muted">No run yet.</span>}
      </div>
    </>
  );
}

function Detail({ t, onClose, onEdit }: { t: SyntheticTest; onClose: () => void; onEdit: () => void }) {
  const qc = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [picked, setPicked] = useState<SyntheticResult | null>(null);
  const id = t.public_id!;
  const act = async (f: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await f();
      await qc.invalidateQueries({ queryKey: ["synthetics"] });
    } finally {
      setBusy(false);
    }
  };
  const res = picked ?? t.last_result;
  const kind = kindOf(t);
  const metric = kind === "browser" ? "synthetics.browser.test_duration" : `synthetics.${kind === "multi" ? "multi" : t.subtype || "http"}.response.time`;
  return (
    <SidePanel
      onClose={onClose}
      storageKey="synthetics"
      header={
        <>
          <Outcome t={t} /> <strong style={{ marginLeft: 8 }}>{t.name}</strong> <span className="muted">· {typeLabel(t)}</span>
        </>
      }
    >
      <div className="infra-detail">
        <div className="mono ellipsis" style={{ marginBottom: 10 }}>
          {describeTarget(t)}
        </div>
        <div className="toolbar" style={{ marginBottom: 12 }}>
          <button
            className="btn small primary"
            disabled={busy}
            onClick={() =>
              act(async () => {
                setPicked(await api.triggerSynthetic(id));
              })
            }
          >
            {busy ? "Running…" : "Run now"}
          </button>
          <button className="btn small" onClick={onEdit}>
            Edit
          </button>
          <button className="btn small" disabled={busy} onClick={() => act(() => api.setSyntheticStatus(id, t.status === "paused" ? "live" : "paused"))}>
            {t.status === "paused" ? "Resume" : "Pause"}
          </button>
          <Link className="btn small" to={`/logs?query=${encodeURIComponent(`source:synthetics @synthetics.check_id:${id}`)}`}>
            View results in Logs
          </Link>
          <div className="spacer" />
          <button
            className="btn small ghost"
            disabled={busy}
            onClick={() =>
              act(async () => {
                await api.deleteSynthetic(id);
                onClose();
              })
            }
          >
            Delete
          </button>
        </div>
        <History id={id} picked={res?.result_id} onPick={setPicked} />
        {res && <ResultView key={res.result_id} r={res} />}
        <div className="infra-charts" style={{ gridTemplateColumns: "1fr" }}>
          <Graph title={kind === "browser" ? "Test duration" : "Response time"} query={`avg:${metric}{check_id:${id}}`} unit="ms" />
          <Graph title="Runs" query={`sum:synthetics.test_runs{check_id:${id}} by {status}.as_count()`} unit="" />
        </div>
        {kind === "api" && (
          <>
            <div className="section-title">Assertions</div>
            <ul className="synth-list">
              {t.config.assertions.map((a, i) => (
                <li key={i} className="mono">
                  {a.type}
                  {a.property ? ` ${a.property}` : ""} {OP_LABEL[a.operator] ?? a.operator} {targetText(a.target)}
                </li>
              ))}
              {t.config.assertions.length === 0 && <li className="muted">None: any answer passes.</li>}
            </ul>
          </>
        )}
        <div className="section-title">Alerting</div>
        <p className="muted">
          The test alerts by itself (see <Link to="/monitors">Monitors</Link>, <code>[Synthetics] {t.name}</code>). Run it from CI with{" "}
          <code>DD_API_HOST_OVERRIDE=… datadog-ci synthetics run-tests --public-id {id}</code>.
        </p>
      </div>
    </SidePanel>
  );
}

function NewTestMenu({ onPick }: { onPick: (k: Kind) => void }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="synth-new">
      <button className="btn primary" onClick={() => setOpen((o) => !o)}>
        <PlusIcon /> New Test
      </button>
      {open && (
        <div className="menu synth-new-menu" onMouseLeave={() => setOpen(false)}>
          {(
            [
              ["api", "API Test", "HTTP, SSL, TCP, DNS, UDP, WebSocket, gRPC, ICMP"],
              ["multi", "Multistep API Test", "Chained requests sharing variables"],
              ["browser", "Browser Test", "Steps in a headless Chromium"],
            ] as [Kind, string, string][]
          ).map(([k, label, hint]) => (
            <button
              key={k}
              onClick={() => {
                setOpen(false);
                onPick(k);
              }}
            >
              <span>
                <strong>{label}</strong>
                <span className="muted synth-hint">{hint}</span>
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

export function Synthetics() {
  const list = useQuery({ queryKey: ["synthetics"], queryFn: api.synthetics, refetchInterval: 15_000 });
  const [editing, setEditing] = useState<SyntheticTest | null>(null);
  const [params, setParams] = useSearchParams();
  const selected = params.get("test");
  const setSelected = (id: string | null) =>
    setParams(
      (p) => {
        if (id) p.set("test", id);
        else p.delete("test");
        return p;
      },
      { replace: true },
    );
  const [filter, setFilter] = useState("");
  const [type, setType] = useState("");
  const tests = (list.data?.tests ?? []).filter(
    (t) =>
      (!filter || `${t.name} ${t.tags.join(" ")} ${describeTarget(t)}`.toLowerCase().includes(filter.toLowerCase())) &&
      (!type || typeLabel(t) === type),
  );
  const current = list.data?.tests.find((t) => t.public_id === selected);
  const types = [...new Set((list.data?.tests ?? []).map(typeLabel))].sort();
  return (
    <>
      <PageHeader title="Synthetic Monitoring" tabs={SYNTH_TABS}>
        <NewTestMenu onPick={(k) => setEditing(blank(k))} />
      </PageHeader>
      <div className="filters-row">
        <input className="input small" style={{ width: 260 }} value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Filter by name, tag or target" />
        <select className="input small" value={type} onChange={(e) => setType(e.target.value)} aria-label="Type">
          <option value="">All types</option>
          {types.map((x) => (
            <option key={x}>{x}</option>
          ))}
        </select>
      </div>
      <div className="page-scroll page-padding">
        <div className="card" style={{ padding: 0 }}>
          <table className="list infra-table">
            <thead>
              <tr>
                <th style={{ width: 90 }}>Status</th>
                <th style={{ width: "24%" }}>Name</th>
                <th style={{ width: 115 }}>Type</th>
                <th>Target</th>
                <th className="num" style={{ width: 110 }}>
                  Duration
                </th>
                <th style={{ width: 120 }}>Last run</th>
                <th style={{ width: 110 }}>Frequency</th>
              </tr>
            </thead>
            <tbody>
              {tests.map((t) => (
                <tr key={t.public_id} className={t.public_id === selected ? "selected" : ""} onClick={() => setSelected(t.public_id!)}>
                  <td>
                    <Outcome t={t} />
                  </td>
                  <td className="infra-name ellipsis">{t.name}</td>
                  <td>
                    <span className="tag">{typeLabel(t)}</span>
                  </td>
                  <td className="mono ellipsis muted">{describeTarget(t)}</td>
                  <td className="num">{t.last_result ? ms(t.last_result.duration_ms) : "–"}</td>
                  <td className="muted">{t.last_result ? ago(t.last_result.ts) : "–"}</td>
                  <td className="muted">{FREQUENCIES.find(([, s]) => s === (t.options.tick_every ?? 60))?.[0] ?? `${t.options.tick_every} s`}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {list.data && list.data.tests.length === 0 && (
            <div className="empty-state">
              <h3>No test yet</h3>
              Create one to watch an endpoint, a certificate, a DNS record or a user journey: availability and timings become metrics and logs.
            </div>
          )}
        </div>
      </div>
      {current && !editing && <Detail t={current} onClose={() => setSelected(null)} onEdit={() => setEditing(current)} />}
      {editing && <TestModal test={editing} onClose={() => setEditing(null)} />}
    </>
  );
}

// --- global variables ----------------------------------------------------------------------------------------------

function VariableModal({ v, onClose }: { v: SyntheticVariable; onClose: () => void }) {
  const qc = useQueryClient();
  const [d, setD] = useState<SyntheticVariable>(v);
  const [tags, setTags] = useState(v.tags.join(", "));
  const [error, setError] = useState<string | null>(null);
  const save = async () => {
    const next = {
      ...d,
      tags: tags
        .split(",")
        .map((x) => x.trim())
        .filter(Boolean),
    };
    try {
      if (v.id) await api.updateSyntheticVariable(v.id, next);
      else await api.createSyntheticVariable(next);
      await qc.invalidateQueries({ queryKey: ["synthetic-variables"] });
      onClose();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : String(e));
    }
  };
  return (
    <Modal title={v.id ? "Edit Global Variable" : "New Global Variable"} onClose={onClose} width={520}>
      <div className="modal-body form">
        <Field label="Name">
          <input className="input mono" autoFocus value={d.name} placeholder="API_TOKEN" onChange={(e) => setD({ ...d, name: e.target.value.toUpperCase() })} />
        </Field>
        <Field label="Value">
          <input
            className="input mono"
            type={d.value.secure ? "password" : "text"}
            value={d.value.value ?? ""}
            placeholder={v.id && v.value.secure ? "Hidden: leave empty to keep it" : ""}
            onChange={(e) => setD({ ...d, value: { ...d.value, value: e.target.value } })}
          />
        </Field>
        <label className="checkbox">
          <input type="checkbox" checked={d.value.secure} onChange={(e) => setD({ ...d, value: { ...d.value, secure: e.target.checked } })} /> Secure: never shown
          again, hidden in results
        </label>
        <Field label="Description">
          <input className="input" value={d.description} onChange={(e) => setD({ ...d, description: e.target.value })} />
        </Field>
        <Field label="Tags">
          <input className="input mono" value={tags} onChange={(e) => setTags(e.target.value)} />
        </Field>
        {error && <div className="form-error">{error}</div>}
      </div>
      <div className="modal-footer">
        <button className="btn" onClick={onClose}>
          Cancel
        </button>
        <button className="btn primary" onClick={save}>
          Save
        </button>
      </div>
    </Modal>
  );
}

export function SyntheticVariables() {
  const qc = useQueryClient();
  const list = useQuery({ queryKey: ["synthetic-variables"], queryFn: api.syntheticVariables });
  const [editing, setEditing] = useState<SyntheticVariable | null>(null);
  const vars = list.data?.variables ?? [];
  return (
    <>
      <PageHeader title="Synthetic Monitoring" tabs={SYNTH_TABS}>
        <button className="btn primary" onClick={() => setEditing({ name: "", description: "", tags: [], value: { value: "", secure: false } })}>
          <PlusIcon /> New Variable
        </button>
      </PageHeader>
      <div className="page-scroll page-padding">
        <p className="muted" style={{ maxWidth: 900, marginTop: 0 }}>
          Values shared by every test, used as <code>{"{{ NAME }}"}</code> in requests, assertions and browser steps. Same API as Datadog's (
          <code>/api/v1/synthetics/variables</code>).
        </p>
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th style={{ width: "25%" }}>Name</th>
                <th>Value</th>
                <th>Description</th>
                <th style={{ width: 200 }}>Tags</th>
                <th style={{ width: 80 }} />
              </tr>
            </thead>
            <tbody>
              {vars.map((v) => (
                <tr key={v.id} onClick={() => setEditing(v)}>
                  <td className="mono">{v.name}</td>
                  <td className="mono muted ellipsis">{v.value.secure ? "••••••••" : v.value.value}</td>
                  <td className="muted ellipsis">{v.description}</td>
                  <td className="muted ellipsis">{v.tags.join(", ")}</td>
                  <td>
                    <button
                      className="btn small ghost"
                      onClick={async (e) => {
                        e.stopPropagation();
                        await api.deleteSyntheticVariable(v.id!);
                        await qc.invalidateQueries({ queryKey: ["synthetic-variables"] });
                      }}
                    >
                      Delete
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {list.data && vars.length === 0 && (
            <div className="empty-state">
              <h3>No global variable</h3>A host name, an API token, credentials for a login step…
            </div>
          )}
        </div>
      </div>
      {editing && <VariableModal v={editing} onClose={() => setEditing(null)} />}
    </>
  );
}

// --- CI batches ----------------------------------------------------------------------------------------------------

export function SyntheticCi() {
  const list = useQuery({ queryKey: ["synthetics", "batches"], queryFn: api.syntheticBatches, refetchInterval: 10_000 });
  const batches = list.data?.batches ?? [];
  const meta = (m: Record<string, unknown> | undefined) => {
    const ci = (m?.ci ?? {}) as { pipeline?: { name?: string; url?: string }; provider?: { name?: string } };
    const git = (m?.git ?? {}) as { branch?: string; commitSha?: string; commit?: { sha?: string } };
    return [ci.provider?.name, ci.pipeline?.name, git.branch, (git.commitSha ?? git.commit?.sha)?.slice(0, 8)].filter(Boolean).join(" · ");
  };
  return (
    <>
      <PageHeader title="Synthetic Monitoring" tabs={SYNTH_TABS} />
      <div className="page-scroll page-padding">
        <p className="muted" style={{ maxWidth: 900, marginTop: 0 }}>
          Tests run from CI by <code>datadog-ci synthetics run-tests</code>, pointed here with{" "}
          <code>DD_API_HOST_OVERRIDE={window.location.origin}</code> and an API key. Overrides (<code>startUrl</code>, <code>variables</code>,{" "}
          <code>headers</code>, <code>executionRule</code>…) apply.
        </p>
        {batches.map((b) => (
          <div key={b.batch_id} className="card synth-batch">
            <div className="card-title">
              <span className={`synth-state ${b.status === "passed" ? "ok" : b.status === "failed" ? "alert" : "paused"}`}>{b.status.replace("_", " ")}</span>{" "}
              <span className="mono">{b.batch_id.slice(0, 8)}</span> <span className="muted">· {ago(b.created)}</span>{" "}
              <span className="muted">{meta(b.metadata)}</span>
            </div>
            <table className="list">
              <tbody>
                {b.results.map((r, i) => (
                  <tr key={r.result_id ?? i}>
                    <td style={{ width: 30 }}>
                      <span className={`synth-dot ${r.status === "passed" ? "ok" : r.status === "failed" ? "alert" : "warn"}`} />
                    </td>
                    <td>
                      <Link to={`/synthetics/tests?test=${r.test_public_id}`}>{r.test_name}</Link>
                      {r.device && <span className="muted"> · {r.device.id}</span>}
                    </td>
                    <td className="muted" style={{ width: 120 }}>
                      {r.execution_rule.replace("_", " ")}
                    </td>
                    <td className="muted" style={{ width: 110 }}>
                      {r.timed_out ? "timed out" : r.status.replace("_", " ")}
                    </td>
                    <td className="num" style={{ width: 90 }}>
                      {r.duration ? ms(r.duration) : "–"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ))}
        {list.data && batches.length === 0 && (
          <div className="card empty-state">
            <h3>No CI run yet</h3>
            <code>DD_API_HOST_OVERRIDE=… DD_API_KEY=… datadog-ci synthetics run-tests --public-id abc-def-ghi</code>
          </div>
        )}
      </div>
    </>
  );
}
