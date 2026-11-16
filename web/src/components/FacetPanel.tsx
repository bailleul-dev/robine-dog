import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { api, type FacetDef } from "../lib/api";
import { compact, duration, serviceColor, statusColor, STATUS_ORDER } from "../lib/format";
import { selectedValues, setFacetValues } from "../lib/query";
import { ChevronDown, ChevronRight, PlusIcon, TrashIcon } from "./Icons";

export type CoreFacet = { path: string; name: string; group: string; kind?: "status" | "service" | "duration" };

type Props = {
  dataset: "logs" | "spans";
  query: string;
  setQuery: (q: string) => void;
  from: number;
  to: number;
  core: CoreFacet[];
};

type Facet = CoreFacet & { id?: number };

export function FacetPanel({ dataset, query, setQuery, from, to, core }: Props) {
  const qc = useQueryClient();
  const defs = useQuery({ queryKey: ["facetDefs", dataset], queryFn: () => api.facetDefs(dataset) });
  const [filter, setFilter] = useState("");
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const [more, setMore] = useState<Record<string, boolean>>({});
  const [adding, setAdding] = useState(false);

  const facets: Facet[] = useMemo(() => {
    const user: Facet[] = (defs.data ?? []).filter((d) => !core.some((c) => c.path === d.path)).map((d: FacetDef) => ({ ...d }));
    return [...core, ...user];
  }, [defs.data, core]);

  // Core facets start expanded, the others collapsed (as in Datadog) unless they filter the query.
  const isOpen = (f: Facet) => {
    const c = collapsed[f.path];
    if (c !== undefined) return !c;
    return f.group === "Core" || f.group === "Tags" || selectedValues(query, f.path).length > 0;
  };
  const visible = facets.filter(
    (f) => !filter || f.name.toLowerCase().includes(filter.toLowerCase()) || f.path.toLowerCase().includes(filter.toLowerCase()),
  );
  const openPaths = visible.filter(isOpen).map((f) => f.path);
  const anyMore = openPaths.some((p) => more[p]);

  const values = useQuery({
    queryKey: ["facetValues", dataset, query, from, to, openPaths.join("|"), anyMore],
    queryFn: ({ signal }) => api.facetValues(dataset, { query, from, to }, openPaths, anyMore ? 50 : 10, signal),
    enabled: openPaths.length > 0,
    placeholderData: (prev) => prev,
  });
  const byPath = new Map((values.data?.facets ?? []).map((f) => [f.path, f]));

  const groups: [string, Facet[]][] = [];
  for (const f of visible) {
    const g = groups.find(([name]) => name === f.group);
    if (g) g[1].push(f);
    else groups.push([f.group, [f]]);
  }

  return (
    <aside className="facets" aria-label="Facets">
      <div className="facets-search">
        <input className="input" placeholder="Search facets" value={filter} onChange={(e) => setFilter(e.target.value)} />
      </div>
      {groups.map(([group, list]) => (
        <div key={group}>
          <div className="facet-group-title">
            {group}
            {group === groups[groups.length - 1][0] && (
              <button className="btn ghost small" style={{ marginLeft: "auto" }} onClick={() => setAdding((a) => !a)} title="Add a facet">
                <PlusIcon /> Add
              </button>
            )}
          </div>
          {list.map((f) => {
            const open = isOpen(f);
            const data = byPath.get(f.path);
            const selected = selectedValues(query, f.path);
            return (
              <div className="facet" key={f.path}>
                <button className="facet-header" onClick={() => setCollapsed((c) => ({ ...c, [f.path]: open }))} aria-expanded={open}>
                  {open ? <ChevronDown /> : <ChevronRight />}
                  {f.name}
                  <span className="facet-path">{f.path}</span>
                  {f.id !== undefined && (
                    <span
                      role="button"
                      title="Remove this facet"
                      onClick={(e) => {
                        e.stopPropagation();
                        api.deleteFacet(f.id!).then(() => qc.invalidateQueries({ queryKey: ["facetDefs", dataset] }));
                      }}
                      style={{ display: "flex", color: "var(--text-faint)" }}
                    >
                      <TrashIcon />
                    </span>
                  )}
                </button>
                {open && f.kind === "duration" && <DurationFacet query={query} setQuery={setQuery} min={data?.min} max={data?.max} />}
                {open && f.kind !== "duration" && (
                  <FacetValuesList
                    facet={f}
                    values={data?.values ?? []}
                    selected={selected}
                    loading={!data && values.isFetching}
                    showMore={!!more[f.path]}
                    onMore={() => setMore((m) => ({ ...m, [f.path]: !m[f.path] }))}
                    onToggle={(v) => {
                      const next = selected.includes(v) ? selected.filter((x) => x !== v) : [...selected, v];
                      setQuery(setFacetValues(query, f.path, next));
                    }}
                    onOnly={(v) => {
                      const only = selected.length === 1 && selected[0] === v;
                      setQuery(setFacetValues(query, f.path, only ? [] : [v]));
                    }}
                  />
                )}
              </div>
            );
          })}
        </div>
      ))}
      {adding && <AddFacet dataset={dataset} onDone={() => setAdding(false)} />}
    </aside>
  );
}

