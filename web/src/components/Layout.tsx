import { NavLink, Outlet, useLocation } from "react-router-dom";
import type { Identity } from "../lib/api";
import { useEffect, useMemo, useRef, useState } from "react";
import { QuickNav, type QuickNavPage } from "./QuickNav";
import {
  AiIcon,
  ApmIcon,
  BellIcon,
  BugIcon,
  CollapseIcon,
  DashboardIcon,
  DeliveryIcon,
  DigitalExperienceIcon,
  HelpIcon,
  InfraIcon,
  IntegrationsIcon,
  Logo,
  LogsIcon,
  MetricsIcon,
  SearchIcon,
  SecurityIcon,
  ServiceMgmtIcon,
  SignOutIcon,
  WatchdogIcon,
} from "./Icons";

/** A link of the navigation; without `to`, Datadog has it and robine-dog does not. */
type NavEntry = { label: string; to?: string; hint?: string };
type NavGroup = { title: string; links: NavEntry[] };
type Item = {
  id: string;
  label: string;
  icon: React.ReactNode;
  /** Where a click on the item goes; none when nothing under it exists here. */
  to?: string;
  /** Path prefixes that make the item the current one. */
  match: string[];
  /** The products, on the left of the menu. */
  links: NavEntry[];
  /** Configuration and setup, on its right. */
  groups?: NavGroup[];
};

/**
 * Datadog's navigation, section by section and in its order: shortcuts,
 * product areas, then metrics and logs. What robine-dog implements is a
 * link; the rest is listed, greyed out.
 */
