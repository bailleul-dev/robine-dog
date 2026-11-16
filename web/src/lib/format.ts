const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const pad = (n: number, w = 2) => String(n).padStart(w, "0");

/** `Nov 16 14:21:28.200` — the log list's date column. */
export function logDate(ms: number): string {
  const d = new Date(ms);
  return `${MONTHS[d.getMonth()]} ${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}`;
}

export function fullDate(ms: number): string {
  const d = new Date(ms);
  return `${MONTHS[d.getMonth()]} ${d.getDate()}, ${d.getFullYear()} at ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}`;
}

export function axisTime(ms: number, spanMs: number): string {
  const d = new Date(ms);
  if (spanMs > 2 * 86_400_000) return `${MONTHS[d.getMonth()]} ${d.getDate()}`;
  if (spanMs > 6 * 3_600_000 && d.getHours() === 0 && d.getMinutes() === 0) return `${MONTHS[d.getMonth()]} ${d.getDate()}`;
  if (spanMs <= 5 * 60_000) return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export function rangeLabel(from: number, to: number): string {
  const f = new Date(from);
  const t = new Date(to);
  const sameDay = f.toDateString() === t.toDateString();
  const fmt = (d: Date) => `${pad(d.getHours())}:${pad(d.getMinutes())}`;
  return sameDay
    ? `${MONTHS[f.getMonth()]} ${f.getDate()}, ${fmt(f)} – ${fmt(t)}`
    : `${MONTHS[f.getMonth()]} ${f.getDate()}, ${fmt(f)} – ${MONTHS[t.getMonth()]} ${t.getDate()}, ${fmt(t)}`;
}

/** Nanoseconds → `1.24 s`, `312 ms`, `85.2 µs`. */
export function duration(ns: number): string {
  const abs = Math.abs(ns);
  if (abs >= 60e9) return `${(ns / 60e9).toFixed(abs >= 600e9 ? 0 : 1)} min`;
  if (abs >= 1e9) return `${(ns / 1e9).toFixed(abs >= 10e9 ? 1 : 2)} s`;
  if (abs >= 1e6) return `${(ns / 1e6).toFixed(abs >= 100e6 ? 0 : abs >= 10e6 ? 1 : 2)} ms`;
  if (abs >= 1e3) return `${(ns / 1e3).toFixed(abs >= 100e3 ? 0 : 1)} µs`;
  return `${Math.round(ns)} ns`;
}

export function compact(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)}B`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(n >= 1e7 ? 0 : 1)}M`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(n >= 1e4 ? 0 : 1)}K`;
  if (n > 0 && n < 10 && n % 1 !== 0) return n.toFixed(1);
  return String(Math.round(n));
}

export function thousands(n: number): string {
  return n.toLocaleString("en-US");
}

export function percent(r: number): string {
  if (r === 0) return "0%";
  if (r < 0.0001) return "<0.01%";
  return `${(r * 100).toFixed(r < 0.1 ? 2 : 1)}%`;
}

export function rate(perSecond: number): string {
  if (perSecond >= 1) return `${perSecond.toFixed(perSecond >= 100 ? 0 : 1)} req/s`;
  const perMin = perSecond * 60;
  return `${perMin.toFixed(perMin >= 10 ? 0 : 1)} req/min`;
}

export const STATUS_COLORS: Record<string, string> = {
  emergency: "#a3132a",
  alert: "#c4162a",
  critical: "#d81f36",
  error: "#e74c50",
  warn: "#f5a524",
  notice: "#3e88e6",
  info: "#3e88e6",
  debug: "#a8acb8",
  ok: "#3db67b",
};

export const STATUS_ORDER = ["emergency", "alert", "critical", "error", "warn", "notice", "info", "debug", "ok"];

export function statusColor(s: string): string {
  return STATUS_COLORS[s] ?? STATUS_COLORS.info;
}

const PALETTE = [
  "#5d8ae6", "#e2735a", "#46b39d", "#9b6fd6", "#e3a63c", "#4fb3d9", "#d65c9b", "#7fae4c",
  "#c7703f", "#5a6fd6", "#38a38a", "#c9b23a", "#8a5ac7", "#d6574f", "#4b9be0", "#a3b84a",
];

/** A stable color per service, as in flame graphs. */
export function serviceColor(name: string): string {
  let h = 2166136261;
  for (let i = 0; i < name.length; i++) {
    h ^= name.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return PALETTE[Math.abs(h) % PALETTE.length];
}

export function spanTypeIcon(t: string | null): string {
  switch (t) {
    case "web":
      return "WEB";
    case "sql":
    case "db":
    case "cassandra":
    case "mongodb":
      return "DB";
    case "redis":
    case "memcached":
    case "cache":
      return "CACHE";
    case "http":
      return "HTTP";
    case "worker":
    case "queue":
      return "WORKER";
    default:
      return (t ?? "custom").toUpperCase().slice(0, 6);
  }
}

/** `3m ago`, `2h ago`, `5d ago`. */
export function ago(ms: number, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}
