import { SavedViews } from "../components/SavedViews";
import {
  keepPreviousData,
  useInfiniteQuery,
  useQuery,
} from "@tanstack/react-query";
import { useCallback, useMemo, useRef } from "react";
import { useSearchParams } from "react-router-dom";
import { AnalyticsView, useViz, VizPicker } from "../components/Analytics";
import { TimeChart } from "../components/Charts";
import { HttpCode, ServiceChip } from "../components/Common";
import { FacetPanel, type CoreFacet } from "../components/FacetPanel";
import { PageHeader } from "../components/Layout";
import { SearchBar, type KeyHint } from "../components/SearchBar";
import { TimePicker } from "../components/TimePicker";
import { api, ApiError } from "../lib/api";
import { duration, logDate, thousands } from "../lib/format";
import { addClause } from "../lib/query";
import { useQueryParam, useTimeRange } from "../lib/time";
import { TracePanel } from "./TracePanel";

export const SPAN_CORE_FACETS: CoreFacet[] = [
  { path: "env", name: "Env", group: "Core" },
  { path: "service", name: "Service", group: "Core", kind: "service" },
  { path: "resource_name", name: "Resource", group: "Core" },
  { path: "operation_name", name: "Operation Name", group: "Core" },
  { path: "status", name: "Status", group: "Core", kind: "status" },
  { path: "@duration", name: "Duration", group: "Core", kind: "duration" },
  { path: "type", name: "Span Type", group: "Core" },
];

export const APM_TABS = [
  { to: "/apm/services", label: "Service Catalog" },
  { to: "/apm/traces", label: "Traces" },
  { to: "/apm/map", label: "Service Map" },
  { to: "/apm/generate-metrics", label: "Generate Metrics" },
];

const SPAN_SCOPES = [
  { key: "entry", label: "Service Entry Spans", clause: "_entry:1" },
  { key: "root", label: "Root Spans", clause: "_root:1" },
  { key: "all", label: "All Spans", clause: "" },
];