const SECTIONS: Item[][] = [
  [
    {
      id: "watchdog",
      label: "Watchdog",
      icon: <WatchdogIcon />,
      to: "/watchdog",
      match: ["/watchdog"],
      links: [
        {
          label: "Alerts",
          to: "/watchdog",
          hint: "Anomalies detected automatically",
        },
        {
          label: "Insights",
          to: "/watchdog/insights",
          hint: "What stands out in the errors",
        },
        {
          label: "Root Cause Analysis",
          to: "/watchdog",
          hint: "A story's anomalous dependencies",
        },
        {
          label: "Impact Analysis",
          to: "/watchdog",
          hint: "RUM sessions and users behind a story",
        },
      ],
    },
    {
      id: "service-mgmt",
      label: "Service Mgmt",
      icon: <ServiceMgmtIcon />,
      to: "/slo/manage",
      match: ["/slo", "/event", "/cases", "/on-call", "/workflows", "/apps"],
      links: [
        {
          label: "SLOs",
          to: "/slo/manage",
          hint: "Service level objectives and error budgets",
        },
        {
          label: "Incidents",
          to: "/incidents",
          hint: "Declare and manage incidents",
        },
        {
          label: "On-Call",
          to: "/on-call",
          hint: "Schedules, escalation policies and pages",
        },
        {
          label: "Case Management",
          to: "/cases",
          hint: "Cases opened from monitors, signals and issues",
        },
        {
          label: "Event Management",
          to: "/event/explorer",
          hint: "Monitor alerts, Agent and API events",
        },
        {
          label: "Workflow Automation",
          to: "/workflows",
          hint: "HTTP, Kubernetes and robine-dog actions on triggers",
        },
        {
          label: "App Builder",
          to: "/apps",
          hint: "Forms whose buttons run workflows",
        },
      ],
      groups: [
        {
          title: "Settings",
          links: [
            { label: "Incident Settings" },
            {
              label: "Status Pages",
              to: "/status-pages",
              hint: "A public page of components and incidents",
            },
          ],
        },
      ],
    },
    {
      id: "dashboards",
      label: "Dashboards",
      icon: <DashboardIcon />,
      to: "/dashboard/lists",
      match: ["/dashboard", "/notebook", "/sheets"],
      links: [
        {
          label: "Dashboard List",
          to: "/dashboard/lists",
          hint: "Browse, create and import dashboards",
        },
        {
          label: "Notebooks",
          to: "/notebook/list",
          hint: "Text and graphs, for investigations",
        },
        {
          label: "Reports",
          to: "/dashboard/reports",
          hint: "Dashboards e-mailed as PDF on a schedule",
        },
        {
          label: "Sheets",
          to: "/sheets",
          hint: "Tables over logs, spans and RUM, with computed columns",
        },
      ],
      groups: [
        {
          title: "Settings",
          links: [{ label: "Powerpacks" }, { label: "Datasets" }],
        },
      ],
    },
    {
      id: "monitors",
      label: "Monitors",
      icon: <BellIcon />,
      to: "/monitors/manage",
      match: ["/monitors"],
      links: [
        {
          label: "Monitor List",
          to: "/monitors/manage",
          hint: "Every monitor and its state",
        },
        {
          label: "Triggered Monitors",
          to: "/monitors/triggered",
          hint: "What is alerting now",
        },
        { label: "New Monitor", to: "/monitors/create" },
        {
          label: "Downtimes",
          to: "/monitors/downtimes",
          hint: "Scheduled silences",
        },
        {
          label: "Monitor Quality",
          to: "/monitors/quality",
          hint: "Monitors that notify nobody, stay muted or alerting",
        },
        {
          label: "Templates",
          to: "/monitors/templates",
          hint: "Start from a ready-made monitor",
        },
      ],
      groups: [
        {
          title: "Settings",
          links: [{ label: "Notification Rules" }, { label: "Tag Policies" }],
        },
      ],
    },
  ],
  [
    {
      id: "infrastructure",
      label: "Infrastructure",
      icon: <InfraIcon />,
      to: "/infrastructure",
      match: [
        "/infrastructure",
        "/containers",
        "/orchestration",
        "/processes",
        "/resources",
        "/network",
        "/devices",
        "/fleet",
        "/cost",
      ],
      links: [
        {
          label: "Hosts",
          to: "/infrastructure",
          hint: "Every host an Agent reports from",
        },
        {
          label: "Containers",
          to: "/containers",
          hint: "CPU, memory and restarts per container",
        },
        {
          label: "Kubernetes",
          to: "/orchestration/overview/pod",
          hint: "Pods, deployments, nodes, namespaces",
        },
        {
          label: "Processes",
          to: "/processes",
          hint: "Live processes of every host",
        },
        { label: "Serverless" },
        {
          label: "Cloud Network",
          to: "/network",
          hint: "Flows between hosts (system-probe)",
        },
        {
          label: "Network Devices",
          to: "/devices",
          hint: "SNMP devices and interfaces",
        },
        {
          label: "Resource Catalog",
          to: "/resources",
          hint: "Every host, container, pod and device",
        },
        {
          label: "Cloud Cost",
          to: "/cost",
          hint: "Cloud bills, Custom Costs and Kubernetes cost allocation",
        },
      ],
      groups: [
        {
          title: "Agents",
          links: [
            {
              label: "Fleet Automation",
              to: "/fleet",
              hint: "Agents, versions and checks",
            },
            { label: "Remote Configuration" },
          ],
        },
      ],
    },
    {
      id: "apm",
      label: "APM",
      icon: <ApmIcon />,
      to: "/apm/services",
      match: [
        "/apm",
        "/teams",
        "/profiling",
        "/databases",
        "/data-streams",
        "/data-jobs",
      ],
      links: [
        {
          label: "Software Catalog",
          to: "/apm/services",
          hint: "Services, their health and owners",
        },
        {
          label: "Traces",
          to: "/apm/traces",
          hint: "Search and analyze spans",
        },
        {
          label: "Service Map",
          to: "/apm/map",
          hint: "How services call each other",
        },
        {
          label: "Scorecards",
          to: "/apm/scorecards",
          hint: "Production readiness, observability, ownership",
        },
        { label: "Teams", to: "/teams", hint: "Owners of services" },
        {
          label: "Error Tracking",
          to: "/error-tracking",
          hint: "Errors grouped into issues",
        },
        {
          label: "Profiles",
          to: "/profiling",
          hint: "Continuous Profiler: where code spends CPU, time and memory",
        },
        {
          label: "Database Monitoring",
          to: "/databases",
          hint: "Top queries, plans, active sessions",
        },
        {
          label: "Data Streams Monitoring",
          to: "/data-streams",
          hint: "Queues: throughput, latency, consumer lag",
        },
        {
          label: "Data Jobs Monitoring",
          to: "/data-jobs",
          hint: "Spark applications, jobs and stages",
        },
      ],
      groups: [
        {
          title: "Settings",
          links: [
            { label: "Generate Metrics", to: "/apm/generate-metrics" },
            {
              label: "Ingestion Control",
              to: "/apm/ingestion-control",
              hint: "Spans received per service, and sampling",
            },
            {
              label: "Retention Filters",
              to: "/apm/retention-filters",
              hint: "Which traces are stored",
            },
            { label: "Dynamic Instrumentation" },
          ],
        },
      ],
    },
    {
      id: "digital-experience",
      label: "Digital Experience",
      icon: <DigitalExperienceIcon />,
      to: "/synthetics/tests",
      match: ["/synthetics", "/rum"],
      links: [
        {
          label: "Synthetic Monitoring",
          to: "/synthetics/tests",
          hint: "API, multistep and browser tests, from CI too",
        },
        {
          label: "Real User Monitoring",
          to: "/rum/performance-monitoring",
          hint: "Page performance, sessions and browser errors",
        },
        {
          label: "Session Replay",
          to: "/rum/sessions",
          hint: "Replay recorded browser sessions",
        },
        {
          label: "Product Analytics",
          to: "/rum/funnels",
          hint: "Funnels and retention",
        },
        { label: "Mobile App Testing" },
        {
          label: "Error Tracking",
          to: "/error-tracking",
          hint: "Errors grouped into issues",
        },
      ],
    },
    {
      id: "software-delivery",
      label: "Software Delivery",
      icon: <DeliveryIcon />,
      to: "/ci/pipelines",
      match: ["/ci"],
      links: [
        {
          label: "CI Visibility",
          to: "/ci/pipelines",
          hint: "Pipelines and their failures",
        },
        {
          label: "Test Optimization",
          to: "/ci/tests",
          hint: "Test runs, flaky and slow tests",
        },
        {
          label: "Continuous Testing",
          to: "/synthetics/ci",
          hint: "Synthetic tests run from CI (datadog-ci)",
        },
        {
          label: "CD Visibility",
          to: "/ci/deployments",
          hint: "Deployments by service and env",
        },
        {
          label: "DORA Metrics",
          to: "/ci/dora",
          hint: "Frequency, lead time, failure rate, restore time",
        },
        {
          label: "Code Coverage",
          to: "/ci/coverage",
          hint: "Coverage by repository and branch",
        },
      ],
    },
    {
      id: "security",
      label: "Security",
      icon: <SecurityIcon />,
      to: "/security/signals",
      match: ["/security", "/audit"],
      links: [
        {
          label: "Signals",
          to: "/security/signals",
          hint: "Cloud SIEM: threats detected in logs",
        },
        {
          label: "Detection Rules",
          to: "/security/rules",
          hint: "Thresholds and new values on logs",
        },
        {
          label: "Audit Trail",
          to: "/audit",
          hint: "Who changed what, through the UI or the API",
        },
        {
          label: "Code Security",
          to: "/security/code",
          hint: "Vulnerable libraries (OSV)",
        },
        {
          label: "Cloud Security",
          to: "/security/cloud",
          hint: "Host and Kubernetes compliance findings",
        },
        {
          label: "App & API Protection",
          to: "/security/appsec",
          hint: "Attacks and API endpoints",
        },
        {
          label: "Workload Protection",
          to: "/security/workload",
          hint: "Runtime security events",
        },
        { label: "Sensitive Data Scanner", to: "/logs/sensitive-data-scanner" },
      ],
    },
    {
      id: "llm",
      label: "LLM Observability",
      icon: <AiIcon />,
      to: "/llm/overview",
      match: ["/llm"],
      links: [
        {
          label: "Overview",
          to: "/llm/overview",
          hint: "Calls, tokens, cost, models, evaluations",
        },
        {
          label: "Traces",
          to: "/llm/traces",
          hint: "Prompts, completions and tool calls",
        },
        {
          label: "Experiments",
          to: "/llm/experiments",
          hint: "Datasets and experiment runs compared",
        },
        {
          label: "Agent Observability",
          to: "/llm/agents",
          hint: "Agents, their tools and each run's steps",
        },
      ],
    },
  ],
  [
    {
      id: "errors",
      label: "Errors",
      icon: <BugIcon />,
      to: "/error-tracking",
      match: ["/error-tracking"],
      links: [
        {
          label: "Issues",
          to: "/error-tracking",
          hint: "Errors grouped into issues",
        },
        {
          label: "Explorer",
          to: "/error-tracking",
          hint: "Issues across logs, traces and RUM",
        },
        {
          label: "Exception Replay",
          to: "/error-tracking",
          hint: "Local variables of an error, from its issue",
        },
      ],
      groups: [
        {
          title: "Settings",
          links: [
            {
              label: "Issue Teams",
              to: "/error-tracking",
              hint: "Team facet, from the Software Catalog",
            },
            {
              label: "Custom Grouping",
              to: "/error-tracking",
              hint: "error.fingerprint groups errors",
            },
            { label: "Dynamic Sampling" },
          ],
        },
      ],
    },
    {
      id: "metrics",
      label: "Metrics",
      icon: <MetricsIcon />,
      to: "/metrics/explorer",
      match: ["/metrics"],
      links: [
        {
          label: "Explorer",
          to: "/metrics/explorer",
          hint: "Graph any metric",
        },
        {
          label: "Summary",
          to: "/metrics/summary",
          hint: "Every metric, its type, tags and hosts",
        },
        {
          label: "Volume",
          to: "/metrics/volume",
          hint: "Points and series stored per metric",
        },
      ],
      groups: [
        {
          title: "Settings",
          links: [
            {
              label: "Metrics without Limits",
              to: "/metrics/tags",
              hint: "Tags kept per metric",
            },
            {
              label: "Reference Tables",
              to: "/logs/reference-tables",
              hint: "CSV tables for lookup processors",
            },
          ],
        },
      ],
    },
    {
      id: "logs",
      label: "Logs",
      icon: <LogsIcon />,
      to: "/logs",
      match: ["/logs"],
      links: [
        {
          label: "Explorer",
          to: "/logs",
          hint: "Search, analytics and patterns",
        },
        {
          label: "Live Tail",
          to: "/logs/livetail",
          hint: "Logs as they arrive",
        },
        {
          label: "Error Tracking",
          to: "/error-tracking",
          hint: "Error logs grouped into issues",
        },
      ],
      groups: [
        {
          title: "Log Configuration",
          links: [
            { label: "Pipelines", to: "/logs/pipelines" },
            { label: "Generate Metrics", to: "/logs/generate-metrics" },
            {
              label: "Indexes",
              to: "/logs/indexes",
              hint: "Exclusion filters, sampling, daily quotas",
            },
            {
              label: "Archives",
              to: "/logs/archives",
              hint: "Hourly gzip archives to S3 or disk",
            },
            {
              label: "Rehydrate from Archives",
              to: "/logs/archives",
              hint: "Reload archived logs into a temporary index",
            },
            {
              label: "Log Forwarding",
              to: "/logs/forwarding",
              hint: "HTTP, Splunk HEC and Elasticsearch destinations",
            },
            {
              label: "Sensitive Data Scanner",
              to: "/logs/sensitive-data-scanner",
              hint: "Redact card numbers, e-mails, tokens at intake",
            },
          ],
        },
      ],
    },
  ],
  [
    {
      id: "integrations",
      label: "Integrations",
      icon: <IntegrationsIcon />,
      to: "/integrations",
      match: ["/integrations", "/monitors/webhooks", "/organization-settings"],
      links: [
        {
          label: "Integrations",
          to: "/integrations",
          hint: "What sends logs and metrics here",
        },
        {
          label: "Webhooks",
          to: "/monitors/webhooks",
          hint: "Where monitors send their notifications",
        },
        {
          label: "APIs",
          to: "/organization-settings/api-keys",
          hint: "API and application keys",
        },
        { label: "Marketplace" },
      ],
      groups: [
        {
          title: "Install",
          links: [
            {
              label: "Agent",
              to: "/organization-settings/agent",
              hint: "Install the Datadog Agent against robine-dog",
            },
            {
              label: "OpenTelemetry",
              to: "/organization-settings/opentelemetry",
              hint: "SDK and Collector configuration",
            },
            {
              label: "Fleet Automation",
              to: "/fleet",
              hint: "Agents, versions and checks",
            },
          ],
        },
      ],
    },
  ],
];

