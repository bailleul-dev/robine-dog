import { NavLink, Outlet, useLocation } from "react-router-dom";
import type { Identity } from "../lib/api";
import {
  ApmIcon,
  BellIcon,
  BugIcon,
  DashboardIcon,
  Logo,
  LogsIcon,
  MetricsIcon,
  SignOutIcon,
} from "./Icons";

type Item = {
  to: string;
  match: string;
  label: string;
  icon: React.ReactNode;
  title: string;
  links: { to: string; label: string }[];
};

const ITEMS: Item[] = [
  {
    to: "/logs",
    match: "/logs",
    label: "Logs",
    icon: <LogsIcon />,
    title: "Logs",
    links: [
      { to: "/logs", label: "Explorer" },
      { to: "/logs/livetail", label: "Live Tail" },
      { to: "/logs/pipelines", label: "Pipelines" },
      { to: "/logs/generate-metrics", label: "Generate Metrics" },
    ],
  },
  {
    to: "/apm/services",
    match: "/apm",
    label: "APM",
    icon: <ApmIcon />,
    title: "APM",
    links: [
      { to: "/apm/services", label: "Service Catalog" },
      { to: "/apm/traces", label: "Traces" },
      { to: "/apm/map", label: "Service Map" },
      { to: "/apm/generate-metrics", label: "Generate Metrics" },
    ],
  },
  {
    to: "/metrics/explorer",
    match: "/metrics",
    label: "Metrics",
    icon: <MetricsIcon />,
    title: "Metrics",
    links: [
      { to: "/metrics/explorer", label: "Explorer" },
      { to: "/metrics/summary", label: "Summary" },
    ],
  },
  {
    to: "/dashboard/lists",
    match: "/dashboard",
    label: "Dashboards",
    icon: <DashboardIcon />,
    title: "Dashboards",
    links: [{ to: "/dashboard/lists", label: "Dashboard List" }],
  },
  {
    to: "/error-tracking",
    match: "/error-tracking",
    label: "Errors",
    icon: <BugIcon />,
    title: "Error Tracking",
    links: [{ to: "/error-tracking", label: "Issues" }],
  },
  {
    to: "/monitors/manage",
    match: "/monitors",
    label: "Monitors",
    icon: <BellIcon />,
    title: "Monitors",
    links: [
      { to: "/monitors/manage", label: "Manage Monitors" },
      { to: "/monitors/create", label: "New Monitor" },
      { to: "/monitors/triggered", label: "Triggered Monitors" },
      { to: "/monitors/webhooks", label: "Webhooks" },
    ],
  },
];

function initials(id: Identity): string {
  const n = id.name ?? id.email ?? "?";
  return n
    .split(/[\s@.]+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((s) => s[0]?.toUpperCase())
    .join("");
}

export function Layout({ me }: { me: Identity }) {
  const { pathname } = useLocation();
  return (
    <div className="app">
      <nav className="nav" aria-label="Main">
        <NavLink to="/logs" className="nav-logo" aria-label="Home">
          <Logo />
        </NavLink>
        <div className="nav-items">
          {ITEMS.map((it) => (
            <div className="nav-item" key={it.to}>
              <NavLink
                to={it.to}
                className={`nav-link${pathname.startsWith(it.match) ? " active" : ""}`}
              >
                {it.icon}
                <span>{it.label}</span>
              </NavLink>
              <div className="nav-flyout">
                <div className="nav-flyout-title">{it.title}</div>
                {it.links.map((l) => (
                  <NavLink key={l.to} to={l.to} end>
                    {l.label}
                  </NavLink>
                ))}
              </div>
            </div>
          ))}
        </div>
        <div className="nav-bottom">
          <div className="nav-item">
            <div className="nav-link">
              <div className="nav-avatar" title={me.email ?? me.sub}>
                {initials(me)}
              </div>
            </div>
            <div className="nav-flyout" style={{ top: "auto", bottom: 0 }}>
              <div className="nav-flyout-title">{me.name ?? "Signed in"}</div>
              {me.email && (
                <div style={{ padding: "0 16px 8px", fontSize: 12 }}>
                  {me.email}
                </div>
              )}
              <a
                href="/auth/logout"
                style={{ display: "flex", gap: 8, alignItems: "center" }}
              >
                <SignOutIcon width={14} height={14} /> Sign out
              </a>
            </div>
          </div>
        </div>
      </nav>
      <main className="main">
        <Outlet />
      </main>
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
