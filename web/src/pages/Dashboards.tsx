import {
  keepPreviousData,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  Link,
  useNavigate,
  useParams,
  useSearchParams,
} from "react-router-dom";
import { TimeChart, type Series } from "../components/Charts";
import { Modal } from "../components/Common";
import { PlusIcon, StarIcon, TrashIcon } from "../components/Icons";
import { PageHeader } from "../components/Layout";
import { TimePicker } from "../components/TimePicker";
import {
  BARE_TYPES,
  EXTRA_TYPES,
  ExtraFields,
  ExtraWidget,
  extraBlank,
  isExtra,
  QUERY_TYPES,
  registerBaseRenderer,
} from "../components/Widgets";
import {
  api,
  type ConditionalFormat,
  type Dashboard,
  type DashQuery,
  type ScalarColumn,
  type TemplatePreset,
  type TemplateVar,
  type Widget,
  type WidgetDef,
  type WidgetLayout,
  type WidgetRequest,
} from "../lib/api";
import {
  errorText,
  fmt,
  normalizeRequest,
  NOT_PUBLIC,
  PALETTE,
  runScalar,
  runTimeseries,
  type Ctx,
  type Vars,
} from "../lib/dash";
import { ago, statusColor } from "../lib/format";
import { useTimeRange, type TimeRange } from "../lib/time";

const ROW = 92;
const GAP = 12;
const slug = (t: string) =>
  t
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");

// --- widgets ---------------------------------------------------------------------

function TimeseriesWidget({
  def,
  ctx,
  height,
}: {
  def: WidgetDef;
  ctx: Ctx;
  height: number;
}) {
  const { time, vars } = ctx;
  const requests = def.requests ?? [];
  const res = useQuery({
    queryKey: [
      "widget-ts",
      JSON.stringify(requests),
      JSON.stringify(vars),
      time.from,
      time.to,
    ],
    queryFn: ({ signal }) =>
      Promise.all(
        requests.map((r) => runTimeseries(r, ctx, time.from, time.to, signal)),
      ),
    placeholderData: keepPreviousData,
    refetchInterval: 60_000,
  });
  const chart = useMemo(() => {
    const out: Series[] = [];
    let times: number[] = [];
    let interval = 60_000;
    const responses = res.data ?? [];
    const outputs = new Set(
      responses.flatMap((r, ri) =>
        r.data.attributes.series.map((s) => `${ri}-${s.query_index}`),
      ),
    );
    responses.forEach((r, ri) => {
      const a = r.data.attributes;
      times = a.times;
      interval = a.interval;
      a.series.forEach((s, i) => {
        // Like Datadog: the group when there is one, the formula's name otherwise.
        const tags = s.group_tags.join(", ");
        const label = !tags
          ? s.name
          : outputs.size > 1
            ? `${s.name} · ${tags}`
            : tags;
        const status =
          s.group_tags.length === 1 && s.group_tags[0].startsWith("status:")
            ? s.group_tags[0].slice(7)
            : null;
        const color = status
          ? statusColor(status)
          : PALETTE[out.length % PALETTE.length];
        const nan = (vs: (number | null)[]) =>
          vs.map((v) => (v === null ? NaN : v));
        out.push({
          key: `${ri}-${i}`,
          label,
          color,
          values: nan(a.values[i]),
          band: s.bounds && {
            lower: nan(s.bounds.lower),
            upper: nan(s.bounds.upper),
          },
          dim: s.outlier === false,
        });
      });
    });
    return { series: out, times, interval };
  }, [res.data]);
  if (res.error)
    return <div className="widget-error">{errorText(res.error)}</div>;
  if (!res.data) return <div className="widget-loading" />;
  if (!chart.series.length) return <div className="widget-empty">No data</div>;
  const legend = def.show_legend !== false && chart.series.length > 1;
  const kind = requests[0]?.display_type === "bars" ? "bars" : "lines";
  return (
    <>
      <TimeChart
        times={chart.times}
        interval={chart.interval}
        from={time.from}
        to={Math.max(
          time.to,
          (chart.times[chart.times.length - 1] ?? 0) + chart.interval,
        )}
        series={chart.series.slice(0, 50)}
        kind={kind}
        height={Math.max(60, height - (legend ? 26 : 0))}
        format={(v) => fmt(v)}
        onZoom={time.setAbsolute}
      />
      {legend && (
        <div className="widget-legend">
          {chart.series.slice(0, 12).map((s) => (
            <span key={s.key} title={s.label}>
              <i style={{ background: s.color }} />
              {s.label}
            </span>
          ))}
        </div>
      )}
    </>
  );
}

function useScalar(def: WidgetDef, ctx: Ctx) {
  const { time, vars } = ctx;
  const r = def.requests?.[0];
  return useQuery({
    queryKey: [
      "widget-scalar",
      JSON.stringify(r),
      JSON.stringify(vars),
      time.from,
      time.to,
    ],
    queryFn: ({ signal }) => runScalar(r!, ctx, time.from, time.to, signal),
    enabled: !!r,
    placeholderData: keepPreviousData,
    refetchInterval: 60_000,
  });
}

const PALETTES: Record<string, [string, string]> = {
  white_on_red: ["#fff", "#d8616a"],
  white_on_yellow: ["#fff", "#e5a24a"],
  white_on_green: ["#fff", "#55ab83"],
  white_on_gray: ["#fff", "#8f8794"],
  red_on_white: ["#d8616a", "transparent"],
  yellow_on_white: ["#bd8634", "transparent"],
  green_on_white: ["#55ab83", "transparent"],
  black_on_light_red: ["#2b2333", "#fde2e2"],
  black_on_light_yellow: ["#2b2333", "#fff3cd"],
  black_on_light_green: ["#2b2333", "#e3f2e9"],
};

function matches(c: ConditionalFormat, v: number): boolean {
  switch (c.comparator) {
    case ">":
      return v > c.value;
    case ">=":
      return v >= c.value;
    case "<":
      return v < c.value;
    case "<=":
      return v <= c.value;
    default:
      return v === c.value;
  }
}

function numberColumns(cols: ScalarColumn[]) {
  return cols.filter(
    (c): c is Extract<ScalarColumn, { type: "number" }> => c.type === "number",
  );
}

function QueryValueWidget({ def, ctx }: { def: WidgetDef; ctx: Ctx }) {
  const res = useScalar(def, ctx);
  if (res.error)
    return <div className="widget-error">{errorText(res.error)}</div>;
  if (!res.data) return <div className="widget-loading" />;
  const col = numberColumns(res.data.data.attributes.columns)[0];
  const values = (col?.values ?? []).filter((v): v is number => v !== null);
  // Grouped results add up (Datadog expects one group here).
  const v = values.length ? values.reduce((a, b) => a + b, 0) : null;
  const cf =
    v !== null
      ? def.conditional_formats?.find((c) => matches(c, v))
      : undefined;
  const [fg, bg] = cf
    ? (PALETTES[cf.palette] ?? ["inherit", "transparent"])
    : ["inherit", "transparent"];
  return (
    <div className="query-value" style={{ color: fg, background: bg }}>
      <span className="query-value-number">{fmt(v, def.precision ?? 2)}</span>
      {def.custom_unit && (
        <span className="query-value-unit">{def.custom_unit}</span>
      )}
    </div>
  );
}

function ToplistWidget({ def, ctx }: { def: WidgetDef; ctx: Ctx }) {
  const res = useScalar(def, ctx);
  if (res.error)
    return <div className="widget-error">{errorText(res.error)}</div>;
  if (!res.data) return <div className="widget-loading" />;
  const cols = res.data.data.attributes.columns;
  const groups = cols.filter(
    (c): c is Extract<ScalarColumn, { type: "group" }> => c.type === "group",
  );
  const num = numberColumns(cols)[0];
  const rows = (num?.values ?? [])
    .map((v, i) => ({
      label:
        groups
          .map((g) => g.values[i]?.[0] ?? "")
          .filter(Boolean)
          .join(", ") || num.name,
      v,
    }))
    .filter((r): r is { label: string; v: number } => r.v !== null)
    .sort((a, b) => b.v - a.v)
    .slice(0, 25);
  const max = Math.max(...rows.map((r) => r.v), 0) || 1;
  if (!rows.length) return <div className="widget-empty">No data</div>;
  return (
    <div className="widget-toplist">
      {rows.map((r, i) => (
        <div className="widget-toplist-row" key={r.label}>
          <span className="widget-toplist-label" title={r.label}>
            {r.label}
          </span>
          <span className="widget-toplist-bar">
            <i
              style={{
                width: `${(r.v / max) * 100}%`,
                background: PALETTE[i % PALETTE.length],
              }}
            />
          </span>
          <span className="widget-toplist-value">{fmt(r.v)}</span>
        </div>
      ))}
    </div>
  );
}