function FacetValuesList({
  facet,
  values,
  selected,
  loading,
  showMore,
  onMore,
  onToggle,
  onOnly,
}: {
  facet: Facet;
  values: { value: string; count: number }[];
  selected: string[];
  loading: boolean;
  showMore: boolean;
  onMore: () => void;
  onToggle: (v: string) => void;
  onOnly: (v: string) => void;
}) {
  let list = [...values];
  for (const s of selected) if (!list.some((v) => v.value === s)) list.push({ value: s, count: 0 });
  if (facet.kind === "status") list.sort((a, b) => STATUS_ORDER.indexOf(a.value) - STATUS_ORDER.indexOf(b.value));
  const shown = showMore ? list : list.slice(0, 10);
  if (loading) return <div className="facet-empty">Loading…</div>;
  if (!list.length) return <div className="facet-empty">No values in this time range</div>;
  return (
    <div className="facet-body">
      {shown.map((v) => {
        const checked = selected.length === 0 || selected.includes(v.value);
        return (
          <div className="facet-value" key={v.value} onClick={() => onOnly(v.value)} title={v.value}>
            <input
              type="checkbox"
              checked={checked}
              onClick={(e) => e.stopPropagation()}
              onChange={() => onToggle(v.value)}
              aria-label={`Toggle ${v.value}`}
            />
            {facet.kind === "status" && <span className="status-dot" style={{ background: statusColor(v.value) }} />}
            {facet.kind === "service" && <span className="status-dot" style={{ background: serviceColor(v.value) }} />}
            <span className="facet-value-label">{v.value || <span className="faint">(empty)</span>}</span>
            <span className="facet-value-count">{compact(v.count)}</span>
            <button
              className="facet-only"
              onClick={(e) => {
                e.stopPropagation();
                onOnly(v.value);
              }}
            >
              only
            </button>
          </div>
        );
      })}
      {list.length >= 10 && (
        <button className="facet-more" onClick={onMore}>
          {showMore ? "Show less" : "Show more"}
        </button>
      )}
    </div>
  );
}

function DurationFacet({ query, setQuery, min, max }: { query: string; setQuery: (q: string) => void; min?: number | null; max?: number | null }) {
  const [lo, setLo] = useState("");
  const [hi, setHi] = useState("");
  const apply = () => {
    const base = setFacetValues(query, "@duration", []);
    const a = lo.trim() || "0ns";
    const b = hi.trim() || "1000h";
    const clause = lo.trim() || hi.trim() ? `@duration:[${a} TO ${b}]` : "";
    setQuery([base, clause].filter(Boolean).join(" "));
  };
  return (
    <div className="facet-body">
      {min != null && max != null && (
        <div className="facet-empty">
          {duration(min)} – {duration(max)}
        </div>
      )}
      <div className="facet-range">
        <input className="input" placeholder="min (5ms)" value={lo} onChange={(e) => setLo(e.target.value)} onKeyDown={(e) => e.key === "Enter" && apply()} />
        <span className="faint">–</span>
        <input className="input" placeholder="max (2s)" value={hi} onChange={(e) => setHi(e.target.value)} onKeyDown={(e) => e.key === "Enter" && apply()} />
      </div>
    </div>
  );
}

export function AddFacet({ dataset, onDone, initialPath = "" }: { dataset: "logs" | "spans"; onDone: () => void; initialPath?: string }) {
  const qc = useQueryClient();
  const [path, setPath] = useState(initialPath);
  const [name, setName] = useState(() => defaultName(initialPath));
  const [group, setGroup] = useState("Attributes");
  const submit = async () => {
    if (!path.trim()) return;
    await api.addFacet({ dataset, path: path.trim(), name: name.trim() || defaultName(path), group: group.trim() || "Attributes" });
    await qc.invalidateQueries({ queryKey: ["facetDefs", dataset] });
    onDone();
  };
  return (
    <div className="panel-section" style={{ display: "grid", gap: 6, borderBottom: 0 }}>
      <input className="input mono" placeholder="@http.status_code or env" value={path} onChange={(e) => { setPath(e.target.value); setName(defaultName(e.target.value)); }} autoFocus />
      <input className="input" placeholder="Display name" value={name} onChange={(e) => setName(e.target.value)} />
      <input className="input" placeholder="Group" value={group} onChange={(e) => setGroup(e.target.value)} />
      <div style={{ display: "flex", gap: 6 }}>
        <button className="btn primary small" onClick={submit}>Add facet</button>
        <button className="btn small" onClick={onDone}>Cancel</button>
      </div>
    </div>
  );
}

export function defaultName(path: string): string {
  const last = path.replace(/^@/, "").split(".").pop() ?? path;
  return last.replace(/_/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}
