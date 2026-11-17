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
        { label: "Alerts", to: "/watchdog", hint: "Anomalies detected automatically" },
        { label: "Insights" },
        { label: "Root Cause Analysis" },
        { label: "Impact Analysis" },
      ],
    },
    {
      id: "service-mgmt",
      label: "Service Mgmt",
      icon: <ServiceMgmtIcon />,
      to: "/slo/manage",
      match: ["/slo", "/event"],
      links: [
        { label: "SLOs", to: "/slo/manage", hint: "Service level objectives and error budgets" },
        { label: "Incidents", hint: "Declare and manage incidents" },
        { label: "On-Call", hint: "Schedules, escalations and pages" },
        { label: "Case Management" },
        { label: "Event Management", to: "/event/explorer", hint: "Monitor alerts, Agent and API events" },
        { label: "Workflow Automation" },
        { label: "App Builder" },
      ],
      groups: [
        {
          title: "Settings",
          links: [{ label: "Incident Settings" }, { label: "Status Pages" }],
        },
      ],
    },
    {
      id: "dashboards",
      label: "Dashboards",
      icon: <DashboardIcon />,
      to: "/dashboard/lists",
      match: ["/dashboard", "/notebook"],
      links: [
        {
          label: "Dashboard List",
          to: "/dashboard/lists",
          hint: "Browse, create and import dashboards",
        },
        { label: "Notebooks", to: "/notebook/list", hint: "Text and graphs, for investigations" },
        { label: "Reports" },
        { label: "Sheets" },
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
        { label: "Downtimes", to: "/monitors/downtimes", hint: "Scheduled silences" },
        { label: "Monitor Quality" },
        { label: "Templates" },
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
      match: ["/infrastructure", "/containers", "/orchestration"],
      links: [
        { label: "Hosts", to: "/infrastructure", hint: "Every host an Agent reports from" },
        { label: "Containers", to: "/containers", hint: "CPU, memory and restarts per container" },
        { label: "Kubernetes", to: "/orchestration/overview/pod", hint: "Pods, deployments, nodes, namespaces" },
        { label: "Processes" },
        { label: "Serverless" },
        { label: "Cloud Network" },
        { label: "Network Devices" },
        { label: "Resource Catalog" },
        { label: "Cloud Cost" },
      ],
      groups: [
        {
          title: "Agents",
          links: [
            { label: "Fleet Automation" },
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
      match: ["/apm"],
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
          label: "Error Tracking",
          to: "/error-tracking",
          hint: "Errors grouped into issues",
        },
        { label: "Profiles" },
        { label: "Database Monitoring" },
        { label: "Data Streams Monitoring" },
        { label: "Data Jobs Monitoring" },
      ],
      groups: [
        {
          title: "Settings",
          links: [
            { label: "Generate Metrics", to: "/apm/generate-metrics" },
            { label: "Ingestion Control" },
            { label: "Retention Filters" },
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
        { label: "Synthetic Monitoring", to: "/synthetics/tests", hint: "HTTP tests run on a schedule" },
        { label: "Real User Monitoring", to: "/rum/performance-monitoring", hint: "Page performance, sessions and browser errors" },
        { label: "Session Replay" },
        { label: "Product Analytics" },
        { label: "Mobile App Testing" },
        { label: "Error Tracking" },
      ],
    },
    {
      id: "software-delivery",
      label: "Software Delivery",
      icon: <DeliveryIcon />,
      match: [],
      links: [
        { label: "CI Visibility", hint: "Pipelines and their failures" },
        { label: "Test Optimization" },
        { label: "Continuous Testing" },
        { label: "CD Visibility" },
        { label: "DORA Metrics" },
        { label: "Code Coverage" },
      ],
    },
    {
      id: "security",
      label: "Security",
      icon: <SecurityIcon />,
      match: [],
      links: [
        { label: "Cloud SIEM", hint: "Threat detection on logs" },
        { label: "Code Security" },
        { label: "Cloud Security" },
        { label: "App & API Protection" },
        { label: "Workload Protection" },
        { label: "Sensitive Data Scanner" },
      ],
    },
    {
      id: "llm",
      label: "LLM Observability",
      icon: <AiIcon />,
      match: [],
      links: [
        { label: "Traces", hint: "Prompts, completions and tool calls" },
        { label: "Evaluations" },
        { label: "Experiments" },
        { label: "Agent Observability" },
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
        { label: "Explorer" },
        { label: "Exception Replay" },
      ],
      groups: [
        {
          title: "Settings",
          links: [
            { label: "Issue Teams" },
            { label: "Custom Grouping" },
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
        { label: "Volume", to: "/metrics/volume", hint: "Points and series stored per metric" },
      ],
      groups: [
        {
          title: "Settings",
          links: [
            { label: "Metrics without Limits" },
            { label: "Reference Tables" },
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
            { label: "Indexes" },
            { label: "Archives" },
            { label: "Rehydrate from Archives" },
            { label: "Log Forwarding" },
            { label: "Sensitive Data Scanner" },
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
      match: ["/integrations", "/monitors/webhooks"],
      links: [
        { label: "Integrations", to: "/integrations", hint: "What sends logs and metrics here" },
        {
          label: "Webhooks",
          to: "/monitors/webhooks",
          hint: "Where monitors send their notifications",
        },
        { label: "APIs" },
        { label: "Marketplace" },
      ],
      groups: [
        {
          title: "Install",
          links: [
            { label: "Agent" },
            { label: "OpenTelemetry" },
            { label: "Fleet Automation" },
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

export function Layout({ me }: { me: Identity }) {
  const { pathname } = useLocation();
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
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setOpen(id), delay);
  };
  useEffect(() => () => window.clearTimeout(timer.current), []);
  useEffect(() => setOpen(null), [pathname]);

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
          className={`nav${collapsed ? " collapsed" : ""}`}
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
                      onFocus: () => setOpen(it.id),
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
              <span
                className="nav-link disabled"
                aria-disabled="true"
                title={UNAVAILABLE}
                onMouseEnter={() => later(null, 120)}
              >
                <HelpIcon />
                <span>Help</span>
              </span>
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
                <Entry link={{ label: "API Keys" }} onNavigate={close} />
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