function TableWidget({ def, ctx }: { def: WidgetDef; ctx: Ctx }) {
  const res = useScalar(def, ctx);
  if (res.error)
    return <div className="widget-error">{errorText(res.error)}</div>;
  if (!res.data) return <div className="widget-loading" />;
  const cols = res.data.data.attributes.columns;
  const n = Math.max(0, ...cols.map((c) => c.values.length));
  return (
    <div className="widget-table">
      <table className="list">
        <thead>
          <tr>
            {cols.map((c) => (
              <th key={c.name} className={c.type === "number" ? "num" : ""}>
                {c.name}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {Array.from({ length: n }, (_, i) => (
            <tr key={i}>
              {cols.map((c) => (
                <td
                  key={c.name}
                  className={c.type === "number" ? "num" : "mono"}
                >
                  {c.type === "number" ? fmt(c.values[i]) : c.values[i]?.[0]}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

const NOTE_COLORS: Record<string, string> = {
  white: "var(--bg)",
  gray: "#f3efed",
  yellow: "#fff8d6",
  blue: "#eaf0f8",
  green: "#e6f3ea",
  pink: "#fde8f1",
  purple: "#f0eafa",
  red: "#fde7e7",
  orange: "#fff0e0",
  vivid_yellow: "#ffe57a",
  transparent: "transparent",
};

/** Bold, italics, code, links and headings: enough for notes. */
export function Markdown({ text }: { text: string }) {
  const inline = (s: string, key: number) => {
    const parts = s.split(
      /(\*\*[^*]+\*\*|`[^`]+`|\[[^\]]+\]\([^)]+\)|_[^_]+_)/g,
    );
    return (
      <span key={key}>
        {parts.map((p, i) => {
          if (p.startsWith("**") && p.endsWith("**"))
            return <strong key={i}>{p.slice(2, -2)}</strong>;
          if (p.startsWith("`") && p.endsWith("`"))
            return <code key={i}>{p.slice(1, -1)}</code>;
          if (p.startsWith("_") && p.endsWith("_") && p.length > 2)
            return <em key={i}>{p.slice(1, -1)}</em>;
          const link = p.match(/^\[([^\]]+)\]\(([^)]+)\)$/);
          if (link && /^(https?:\/\/|\/)/.test(link[2]))
            return (
              <a
                key={i}
                href={link[2]}
                target={link[2].startsWith("/") ? undefined : "_blank"}
                rel="noreferrer"
              >
                {link[1]}
              </a>
            );
          return p;
        })}
      </span>
    );
  };
  return (
    <>
      {text.split(/\n{2,}/).map((block, i) => {
        const h = block.match(/^(#{1,3})\s+(.*)$/);
        if (h) {
          const Tag = (["h3", "h4", "h5"] as const)[h[1].length - 1];
          return <Tag key={i}>{inline(h[2], 0)}</Tag>;
        }
        if (block.split("\n").every((l) => /^\s*[-*]\s/.test(l)))
          return (
            <ul key={i}>
              {block.split("\n").map((l, j) => (
                <li key={j}>{inline(l.replace(/^\s*[-*]\s/, ""), j)}</li>
              ))}
            </ul>
          );
        return (
          <p key={i}>
            {block
              .split("\n")
              .map((l, j) => [j > 0 && <br key={`b${j}`} />, inline(l, j)])}
          </p>
        );
      })}
    </>
  );
}

const TYPE_NAMES: Record<string, string> = {
  timeseries: "Timeseries",
  query_value: "Query Value",
  toplist: "Top List",
  query_table: "Table",
  note: "Note",
  group: "Group",
  ...EXTRA_TYPES,
};

/** Widgets made of query rows in the editor. */
const hasQueries = (type: string) =>
  ["timeseries", "query_value", "toplist", "query_table"].includes(type) ||
  type in QUERY_TYPES;

function WidgetBody({
  def,
  ctx,
  height,
}: {
  def: WidgetDef;
  ctx: Ctx;
  height: number;
}) {
  if (ctx.remote && NOT_PUBLIC.includes(def.type))
    return (
      <div className="widget-empty">
        This widget is not available on a public dashboard.
      </div>
    );
  switch (def.type) {
    case "timeseries":
      return <TimeseriesWidget def={def} ctx={ctx} height={height} />;
    case "query_value":
      return <QueryValueWidget def={def} ctx={ctx} />;
    case "toplist":
      return <ToplistWidget def={def} ctx={ctx} />;
    case "query_table":
      return <TableWidget def={def} ctx={ctx} />;
    default:
      return <ExtraWidget def={def} ctx={ctx} height={height} />;
  }
}

registerBaseRenderer((p) => <WidgetBody {...p} />);

type Actions = {
  edit: (w: Widget) => void;
  clone: (w: Widget) => void;
  remove: (w: Widget) => void;
};

function place(l: Widget["layout"]) {
  return l
    ? {
        gridColumn: `${l.x + 1} / span ${l.width}`,
        gridRow: `${l.y + 1} / span ${l.height}`,
      }
    : { gridColumn: "span 4", gridRow: "span 2" };
}

/** A free layout's grid: the size of a unit, and where moved or resized
 * widgets go. */
type Free = {
  unit: number;
  onLayout?: (w: Widget, l: WidgetLayout) => void;
};

/** Moving (from the title bar) and resizing (from the corner) a widget of a
 * free layout, snapped to the grid's units. */
function useFreeDrag(w: Widget, free?: Free) {
  const [drag, setDrag] = useState<{
    dx: number;
    dy: number;
    dw: number;
    dh: number;
  } | null>(null);
  const start = (e: React.PointerEvent, mode: "move" | "resize") => {
    if (!free?.onLayout || !w.layout || e.button !== 0) return;
    if ((e.target as HTMLElement).closest("button, input, select, a")) return;
    e.preventDefault();
    const [x0, y0] = [e.clientX, e.clientY];
    const l = w.layout;
    const u = free.unit;
    let last = { dx: 0, dy: 0, dw: 0, dh: 0 };
    const move = (ev: PointerEvent) => {
      const dx = Math.round((ev.clientX - x0) / u);
      const dy = Math.round((ev.clientY - y0) / u);
      last =
        mode === "move"
          ? { dx: Math.max(-l.x, dx), dy: Math.max(-l.y, dy), dw: 0, dh: 0 }
          : {
              dx: 0,
              dy: 0,
              dw: Math.max(4 - l.width, dx),
              dh: Math.max(4 - l.height, dy),
            };
      setDrag(last);
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      setDrag(null);
      if (last.dx || last.dy || last.dw || last.dh)
        free.onLayout?.(w, {
          x: l.x + last.dx,
          y: l.y + last.dy,
          width: l.width + last.dw,
          height: l.height + last.dh,
        });
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };
  const l = w.layout ?? { x: 0, y: 0, width: 32, height: 16 };
  const style = free
    ? {
        position: "absolute" as const,
        left: (l.x + (drag?.dx ?? 0)) * free.unit,
        top: (l.y + (drag?.dy ?? 0)) * free.unit,
        width: (l.width + (drag?.dw ?? 0)) * free.unit - 8,
        height: (l.height + (drag?.dh ?? 0)) * free.unit - 8,
        zIndex: drag ? 5 : undefined,
      }
    : place(w.layout);
  const bodyHeight = free
    ? l.height * free.unit - 8 - 44
    : (w.layout?.height ?? 2) * ROW + ((w.layout?.height ?? 2) - 1) * GAP - 44;
  const editable = !!free?.onLayout;
  return {
    style,
    bodyHeight,
    onMove: editable ? (e: React.PointerEvent) => start(e, "move") : undefined,
    handle: editable ? (
      <div
        className="widget-resize"
        onPointerDown={(e) => start(e, "resize")}
        title="Resize"
      />
    ) : null,
  };
}

export function WidgetCard({
  w,
  ctx,
  actions,
  free,
}: {
  w: Widget;
  ctx: Ctx;
  actions?: Actions;
  free?: Free;
}) {
  const def = w.definition;
  const { style, bodyHeight, onMove, handle } = useFreeDrag(w, free);
  const menu = actions && (
    <span className="widget-actions">
      <button className="btn ghost small" onClick={() => actions.edit(w)}>
        Edit
      </button>
      <button className="btn ghost small" onClick={() => actions.clone(w)}>
        Clone
      </button>
      <button
        className="btn ghost small"
        title="Delete"
        onClick={() => actions.remove(w)}
      >
        <TrashIcon />
      </button>
    </span>
  );
  if (def.type === "note")
    return (
      <div
        className={`widget widget-note${onMove ? " movable" : ""}`}
        onPointerDown={onMove}
        style={{
          ...style,
          background:
            NOTE_COLORS[def.background_color ?? "white"] ??
            def.background_color,
        }}
      >
        {menu}
        <div className="widget-note-body">
          <Markdown text={def.content ?? ""} />
        </div>
        {handle}
      </div>
    );
  if (def.type === "group")
    return (
      <div className="widget widget-group" style={style}>
        <div className="widget-title" onPointerDown={onMove}>
          <span>{def.title}</span>
          {menu}
        </div>
        <div className="dash-grid nested">
          {(def.widgets ?? []).map((c, i) => (
            <WidgetCard key={c.id ?? i} w={c} ctx={ctx} />
          ))}
        </div>
        {handle}
      </div>
    );
  if (BARE_TYPES.includes(def.type))
    return (
      <div
        className={`widget widget-bare${def.has_border === false ? " borderless" : ""}${onMove ? " movable" : ""}`}
        style={style}
        onPointerDown={onMove}
      >
        {menu}
        <ExtraWidget def={def} ctx={ctx} height={bodyHeight + 44} />
        {handle}
      </div>
    );
  return (
    <div className="widget" style={style}>
      <div
        className={`widget-title${onMove ? " movable" : ""}`}
        onPointerDown={onMove}
      >
        <span title={def.title}>
          {def.title || TYPE_NAMES[def.type] || def.type}
        </span>
        {menu}
      </div>
      <div className="widget-body">
        <WidgetBody def={def} ctx={ctx} height={bodyHeight} />
      </div>
      {handle}
    </div>
  );
}

// --- widget editor ------------------------------------------------------------------

const AGGS = [
  "count",
  "cardinality",
  "avg",
  "sum",
  "min",
  "max",
  "median",
  "pc75",
  "pc90",
  "pc95",
  "pc99",
];
const letters = "abcdefghijklmnopqrstuvwxyz";

function blankWidget(type: string, y: number): Widget {
  const layout = {
    x: 0,
    y,
    width: type === "query_value" || type === "note" ? 3 : 6,
    height: type === "query_value" || type === "note" ? 2 : 3,
  };
  if (type === "note")
    return {
      definition: {
        type,
        content: "Write **markdown** here.",
        background_color: "yellow",
      },
      layout,
    };
  if (isExtra(type)) return { definition: extraBlank(type), layout };
  const scalar = type !== "timeseries";
  return {
    definition: {
      type,
      title: "",
      ...(type === "query_value" ? { precision: 2 } : {}),
      ...(type === "timeseries" ? { show_legend: true } : {}),
      requests: [
        {
          response_format: scalar ? "scalar" : "timeseries",
          ...(type === "timeseries" ? { display_type: "line" } : {}),
          queries: [
            {
              data_source: "metrics",
              name: "a",
              query: "avg:system.cpu.user{*} by {host}",
              ...(scalar ? { aggregator: "avg" } : {}),
            },
          ],
          formulas: [{ formula: "a" }],
        },
      ],
    },
    layout,
  };
}

function QueryRow({
  q,
  onChange,
  onRemove,
  scalar,
}: {
  q: DashQuery;
  onChange: (q: DashQuery) => void;
  onRemove?: () => void;
  scalar: boolean;
}) {
  const events = q.data_source !== "metrics";
  const agg = q.compute?.aggregation ?? "count";
  return (
    <div className="query-row">
      <span className="query-letter">{q.name}</span>
      <select
        className="input"
        value={q.data_source}
        onChange={(e) =>
          onChange(
            e.target.value === "metrics"
              ? {
                  data_source: "metrics",
                  name: q.name,
                  query: "avg:system.cpu.user{*}",
                }
              : {
                  data_source: e.target.value,
                  name: q.name,
                  search: { query: "" },
                  compute: { aggregation: "count" },
                  group_by: [],
                },
          )
        }
      >
        <option value="metrics">Metrics</option>
        <option value="logs">Logs</option>
        <option value="spans">Spans</option>
      </select>
      {!events ? (
        <>
          <input
            className="input mono grow"
            value={q.query ?? ""}
            onChange={(e) => onChange({ ...q, query: e.target.value })}
            placeholder="avg:system.cpu.user{env:prod} by {host}"
          />
          {scalar && (
            <select
              className="input"
              value={q.aggregator ?? "avg"}
              onChange={(e) => onChange({ ...q, aggregator: e.target.value })}
              title="Reduce over the time range"
            >
              {["avg", "sum", "min", "max", "last"].map((a) => (
                <option key={a}>{a}</option>
              ))}
            </select>
          )}
        </>
      ) : (
        <>
          <input
            className="input mono grow"
            value={q.search?.query ?? ""}
            onChange={(e) =>
              onChange({ ...q, search: { query: e.target.value } })
            }
            placeholder="service:web status:error"
          />
          <select
            className="input"
            value={agg}
            onChange={(e) =>
              onChange({
                ...q,
                compute: {
                  aggregation: e.target.value,
                  metric:
                    e.target.value === "count"
                      ? undefined
                      : (q.compute?.metric ?? "@duration"),
                },
              })
            }
          >
            {AGGS.map((a) => (
              <option key={a}>{a}</option>
            ))}
          </select>
          {agg !== "count" && (
            <input
              className="input mono"
              style={{ width: 120 }}
              value={q.compute?.metric ?? ""}
              onChange={(e) =>
                onChange({
                  ...q,
                  compute: { aggregation: agg, metric: e.target.value },
                })
              }
              placeholder="@duration"
            />
          )}
          <span className="faint">by</span>
          <input
            className="input mono"
            style={{ width: 140 }}
            value={(q.group_by ?? []).map((g) => g.facet).join(",")}
            onChange={(e) =>
              onChange({
                ...q,
                group_by: e.target.value
                  .split(",")
                  .map((s) => s.trim())
                  .filter(Boolean)
                  .map((facet) => ({ facet, limit: 10 })),
              })
            }
            placeholder="(everything)"
          />
        </>
      )}
      {onRemove && (
        <button className="btn ghost small" onClick={onRemove} title="Remove">
          <TrashIcon />
        </button>
      )}
    </div>
  );
}

function WidgetEditor({
  initial,
  ctx,
  onSave,
  onClose,
}: {
  initial: Widget;
  ctx: Ctx;
  onSave: (w: Widget) => void;
  onClose: () => void;
}) {
  const [w, setW] = useState<Widget>(initial);
  const [json, setJson] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const def = w.definition;
  const layout = w.layout ?? { x: 0, y: 0, width: 4, height: 2 };
  const setDef = (patch: Partial<WidgetDef>) =>
    setW({ ...w, definition: { ...def, ...patch } });
  const req = def.requests?.[0];
  const norm = req ? normalizeRequest(req) : { queries: [], formulas: [] };
  const setReq = (patch: Partial<WidgetRequest>) =>
    setDef({
      requests: [
        {
          ...(req ?? {}),
          queries: norm.queries,
          formulas: norm.formulas,
          q: undefined,
          ...patch,
        },
      ],
    });
  const scalar = def.type !== "timeseries" && def.type !== "heatmap";
  const changeType = (type: string) => {
    if (
      type === "note" ||
      def.type === "note" ||
      !hasQueries(type) ||
      !hasQueries(def.type)
    )
      return setW({ ...blankWidget(type, layout.y), layout, id: w.id });
    setDef({
      type,
      requests: req
        ? [
            {
              ...req,
              queries: norm.queries,
              formulas: norm.formulas,
              q: undefined,
              response_format:
                type === "timeseries" || type === "heatmap"
                  ? "timeseries"
                  : "scalar",
            },
          ]
        : [],
    });
  };
  return (
    <Modal
      title={initial.id ? "Edit widget" : "New widget"}
      onClose={onClose}
      width={1000}
    >
      <div className="modal-body widget-editor">
        <div className="widget-editor-preview">
          <div className="dash-grid preview">
            <WidgetCard
              w={{ ...w, layout: { x: 0, y: 0, width: 12, height: 3 } }}
              ctx={ctx}
            />
          </div>
        </div>
        <div className="widget-editor-tabs">
          <button
            className={`btn small${json === null ? " primary" : ""}`}
            onClick={() => setJson(null)}
          >
            Edit
          </button>
          <button
            className={`btn small${json !== null ? " primary" : ""}`}
            onClick={() => setJson(JSON.stringify(def, null, 2))}
          >
            JSON
          </button>
        </div>
        {json !== null ? (
          <>
            <textarea
              className="input mono json-editor"
              rows={18}
              value={json}
              onChange={(e) => setJson(e.target.value)}
            />
            <button
              className="btn small"
              onClick={() => {
                try {
                  setW({ ...w, definition: JSON.parse(json) });
                  setError(null);
                } catch (e) {
                  setError(String(e));
                }
              }}
            >
              Apply JSON
            </button>
          </>
        ) : (
          <div className="form">
            <div className="form-row">
              <label className="form-field" style={{ flex: "0 0 180px" }}>
                <span className="form-label">Visualization</span>
                <select
                  className="input"
                  value={def.type}
                  onChange={(e) => changeType(e.target.value)}
                >
                  {Object.entries(TYPE_NAMES)
                    .filter(([k]) => k !== "group")
                    .map(([k, v]) => (
                      <option key={k} value={k}>
                        {v}
                      </option>
                    ))}
                </select>
              </label>
              {def.type !== "note" && !BARE_TYPES.includes(def.type) && (
                <label className="form-field">
                  <span className="form-label">Title</span>
                  <input
                    className="input"
                    value={def.title ?? ""}
                    onChange={(e) => setDef({ title: e.target.value })}
                  />
                </label>
              )}
            </div>
            {def.type === "note" ? (
              <>
                <label className="form-field">
                  <span className="form-label">Content (Markdown)</span>
                  <textarea
                    className="input mono"
                    rows={6}
                    value={def.content ?? ""}
                    onChange={(e) => setDef({ content: e.target.value })}
                  />
                </label>
                <label className="form-field" style={{ maxWidth: 200 }}>
                  <span className="form-label">Background</span>
                  <select
                    className="input"
                    value={def.background_color ?? "white"}
                    onChange={(e) =>
                      setDef({ background_color: e.target.value })
                    }
                  >
                    {Object.keys(NOTE_COLORS).map((c) => (
                      <option key={c}>{c}</option>
                    ))}
                  </select>
                </label>
              </>
            ) : !hasQueries(def.type) ? (
              <ExtraFields def={def} setDef={setDef} />
            ) : (
              <>
                {isExtra(def.type) && <ExtraFields def={def} setDef={setDef} />}
                <div className="form-label">Queries</div>
                {norm.queries.map((q, i) => (
                  <QueryRow
                    key={i}
                    q={q}
                    scalar={scalar}
                    onChange={(nq) =>
                      setReq({
                        queries: norm.queries.map((x, j) => (j === i ? nq : x)),
                      })
                    }
                    onRemove={
                      norm.queries.length > 1
                        ? () =>
                            setReq({
                              queries: norm.queries.filter((_, j) => j !== i),
                            })
                        : undefined
                    }
                  />
                ))}
                <div>
                  <button
                    className="btn small"
                    onClick={() => {
                      const name =
                        letters
                          .split("")
                          .find(
                            (l) => !norm.queries.some((q) => q.name === l),
                          ) ?? `q${norm.queries.length}`;
                      setReq({
                        queries: [
                          ...norm.queries,
                          {
                            data_source: "metrics",
                            name,
                            query: "avg:system.cpu.user{*}",
                          },
                        ],
                      });
                    }}
                  >
                    <PlusIcon /> Add query
                  </button>
                </div>
                <div className="form-label">Formulas</div>
                {norm.formulas.map((f, i) => (
                  <div className="query-row" key={i}>
                    <span className="query-letter">ƒ</span>
                    <input
                      className="input mono grow"
                      value={f.formula}
                      onChange={(e) =>
                        setReq({
                          formulas: norm.formulas.map((x, j) =>
                            j === i ? { ...x, formula: e.target.value } : x,
                          ),
                        })
                      }
                      placeholder="a / b * 100"
                    />
                    <input
                      className="input"
                      style={{ width: 160 }}
                      value={f.alias ?? ""}
                      onChange={(e) =>
                        setReq({
                          formulas: norm.formulas.map((x, j) =>
                            j === i
                              ? { ...x, alias: e.target.value || undefined }
                              : x,
                          ),
                        })
                      }
                      placeholder="Alias"
                    />
                    <button
                      className="btn ghost small"
                      onClick={() =>
                        setReq({
                          formulas: norm.formulas.filter((_, j) => j !== i),
                        })
                      }
                      title="Remove"
                    >
                      <TrashIcon />
                    </button>
                  </div>
                ))}
                <div className="form-hint">
                  <button
                    className="btn small"
                    onClick={() =>
                      setReq({
                        formulas: [
                          ...norm.formulas,
                          {
                            formula:
                              norm.queries.map((q) => q.name).join(" + ") ||
                              "a",
                          },
                        ],
                      })
                    }
                  >
                    <PlusIcon /> Add formula
                  </button>{" "}
                  Arithmetic and functions: <code>abs</code> <code>log10</code>{" "}
                  <code>cumsum</code> <code>diff</code> <code>derivative</code>{" "}
                  <code>per_second</code> <code>per_minute</code>{" "}
                  <code>default_zero</code> <code>clamp_min</code>{" "}
                  <code>timeshift(a, -3600)</code> <code>hour_before</code>{" "}
                  <code>day_before</code> <code>week_before</code>{" "}
                  <code>top(a, 10, 'mean', 'desc')</code>. Template variables:{" "}
                  <code>$name</code>.
                </div>
                <div className="form-row">
                  {def.type === "timeseries" && (
                    <>
                      <label
                        className="form-field"
                        style={{ flex: "0 0 160px" }}
                      >
                        <span className="form-label">Display</span>
                        <select
                          className="input"
                          value={req?.display_type ?? "line"}
                          onChange={(e) =>
                            setReq({ display_type: e.target.value })
                          }
                        >
                          <option value="line">Lines</option>
                          <option value="bars">Bars</option>
                        </select>
                      </label>
                      <label
                        className="form-check"
                        style={{ alignSelf: "flex-end" }}
                      >
                        <input
                          type="checkbox"
                          checked={def.show_legend !== false}
                          onChange={(e) =>
                            setDef({ show_legend: e.target.checked })
                          }
                        />{" "}
                        Legend
                      </label>
                    </>
                  )}
                  {def.type === "query_value" && (
                    <>
                      <label
                        className="form-field"
                        style={{ flex: "0 0 120px" }}
                      >
                        <span className="form-label">Decimals</span>
                        <input
                          className="input"
                          type="number"
                          min={0}
                          max={6}
                          value={def.precision ?? 2}
                          onChange={(e) =>
                            setDef({ precision: Number(e.target.value) })
                          }
                        />
                      </label>
                      <label
                        className="form-field"
                        style={{ flex: "0 0 120px" }}
                      >
                        <span className="form-label">Unit</span>
                        <input
                          className="input"
                          value={def.custom_unit ?? ""}
                          onChange={(e) =>
                            setDef({ custom_unit: e.target.value || undefined })
                          }
                          placeholder="%"
                        />
                      </label>
                    </>
                  )}
                </div>
                {def.type === "query_value" && (
                  <>
                    <div className="form-label">Conditional formatting</div>
                    {(def.conditional_formats ?? []).map((c, i) => {
                      const list = def.conditional_formats ?? [];
                      const set = (patch: Partial<ConditionalFormat>) =>
                        setDef({
                          conditional_formats: list.map((x, j) =>
                            j === i ? { ...x, ...patch } : x,
                          ),
                        });
                      return (
                        <div className="query-row" key={i}>
                          <span className="faint">If value is</span>
                          <select
                            className="input"
                            value={c.comparator}
                            onChange={(e) =>
                              set({ comparator: e.target.value })
                            }
                          >
                            {[">", ">=", "<", "<="].map((o) => (
                              <option key={o}>{o}</option>
                            ))}
                          </select>
                          <input
                            className="input"
                            type="number"
                            style={{ width: 100 }}
                            value={c.value}
                            onChange={(e) =>
                              set({ value: Number(e.target.value) })
                            }
                          />
                          <span className="faint">show</span>
                          <select
                            className="input"
                            value={c.palette}
                            onChange={(e) => set({ palette: e.target.value })}
                          >
                            {Object.keys(PALETTES).map((p) => (
                              <option key={p}>{p}</option>
                            ))}
                          </select>
                          <button
                            className="btn ghost small"
                            onClick={() =>
                              setDef({
                                conditional_formats: list.filter(
                                  (_, j) => j !== i,
                                ),
                              })
                            }
                          >
                            <TrashIcon />
                          </button>
                        </div>
                      );
                    })}
                    <div>
                      <button
                        className="btn small"
                        onClick={() =>
                          setDef({
                            conditional_formats: [
                              ...(def.conditional_formats ?? []),
                              {
                                comparator: ">",
                                value: 0,
                                palette: "white_on_red",
                              },
                            ],
                          })
                        }
                      >
                        <PlusIcon /> Add rule
                      </button>
                    </div>
                  </>
                )}
              </>
            )}
            <div className="form-row">
              {(["x", "y", "width", "height"] as const).map((k) => (
                <label
                  className="form-field"
                  key={k}
                  style={{ flex: "0 0 90px" }}
                >
                  <span className="form-label">
                    {k === "x"
                      ? "Column"
                      : k === "y"
                        ? "Row"
                        : k[0].toUpperCase() + k.slice(1)}
                  </span>
                  <input
                    className="input"
                    type="number"
                    min={k === "width" || k === "height" ? 1 : 0}
                    max={k === "x" ? 11 : k === "width" ? 12 : 20}
                    value={layout[k]}
                    onChange={(e) =>
                      setW({
                        ...w,
                        layout: {
                          ...layout,
                          [k]: Math.max(0, Number(e.target.value)),
                        },
                      })
                    }
                  />
                </label>
              ))}
            </div>
          </div>
        )}
        {error && <div className="error-box">{error}</div>}
      </div>
      <div className="modal-footer">
        <button className="btn" onClick={onClose}>
          Cancel
        </button>
        <button className="btn primary" onClick={() => onSave(w)}>
          Save
        </button>
      </div>
    </Modal>
  );
}

// --- dashboard page ------------------------------------------------------------------

function VariablePicker({
  def,
  value,
  onChange,
  time,
}: {
  def: TemplateVar;
  value: string;
  onChange: (v: string) => void;
  time: TimeRange;
}) {
  const facet = def.prefix || def.name;
  const values = useQuery({
    queryKey: ["tvar", facet, Math.floor(time.to / 300_000)],
    queryFn: async () => {
      if (def.available_values?.length) return def.available_values;
      const scope = { query: "", from: time.from, to: time.to };
      const [spans, logs] = await Promise.all([
        api.facetValues("spans", scope, [facet], 50).catch(() => null),
        api.facetValues("logs", scope, [facet], 50).catch(() => null),
      ]);
      const all = new Set<string>();
      for (const r of [spans, logs])
        for (const v of r?.facets[0]?.values ?? [])
          if (v.value) all.add(v.value);
      return [...all].sort();
    },
    staleTime: 60_000,
  });
  return (
    <label className="tvar">
      <span className="tvar-name">{def.name}</span>
      <select value={value} onChange={(e) => onChange(e.target.value)}>
        <option value="*">*</option>
        {value !== "*" && !(values.data ?? []).includes(value) && (
          <option value={value}>{value}</option>
        )}
        {(values.data ?? []).map((v) => (
          <option key={v} value={v}>
            {v}
          </option>
        ))}
      </select>
    </label>
  );
}

function VariablesModal({
  initial,
  onSave,
  onClose,
}: {
  initial: TemplateVar[];
  onSave: (v: TemplateVar[]) => void;
  onClose: () => void;
}) {
  const [vars, setVars] = useState<TemplateVar[]>(initial);
  const set = (i: number, patch: Partial<TemplateVar>) =>
    setVars(vars.map((v, j) => (j === i ? { ...v, ...patch } : v)));
  return (
    <Modal title="Template variables" onClose={onClose} width={640}>
      <div className="modal-body form">
        <p className="muted" style={{ margin: 0 }}>
          Use <code>$name</code> in queries: it becomes{" "}
          <code>prefix:value</code>, or matches everything with <code>*</code>.
        </p>
        {vars.map((v, i) => (
          <div className="query-row" key={i}>
            <input
              className="input mono"
              placeholder="name"
              value={v.name}
              onChange={(e) => set(i, { name: e.target.value })}
            />
            <input
              className="input mono"
              placeholder="tag or facet (prefix)"
              value={v.prefix ?? ""}
              onChange={(e) => set(i, { prefix: e.target.value || null })}
            />
            <input
              className="input mono"
              placeholder="default"
              value={v.default ?? "*"}
              onChange={(e) => set(i, { default: e.target.value })}
            />
            <button
              className="btn ghost small"
              onClick={() => setVars(vars.filter((_, j) => j !== i))}
            >
              <TrashIcon />
            </button>
          </div>
        ))}
        <div>
          <button
            className="btn small"
            onClick={() =>
              setVars([
                ...vars,
                {
                  name: "env",
                  prefix: "env",
                  default: "*",
                  available_values: [],
                },
              ])
            }
          >
            <PlusIcon /> Add variable
          </button>
        </div>
      </div>
      <div className="modal-footer">
        <button className="btn" onClick={onClose}>
          Cancel
        </button>
        <button
          className="btn primary"
          onClick={() => onSave(vars.filter((v) => v.name.trim()))}
        >
          Save
        </button>
      </div>
    </Modal>
  );
}

function JsonModal({
  dash,
  onSave,
  onClose,
}: {
  dash: Dashboard;
  onSave: (d: Dashboard) => Promise<void>;
  onClose: () => void;
}) {
  const exported = useMemo(() => {
    const {
      id: _id,
      url: _url,
      created_at: _c,
      modified_at: _m,
      ...rest
    } = dash;
    return JSON.stringify(rest, null, 2);
  }, [dash]);
  const [text, setText] = useState(exported);
  const [error, setError] = useState<string | null>(null);
  return (
    <Modal title="Dashboard JSON" onClose={onClose} width={900}>
      <div className="modal-body form">
        <p className="muted" style={{ margin: 0 }}>
          Same format as Datadog's dashboard JSON: paste an exported Datadog
          dashboard to import it.
        </p>
        <textarea
          className="input mono json-editor"
          rows={24}
          value={text}
          onChange={(e) => setText(e.target.value)}
        />
        {error && <div className="error-box">{error}</div>}
      </div>
      <div className="modal-footer">
        <button
          className="btn"
          onClick={() => navigator.clipboard?.writeText(text)}
        >
          Copy
        </button>
        <div className="spacer" />
        <button className="btn" onClick={onClose}>
          Cancel
        </button>
        <button
          className="btn primary"
          onClick={async () => {
            try {
              await onSave({ ...dash, ...JSON.parse(text) });
              onClose();
            } catch (e) {
              setError(errorText(e));
            }
          }}
        >
          Save
        </button>
      </div>
    </Modal>
  );
}

// --- free layout, sharing, saved views --------------------------------------------

/** Free layouts are drawn on a grid of square units: as many columns as the
 * widest widget reaches, at least 96 (12 columns of the ordered layout, 8
 * units each). */
export function FreeCanvas({
  widgets,
  ctx,
  actions,
  onLayout,
}: {
  widgets: Widget[];
  ctx: Ctx;
  actions?: Actions;
  onLayout?: (w: Widget, l: WidgetLayout) => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(1200);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setWidth(el.clientWidth));
    ro.observe(el);
    setWidth(el.clientWidth);
    return () => ro.disconnect();
  }, []);
  const cols = Math.max(
    96,
    ...widgets.map((w) => (w.layout ? w.layout.x + w.layout.width : 0)),
  );
  const unit = width / cols;
  const rows = Math.max(
    0,
    ...widgets.map((w) => (w.layout ? w.layout.y + w.layout.height : 0)),
  );
  return (
    <div ref={ref} className="dash-free" style={{ height: (rows + 4) * unit }}>
      {widgets.map((w, i) => (
        <WidgetCard
          key={w.id ?? i}
          w={w}
          ctx={ctx}
          actions={actions}
          free={{ unit, onLayout }}
        />
      ))}
    </div>
  );
}

/** Ordered layouts have 12 columns of 92 px rows; free ones square units of
 * about 12 px. */
const FREE_X = 8;
const FREE_Y = 8;
function toFree(widgets: Widget[]): Widget[] {
  return widgets.map((w) => ({
    ...w,
    layout: w.layout && {
      x: w.layout.x * FREE_X,
      y: w.layout.y * FREE_Y,
      width: w.layout.width * FREE_X,
      height: w.layout.height * FREE_Y,
    },
  }));
}
function toOrdered(widgets: Widget[]): Widget[] {
  return widgets.map((w) => {
    if (!w.layout) return w;
    const width = Math.min(
      12,
      Math.max(1, Math.round(w.layout.width / FREE_X)),
    );
    return {
      ...w,
      layout: {
        x: Math.min(12 - width, Math.round(w.layout.x / FREE_X)),
        y: Math.round(w.layout.y / FREE_Y),
        width,
        height: Math.max(1, Math.round(w.layout.height / FREE_Y)),
      },
    };
  });
}

const SPANS: [string, string][] = [
  ["15m", "Past 15 minutes"],
  ["1h", "Past 1 hour"],
  ["4h", "Past 4 hours"],
  ["1d", "Past 1 day"],
  ["2d", "Past 2 days"],
  ["1w", "Past 1 week"],
];

function ShareModal({
  dash,
  onClose,
}: {
  dash: Dashboard;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const shares = useQuery({
    queryKey: ["dashboard-shares", dash.id],
    queryFn: () => api.dashboardShares(dash.id),
  });
  const [span, setSpan] = useState("1h");
  const [timeSelectable, setTimeSelectable] = useState(true);
  const [vars, setVars] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const refresh = () =>
    qc.invalidateQueries({ queryKey: ["dashboard-shares", dash.id] });
  const create = async () => {
    setError(null);
    try {
      await api.shareDashboard({
        dashboard_id: dash.id,
        share_type: "open",
        global_time: { live_span: span },
        global_time_selectable_enabled: timeSelectable,
        selectable_template_vars: (dash.template_variables ?? [])
          .filter((v) => vars.includes(v.name))
          .map((v) => ({
            name: v.name,
            prefix: v.prefix ?? null,
            default_value: v.default ?? "*",
            visible_tags: v.available_values ?? [],
          })),
      });
      await refresh();
    } catch (e) {
      setError(errorText(e));
    }
  };
  const hidden = dash.widgets.filter((w) =>
    NOT_PUBLIC.includes(w.definition.type),
  ).length;
  return (
    <Modal title="Share publicly" onClose={onClose} width={620}>
      <div className="modal-body form">
        <p className="muted" style={{ margin: 0 }}>
          Anyone with the link sees this dashboard, read-only, without signing
          in. Only the queries of its widgets run.
          {hidden > 0 &&
            ` ${hidden} widget${hidden > 1 ? "s" : ""} (lists, monitors, SLOs…) stay empty on the public page.`}
        </p>
        {(shares.data ?? []).map((s) => {
          const url = s.public_url.startsWith("http")
            ? s.public_url
            : `${window.location.origin}${s.public_url}`;
          return (
            <div key={s.token} className="share-row">
              <a
                href={url}
                target="_blank"
                rel="noreferrer"
                className="mono ellipsis"
              >
                {url}
              </a>
              <span className="faint">
                {s.global_time.live_span}
                {s.global_time_selectable_enabled ? ", time selectable" : ""}
                {s.selectable_template_vars.length
                  ? `, ${s.selectable_template_vars.map((v) => `$${v.name}`).join(" ")}`
                  : ""}
              </span>
              <button
                className="btn small"
                onClick={() => void navigator.clipboard?.writeText(url)}
              >
                Copy
              </button>
              <button
                className="btn small"
                onClick={async () => {
                  await api.revokeDashboardShare(s.token);
                  await refresh();
                }}
              >
                Revoke
              </button>
            </div>
          );
        })}
        <div className="form-row">
          <label className="form-field" style={{ flex: "0 0 200px" }}>
            <span className="form-label">Default time frame</span>
            <select
              className="input"
              value={span}
              onChange={(e) => setSpan(e.target.value)}
            >
              {SPANS.map(([k, l]) => (
                <option key={k} value={k}>
                  {l}
                </option>
              ))}
            </select>
          </label>
          <label className="form-check" style={{ alignSelf: "flex-end" }}>
            <input
              type="checkbox"
              checked={timeSelectable}
              onChange={(e) => setTimeSelectable(e.target.checked)}
            />{" "}
            Viewers can change the time frame
          </label>
        </div>
        {(dash.template_variables ?? []).length > 0 && (
          <div>
            <div className="form-label">Variables viewers can change</div>
            {dash.template_variables.map((v) => (
              <label
                key={v.name}
                className="form-check"
                style={{ marginRight: 12 }}
              >
                <input
                  type="checkbox"
                  checked={vars.includes(v.name)}
                  onChange={(e) =>
                    setVars(
                      e.target.checked
                        ? [...vars, v.name]
                        : vars.filter((x) => x !== v.name),
                    )
                  }
                />{" "}
                ${v.name}
              </label>
            ))}
          </div>
        )}
        {error && <div className="error-text">{error}</div>}
      </div>
      <div className="modal-footer">
        <button className="btn" onClick={onClose}>
          Close
        </button>
        <button className="btn primary" onClick={create}>
          Create a public link
        </button>
      </div>
    </Modal>
  );
}

/** Saved views: named values of the template variables
 * (`template_variable_presets`). */
function SavedViews({
  dash,
  vars,
  apply,
  save,
}: {
  dash: Dashboard;
  vars: Vars;
  apply: (v: Vars) => void;
  save: (presets: TemplatePreset[]) => Promise<void>;
}) {
  const presets = (dash.template_variable_presets ?? []) as TemplatePreset[];
  const [naming, setNaming] = useState<string | null>(null);
  const current = presets.find((p) =>
    (dash.template_variables ?? []).every((d) => {
      const pv = p.template_variables.find((t) => t.name === d.name);
      const value = pv?.value ?? pv?.values?.[0] ?? d.default ?? "*";
      return value === vars[d.name];
    }),
  );
  if (!(dash.template_variables ?? []).length) return null;
  if (naming !== null)
    return (
      <span className="saved-views">
        <input
          className="input small"
          autoFocus
          placeholder="View name"
          value={naming}
          onChange={(e) => setNaming(e.target.value)}
          onKeyDown={async (e) => {
            if (e.key === "Escape") setNaming(null);
            if (e.key !== "Enter" || !naming.trim()) return;
            const view: TemplatePreset = {
              name: naming.trim(),
              template_variables: Object.entries(vars).map(([name, value]) => ({
                name,
                value,
              })),
            };
            await save([
              ...presets.filter((p) => p.name !== view.name),
              view,
            ]).catch(() => {});
            setNaming(null);
          }}
        />
        <button className="btn ghost small" onClick={() => setNaming(null)}>
          Cancel
        </button>
      </span>
    );
  return (
    <span className="saved-views">
      <select
        className="input small"
        value={current?.name ?? ""}
        onChange={(e) => {
          const p = presets.find((x) => x.name === e.target.value);
          if (!p) {
            apply(
              Object.fromEntries(
                (dash.template_variables ?? []).map((d) => [
                  d.name,
                  d.default ?? "*",
                ]),
              ),
            );
            return;
          }
          apply(
            Object.fromEntries(
              (dash.template_variables ?? []).map((d) => {
                const pv = p.template_variables.find((t) => t.name === d.name);
                return [
                  d.name,
                  pv?.value ?? pv?.values?.[0] ?? d.default ?? "*",
                ];
              }),
            ),
          );
        }}
      >
        <option value="">Default view</option>
        {presets.map((p) => (
          <option key={p.name} value={p.name}>
            {p.name}
          </option>
        ))}
      </select>
      <button
        className="btn ghost small"
        onClick={() => setNaming(current?.name ?? "")}
      >
        Save view
      </button>
      {current && (
        <button
          className="btn ghost small"
          title={`Delete the view ${current.name}`}
          onClick={() =>
            void save(presets.filter((p) => p !== current)).catch(() => {})
          }
        >
          <TrashIcon />
        </button>
      )}
    </span>
  );
}

export function DashboardPage() {
  const { id } = useParams();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const time = useTimeRange("1h", 60_000);
  const [params, setParams] = useSearchParams();
  const dash = useQuery({
    queryKey: ["dashboard", id],
    queryFn: () => api.dashboard(id!),
  });
  const [editing, setEditing] = useState<{
    widget: Widget;
    index: number | null;
  } | null>(null);
  const [modal, setModal] = useState<"json" | "vars" | "share" | null>(null);
  const favorites = useQuery({
    queryKey: ["dashboard-favorites"],
    queryFn: api.dashboardFavorites,
  });
  const [title, setTitle] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const d = dash.data;

  if (dash.error)
    return (
      <div className="page-padding error-box">{errorText(dash.error)}</div>
    );
  if (!d) return <div className="page-padding faint">Loading…</div>;
  const defs = d.template_variables ?? [];
  const vars: Vars = Object.fromEntries(
    defs.map((v) => [
      v.name,
      params.get(`tpl_var_${v.name}`) ?? v.default ?? "*",
    ]),
  );
  const ctx: Ctx = { time, defs, vars };

  const save = async (next: Dashboard) => {
    setError(null);
    try {
      const saved = await api.updateDashboard(d.id, next);
      qc.setQueryData(["dashboard", d.id], saved);
      await qc.invalidateQueries({ queryKey: ["dashboards"] });
    } catch (e) {
      setError(errorText(e));
      throw e;
    }
  };
  const nextRow = Math.max(
    0,
    ...d.widgets.map((w) => (w.layout ? w.layout.y + w.layout.height : 0)),
  );
  const free = d.layout_type === "free";
  /** A new widget, sized for the layout. */
  const fresh = (type: string) => {
    if (!free) return blankWidget(type, nextRow);
    const [b] = toFree([blankWidget(type, 0)]);
    return { ...b, layout: b.layout && { ...b.layout, y: nextRow } };
  };
  const starred = favorites.data?.includes(d.id) ?? false;
  const setVars = (v: Vars) =>
    setParams((prev) => {
      const p = new URLSearchParams(prev);
      for (const [k, value] of Object.entries(v)) p.set(`tpl_var_${k}`, value);
      return p;
    });
  const actions: Actions = {
    edit: (w) => setEditing({ widget: w, index: d.widgets.indexOf(w) }),
    clone: (w) => {
      const copy = structuredClone(w);
      delete copy.id;
      if (copy.layout) copy.layout = { ...copy.layout, y: nextRow };
      save({ ...d, widgets: [...d.widgets, copy] }).catch(() => {});
    },
    remove: (w) => {
      if (
        window.confirm(
          `Delete the widget "${w.definition.title || TYPE_NAMES[w.definition.type] || w.definition.type}"?`,
        )
      )
        save({ ...d, widgets: d.widgets.filter((x) => x !== w) }).catch(
          () => {},
        );
    },
  };

  return (
    <>
      <PageHeader
        title={
          title !== null ? (
            <input
              className="input dash-title-input"
              autoFocus
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              onBlur={() => {
                if (title.trim() && title !== d.title)
                  save({ ...d, title: title.trim() }).catch(() => {});
                setTitle(null);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter") (e.target as HTMLInputElement).blur();
                if (e.key === "Escape") setTitle(null);
              }}
            />
          ) : (
            <span
              className="dash-title"
              onClick={() => setTitle(d.title)}
              title="Rename"
            >
              {d.title}
            </span>
          )
        }
      >
        <button
          className={`btn ghost star${starred ? " on" : ""}`}
          title={starred ? "Remove from favorites" : "Add to favorites"}
          onClick={async () => {
            await api.setDashboardFavorite(d.id, !starred);
            await qc.invalidateQueries({ queryKey: ["dashboard-favorites"] });
          }}
        >
          <StarIcon filled={starred} width={16} height={16} />
        </button>
        <select
          className="input"
          style={{ width: 130 }}
          title="Layout"
          value={d.layout_type}
          onChange={(e) => {
            const layout_type = e.target.value;
            if (layout_type === d.layout_type) return;
            save({
              ...d,
              layout_type,
              widgets:
                layout_type === "free"
                  ? toFree(d.widgets)
                  : toOrdered(d.widgets),
            }).catch(() => {});
          }}
        >
          <option value="ordered">Grid layout</option>
          <option value="free">Free layout</option>
        </select>
        <button className="btn" onClick={() => setModal("share")}>
          Share
        </button>
        <button className="btn" onClick={() => setModal("json")}>
          JSON
        </button>
        <button
          className="btn"
          onClick={async () => {
            const c = await api.cloneDashboard(d.id);
            await qc.invalidateQueries({ queryKey: ["dashboards"] });
            navigate(`/dashboard/${c.id}/${slug(c.title)}`);
          }}
        >
          Clone
        </button>
        <button
          className="btn"
          title="Delete dashboard"
          onClick={async () => {
            if (!window.confirm(`Delete the dashboard "${d.title}"?`)) return;
            await api.deleteDashboard(d.id);
            await qc.invalidateQueries({ queryKey: ["dashboards"] });
            navigate("/dashboard/lists");
          }}
        >
          <TrashIcon />
        </button>
        <TimePicker time={time} allowLive={false} />
      </PageHeader>
      <div className="dash-toolbar">
        {defs.map((v) => (
          <VariablePicker
            key={v.name}
            def={v}
            value={vars[v.name]}
            time={time}
            onChange={(value) =>
              setParams((prev) => {
                const p = new URLSearchParams(prev);
                p.set(`tpl_var_${v.name}`, value);
                return p;
              })
            }
          />
        ))}
        <button className="btn ghost small" onClick={() => setModal("vars")}>
          {defs.length ? "Edit variables" : "+ Add variables"}
        </button>
        <SavedViews
          dash={d}
          vars={vars}
          apply={setVars}
          save={(template_variable_presets) =>
            save({ ...d, template_variable_presets })
          }
        />
        <div className="spacer" />
        {error && <span className="error-text">{error}</span>}
        <span className="muted">{d.description}</span>
        <button
          className="btn primary small"
          onClick={() =>
            setEditing({
              widget: fresh("timeseries"),
              index: null,
            })
          }
        >
          <PlusIcon /> Add Widget
        </button>
      </div>
      <div className="page-scroll dash-scroll">
        {free ? (
          <FreeCanvas
            widgets={d.widgets}
            ctx={ctx}
            actions={actions}
            onLayout={(w, layout) =>
              save({
                ...d,
                widgets: d.widgets.map((x) => (x === w ? { ...x, layout } : x)),
              }).catch(() => {})
            }
          />
        ) : (
          <div className="dash-grid">
            {d.widgets.map((w, i) => (
              <WidgetCard key={w.id ?? i} w={w} ctx={ctx} actions={actions} />
            ))}
          </div>
        )}
        {!d.widgets.length && (
          <div className="empty-state">
            <h3>This dashboard is empty</h3>
            <button
              className="btn primary"
              onClick={() =>
                setEditing({
                  widget: fresh("timeseries"),
                  index: null,
                })
              }
            >
              <PlusIcon /> Add a widget
            </button>
          </div>
        )}
      </div>
      {editing && (
        <WidgetEditor
          initial={editing.widget}
          ctx={ctx}
          onClose={() => setEditing(null)}
          onSave={async (w) => {
            const widgets =
              editing.index === null
                ? [...d.widgets, w]
                : d.widgets.map((x, i) => (i === editing.index ? w : x));
            await save({ ...d, widgets }).then(
              () => setEditing(null),
              () => {},
            );
          }}
        />
      )}
      {modal === "share" && (
        <ShareModal dash={d} onClose={() => setModal(null)} />
      )}
      {modal === "json" && (
        <JsonModal dash={d} onSave={save} onClose={() => setModal(null)} />
      )}
      {modal === "vars" && (
        <VariablesModal
          initial={defs}
          onClose={() => setModal(null)}
          onSave={(template_variables) => {
            save({ ...d, template_variables }).then(
              () => setModal(null),
              () => {},
            );
          }}
        />
      )}
    </>
  );
}

// --- list --------------------------------------------------------------------------

export function DashboardList() {
  const list = useQuery({ queryKey: ["dashboards"], queryFn: api.dashboards });
  const lists = useQuery({
    queryKey: ["dashboard-lists"],
    queryFn: api.dashboardLists,
  });
  const favorites = useQuery({
    queryKey: ["dashboard-favorites"],
    queryFn: api.dashboardFavorites,
  });
  const navigate = useNavigate();
  const qc = useQueryClient();
  const [params, setParams] = useSearchParams();
  const [text, setText] = useState("");
  const [picked, setPicked] = useState<string[]>([]);
  const [naming, setNaming] = useState<string | null>(null);
  const filter = params.get("list") ?? "all";
  const favs = favorites.data ?? [];
  const current = (lists.data ?? []).find((l) => String(l.id) === filter);
  const inFilter = (id: string) =>
    filter === "all"
      ? true
      : filter === "favorites"
        ? favs.includes(id)
        : (current?.dashboard_ids.includes(id) ?? false);
  const rows = (list.data?.dashboards ?? [])
    .filter((d) => inFilter(d.id))
    .filter((d) => !text || d.title.toLowerCase().includes(text.toLowerCase()))
    .sort(
      (a, b) =>
        Number(favs.includes(b.id)) - Number(favs.includes(a.id)) ||
        b.modified_at.localeCompare(a.modified_at),
    );
  const refreshLists = () =>
    qc.invalidateQueries({ queryKey: ["dashboard-lists"] });
  const choose = (f: string) => {
    setPicked([]);
    setParams((prev) => {
      const p = new URLSearchParams(prev);
      if (f === "all") p.delete("list");
      else p.set("list", f);
      return p;
    });
  };
  return (
    <>
      <PageHeader
        title="Dashboards"
        tabs={[{ to: "/dashboard/lists", label: "Dashboard List" }]}
      >
        <button
          className="btn primary"
          onClick={async () => {
            const d = await api.createDashboard({
              title: `New Dashboard ${new Date().toLocaleDateString("en-US", { month: "short", day: "numeric" })}`,
              layout_type: "ordered",
              widgets: [],
              template_variables: [],
            });
            await qc.invalidateQueries({ queryKey: ["dashboards"] });
            navigate(`/dashboard/${d.id}/${slug(d.title)}`);
          }}
        >
          <PlusIcon /> New Dashboard
        </button>
      </PageHeader>
      <div className="page-scroll page-padding dash-list-page">
        <aside className="dash-lists">
          <button
            className={`dash-list-item${filter === "all" ? " active" : ""}`}
            onClick={() => choose("all")}
          >
            All dashboards
            <span className="faint">{list.data?.dashboards.length ?? ""}</span>
          </button>
          <button
            className={`dash-list-item${filter === "favorites" ? " active" : ""}`}
            onClick={() => choose("favorites")}
          >
            <StarIcon width={13} height={13} /> Favorites
            <span className="faint">{favs.length}</span>
          </button>
          <div className="dash-lists-title">Lists</div>
          {(lists.data ?? []).map((l) => (
            <button
              key={l.id}
              className={`dash-list-item${filter === String(l.id) ? " active" : ""}`}
              onClick={() => choose(String(l.id))}
            >
              <span className="ellipsis">{l.name}</span>
              <span className="faint">{l.dashboard_count}</span>
            </button>
          ))}
          {naming !== null ? (
            <input
              className="input small"
              autoFocus
              placeholder="List name"
              value={naming}
              onChange={(e) => setNaming(e.target.value)}
              onBlur={() => setNaming(null)}
              onKeyDown={async (e) => {
                if (e.key === "Escape") setNaming(null);
                if (e.key !== "Enter" || !naming.trim()) return;
                const l = await api.createDashboardList(naming.trim());
                setNaming(null);
                await refreshLists();
                choose(String(l.id));
              }}
            />
          ) : (
            <button className="btn ghost small" onClick={() => setNaming("")}>
              <PlusIcon /> New list
            </button>
          )}
        </aside>
        <div className="dash-list-main">
          <div className="dash-list-toolbar">
            <input
              className="input"
              style={{ width: 320 }}
              placeholder="Search dashboards"
              value={text}
              onChange={(e) => setText(e.target.value)}
            />
            {picked.length > 0 && (lists.data ?? []).length > 0 && (
              <select
                className="input"
                style={{ width: 220 }}
                value=""
                onChange={async (e) => {
                  await api.addToDashboardList(Number(e.target.value), picked);
                  setPicked([]);
                  await refreshLists();
                }}
              >
                <option value="">Add {picked.length} to a list…</option>
                {(lists.data ?? []).map((l) => (
                  <option key={l.id} value={l.id}>
                    {l.name}
                  </option>
                ))}
              </select>
            )}
            {picked.length > 0 && current && (
              <button
                className="btn"
                onClick={async () => {
                  await api.removeFromDashboardList(current.id, picked);
                  setPicked([]);
                  await refreshLists();
                }}
              >
                Remove {picked.length} from {current.name}
              </button>
            )}
            <div className="spacer" />
            {current && (
              <>
                <button
                  className="btn small"
                  onClick={async () => {
                    const name = window.prompt("Rename the list", current.name);
                    if (!name?.trim()) return;
                    await api.renameDashboardList(current.id, name.trim());
                    await refreshLists();
                  }}
                >
                  Rename list
                </button>
                <button
                  className="btn small"
                  title="Delete the list (not its dashboards)"
                  onClick={async () => {
                    if (!window.confirm(`Delete the list "${current.name}"?`))
                      return;
                    await api.deleteDashboardList(current.id);
                    choose("all");
                    await refreshLists();
                  }}
                >
                  <TrashIcon />
                </button>
              </>
            )}
          </div>
          <div className="card" style={{ padding: 0 }}>
            <table className="list">
              <thead>
                <tr>
                  <th style={{ width: 28 }}>
                    <input
                      type="checkbox"
                      checked={rows.length > 0 && picked.length === rows.length}
                      onChange={(e) =>
                        setPicked(e.target.checked ? rows.map((r) => r.id) : [])
                      }
                    />
                  </th>
                  <th style={{ width: 28 }} />
                  <th>Name</th>
                  <th style={{ width: 110 }} className="num">
                    Widgets
                  </th>
                  <th style={{ width: 160 }}>Modified</th>
                  <th style={{ width: 160 }}>Created</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((d) => {
                  const starred = favs.includes(d.id);
                  return (
                    <tr
                      key={d.id}
                      style={{ cursor: "pointer" }}
                      onClick={() =>
                        navigate(`/dashboard/${d.id}/${slug(d.title)}`)
                      }
                    >
                      <td onClick={(e) => e.stopPropagation()}>
                        <input
                          type="checkbox"
                          checked={picked.includes(d.id)}
                          onChange={(e) =>
                            setPicked(
                              e.target.checked
                                ? [...picked, d.id]
                                : picked.filter((x) => x !== d.id),
                            )
                          }
                        />
                      </td>
                      <td>
                        <button
                          className={`btn ghost small star${starred ? " on" : ""}`}
                          title={
                            starred
                              ? "Remove from favorites"
                              : "Add to favorites"
                          }
                          onClick={async (e) => {
                            e.stopPropagation();
                            await api.setDashboardFavorite(d.id, !starred);
                            await qc.invalidateQueries({
                              queryKey: ["dashboard-favorites"],
                            });
                          }}
                        >
                          <StarIcon filled={starred} width={14} height={14} />
                        </button>
                      </td>
                      <td>
                        <Link
                          to={`/dashboard/${d.id}/${slug(d.title)}`}
                          className="dash-link"
                          onClick={(e) => e.stopPropagation()}
                        >
                          {d.title}
                        </Link>
                        {d.layout_type === "free" && (
                          <span className="faint"> · free layout</span>
                        )}
                        {d.description && (
                          <div className="muted ellipsis">{d.description}</div>
                        )}
                      </td>
                      <td className="num">{d.widget_count}</td>
                      <td className="muted">
                        {ago(Date.parse(d.modified_at))}
                      </td>
                      <td className="muted">{ago(Date.parse(d.created_at))}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
            {list.data && !rows.length && (
              <div className="empty-state">No dashboard.</div>
            )}
          </div>
        </div>
      </div>
    </>
  );
}