const MOD = /Mac|iPhone|iPad/.test(navigator.platform) ? "⌘" : "Ctrl";
const UNAVAILABLE = "In Datadog, not in robine-dog";

function initials(id: Identity): string {
  const n = id.name ?? id.email ?? "?";
  return n
    .split(/[\s@.]+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((s) => s[0]?.toUpperCase())
    .join("");
}

function Entry({
  link,
  onNavigate,
}: {
  link: NavEntry;
  onNavigate: () => void;
}) {
  const body = (
    <>
      <span className="nav-entry-label">{link.label}</span>
      {link.hint && <span className="nav-entry-hint">{link.hint}</span>}
    </>
  );
  return link.to ? (
    <NavLink to={link.to} end className="nav-entry" onClick={onNavigate}>
      {body}
    </NavLink>
  ) : (
    <span
      className="nav-entry disabled"
      aria-disabled="true"
      title={UNAVAILABLE}
    >
      {body}
    </span>
  );
}

function stored(key: string): boolean {
  try {
    return window.localStorage.getItem(key) === "1";
  } catch {
    return false;
  }
}

const PHONE = "(max-width: 760px)";

// On a phone the navigation is a drawer, and its menus open on tap.
function usePhone() {
  const [phone, setPhone] = useState(() => window.matchMedia(PHONE).matches);
  useEffect(() => {
    const m = window.matchMedia(PHONE);
    const on = () => setPhone(m.matches);
    m.addEventListener("change", on);
    return () => m.removeEventListener("change", on);
  }, []);
  return phone;
}

export function Layout({ me }: { me: Identity }) {
  const { pathname } = useLocation();
  const phone = usePhone();
  const [drawer, setDrawer] = useState(false);
  const [collapsed, setCollapsed] = useState(() =>
    stored("robine-dog:nav-collapsed"),
  );
  // The menu shown beside the bar: an item's id, or "user".
  const [open, setOpen] = useState<string | null>(null);
  const timer = useRef<number | undefined>(undefined);
  const [searching, setSearching] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setSearching((v) => !v);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const pages = useMemo<QuickNavPage[]>(
    () =>
      SECTIONS.flat()
        .flatMap((it) =>
          [...it.links, ...(it.groups ?? []).flatMap((g) => g.links)]
            .filter((l) => l.to)
            .map((l) => ({ label: l.label, section: it.label, to: l.to! })),
        )
        // Error Tracking is under both APM and Logs: once is enough.
        .filter((p, i, all) => all.findIndex((o) => o.to === p.to) === i),
    [],
  );

  // Opening waits a moment, so crossing other items on the way to a menu
  // does not switch it; leaving the navigation closes it.
  const later = (id: string | null, delay: number) => {
    if (phone) return;
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setOpen(id), delay);
  };
  useEffect(() => () => window.clearTimeout(timer.current), []);
  useEffect(() => {
    setOpen(null);
    setDrawer(false);
  }, [pathname]);

  const toggle = () => {
    const next = !collapsed;
    setCollapsed(next);
    try {
      window.localStorage.setItem("robine-dog:nav-collapsed", next ? "1" : "0");
    } catch {
      // Private browsing: the choice lasts for the page.
    }
  };

  const items = SECTIONS.flat();
  // The most specific item wins: /monitors/webhooks is Integrations.
  const current = items
    .flatMap((it) => it.match.map((m) => ({ id: it.id, m })))
    .filter(({ m }) => pathname.startsWith(m))
    .sort((a, b) => b.m.length - a.m.length)[0]?.id;
  const shown = items.find((it) => it.id === open);
  const close = () => setOpen(null);

  return (
    <div className="app">
      <header className="topbar">
        <button
          type="button"
          className="nav-burger"
          aria-label={drawer ? "Close the navigation" : "Open the navigation"}
          aria-expanded={drawer}
          onClick={() => {
            setDrawer(!drawer);
            setOpen(null);
          }}
        >
          <span />
          <span />
          <span />
        </button>
        <NavLink to="/logs" className="nav-logo" aria-label="Home">
          <Logo />
          <span className="nav-brand">robine-dog</span>
        </NavLink>
        <button
          type="button"
          className="nav-search"
          onClick={() => setSearching(true)}
          title={`Search (${MOD}+K)`}
        >
          <SearchIcon />
          <span>Search pages, dashboards, monitors, services, metrics…</span>
          <kbd>{MOD} K</kbd>
        </button>
      </header>
      <div className="app-body">
        <nav
          className={`nav${collapsed && !phone ? " collapsed" : ""}${drawer ? " drawer-open" : ""}`}
          aria-label="Main"
          onMouseLeave={() => later(null, 250)}
        >
          <div
            className="nav-bar"
            onMouseEnter={() => window.clearTimeout(timer.current)}
          >
            <div className="nav-items">
              {SECTIONS.map((section, i) => (
                <div className="nav-section" key={i}>
                  {section.map((it) => {
                    const cls = `nav-link${it.id === current ? " active" : ""}${it.to ? "" : " disabled"}${open === it.id ? " open" : ""}`;
                    const body = (
                      <>
                        {it.icon}
                        <span>{it.label}</span>
                      </>
                    );
                    const hover = {
                      onMouseEnter: () => later(it.id, open ? 120 : 60),
                      onFocus: () => !phone && setOpen(it.id),
                      // On a phone, a tap shows the item's pages.
                      onClick: (e: React.MouseEvent) => {
                        if (!phone) return;
                        e.preventDefault();
                        setOpen(it.id);
                      },
                    };
                    return it.to ? (
                      <NavLink
                        key={it.id}
                        to={it.to}
                        className={cls}
                        title={collapsed ? it.label : undefined}
                        {...hover}
                      >
                        {body}
                      </NavLink>
                    ) : (
                      <span
                        key={it.id}
                        className={cls}
                        aria-disabled="true"
                        title={collapsed ? it.label : undefined}
                        {...hover}
                      >
                        {body}
                      </span>
                    );
                  })}
                </div>
              ))}
            </div>
            <div className="nav-bottom">
              <NavLink
                to="/help"
                className="nav-link"
                onMouseEnter={() => later(null, 120)}
              >
                <HelpIcon />
                <span>Help</span>
              </NavLink>
              <button
                type="button"
                className={`nav-link nav-user${open === "user" ? " open" : ""}`}
                onMouseEnter={() => later("user", open ? 120 : 60)}
                onClick={() => setOpen(open === "user" ? null : "user")}
              >
                <span className="nav-avatar">{initials(me)}</span>
                <span className="nav-user-name">
                  {me.name ?? me.email ?? "Signed in"}
                  {me.name && me.email && <small>{me.email}</small>}
                </span>
              </button>
              <button
                type="button"
                className="nav-link nav-collapse"
                onClick={toggle}
                onMouseEnter={() => later(null, 120)}
                aria-label={
                  collapsed
                    ? "Expand the navigation"
                    : "Collapse the navigation"
                }
                title={collapsed ? "Expand" : "Collapse"}
              >
                <CollapseIcon />
                <span>Collapse</span>
              </button>
            </div>
          </div>
          {shown && (
            <div
              className="nav-menu"
              onMouseEnter={() => window.clearTimeout(timer.current)}
            >
              <div className="nav-menu-main">
                <BackEntry onBack={close} />
                <div className="nav-menu-title">{shown.label}</div>
                {shown.links.map((l) => (
                  <Entry key={l.label} link={l} onNavigate={close} />
                ))}
              </div>
              {shown.groups && (
                <div className="nav-menu-side">
                  {shown.groups.map((g) => (
                    <div className="nav-menu-group" key={g.title}>
                      <div className="nav-menu-heading">{g.title}</div>
                      {g.links.map((l) => (
                        <Entry key={l.label} link={l} onNavigate={close} />
                      ))}
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}
          {open === "user" && (
            <div
              className="nav-menu nav-menu-user"
              onMouseEnter={() => window.clearTimeout(timer.current)}
            >
              <div className="nav-menu-main">
                <BackEntry onBack={close} />
                <div className="nav-menu-title">{me.name ?? "Signed in"}</div>
                {me.email && <div className="nav-menu-note">{me.email}</div>}
                <Entry
                  link={{ label: "Personal Settings" }}
                  onNavigate={close}
                />
                <Entry
                  link={{ label: "Organization Settings" }}
                  onNavigate={close}
                />
                <Entry
                  link={{
                    label: "API Keys",
                    to: "/organization-settings/api-keys",
                  }}
                  onNavigate={close}
                />
                <Entry link={{ label: "Plan & Usage" }} onNavigate={close} />
                <a href="/auth/logout" className="nav-entry nav-signout">
                  <span className="nav-entry-label">
                    <SignOutIcon width={14} height={14} /> Sign out
                  </span>
                </a>
              </div>
            </div>
          )}
        </nav>
        {drawer && (
          <div
            className="nav-drawer-backdrop"
            onClick={() => setDrawer(false)}
          />
        )}
        {searching && (
          <QuickNav pages={pages} onClose={() => setSearching(false)} />
        )}
        <main className="main">
          <Outlet />
        </main>
      </div>
    </div>
  );
}

// Phone only (hidden by CSS elsewhere): back from a menu to the sections.
function BackEntry({ onBack }: { onBack: () => void }) {
  return (
    <button type="button" className="nav-menu-back" onClick={onBack}>
      ‹ All sections
    </button>
  );
}

export function PageHeader({
  title,
  tabs,
  children,
}: {
  title: React.ReactNode;
  tabs?: { to: string; label: string; end?: boolean }[];
  children?: React.ReactNode;
}) {
  const { search } = useLocation();
  // Keep the time range when switching tabs.
  const keep = new URLSearchParams(search);
  for (const k of [...keep.keys()])
    if (!["range", "from_ts", "to_ts", "live"].includes(k)) keep.delete(k);
  const qs = keep.toString() ? `?${keep}` : "";
  return (
    <header className="page-header">
      <div className="page-title">{title}</div>
      {tabs && (
        <div className="page-tabs">
          {tabs.map((t) => (
            <NavLink key={t.to} to={t.to + qs} end={t.end}>
              {t.label}
            </NavLink>
          ))}
        </div>
      )}
      <div className="spacer" />
      {children}
    </header>
  );
}