export function TraceExplorer() {
  const time = useTimeRange("15m");
  const [query, setQuery] = useQueryParam("query");
  const [params, setParams] = useSearchParams();
  const scopeKey = params.get("spans") ?? "entry";
  const sort = params.get("sort") === "duration" ? "duration" : "start";
  const clause = SPAN_SCOPES.find((s) => s.key === scopeKey)?.clause ?? "";
  // Looking up one trace by id searches every span.
  const effective = /trace_id:/.test(query)
    ? query
    : [query, clause].filter(Boolean).join(" ");
  const scope = { query: effective, from: time.from, to: time.to };
  const traceId = params.get("trace");
  const viz = useViz("spans");
  const listing = viz.viz === "list";

  const setParam = useCallback(
    (k: string, v: string | null) =>
      setParams((prev) => {
        const p = new URLSearchParams(prev);
        if (v === null) p.delete(k);
        else p.set(k, v);
        return p;
      }),
    [setParams],
  );

  const histogram = useQuery({
    queryKey: ["spansHistogram", scope],
    queryFn: ({ signal }) => api.spansHistogram(scope, signal),
    enabled: listing,
    placeholderData: keepPreviousData,
  });

  const list = useInfiniteQuery({
    queryKey: ["spans", scope, sort],
    queryFn: ({ pageParam, signal }) =>
      api.searchSpans(
        { ...scope, limit: 100, cursor: pageParam ?? undefined, sort },
        signal,
      ),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.next_cursor,
    placeholderData: keepPreviousData,
  });
  const spans = useMemo(
    () => list.data?.pages.flatMap((p) => p.spans) ?? [],
    [list.data],
  );
  const total = list.data?.pages[0]?.total ?? null;
  const error =
    list.error instanceof ApiError && list.error.status === 400
      ? list.error.message
      : null;

  const defs = useQuery({
    queryKey: ["facetDefs", "spans"],
    queryFn: () => api.facetDefs("spans"),
  });
  const keys: KeyHint[] = useMemo(() => {
    const out: KeyHint[] = [
      ...SPAN_CORE_FACETS.map((f) => ({ path: f.path, name: f.name })),
      { path: "trace_id", name: "Trace ID" },
      { path: "version", name: "Version" },
    ];
    for (const d of defs.data ?? [])
      if (!out.some((k) => k.path === d.path))
        out.push({ path: d.path, name: d.name });
    const seen = new Set<string>();
    for (const s of spans.slice(0, 200)) {
      for (const k of Object.keys(s.meta))
        if (!k.startsWith("_")) seen.add(`@${k}`);
      for (const k of Object.keys(s.metrics))
        if (!k.startsWith("_")) seen.add(`@${k}`);
    }
    for (const p of [...seen].sort())
      if (!out.some((k) => k.path === p))
        out.push({ path: p, name: "attribute" });
    return out;
  }, [defs.data, spans]);

  const wrap = useRef<HTMLDivElement>(null);
  const onScroll = () => {
    const el = wrap.current;
    if (!el || !list.hasNextPage || list.isFetchingNextPage) return;
    if (el.scrollTop + el.clientHeight > el.scrollHeight - 400)
      list.fetchNextPage();
  };

  const selectedSpan = params.get("span");

  return (
    <>
      <PageHeader title="APM" tabs={APM_TABS}>
        <TimePicker time={time} />
      </PageHeader>
      <div className="searchbar-row">
        <SavedViews page="traces" />
        <SearchBar
          value={query}
          onSubmit={setQuery}
          dataset="spans"
          scope={time}
          keys={keys}
          error={error}
          placeholder={
            'Search spans: service:shop-api resource_name:"POST /api/orders" @duration:>500ms status:error'
          }
        />
        <select
          className="input"
          value={scopeKey}
          onChange={(e) => setParam("spans", e.target.value)}
          style={{ height: 32 }}
        >
          {SPAN_SCOPES.map((s) => (
            <option key={s.key} value={s.key}>
              {s.label}
            </option>
          ))}
        </select>
      </div>
      <div className="explorer">
        <FacetPanel
          dataset="spans"
          query={effective}
          setQuery={(q) =>
            setQuery(q.replace(clause, "").replace(/\s+/g, " ").trim())
          }
          from={time.from}
          to={time.to}
          core={SPAN_CORE_FACETS}
        />
        <div className="content">
          {listing && (
            <div className="histogram">
              {histogram.data ? (
                <TimeChart
                  times={histogram.data.buckets.map((b) => b.t)}
                  interval={histogram.data.interval_ms}
                  from={time.from}
                  to={time.to}
                  height={92}
                  series={[
                    {
                      key: "ok",
                      label: "Requests",
                      color: "#8b6fd0",
                      values: histogram.data.buckets.map(
                        (b) => b.counts.ok ?? 0,
                      ),
                    },
                    {
                      key: "error",
                      label: "Errors",
                      color: "#e74c50",
                      values: histogram.data.buckets.map(
                        (b) => b.counts.error ?? 0,
                      ),
                    },
                  ]}
                  onZoom={time.setAbsolute}
                />
              ) : (
                <div style={{ height: 92 }} />
              )}
            </div>
          )}
          <div className="toolbar">
            <VizPicker
              viz={viz.viz}
              options={["list", "timeseries", "toplist", "table"]}
              onChange={(v) => viz.set({ viz: v === "list" ? null : v })}
            />
            {!listing ? null : total !== null ? (
              <span>
                <strong>{thousands(total)}</strong> spans found
              </span>
            ) : (
              <span className="faint">Searching…</span>
            )}
            {listing && list.isFetching && <span className="spinner" />}
            <div className="spacer" />
            {listing && (
              <div className="btn-group">
                <button
                  className={`btn small${sort === "start" ? " selected" : ""}`}
                  onClick={() => setParam("sort", null)}
                >
                  Newest
                </button>
                <button
                  className={`btn small${sort === "duration" ? " selected" : ""}`}
                  onClick={() => setParam("sort", "duration")}
                >
                  Slowest
                </button>
              </div>
            )}
          </div>
          {!listing && (
            <AnalyticsView
              dataset="spans"
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
                  <col />
                  <col style={{ width: 100 }} />
                  <col style={{ width: 80 }} />
                  <col style={{ width: 100 }} />
                </colgroup>
                <thead>
                  <tr>
                    <th />
                    <th>Date</th>
                    <th>Service</th>
                    <th>Resource</th>
                    <th
                      className="num sortable"
                      onClick={() =>
                        setParam(
                          "sort",
                          sort === "duration" ? null : "duration",
                        )
                      }
                    >
                      Duration {sort === "duration" ? "↓" : ""}
                    </th>
                    <th>Method</th>
                    <th>Status Code</th>
                  </tr>
                </thead>
                <tbody>
                  {spans.map((s) => (
                    <tr
                      key={s.id}
                      className={
                        traceId === s.trace_id && selectedSpan === s.span_id
                          ? "selected"
                          : undefined
                      }
                      onClick={() =>
                        setParams((prev) => {
                          const p = new URLSearchParams(prev);
                          p.set("trace", s.trace_id);
                          p.set("span", s.span_id);
                          return p;
                        })
                      }
                    >
                      <td className="status-cell">
                        <span
                          className="status-bar"
                          style={{
                            background: s.error ? "#e74c50" : "transparent",
                          }}
                        />
                      </td>
                      <td className="date">{logDate(s.start / 1e6)}</td>
                      <td>
                        <ServiceChip name={s.service} />
                      </td>
                      <td title={s.resource}>
                        {s.resource} <span className="faint">{s.name}</span>
                      </td>
                      <td className="num mono">{duration(s.duration)}</td>
                      <td className="mono">
                        {s.meta["http.method"] ?? (
                          <span className="faint">—</span>
                        )}
                      </td>
                      <td>
                        <HttpCode code={s.meta["http.status_code"]} />
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              {!list.isLoading && spans.length === 0 && (
                <div className="empty-state">
                  <h3>No spans match this query</h3>
                  Point a ddtrace-instrumented app at this server's trace port
                  (8126), or widen the time range.
                </div>
              )}
              {list.isFetchingNextPage && (
                <div className="list-footer">Loading more…</div>
              )}
            </div>
          )}
        </div>
      </div>
      {traceId && (
        <TracePanel
          traceId={traceId}
          spanId={selectedSpan}
          onSelectSpan={(id) => setParam("span", id)}
          onClose={() =>
            setParams((prev) => {
              const p = new URLSearchParams(prev);
              p.delete("trace");
              p.delete("span");
              return p;
            })
          }
        />
      )}
    </>
  );
}
