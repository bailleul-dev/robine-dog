import { useQuery } from "@tanstack/react-query";
import { Fragment, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import {
  ApmIcon,
  BugIcon,
  LogsIcon,
  MetricsIcon,
  SearchIcon,
} from "../components/Icons";
import { MOD, NavEntry, PageHeader, SECTIONS } from "../components/Layout";
import { api } from "../lib/api";
import { Snippet } from "./Setup";

/**
 * Help, laid out like Datadog's help center: a search, how to get data in,
 * every product area with its pages (from the navigation, so it stays in
 * step), the search syntax, the shortcuts and the ports. The documentation
 * itself is at /docs.
 */
export function Help() {
  const q = useQuery({ queryKey: ["help"], queryFn: api.help });
  const [filter, setFilter] = useState("");
  const f = filter.trim().toLowerCase();
  // The documentation loads with the first search.
  const docs = useQuery({
    queryKey: ["docs-module"],
    queryFn: () => import("../lib/docs"),
    enabled: !!f,
    staleTime: Infinity,
  });
  const docHits = useMemo(
    () => (f && docs.data ? docs.data.search(f).slice(0, 6) : []),
    [f, docs.data],
  );

  return (
    <>
      <PageHeader title="Help">
        <span className="muted">Robine Dog {q.data?.version}</span>
        <Link className="btn small primary" to="/docs">
          Documentation
        </Link>
      </PageHeader>
      <div className="page-scroll">
        <div className="help-hero">
          <h1>How can we help?</h1>
          <label className="help-search">
            <SearchIcon />
            <input
              className="input"
              placeholder="Search the documentation, pages, syntax…"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              autoFocus
            />
          </label>
          {!f && (
            <div className="help-quick">
              {QUICK.map(([label, slug]) => (
                <Link key={slug} to={docPath(slug)}>
                  {label}
                </Link>
              ))}
            </div>
          )}
        </div>
        {f && (
          <div className="help-body">
            <h2 className="section-title">Documentation</h2>
            {!docs.data ? (
              <p className="muted">Searching…</p>
            ) : docHits.length ? (
              <div className="help-grid">
                {docHits.map((h) => (
                  <Link key={h.page.slug} className="card help-doc-hit" to={docPath(h.page.slug)}>
                    <span className="docs-hit-section">{h.page.section}</span>
                    <b>{h.page.title}</b>
                    {h.snippet && <span className="muted">{h.snippet}</span>}
                  </Link>
                ))}
              </div>
            ) : (
              <p className="muted">No page of the documentation mentions it.</p>
            )}
          </div>
        )}
        <Overview f={f} apiKey={q.data?.api_key_configured} />
      </div>
    </>
  );
}

const docPath = (slug: string) => `/docs/${slug}`;

/** The documentation's most read pages, under the search. */
const QUICK: [string, string][] = [
  ["Getting started", "getting-started"],
  ["Datadog Agent", "agent"],
  ["Search syntax", "search-syntax"],
  ["Metric queries", "metrics"],
  ["Monitors", "monitors"],
  ["Dashboards", "dashboards"],
  ["API", "api"],
];

const origin = () => window.location.origin;
const host = () => window.location.hostname;

type Source = {
  title: string;
  icon: React.ReactNode;
  text: string;
  endpoint: string;
  to?: string;
  doc: string;
  snippet: () => string;
};

const SOURCES: Source[] = [
  {
    title: "Datadog Agent",
    doc: "agent",
    icon: <MetricsIcon />,
    text: "Hosts, containers and Kubernetes: the Agent forwards metrics, traces and logs as it would to Datadog.",
    endpoint: "HTTP 8080",
    to: "/organization-settings/agent",
    snippet: () => `DD_DD_URL=${origin()}\nDD_APM_DD_URL=${origin()}\nDD_SITE=robine-dog.invalid`,
  },
  {
    title: "Traces",
    doc: "tracing",
    icon: <ApmIcon />,
    text: "Any ddtrace library: Robine Dog plays the local trace agent. With log injection, logs and traces link both ways.",
    endpoint: "TCP 8126",
    snippet: () =>
      `DD_TRACE_AGENT_URL=http://${host()}:8126 DD_SERVICE=my-api DD_ENV=dev \\\nDD_LOGS_INJECTION=true ddtrace-run python app.py`,
  },
  {
    title: "Logs",
    doc: "log-collection",
    icon: <LogsIcon />,
    text: "The logs HTTP intake, as Datadog's: JSON, gzip, a DD-API-KEY header. Raw lines go through the log pipelines.",
    endpoint: "POST /api/v2/logs",
    to: "/logs/pipelines",
    snippet: () =>
      `curl -X POST ${origin()}/api/v2/logs \\\n  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H 'Content-Type: application/json' \\\n  -d '[{"message":"payment failed","service":"pay","status":"error"}]'`,
  },
  {
    title: "Metrics",
    doc: "dogstatsd",
    icon: <MetricsIcon />,
    text: "Any DogStatsD client, or the series API as the Agent and scripts post it.",
    endpoint: "UDP 8125",
    to: "/metrics/summary",
    snippet: () =>
      `echo -n 'shop.orders:1|c|#env:dev' | nc -u -w0 ${host()} 8125`,
  },
  {
    title: "OpenTelemetry",
    doc: "opentelemetry",
    icon: <ApmIcon />,
    text: "OTLP over HTTP, protobuf or JSON, mapped as Datadog's OTLP ingestion maps it.",
    endpoint: "/v1/traces · /v1/logs · /v1/metrics",
    to: "/organization-settings/opentelemetry",
    snippet: () =>
      `OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf \\\nOTEL_EXPORTER_OTLP_ENDPOINT=${origin()} \\\nOTEL_EXPORTER_OTLP_HEADERS="dd-api-key=$ROBINE_DOG_API_KEY"`,
  },
  {
    title: "Sentry SDK",
    doc: "sentry",
    icon: <BugIcon />,
    text: "Keep the SDK, change the DSN: each error becomes an error log and an Error Tracking issue.",
    endpoint: "/sentry/<service>/1",
    to: "/error-tracking",
    snippet: () => `dsn="${origin().replace("://", "://<client token>@")}/sentry/my-api/1"`,
  },
];

const SYNTAX: [string, string][] = [
  ["service:web", "A tag or reserved attribute equals a value"],
  ["@http.status_code:502", "An attribute, with the @ prefix"],
  ["payment failed", "Full-text search in the message"],
  ['"payment failed"', "An exact phrase"],
  ["service:web AND env:prod", "Both (AND is implied between terms)"],
  ["status:error OR status:warn", "Either"],
  ["-env:dev  ·  NOT env:dev", "Exclude"],
  ["service:(web OR api)", "Any of several values"],
  ["host:web-*  ·  @user.id:*", "Wildcard, and attribute present"],
  ["@http.status_code:>=500", "Comparison on a number"],
  ["@duration:>1.5s", "Durations with their unit"],
  ["@retries:[1 TO 5]", "A range, bounds included"],
];

const SHORTCUTS: [string, string][] = [
  [`${MOD} K`, "Go to any page, dashboard, monitor, service or metric"],
  ["↑ ↓  ·  Tab  ·  Enter", "In a search bar: pick a suggestion, complete it, run the query"],
  ["j  ·  k", "Log Explorer: next and previous log"],
  ["Esc", "Close a side panel, a menu or a dialog"],
];

const PORTS: [string, string, string][] = [
  ["8080", "HTTP", "The UI, the API and the intakes: logs, series, Agent, OTLP, Sentry"],
  ["8126", "TCP", "Trace agent API: /v0.3, /v0.4, /v0.5/traces, /info"],
  ["8125", "UDP", "DogStatsD"],
];

function Overview({ f, apiKey }: { f: string; apiKey?: boolean }) {
  const has = (...texts: (string | undefined)[]) =>
    !f || texts.some((t) => t?.toLowerCase().includes(f));

  const sources = SOURCES.filter((s) => has(s.title, s.text, s.endpoint));
  const areas = SECTIONS.flat()
    .map((item) => {
      const entries = [
        ...item.links,
        ...(item.groups ?? []).flatMap((g) => g.links),
      ].filter((e): e is NavEntry & { to: string } => !!e.to);
      const unique = entries.filter(
        (e, i) => entries.findIndex((x) => x.label === e.label) === i,
      );
      const shown = has(item.label)
        ? unique
        : unique.filter((e) => has(e.label, e.hint));
      return { item, links: shown };
    })
    .filter((a) => a.links.length > 0);
  const syntax = SYNTAX.filter(([a, b]) => has(a, b));
  const shortcuts = SHORTCUTS.filter(([a, b]) => has(a, b));
  const ports = PORTS.filter((p) => has(...p));
  const nothing =
    !sources.length && !areas.length && !syntax.length && !shortcuts.length && !ports.length;

  return (
    <div className="help-body">
      {apiKey === false && !f && (
        <div className="help-notice">
          No API key yet: the intakes need one.{" "}
          <Link to="/organization-settings/api-keys">Create an API key</Link>
        </div>
      )}

      {sources.length > 0 && (
        <section>
          <h2 className="section-title">Get data in</h2>
          <div className="help-grid">
            {sources.map((s) => (
              <SourceCard key={s.title} source={s} />
            ))}
          </div>
        </section>
      )}

      {areas.length > 0 && (
        <section>
          <h2 className="section-title">Product areas</h2>
          <div className="help-grid">
            {areas.map(({ item, links }) => (
              <div className="card help-area" key={item.id}>
                <div className="card-title">
                  <span className="help-icon">{item.icon}</span>
                  {item.to ? <Link to={item.to}>{item.label}</Link> : item.label}
                </div>
                <ul>
                  {links.map((e) => (
                    <li key={e.label}>
                      <Link to={e.to}>{e.label}</Link>
                      {e.hint && <span className="muted"> — {e.hint}</span>}
                    </li>
                  ))}
                </ul>
              </div>
            ))}
          </div>
        </section>
      )}

      <div className="help-columns">
        {syntax.length > 0 && (
          <section>
            <h2 className="section-title">Search syntax</h2>
            <div className="card help-table">
              <table>
                <tbody>
                  {syntax.map(([q, meaning]) => (
                    <tr key={q}>
                      <td>
                        <code>{q}</code>
                      </td>
                      <td>{meaning}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
              <p className="muted help-foot">
                The same syntax in the Log, Trace, RUM and Event explorers,
                monitors and dashboards.{" "}
                <Link to={docPath("search-syntax")}>Full reference</Link>
              </p>
            </div>
          </section>
        )}
        <div>
          {shortcuts.length > 0 && (
            <section>
              <h2 className="section-title">Keyboard shortcuts</h2>
              <div className="card help-table">
                <table>
                  <tbody>
                    {shortcuts.map(([keys, what]) => (
                      <tr key={keys}>
                        <td className="help-keys">
                          {keys.split("  ·  ").map((k, i) => (
                            <Fragment key={k}>
                              {i > 0 && <span className="muted"> or </span>}
                              {k.split(" ").map((part) => (
                                <kbd key={part}>{part}</kbd>
                              ))}
                            </Fragment>
                          ))}
                        </td>
                        <td>{what}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </section>
          )}
          {ports.length > 0 && (
            <section>
              <h2 className="section-title">Ports</h2>
              <div className="card help-table">
                <table>
                  <tbody>
                    {ports.map(([port, proto, what]) => (
                      <tr key={port}>
                        <td>
                          <code>{port}</code> <span className="muted">{proto}</span>
                        </td>
                        <td>{what}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </section>
          )}
        </div>
      </div>

      {nothing && (
        <div className="empty-state">
          <h3>No page of the app matches</h3>
          Browse the <Link to="/docs">documentation</Link>.
        </div>
      )}
    </div>
  );
}

function SourceCard({ source }: { source: Source }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="card help-source">
      <div className="card-title">
        <span className="help-icon">{source.icon}</span>
        {source.title}
        <div className="spacer" />
        <code className="help-endpoint">{source.endpoint}</code>
      </div>
      <p>{source.text}</p>
      <div className="help-actions">
        <button className="btn small" onClick={() => setOpen(!open)}>
          {open ? "Hide example" : "Example"}
        </button>
        <Link className="btn small ghost" to={docPath(source.doc)}>
          Docs
        </Link>
        {source.to && (
          <Link className="btn small ghost" to={source.to}>
            Open
          </Link>
        )}
      </div>
      {open && <Snippet title="Example" text={source.snippet()} />}
    </div>
  );
}
