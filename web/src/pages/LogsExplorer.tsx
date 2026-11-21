import { SavedViews } from "../components/SavedViews";
import {
  keepPreviousData,
  useInfiniteQuery,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import {
  AnalyticsView,
  PatternsView,
  useViz,
  VizPicker,
} from "../components/Analytics";
import { TimeChart } from "../components/Charts";
import {
  ServiceChip,
  SidePanel,
  StatusPill,
  ValueMenu,
  type ValueAction,
} from "../components/Common";
import { AddFacet, FacetPanel, type CoreFacet } from "../components/FacetPanel";
import { ChevronDown, ChevronUp, TraceIcon } from "../components/Icons";
import { JsonTree } from "../components/JsonTree";
import { PageHeader } from "../components/Layout";
import { SearchBar, type KeyHint } from "../components/SearchBar";
import { TimePicker } from "../components/TimePicker";
import { api, ApiError, type Log } from "../lib/api";
import {
  fullDate,
  logDate,
  statusColor,
  STATUS_ORDER,
  thousands,
} from "../lib/format";
import { addClause } from "../lib/query";
import { useQueryParam, useTimeRange } from "../lib/time";

export const LOG_CORE_FACETS: CoreFacet[] = [
  { path: "source", name: "Source", group: "Core" },
  { path: "host", name: "Host", group: "Core" },
  { path: "service", name: "Service", group: "Core", kind: "service" },
  { path: "status", name: "Status", group: "Core", kind: "status" },
];

export const LOG_TABS = [
  { to: "/logs", label: "Explorer", end: true },
  { to: "/logs/livetail", label: "Live Tail" },
  { to: "/logs/pipelines", label: "Pipelines" },
  { to: "/logs/generate-metrics", label: "Generate Metrics" },
  { to: "/logs/indexes", label: "Indexes" },
  { to: "/logs/archives", label: "Archives" },
  { to: "/logs/sensitive-data-scanner", label: "Sensitive Data" },
  { to: "/logs/reference-tables", label: "Reference Tables" },
];

/** Every leaf path of an attributes object, as `@a.b.c`. */
export function attributePaths(
  obj: unknown,
  prefix: string[] = [],
  out = new Set<string>(),
): Set<string> {
  if (obj && typeof obj === "object" && !Array.isArray(obj)) {
    for (const [k, v] of Object.entries(obj))
      attributePaths(v, [...prefix, k], out);
  } else if (prefix.length) {
    out.add(`@${prefix.join(".")}`);
  }
  return out;
}

export function logContent(l: Log): string {
  if (l.message) return l.message;
  const a = { ...l.attributes };
  return JSON.stringify(a);
}

export function LogsExplorer() {
  const time = useTimeRange("15m");
  const [query, setQuery] = useQueryParam("query");
  const [params, setParams] = useSearchParams();
  const selectedId = Number(params.get("event")) || null;
  const scope = { query, from: time.from, to: time.to };
  const viz = useViz("logs");
  const listing = viz.viz === "list";

  const histogram = useQuery({
    queryKey: ["logsHistogram", scope],
    queryFn: ({ signal }) => api.logsHistogram(scope, signal),
    enabled: listing,
    placeholderData: keepPreviousData,
  });

  const list = useInfiniteQuery({
    queryKey: ["logs", scope],
    queryFn: ({ pageParam, signal }) =>
      api.searchLogs(
        { ...scope, limit: 100, cursor: pageParam ?? undefined },
        signal,
      ),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.next_cursor,
    placeholderData: keepPreviousData,
  });

  const logs = useMemo(
    () => list.data?.pages.flatMap((p) => p.logs) ?? [],
    [list.data],
  );
  const total = list.data?.pages[0]?.total ?? null;
  const error =
    list.error instanceof ApiError && list.error.status === 400
      ? list.error.message
      : null;

  const defs = useQuery({
    queryKey: ["facetDefs", "logs"],
    queryFn: () => api.facetDefs("logs"),
  });
  const keys: KeyHint[] = useMemo(() => {
    const out: KeyHint[] = [
      ...LOG_CORE_FACETS.map((f) => ({ path: f.path, name: f.name })),
      { path: "trace_id", name: "Trace ID" },
    ];
    for (const d of defs.data ?? [])
      if (!out.some((k) => k.path === d.path))
        out.push({ path: d.path, name: d.name });
    const seen = new Set<string>();
    for (const l of logs.slice(0, 200)) attributePaths(l.attributes, [], seen);
    for (const t of logs.slice(0, 200).flatMap((l) => l.tags))
      seen.add(t.split(":")[0]);
    for (const p of [...seen].sort())
      if (!out.some((k) => k.path === p))
        out.push({ path: p, name: p.startsWith("@") ? "attribute" : "tag" });
    return out;
  }, [defs.data, logs]);

  const select = useCallback(
    (id: number | null) =>
      setParams((prev) => {
        const p = new URLSearchParams(prev);
        if (id) p.set("event", String(id));
        else p.delete("event");
        return p;
      }),
    [setParams],
  );

  const wrap = useRef<HTMLDivElement>(null);
  const onScroll = () => {
    const el = wrap.current;
    if (!el || !list.hasNextPage || list.isFetchingNextPage) return;
    if (el.scrollTop + el.clientHeight > el.scrollHeight - 400)
      list.fetchNextPage();
  };

  const index = logs.findIndex((l) => l.id === selectedId);
  const selected = index >= 0 ? logs[index] : null;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!selectedId || e.target instanceof HTMLInputElement) return;
      if (e.key === "ArrowDown" || e.key === "j") {
        e.preventDefault();
        if (index >= 0 && index < logs.length - 1) select(logs[index + 1].id);
      } else if (e.key === "ArrowUp" || e.key === "k") {
        e.preventDefault();
        if (index > 0) select(logs[index - 1].id);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selectedId, index, logs, select]);

  const statuses = useMemo(() => {
    const present = new Set<string>();
    for (const b of histogram.data?.buckets ?? [])
      for (const k of Object.keys(b.counts)) present.add(k);
    return STATUS_ORDER.filter((s) => present.has(s));
  }, [histogram.data]);

  return (
    <>
      <PageHeader title="Logs" tabs={LOG_TABS}>
        <TimePicker time={time} />
      </PageHeader>
      <div className="searchbar-row">
        <SavedViews page="logs" />
        <SearchBar
          value={query}
          onSubmit={setQuery}
          dataset="logs"
          scope={time}
          keys={keys}
          error={error}
          placeholder="Search for logs: service:web-store status:error @http.status_code:>=500"
        />
      </div>
      <div className="explorer">
        <FacetPanel
          dataset="logs"
          query={query}
          setQuery={setQuery}
          from={time.from}
          to={time.to}
          core={LOG_CORE_FACETS}
        />
        <div className="content">
          {listing && (
            <div className="histogram">
              {histogram.data && (
                <TimeChart
                  times={histogram.data.buckets.map((b) => b.t)}
                  interval={histogram.data.interval_ms}
                  from={time.from}
                  to={time.to}
                  height={92}
                  series={statuses.map((s) => ({
                    key: s,
                    color: statusColor(s),
                    values: histogram.data!.buckets.map(
                      (b) => b.counts[s] ?? 0,
                    ),
                  }))}
                  onZoom={time.setAbsolute}
                />
              )}
              {!histogram.data && <div style={{ height: 92 }} />}
            </div>
          )}
          <div className="toolbar">
            <VizPicker
              viz={viz.viz}
              options={["list", "patterns", "timeseries", "toplist", "table"]}
              onChange={(v) => viz.set({ viz: v === "list" ? null : v })}
            />
            {!listing ? null : total !== null ? (
              <span>
                <strong>{thousands(total)}</strong> logs found
              </span>
            ) : (
              <span className="faint">Searching…</span>
            )}
            {list.isFetching && <span className="spinner" />}
            <div className="spacer" />
            {listing && (
              <span className="faint">
                Click a log for details · ↑/↓ to move
              </span>
            )}
          </div>
          {viz.viz === "patterns" && (
            <PatternsView
              scope={scope}
              onOpen={(q) => {
                setQuery(q);
                viz.set({ viz: null });
              }}
            />
          )}
          {!listing && viz.viz !== "patterns" && (
            <AnalyticsView
              dataset="logs"
              scope={scope}
              keys={keys}
              v={viz}
              onZoom={time.setAbsolute}
              onFilter={(f, val) => setQuery(addClause(query, f, val))}
            />
          )}
          {listing && (
            <div className="table-wrap" ref={wrap} onScroll={onScroll}>
              <table className="list">
                <colgroup>
                  <col style={{ width: 6 }} />
                  <col style={{ width: 172 }} />
                  <col style={{ width: 170 }} />
                  <col style={{ width: 150 }} />
                  <col />
                </colgroup>
                <thead>
                  <tr>
                    <th />
                    <th>Date</th>
                    <th>Host</th>
                    <th>Service</th>
                    <th>Content</th>
                  </tr>
                </thead>
                <tbody>
                  {logs.map((l) => (
                    <tr
                      key={l.id}
                      className={l.id === selectedId ? "selected" : undefined}
                      onClick={() => select(l.id)}
                    >
                      <td className="status-cell">
                        <span
                          className="status-bar"
                          style={{ background: statusColor(l.status) }}
                        />
                      </td>
                      <td className="date">{logDate(l.timestamp)}</td>
                      <td>{l.host ?? <span className="faint">—</span>}</td>
                      <td>
                        <ServiceChip name={l.service} />
                      </td>
                      <td className="content-cell" title={logContent(l)}>
                        {logContent(l)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              {!list.isLoading && logs.length === 0 && (
                <div className="empty-state">
                  <h3>No logs match this query</h3>
                  Widen the time range, or remove a filter.
                </div>
              )}
              {list.isFetchingNextPage && (
                <div className="list-footer">Loading more…</div>
              )}
              {!list.hasNextPage && logs.length > 0 && (
                <div className="list-footer">End of results</div>
              )}
            </div>
          )}
        </div>
      </div>
      {selectedId && (
        <LogPanel
          id={selectedId}
          initial={selected}
          query={query}
          setQuery={setQuery}
          onClose={() => select(null)}
          onPrev={index > 0 ? () => select(logs[index - 1].id) : undefined}
          onNext={
            index >= 0 && index < logs.length - 1
              ? () => select(logs[index + 1].id)
              : undefined
          }
        />
      )}
    </>
  );
}

export function LogPanel({
  id,
  initial,
  query,
  setQuery,
  onClose,
  onPrev,
  onNext,
}: {
  id: number;
  initial: Log | null;
  query: string;
  setQuery: (q: string) => void;
  onClose: () => void;
  onPrev?: () => void;
  onNext?: () => void;
}) {
  const fetched = useQuery({
    queryKey: ["log", id],
    queryFn: () => api.log(id),
    enabled: !initial,
  });
  const log = initial ?? fetched.data;
  const navigate = useNavigate();
  const qc = useQueryClient();
  const [menu, setMenu] = useState<{
    at: { x: number; y: number };
    action: ValueAction;
  } | null>(null);
  const [facetPath, setFacetPath] = useState<string | null>(null);
  const defs = useQuery({
    queryKey: ["facetDefs", "logs"],
    queryFn: () => api.facetDefs("logs"),
  });
  const facetSet = new Set((defs.data ?? []).map((d) => d.path));

  const filter = (a: ValueAction) =>
    setQuery(addClause(query, a.path, a.value));
  const exclude = (a: ValueAction) =>
    setQuery(addClause(query, a.path, a.value, true));

  const openTrace = () => {
    if (!log?.trace_id) return;
    const from = log.timestamp - 15 * 60_000;
    const to = log.timestamp + 15 * 60_000;
    navigate(
      `/apm/traces?query=${encodeURIComponent(`trace_id:${log.trace_id}`)}&from_ts=${from}&to_ts=${to}&live=false&trace=${log.trace_id}${log.span_id ? `&span=${log.span_id}` : ""}`,
    );
  };

  const field = (path: string, value: string | null) =>
    value ? (
      <span
        className="json-value"
        onClick={(e) =>
          setMenu({
            at: { x: e.clientX, y: e.clientY },
            action: { path, value },
          })
        }
      >
        {value}
      </span>
    ) : (
      <span className="faint">—</span>
    );

  return (
    <SidePanel
      onClose={onClose}
      storageKey="log"
      header={
        log ? (
          <>
            <StatusPill status={log.status} />
            <ServiceChip name={log.service} />
            <span className="muted">{fullDate(log.timestamp)}</span>
            <div className="btn-group" style={{ marginLeft: 8 }}>
              <button
                className="btn icon"
                disabled={!onPrev}
                onClick={onPrev}
                title="Previous log (↑)"
              >
                <ChevronUp />
              </button>
              <button
                className="btn icon"
                disabled={!onNext}
                onClick={onNext}
                title="Next log (↓)"
              >
                <ChevronDown />
              </button>
            </div>
          </>
        ) : (
          <span className="faint">Loading…</span>
        )
      }
    >
      {log && (
        <>
          <div className="panel-section">
            <dl className="kv-grid">
              <dt>Host</dt>
              <dd>{field("host", log.host)}</dd>
              <dt>Service</dt>
              <dd>{field("service", log.service)}</dd>
              <dt>Source</dt>
              <dd>{field("source", log.source)}</dd>
              <dt>Status</dt>
              <dd>{field("status", log.status)}</dd>
              <dt>Trace ID</dt>
              <dd className="mono">{field("trace_id", log.trace_id)}</dd>
            </dl>
            {log.trace_id && (
              <button
                className="btn"
                style={{ marginTop: 12 }}
                onClick={openTrace}
              >
                <TraceIcon /> View Trace
              </button>
            )}
          </div>
          <div className="panel-section">
            <h4>Message</h4>
            <div className="message-box">
              {log.message || <span className="faint">(no message)</span>}
            </div>
          </div>
          <div className="panel-section">
            <h4>Tags</h4>
            {log.tags.length === 0 && <span className="faint">No tags</span>}
            <div>
              {log.tags.map((t) => {
                const i = t.indexOf(":");
                const action =
                  i > 0
                    ? { path: t.slice(0, i), value: t.slice(i + 1) }
                    : { path: t, value: "" };
                return (
                  <span
                    key={t}
                    className="tag"
                    title={t}
                    onClick={(e) =>
                      setMenu({ at: { x: e.clientX, y: e.clientY }, action })
                    }
                  >
                    {t}
                  </span>
                );
              })}
            </div>
          </div>
          <div className="panel-section">
            <h4>Event Attributes</h4>
            <JsonTree
              data={log.attributes}
              facets={facetSet}
              onValue={(path, value, e) =>
                setMenu({
                  at: { x: e.clientX, y: e.clientY },
                  action: { path: `@${path.join(".")}`, value },
                })
              }
            />
          </div>
          {facetPath && (
            <div className="panel-section">
              <h4>New facet</h4>
              <AddFacet
                dataset="logs"
                initialPath={facetPath}
                onDone={() => {
                  setFacetPath(null);
                  qc.invalidateQueries({ queryKey: ["facetValues"] });
                }}
              />
            </div>
          )}
        </>
      )}
      {menu && (
        <ValueMenu
          at={menu.at}
          action={menu.action}
          onClose={() => setMenu(null)}
          onFilter={filter}
          onExclude={exclude}
          onAddFacet={
            menu.action.path.startsWith("@") && !facetSet.has(menu.action.path)
              ? setFacetPath
              : undefined
          }
        />
      )}
    </SidePanel>
  );
}
