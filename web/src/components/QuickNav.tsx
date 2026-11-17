import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api";
import { SearchIcon } from "./Icons";

export type QuickNavPage = { label: string; section: string; to: string };
type Result = { kind: string; label: string; detail?: string; to: string };

const PER_KIND = 6;

/** Every word of the query is in the text; earlier and prefix matches rank first. */
function score(text: string, words: string[]): number {
  const t = text.toLowerCase();
  let s = 0;
  for (const w of words) {
    const i = t.indexOf(w);
    if (i < 0) return -1;
    s += i === 0 ? 0 : /[\s._\-/:]/.test(t[i - 1]) ? 1 : 3;
  }
  return s + t.length / 1000;
}

function best<T>(items: T[], text: (t: T) => string, words: string[], max = PER_KIND): T[] {
  return items
    .map((it) => ({ it, s: score(text(it), words) }))
    .filter((x) => x.s >= 0)
    .sort((a, b) => a.s - b.s)
    .slice(0, max)
    .map((x) => x.it);
}

/**
 * Datadog's global search (Ctrl/Cmd+K): pages, dashboards, monitors,
 * services and metrics by name, or the typed text as a log or span query.
 */
export function QuickNav({ pages, onClose }: { pages: QuickNavPage[]; onClose: () => void }) {
  const navigate = useNavigate();
  const [q, setQ] = useState("");
  const [at, setAt] = useState(0);
  const list = useRef<HTMLDivElement>(null);

  const opts = { staleTime: 60_000 };
  const dashboards = useQuery({ queryKey: ["dashboards"], queryFn: api.dashboards, ...opts });
  const monitors = useQuery({ queryKey: ["monitors"], queryFn: api.monitors, ...opts });
  const metrics = useQuery({ queryKey: ["metrics"], queryFn: api.metrics, ...opts });
  const services = useQuery({
    queryKey: ["quicknav-services"],
    queryFn: () => api.services(Date.now() - 86_400_000, Date.now()),
    ...opts,
  });

  const results = useMemo<Result[]>(() => {
    const words = q.toLowerCase().split(/\s+/).filter(Boolean);
    const out: Result[] = [];
    for (const p of best(pages, (p) => `${p.label} ${p.section}`, words, words.length ? PER_KIND : 8))
      out.push({ kind: "Pages", label: p.label, detail: p.section, to: p.to });
    for (const d of best(dashboards.data?.dashboards ?? [], (d) => d.title, words))
      out.push({ kind: "Dashboards", label: d.title, detail: d.description ?? undefined, to: `/dashboard/${d.id}` });
    for (const m of best(monitors.data ?? [], (m) => m.name, words))
      out.push({ kind: "Monitors", label: m.name, detail: m.type, to: `/monitors/${m.id}` });
    for (const s of best(services.data ?? [], (s) => s.name, words))
      out.push({ kind: "Services", label: s.name, detail: s.type, to: `/apm/services/${encodeURIComponent(s.name)}` });
    if (words.length) {
      for (const m of best(metrics.data ?? [], (m) => m.name, words))
        out.push({
          kind: "Metrics",
          label: m.name,
          detail: m.type,
          to: `/metrics/explorer?q=${encodeURIComponent(`avg:${m.name}{*}`)}`,
        });
      const query = encodeURIComponent(q.trim());
      out.push({ kind: "Search", label: `Logs matching “${q.trim()}”`, to: `/logs?query=${query}` });
      out.push({ kind: "Search", label: `Spans matching “${q.trim()}”`, to: `/apm/traces?query=${query}` });
    }
    return out;
  }, [q, pages, dashboards.data, monitors.data, services.data, metrics.data]);

  useEffect(() => setAt(0), [q]);
  useEffect(() => {
    list.current?.querySelector(".quicknav-row.selected")?.scrollIntoView({ block: "nearest" });
  }, [at]);

  const go = (r: Result | undefined) => {
    if (!r) return;
    onClose();
    navigate(r.to);
  };

  return (
    <div className="quicknav-backdrop" onMouseDown={onClose}>
      <div
        className="quicknav"
        role="dialog"
        aria-label="Search"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === "Escape") onClose();
          else if (e.key === "ArrowDown") setAt((i) => Math.min(i + 1, results.length - 1));
          else if (e.key === "ArrowUp") setAt((i) => Math.max(i - 1, 0));
          else if (e.key === "Enter") go(results[at]);
          else return;
          e.preventDefault();
        }}
      >
        <div className="quicknav-input">
          <SearchIcon />
          <input
            autoFocus
            value={q}
            onChange={(e) => setQ(e.target.value)}
            placeholder="Go to a page, dashboard, monitor, service, metric…"
            aria-label="Search"
          />
          <kbd>Esc</kbd>
        </div>
        <div className="quicknav-results" ref={list}>
          {results.length === 0 && <div className="quicknav-empty">Nothing matches</div>}
          {results.map((r, i) => (
            <div key={`${r.kind}:${r.label}:${r.to}`}>
              {r.kind !== results[i - 1]?.kind && <div className="quicknav-kind">{r.kind}</div>}
              <div
                className={`quicknav-row${i === at ? " selected" : ""}`}
                onMouseMove={() => setAt(i)}
                onClick={() => go(r)}
              >
                <span className="quicknav-label">{r.label}</span>
                {r.detail && <span className="quicknav-detail">{r.detail}</span>}
              </div>
            </div>
          ))}
        </div>
        <div className="quicknav-footer">
          <span><kbd>↑</kbd> <kbd>↓</kbd> to move</span>
          <span><kbd>Enter</kbd> to open</span>
        </div>
      </div>
    </div>
  );
}
