// The dashboard widgets beyond timeseries, query values, top lists, tables
// and notes, with Datadog's widget definitions.
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useMemo, type ReactNode } from "react";
import { Link } from "react-router-dom";
import atlasUrl from "world-atlas/countries-110m.json?url";
import { TimeChart, type Series } from "./Charts";
import {
  api,
  type DashQuery,
  type Log,
  type ScalarResponse,
  type TemplateVar,
  type Widget,
  type WidgetDef,
  type WidgetRequest,
} from "../lib/api";
import {
  buildRequest,
  errorText,
  fmt,
  normalizeRequest,
  PALETTE,
  runScalar,
  runTimeseries,
  type Ctx,
} from "../lib/dash";
import {
  ago,
  duration as nsDuration,
  fullDate,
  statusColor,
} from "../lib/format";

// --- common ------------------------------------------------------------------------------

const Empty = ({ children }: { children?: ReactNode }) => (
  <div className="widget-empty">{children ?? "No data"}</div>
);
const Loading = () => <div className="widget-loading" />;
const Failed = ({ e }: { e: unknown }) => (
  <div className="widget-error">{errorText(e)}</div>
);

type Row = {
  key: string;
  label: string;
  tags: Record<string, string>;
  values: (number | null)[];
};

/** Scalar results by group: each group's value per formula. */
function table(res: ScalarResponse): Row[] {
  const cols = res.data.attributes.columns;
  const groups = cols.filter((c) => c.type === "group") as Extract<
    (typeof cols)[number],
    { type: "group" }
  >[];
  const nums = cols.filter((c) => c.type === "number") as Extract<
    (typeof cols)[number],
    { type: "number" }
  >[];
  const out = new Map<string, Row>();
  const n = nums[0]?.values.length ?? 0;
  for (let i = 0; i < n; i++) {
    const tags = Object.fromEntries(
      groups.map((g) => [g.name, g.values[i]?.[0] ?? ""]),
    );
    const key = groups
      .map((g) => `${g.name}:${g.values[i]?.[0] ?? ""}`)
      .join(",");
    const row = out.get(key) ?? {
      key,
      label:
        groups.map((g) => g.values[i]?.[0]).join(", ") ||
        nums.find((c) => c.values[i] !== null)?.name ||
        "",
      tags,
      values: nums.map(() => null),
    };
    nums.forEach((c, j) => {
      if (c.values[i] !== null) row.values[j] = c.values[i];
    });
    out.set(key, row);
  }
  return [...out.values()];
}

function useScalar(r: WidgetRequest | undefined, ctx: Ctx, shiftMs = 0) {
  const { time, vars } = ctx;
  return useQuery({
    queryKey: [
      "w-scalar",
      JSON.stringify(r),
      JSON.stringify(vars),
      time.from,
      time.to,
      shiftMs,
    ],
    queryFn: ({ signal }) =>
      runScalar(r!, ctx, time.from - shiftMs, time.to - shiftMs, signal),
    enabled: !!r,
    placeholderData: keepPreviousData,
    refetchInterval: 60_000,
  });
}

function useTimeseries(r: WidgetRequest | undefined, ctx: Ctx) {
  const { time, vars } = ctx;
  return useQuery({
    queryKey: [
      "w-ts",
      JSON.stringify(r),
      JSON.stringify(vars),
      time.from,
      time.to,
    ],
    queryFn: ({ signal }) => runTimeseries(r!, ctx, time.from, time.to, signal),
    enabled: !!r,
    placeholderData: keepPreviousData,
    refetchInterval: 60_000,
  });
}

/** The first query of a request, template variables applied; Datadog's
 * single-query requests (`{query: {...}}`) too. */
function firstQuery(r: WidgetRequest | undefined, ctx: Ctx): DashQuery | null {
  if (!r) return null;
  const single = r.query as DashQuery | undefined;
  const q =
    single && typeof single === "object"
      ? single
      : normalizeRequest(r).queries[0];
  if (!q) return null;
  return buildRequest({ queries: [q], formulas: [] }, ctx.defs, ctx.vars, 0, 0)
    .queries[0] as DashQuery;
}

/** Light to saturated purple. */
const shade = (t: number) =>
  `rgba(139, 111, 208, ${0.12 + 0.88 * Math.max(0, Math.min(1, t))})`;
/** Green, yellow, red. */
function heat(t: number): string {
  t = Math.max(0, Math.min(1, t));
  const hue = 130 - 130 * t;
  return `hsl(${hue}, 62%, 48%)`;
}

// --- heatmap -----------------------------------------------------------------------------

function Heatmap({
  def,
  ctx,
  height,
}: {
  def: WidgetDef;
  ctx: Ctx;
  height: number;
}) {
  const res = useTimeseries(def.requests?.[0], ctx);
  const grid = useMemo(() => {
    if (!res.data) return null;
    const a = res.data.data.attributes;
    let lo = Infinity;
    let hi = -Infinity;
    for (const s of a.values)
      for (const v of s)
        if (v !== null) {
          lo = Math.min(lo, v);
          hi = Math.max(hi, v);
        }
    if (!Number.isFinite(lo)) return null;
    const bins = 16;
    const w = hi > lo ? (hi - lo) / bins : 1;
    const cells = a.times.map(() => new Array<number>(bins).fill(0));
    for (const s of a.values)
      s.forEach((v, t) => {
        if (v !== null)
          cells[t][Math.min(bins - 1, Math.floor((v - lo) / w))]++;
      });
    const max = Math.max(1, ...cells.flat());
    return {
      times: a.times,
      interval: a.interval,
      cells,
      lo,
      hi,
      w,
      bins,
      max,
      series: a.values.length,
    };
  }, [res.data]);
  if (res.error) return <Failed e={res.error} />;
  if (!res.data) return <Loading />;
  if (!grid) return <Empty />;
  const T = grid.times.length;
  return (
    <div className="w-heatmap" style={{ height }}>
      <div className="w-heatmap-axis">
        <span>{fmt(grid.hi)}</span>
        <span>{fmt((grid.hi + grid.lo) / 2)}</span>
        <span>{fmt(grid.lo)}</span>
      </div>
      <svg
        viewBox={`0 0 ${T} ${grid.bins}`}
        preserveAspectRatio="none"
        width="100%"
        height="100%"
      >
        {grid.cells.flatMap((col, t) =>
          col.map((c, b) =>
            c ? (
              <rect
                key={`${t}-${b}`}
                x={t}
                y={grid.bins - 1 - b}
                width={1.02}
                height={1.02}
                fill={shade(c / grid.max)}
              >
                <title>
                  {fullDate(grid.times[t])}: {c} of {grid.series} series between{" "}
                  {fmt(grid.lo + b * grid.w)} and{" "}
                  {fmt(grid.lo + (b + 1) * grid.w)}
                </title>
              </rect>
            ) : null,
          ),
        )}
      </svg>
    </div>
  );
}

// --- distribution ------------------------------------------------------------------------

function Distribution({
  def,
  ctx,
  height,
}: {
  def: WidgetDef;
  ctx: Ctx;
  height: number;
}) {
  const q = firstQuery(def.requests?.[0], ctx);
  const { time } = ctx;
  const res = useQuery({
    queryKey: ["w-dist", JSON.stringify(q), time.from, time.to],
    queryFn: ({ signal }) =>
      api.queryDistribution(
        q!.data_source === "metrics"
          ? {
              from: time.from,
              to: time.to,
              data_source: "metrics",
              query: q!.query,
            }
          : {
              from: time.from,
              to: time.to,
              data_source: q!.data_source,
              search: q!.search?.query ?? "",
              metric: q!.compute?.metric ?? "@duration",
            },
        signal,
      ),
    enabled: !!q,
    placeholderData: keepPreviousData,
    refetchInterval: 60_000,
  });
  if (!q) return <Empty>No query</Empty>;
  if (res.error) return <Failed e={res.error} />;
  if (!res.data) return <Loading />;
  const d = res.data;
  if (!d.count) return <Empty />;
  const max = Math.max(...d.buckets.map((b) => b.count));
  const unit =
    q.data_source === "spans" &&
    (q.compute?.metric ?? "@duration") === "@duration"
      ? " s"
      : "";
  return (
    <div className="w-dist" style={{ height }}>
      <div className="w-dist-bars">
        {d.buckets.map((b, i) => (
          <div
            key={i}
            className="w-dist-bar"
            style={{ height: `${(b.count / max) * 100}%` }}
            title={`${fmt(b.lower)}${unit} – ${fmt(b.upper)}${unit}: ${b.count}`}
          />
        ))}
      </div>
      <div className="w-dist-axis">
        <span>
          {fmt(d.min)}
          {unit}
        </span>
        <span className="faint">
          p50 {fmt(d.p50)} · p90 {fmt(d.p90)} · p99 {fmt(d.p99)} · {d.count}{" "}
          values
        </span>
        <span>
          {fmt(d.max)}
          {unit}
        </span>
      </div>
    </div>
  );
}

// --- pie (sunburst) and treemap ---------------------------------------------------------

function useGroups(def: WidgetDef, ctx: Ctx, limit = 12) {
  const res = useScalar(def.requests?.[0], ctx);
  const items = useMemo(() => {
    if (!res.data) return null;
    const rows = table(res.data)
      .map((r) => ({ label: r.label, value: r.values[0] ?? 0 }))
      .filter((r) => r.value > 0)
      .sort((a, b) => b.value - a.value);
    const top = rows.slice(0, limit);
    const rest = rows.slice(limit).reduce((a, r) => a + r.value, 0);
    if (rest > 0) top.push({ label: "other", value: rest });
    return top.map((r, i) => ({ ...r, color: PALETTE[i % PALETTE.length] }));
  }, [res.data, limit]);
  return { res, items };
}

function Sunburst({
  def,
  ctx,
  height,
}: {
  def: WidgetDef;
  ctx: Ctx;
  height: number;
}) {
  const { res, items } = useGroups(def, ctx);
  if (res.error) return <Failed e={res.error} />;
  if (!items) return <Loading />;
  if (!items.length) return <Empty />;
  const total = items.reduce((a, r) => a + r.value, 0);
  let angle = -Math.PI / 2;
  const arc = (a0: number, a1: number) => {
    const [r0, r1] = [32, 50];
    const p = (r: number, a: number) =>
      `${50 + r * Math.cos(a)} ${50 + r * Math.sin(a)}`;
    const large = a1 - a0 > Math.PI ? 1 : 0;
    return `M ${p(r1, a0)} A ${r1} ${r1} 0 ${large} 1 ${p(r1, a1)} L ${p(r0, a1)} A ${r0} ${r0} 0 ${large} 0 ${p(r0, a0)} Z`;
  };
  const legend = (def.legend as { type?: string } | undefined)?.type !== "none";
  return (
    <div className="w-pie" style={{ height }}>
      <svg viewBox="0 0 100 100" height={height - 8}>
        {items.map((r) => {
          const a0 = angle;
          const a1 = angle + (r.value / total) * Math.PI * 2 * 0.99999;
          angle = a1;
          return (
            <path key={r.label} d={arc(a0, a1)} fill={r.color}>
              <title>
                {r.label}: {fmt(r.value)} (
                {((r.value / total) * 100).toFixed(1)} %)
              </title>
            </path>
          );
        })}
        {!def.hide_total && (
          <text x="50" y="53" textAnchor="middle" className="w-pie-total">
            {fmt(total)}
          </text>
        )}
      </svg>
      {legend && (
        <div className="w-pie-legend">
          {items.map((r) => (
            <div key={r.label} title={r.label}>
              <i style={{ background: r.color }} />
              <span className="ellipsis">{r.label}</span>
              <span className="faint">
                {((r.value / total) * 100).toFixed(1)} %
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

type Rect = { x: number; y: number; w: number; h: number };

/** Splits the area between the items in proportion, halving the list
 * along the longer side each time. */
function layout<T extends { value: number }>(
  items: T[],
  r: Rect,
): (T & Rect)[] {
  if (!items.length) return [];
  if (items.length === 1) return [{ ...items[0], ...r }];
  const total = items.reduce((a, i) => a + i.value, 0);
  let acc = 0;
  let cut = 1;
  for (let i = 0; i < items.length - 1; i++) {
    acc += items[i].value;
    cut = i + 1;
    if (acc >= total / 2) break;
  }
  const share = items.slice(0, cut).reduce((a, i) => a + i.value, 0) / total;
  const [a, b] =
    r.w >= r.h
      ? [
          { x: r.x, y: r.y, w: r.w * share, h: r.h },
          { x: r.x + r.w * share, y: r.y, w: r.w * (1 - share), h: r.h },
        ]
      : [
          { x: r.x, y: r.y, w: r.w, h: r.h * share },
          { x: r.x, y: r.y + r.h * share, w: r.w, h: r.h * (1 - share) },
        ];
  return [...layout(items.slice(0, cut), a), ...layout(items.slice(cut), b)];
}

function Treemap({
  def,
  ctx,
  height,
}: {
  def: WidgetDef;
  ctx: Ctx;
  height: number;
}) {
  const { res, items } = useGroups(def, ctx, 30);
  if (res.error) return <Failed e={res.error} />;
  if (!items) return <Loading />;
  if (!items.length) return <Empty />;
  const rects = layout(items, { x: 0, y: 0, w: 100, h: 100 });
  return (
    <div className="w-treemap" style={{ height }}>
      {rects.map((r) => (
        <div
          key={r.label}
          className="w-treemap-cell"
          style={{
            left: `${r.x}%`,
            top: `${r.y}%`,
            width: `${r.w}%`,
            height: `${r.h}%`,
            background: r.color,
          }}
          title={`${r.label}: ${fmt(r.value)}`}
        >
          {r.w > 8 && r.h > 8 && (
            <>
              <span className="ellipsis">{r.label}</span>
              <strong>{fmt(r.value)}</strong>
            </>
          )}
        </div>
      ))}
    </div>
  );
}

// --- scatter plot ------------------------------------------------------------------------

function Scatter({
  def,
  ctx,
  height,
}: {
  def: WidgetDef;
  ctx: Ctx;
  height: number;
}) {
  const reqs = def.requests as unknown as
    Record<string, unknown> | WidgetRequest[] | undefined;
  const request: WidgetRequest | undefined = useMemo(() => {
    if (!reqs || Array.isArray(reqs)) return undefined;
    const t = reqs.table as
      | {
          queries?: DashQuery[];
          formulas?: { formula: string; dimension?: string; alias?: string }[];
        }
      | undefined;
    if (t?.queries) {
      const dims = ["x", "y"].map((d) =>
        t.formulas?.find((f) => f.dimension === d),
      );
      if (dims.some((d) => !d)) return undefined;
      return {
        queries: t.queries,
        formulas: dims.map((d) => ({ formula: d!.formula, alias: d!.alias })),
      };
    }
    const axis = (k: string) =>
      reqs[k] as { q?: string; aggregator?: string } | undefined;
    if (axis("x")?.q && axis("y")?.q)
      return {
        queries: ["x", "y"].map((k) => ({
          data_source: "metrics",
          name: k,
          query: axis(k)!.q!,
          aggregator: axis(k)!.aggregator ?? "avg",
        })),
        formulas: [{ formula: "x" }, { formula: "y" }],
      };
    return undefined;
  }, [reqs]);
  const res = useScalar(request, ctx);
  if (!request)
    return <Empty>Needs `requests.table` with x and y formulas</Empty>;
  if (res.error) return <Failed e={res.error} />;
  if (!res.data) return <Loading />;
  const pts = table(res.data).filter(
    (r) => r.values[0] !== null && r.values[1] !== null,
  );
  if (!pts.length) return <Empty />;
  const xs = pts.map((p) => p.values[0]!);
  const ys = pts.map((p) => p.values[1]!);
  const [x0, x1] = [Math.min(0, ...xs), Math.max(...xs) || 1];
  const [y0, y1] = [Math.min(0, ...ys), Math.max(...ys) || 1];
  const W = 400;
  const H = Math.max(80, height - 20);
  const px = (v: number) => 36 + ((v - x0) / (x1 - x0 || 1)) * (W - 46);
  const py = (v: number) => H - 18 - ((v - y0) / (y1 - y0 || 1)) * (H - 28);
  return (
    <svg
      className="w-scatter"
      viewBox={`0 0 ${W} ${H}`}
      width="100%"
      height={H}
    >
      <line x1={36} y1={H - 18} x2={W - 10} y2={H - 18} className="axis" />
      <line x1={36} y1={10} x2={36} y2={H - 18} className="axis" />
      <text x={36} y={H - 4} className="tick">
        {fmt(x0)}
      </text>
      <text x={W - 10} y={H - 4} className="tick" textAnchor="end">
        {fmt(x1)}
      </text>
      <text x={32} y={14} className="tick" textAnchor="end">
        {fmt(y1)}
      </text>
      {pts.map((p, i) => (
        <circle
          key={p.key}
          cx={px(p.values[0]!)}
          cy={py(p.values[1]!)}
          r={4}
          fill={PALETTE[i % PALETTE.length]}
        >
          <title>
            {p.label}: x {fmt(p.values[0])}, y {fmt(p.values[1])}
          </title>
        </circle>
      ))}
    </svg>
  );
}

// --- change ------------------------------------------------------------------------------

const COMPARE: Record<string, number> = {
  hour_before: 3_600_000,
  day_before: 86_400_000,
  week_before: 7 * 86_400_000,
  month_before: 28 * 86_400_000,
};

function Change({ def, ctx }: { def: WidgetDef; ctx: Ctx }) {
  const r = def.requests?.[0];
  const shift =
    COMPARE[(r?.compare_to as string) ?? "day_before"] ?? COMPARE.day_before;
  const now = useScalar(r, ctx);
  const before = useScalar(r, ctx, shift);
  if (now.error || before.error)
    return <Failed e={now.error ?? before.error} />;
  if (!now.data || !before.data) return <Loading />;
  const past = new Map(table(before.data).map((x) => [x.key, x.values[0]]));
  const relative = r?.change_type === "relative";
  const increaseGood = r?.increase_good !== false;
  const rows = table(now.data).map((x) => {
    const p = past.get(x.key) ?? null;
    const v = x.values[0];
    const change =
      v === null || p === null
        ? null
        : relative
          ? p === 0
            ? null
            : ((v - p) / Math.abs(p)) * 100
          : v - p;
    return { ...x, present: v, past: p, change };
  });
  const order = (r?.order_by as string) ?? "change";
  const dir = r?.order_dir === "asc" ? 1 : -1;
  rows.sort((a, b) =>
    order === "name"
      ? a.label.localeCompare(b.label) * dir
      : (order === "present"
          ? (a.present ?? 0) - (b.present ?? 0)
          : order === "past"
            ? (a.past ?? 0) - (b.past ?? 0)
            : Math.abs(a.change ?? 0) - Math.abs(b.change ?? 0)) * dir,
  );
  if (!rows.length) return <Empty />;
  return (
    <div className="widget-table">
      <table className="list">
        <thead>
          <tr>
            <th>Name</th>
            <th className="num">Change</th>
            {r?.show_present !== false && <th className="num">Value</th>}
          </tr>
        </thead>
        <tbody>
          {rows.slice(0, 50).map((x) => {
            const up = (x.change ?? 0) > 0;
            const good =
              x.change === null || x.change === 0 ? null : up === increaseGood;
            return (
              <tr key={x.key}>
                <td className="mono ellipsis">{x.label}</td>
                <td
                  className={`num ${good === null ? "" : good ? "w-good" : "w-bad"}`}
                >
                  {x.change === null
                    ? "—"
                    : `${up ? "▲" : "▼"} ${fmt(Math.abs(x.change))}${relative ? " %" : ""}`}
                </td>
                {r?.show_present !== false && (
                  <td className="num">{fmt(x.present)}</td>
                )}
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

// --- geomap ------------------------------------------------------------------------------

type Topo = {
  transform?: { scale: [number, number]; translate: [number, number] };
  arcs: [number, number][][];
  objects: {
    countries: {
      geometries: {
        type: string;
        arcs: number[][] | number[][][];
        properties: { name: string };
      }[];
    };
  };
};

/** Natural Earth's short names, against `Intl`'s English region names. */
const NE_NAMES: Record<string, string> = {
  US: "United States of America",
  BA: "Bosnia and Herz.",
  DO: "Dominican Rep.",
  CF: "Central African Rep.",
  SS: "S. Sudan",
  CD: "Dem. Rep. Congo",
  CG: "Congo",
  CI: "Côte d'Ivoire",
  GQ: "Eq. Guinea",
  SZ: "eSwatini",
  MM: "Myanmar",
  MK: "Macedonia",
  SB: "Solomon Is.",
  FK: "Falkland Is.",
  TF: "Fr. S. Antarctic Lands",
  EH: "W. Sahara",
  PS: "Palestine",
  TR: "Turkey",
  CZ: "Czechia",
  KP: "North Korea",
  KR: "South Korea",
  LA: "Laos",
  RU: "Russia",
  TL: "Timor-Leste",
  VN: "Vietnam",
  BN: "Brunei",
  MD: "Moldova",
  SY: "Syria",
  IR: "Iran",
  TZ: "Tanzania",
  VE: "Venezuela",
  BO: "Bolivia",
};

type Shape = { name: string; d: string };

async function loadShapes(): Promise<Shape[]> {
  const topo = (await (await fetch(atlasUrl)).json()) as Topo;
  const [sx, sy] = topo.transform?.scale ?? [1, 1];
  const [tx, ty] = topo.transform?.translate ?? [0, 0];
  const arcs = topo.arcs.map((arc) => {
    let x = 0;
    let y = 0;
    return arc.map(([dx, dy]) => {
      x += dx;
      y += dy;
      return topo.transform ? [x * sx + tx, y * sy + ty] : [dx, dy];
    });
  });
  const ring = (ids: number[]) => {
    const pts: number[][] = [];
    ids.forEach((id, k) => {
      const a = id >= 0 ? arcs[id] : [...arcs[~id]].reverse();
      pts.push(...(k ? a.slice(1) : a));
    });
    return pts;
  };
  const path = (pts: number[][]) => {
    let d = "";
    let prev: number[] | null = null;
    for (const [lon, lat] of pts) {
      // Rings crossing the antimeridian: start over rather than cross the map.
      const jump = prev && Math.abs(lon - prev[0]) > 180;
      d += `${!prev || jump ? "M" : "L"}${(lon + 180).toFixed(2)} ${(90 - lat).toFixed(2)}`;
      prev = [lon, lat];
    }
    return d + "Z";
  };
  return topo.objects.countries.geometries.map((g) => {
    const polys = (g.type === "Polygon" ? [g.arcs] : g.arcs) as number[][][];
    return {
      name: g.properties.name,
      d: polys.flatMap((p) => p.map((r) => path(ring(r)))).join(""),
    };
  });
}

const regionNames = (() => {
  try {
    return new Intl.DisplayNames(["en"], { type: "region" });
  } catch {
    return null;
  }
})();

const norm = (s: string) =>
  s
    .toLowerCase()
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/[^a-z]/g, "");

function Geomap({
  def,
  ctx,
  height,
}: {
  def: WidgetDef;
  ctx: Ctx;
  height: number;
}) {
  const shapes = useQuery({
    queryKey: ["world-atlas"],
    queryFn: loadShapes,
    staleTime: Infinity,
  });
  const res = useScalar(def.requests?.[0], ctx);
  const values = useMemo(() => {
    const m = new Map<string, { code: string; value: number }>();
    for (const r of res.data ? table(res.data) : []) {
      const code = (Object.values(r.tags)[0] ?? r.label).toUpperCase();
      if (r.values[0] === null) continue;
      const name =
        NE_NAMES[code] ??
        (code.length === 2 ? regionNames?.of(code) : code) ??
        code;
      m.set(norm(name), { code, value: r.values[0] });
    }
    return m;
  }, [res.data]);
  if (res.error || shapes.error)
    return <Failed e={res.error ?? shapes.error} />;
  if (!res.data || !shapes.data) return <Loading />;
  const max = Math.max(0, ...[...values.values()].map((v) => v.value));
  return (
    <svg
      className="w-geomap"
      viewBox="0 0 360 150"
      height={height}
      width="100%"
      preserveAspectRatio="xMidYMid meet"
    >
      {shapes.data.map((s) => {
        const v = values.get(norm(s.name));
        return (
          <path
            key={s.name}
            d={s.d}
            fill={v ? shade(max ? v.value / max : 1) : "var(--bg-subtle)"}
            className="country"
          >
            <title>
              {s.name}
              {v ? `: ${fmt(v.value)}` : ""}
            </title>
          </path>
        );
      })}
    </svg>
  );
}

// --- host map ----------------------------------------------------------------------------

/** Hosts as tiles colored by a metric (`avg:system.cpu.user{*}`), grouped
 * by tags. */
export function HostMapView({
  query,
  groupBy = [],
  ctx,
  height,
}: {
  query: string;
  groupBy?: string[];
  ctx: Ctx;
  height?: number;
}) {
  const byHost = /\bby\s*\{[^}]*\bhost\b/.test(query)
    ? query
    : `${query.replace(/\s+by\s*\{[^}]*\}\s*$/, "")} by {host}`;
  const req: WidgetRequest = {
    queries: [
      { data_source: "metrics", name: "a", query: byHost, aggregator: "avg" },
    ],
    formulas: [{ formula: "a" }],
  };
  const res = useScalar(req, ctx);
  const hosts = useQuery({
    queryKey: ["hosts"],
    queryFn: api.hosts,
    staleTime: 60_000,
  });
  const groups = useMemo(() => {
    const values = new Map<string, number>();
    for (const r of res.data ? table(res.data) : [])
      if (r.values[0] !== null) values.set(r.tags.host ?? r.label, r.values[0]);
    const names = new Set([
      ...(hosts.data ?? []).map((h) => h.name),
      ...values.keys(),
    ]);
    const tagsOf = new Map((hosts.data ?? []).map((h) => [h.name, h.tags]));
    const out = new Map<string, { host: string; value: number | null }[]>();
    for (const h of [...names].sort()) {
      const tags = tagsOf.get(h) ?? [];
      const key =
        groupBy
          .map((g) => tags.find((t) => t.startsWith(`${g}:`)) ?? `${g}:N/A`)
          .join(", ") || "all hosts";
      out.set(key, [
        ...(out.get(key) ?? []),
        { host: h, value: values.get(h) ?? null },
      ]);
    }
    const all = [...values.values()];
    return {
      out: [...out.entries()].sort(),
      lo: Math.min(...all),
      hi: Math.max(...all),
    };
  }, [res.data, hosts.data, groupBy]);
  if (res.error) return <Failed e={res.error} />;
  if (!res.data || !hosts.data) return <Loading />;
  if (!groups.out.length) return <Empty>No host</Empty>;
  const t = (v: number) =>
    groups.hi > groups.lo ? (v - groups.lo) / (groups.hi - groups.lo) : 0.5;
  return (
    <div className="w-hostmap" style={height ? { height } : undefined}>
      {groups.out.map(([g, hs]) => (
        <div key={g} className="w-hostmap-group">
          {(groups.out.length > 1 || groupBy.length > 0) && (
            <div className="w-hostmap-title">{g}</div>
          )}
          <div className="w-hostmap-tiles">
            {hs.map((h) => (
              <Link
                key={h.host}
                to={`/infrastructure?host=${encodeURIComponent(h.host)}`}
                className="w-hostmap-tile"
                style={{
                  background:
                    h.value === null ? "var(--bg-subtle)" : heat(t(h.value)),
                }}
                title={`${h.host}: ${h.value === null ? "no data" : fmt(h.value)}`}
              />
            ))}
          </div>
        </div>
      ))}
      <div className="w-hostmap-legend faint">
        {fmt(groups.lo)} <span className="w-hostmap-scale" /> {fmt(groups.hi)}
      </div>
    </div>
  );
}

function Hostmap({
  def,
  ctx,
  height,
}: {
  def: WidgetDef;
  ctx: Ctx;
  height: number;
}) {
  const reqs = def.requests as unknown as { fill?: { q?: string } } | undefined;
  const q = reqs?.fill?.q ?? "avg:system.cpu.user{*} by {host}";
  const scope =
    (def.scope as string[] | undefined)?.filter((s) => s !== "*") ?? [];
  const withScope = scope.length
    ? q.replace(
        /\{([^}]*)\}/,
        (_, f) =>
          `{${[f === "*" ? "" : f, ...scope].filter(Boolean).join(",")}}`,
      )
    : q;
  const query = buildRequest(
    {
      queries: [{ data_source: "metrics", name: "a", query: withScope }],
      formulas: [],
    },
    ctx.defs,
    ctx.vars,
    0,
    0,
  ).queries[0].query!;
  return (
    <HostMapView
      query={query}
      groupBy={(def.group as string[]) ?? []}
      ctx={ctx}
      height={height}
    />
  );
}

// --- streams -----------------------------------------------------------------------------

/** A log's field: `message`, `service`, `@http.status_code`… */
function field(l: Log, f: string): string {
  const k = f.replace(/^@/, "");
  const top: Record<string, unknown> = {
    timestamp: fullDate(l.timestamp),
    status: l.status,
    service: l.service,
    host: l.host,
    source: l.source,
    message: l.message,
    content: l.message,
  };
  if (k in top && !f.startsWith("@")) return String(top[k] ?? "");
  let v: unknown = l.attributes;
  for (const p of k.split("."))
    v =
      v && typeof v === "object"
        ? (v as Record<string, unknown>)[p]
        : undefined;
  if (v === undefined) v = (l.attributes as Record<string, unknown>)[k];
  return v === undefined || v === null
    ? ""
    : typeof v === "object"
      ? JSON.stringify(v)
      : String(v);
}

type StreamQuery = { data_source?: string; query_string?: string };

function ListStream({ def, ctx }: { def: WidgetDef; ctx: Ctx }) {
  const r = def.requests?.[0] as
    | (WidgetRequest & { query?: StreamQuery; columns?: { field: string }[] })
    | undefined;
  const src = r?.query?.data_source ?? "logs_stream";
  const query =
    buildRequest(
      {
        queries: [
          {
            data_source: "logs",
            name: "a",
            search: { query: r?.query?.query_string ?? "" },
          },
        ],
        formulas: [],
      },
      ctx.defs,
      ctx.vars,
      0,
      0,
    ).queries[0].search?.query ?? "";
  const { time } = ctx;
  const scope = { query, from: time.from, to: time.to, limit: 50 };
  const res = useQuery({
    queryKey: ["w-stream", src, query, time.from, time.to],
    queryFn: async ({ signal }) => {
      if (src.includes("issue")) {
        const out = await api.errorIssues(
          { from: time.from, to: time.to, query },
          signal,
        );
        return { issues: out.issues };
      }
      if (src === "trace_stream")
        return { spans: (await api.searchSpans(scope, signal)).spans };
      if (src === "rum_stream")
        return { logs: (await api.searchRum(scope, signal)).logs };
      if (src === "event_stream")
        return { logs: (await api.searchEvents(scope, signal)).logs };
      return { logs: (await api.searchLogs(scope, signal)).logs };
    },
    placeholderData: keepPreviousData,
    refetchInterval: 60_000,
  });
  if (res.error) return <Failed e={res.error} />;
  if (!res.data) return <Loading />;
  const d = res.data as {
    logs?: Log[];
    spans?: import("../lib/api").Span[];
    issues?: import("../lib/api").ErrorIssue[];
  };
  if (d.issues)
    return d.issues.length ? (
      <div className="widget-table">
        <table className="list">
          <tbody>
            {d.issues.map((i) => (
              <tr key={i.id}>
                <td>
                  <Link to={`/error-tracking/issue/${i.id}`}>{i.type}</Link>{" "}
                  <span className="faint ellipsis">{i.message}</span>
                </td>
                <td className="mono">{i.service}</td>
                <td className="num">{i.count}</td>
                <td className="faint">{ago(i.last_seen)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    ) : (
      <Empty />
    );
  if (d.spans)
    return d.spans.length ? (
      <div className="widget-table">
        <table className="list">
          <tbody>
            {d.spans.map((s) => (
              <tr key={s.id}>
                <td className="faint">{fullDate(s.start / 1e6)}</td>
                <td className="mono">{s.service}</td>
                <td className="ellipsis">
                  <Link to={`/apm/traces?trace=${s.trace_id}`}>
                    {s.resource}
                  </Link>
                </td>
                <td className={`num${s.error ? " w-bad" : ""}`}>
                  {nsDuration(s.duration)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    ) : (
      <Empty />
    );
  const logs = d.logs ?? [];
  if (!logs.length) return <Empty />;
  const cols = (r?.columns ?? [])
    .map((c) => c.field)
    .filter((f) => f !== "status_line" && f !== "timestamp");
  const fields = cols.length
    ? cols
    : src === "rum_stream"
      ? ["@type", "@view.name", "service"]
      : ["service", "content"];
  return (
    <div className="widget-table">
      <table className="list">
        <tbody>
          {logs.map((l) => (
            <tr key={l.id}>
              <td
                style={{
                  width: 4,
                  padding: 0,
                  background: statusColor(l.status),
                }}
              />
              <td className="faint" style={{ whiteSpace: "nowrap" }}>
                {fullDate(l.timestamp)}
              </td>
              {fields.map((f) => (
                <td
                  key={f}
                  className={
                    f === "content" || f === "message" ? "ellipsis" : "mono"
                  }
                >
                  {field(l, f)}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function EventStream({
  def,
  ctx,
  height,
}: {
  def: WidgetDef;
  ctx: Ctx;
  height: number;
}) {
  const { time } = ctx;
  const query = (def.query as string) ?? "";
  const timeline = def.type === "event_timeline";
  type Out =
    | { h: Awaited<ReturnType<typeof api.eventsHistogram>> }
    | { e: Awaited<ReturnType<typeof api.searchEvents>> };
  const res = useQuery<Out>({
    queryKey: ["w-events", def.type, query, time.from, time.to],
    queryFn: async ({ signal }) =>
      timeline
        ? {
            h: await api.eventsHistogram(
              { query, from: time.from, to: time.to },
              signal,
            ),
          }
        : {
            e: await api.searchEvents(
              { query, from: time.from, to: time.to, limit: 50 },
              signal,
            ),
          },
    placeholderData: keepPreviousData,
    refetchInterval: 60_000,
  });
  if (res.error) return <Failed e={res.error} />;
  if (!res.data) return <Loading />;
  if ("h" in res.data) {
    const h = res.data.h;
    const statuses = [
      ...new Set(h.buckets.flatMap((b) => Object.keys(b.counts))),
    ];
    const series: Series[] = statuses.map((s) => ({
      key: s,
      label: s,
      color: statusColor(s),
      values: h.buckets.map((b) => b.counts[s] ?? 0),
    }));
    return (
      <TimeChart
        times={h.buckets.map((b) => b.t)}
        interval={h.interval_ms}
        from={time.from}
        to={time.to}
        series={series}
        height={height}
        onZoom={time.setAbsolute}
      />
    );
  }
  const events = res.data.e.logs;
  if (!events.length) return <Empty>No event</Empty>;
  const small = def.event_size === "s";
  return (
    <div className="w-events">
      {events.map((e) => {
        const evt = (e.attributes.evt ?? {}) as {
          title?: string;
          text?: string;
        };
        return (
          <div key={e.id} className="w-event">
            <i style={{ background: statusColor(e.status) }} />
            <div>
              <strong>{evt.title ?? e.message}</strong>{" "}
              <span className="faint">{ago(e.timestamp)}</span>
              {!small && evt.text && (
                <div className="faint w-event-text">{evt.text}</div>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}

// --- SLOs and monitors -------------------------------------------------------------------

function SloWidget({ def }: { def: WidgetDef }) {
  const id = def.slo_id as string | undefined;
  const slo = useQuery({
    queryKey: ["slo", id],
    queryFn: () => api.slo(id!),
    enabled: !!id,
  });
  const status = useQuery({
    queryKey: ["sloStatus", id],
    queryFn: () => api.sloStatus(id!),
    enabled: !!id,
    refetchInterval: 120_000,
  });
  if (!id) return <Empty>No SLO chosen</Empty>;
  if (slo.error) return <Failed e={slo.error} />;
  if (!slo.data || !status.data) return <Loading />;
  const windows = (def.time_windows as string[] | undefined) ?? [];
  const shown = status.data.filter(
    (s) => !windows.length || windows.includes(s.timeframe),
  );
  return (
    <div className="w-slo">
      <Link to={`/slo/${id}`} className="w-slo-name">
        {slo.data.data.name}
      </Link>
      <div className="w-slo-tiles">
        {shown.map((s) => (
          <div key={s.timeframe} className="w-slo-tile">
            <div className="faint">
              {s.timeframe} · target {s.target} %
            </div>
            <div
              className={`w-slo-value ${s.state === "breached" ? "slo-breached" : s.state === "warning" ? "slo-warning" : s.state === "ok" ? "slo-ok" : "muted"}`}
            >
              {s.sli === null
                ? "No data"
                : `${s.sli.toFixed(s.sli >= 99.95 ? 3 : 2)} %`}
            </div>
            {def.show_error_budget !== false &&
              s.error_budget_remaining !== null && (
                <div className="faint">
                  {s.error_budget_remaining.toFixed(0)} % budget left
                </div>
              )}
          </div>
        ))}
      </div>
    </div>
  );
}

const MONITOR_COLORS: Record<string, string> = {
  Alert: "#d8616a",
  Warn: "#e5a24a",
  "No Data": "#8f8794",
  OK: "#55ab83",
};

function ManageStatus({ def }: { def: WidgetDef }) {
  const query = (def.query as string) ?? "";
  const res = useQuery({
    queryKey: ["w-monitors", query],
    queryFn: () => api.monitorSearch(query),
    refetchInterval: 60_000,
  });
  if (res.error) return <Failed e={res.error} />;
  if (!res.data) return <Loading />;
  const counts = new Map(
    (res.data.counts.status ?? []).map((c) => [String(c.name), c.count]),
  );
  const format = (def.display_format as string) ?? "countsAndList";
  const hideZero = def.hide_zero_counts === true;
  const order = ["Alert", "Warn", "No Data", "OK"];
  const list = [...res.data.monitors].sort(
    (a, b) => order.indexOf(a.status) - order.indexOf(b.status),
  );
  return (
    <div className="w-status">
      {format !== "list" && (
        <div className="w-status-counts">
          {order
            .filter((s) => !hideZero || counts.get(s))
            .map((s) => (
              <div
                key={s}
                className="w-status-count"
                style={{ borderColor: MONITOR_COLORS[s] }}
              >
                <strong style={{ color: MONITOR_COLORS[s] }}>
                  {counts.get(s) ?? 0}
                </strong>
                <span className="faint">{s}</span>
              </div>
            ))}
        </div>
      )}
      {format !== "counts" && (
        <div className="widget-table">
          <table className="list">
            <tbody>
              {list.slice(0, 50).map((m) => (
                <tr key={m.id}>
                  <td style={{ width: 70 }}>
                    <span
                      className="w-pill"
                      style={{
                        background: MONITOR_COLORS[m.status] ?? "#8f8794",
                      }}
                    >
                      {m.status}
                    </span>
                  </td>
                  <td className="ellipsis">
                    <Link to={`/monitors/${m.id}`}>{m.name}</Link>
                  </td>
                  {def.show_last_triggered === true && (
                    <td className="faint">
                      {m.last_triggered_ts
                        ? ago(m.last_triggered_ts * 1000)
                        : ""}
                    </td>
                  )}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

function AlertGraph({
  def,
  ctx,
  height,
}: {
  def: WidgetDef;
  ctx: Ctx;
  height: number;
}) {
  const id = Number(def.alert_id);
  const { time } = ctx;
  const monitor = useQuery({
    queryKey: ["monitor", id],
    queryFn: () => api.monitor(id),
    enabled: !!id,
  });
  const history = useQuery({
    queryKey: ["monitorHistory", id, time.from, time.to],
    queryFn: () => api.monitorHistory(id, time.from, time.to),
    enabled: !!id,
    placeholderData: keepPreviousData,
  });
  if (!id) return <Empty>No monitor chosen</Empty>;
  if (monitor.error || history.error)
    return <Failed e={monitor.error ?? history.error} />;
  if (!monitor.data || !history.data) return <Loading />;
  const interval = 60_000;
  const start = Math.floor(time.from / interval) * interval;
  const times: number[] = [];
  for (let t = start; t <= time.to; t += interval) times.push(t);
  const series: Series[] = history.data.slice(0, 20).map((s, i) => {
    const at = new Map<number, number>();
    for (const [t, v] of s.points)
      if (v !== null) at.set(Math.floor(t / interval) * interval, v);
    return {
      key: s.group,
      label: s.group === "*" ? monitor.data.name : s.group,
      color: PALETTE[i % PALETTE.length],
      values: times.map((t) => at.get(t) ?? NaN),
    };
  });
  const th = monitor.data.options.thresholds;
  return (
    <TimeChart
      times={times}
      interval={interval}
      from={time.from}
      to={time.to}
      series={series}
      kind="lines"
      height={height}
      format={(v) => fmt(v)}
      onZoom={time.setAbsolute}
      markers={[
        th.critical !== undefined && {
          value: th.critical,
          color: "#d8616a",
          label: `Alert ${fmt(th.critical)}`,
        },
        th.warning !== undefined && {
          value: th.warning,
          color: "#e5a24a",
          label: `Warn ${fmt(th.warning)}`,
        },
      ].filter(
        (m): m is { value: number; color: string; label: string } => !!m,
      )}
    />
  );
}

function AlertValue({ def }: { def: WidgetDef }) {
  const id = Number(def.alert_id);
  const monitor = useQuery({
    queryKey: ["monitor", id],
    queryFn: () => api.monitor(id),
    enabled: !!id,
    refetchInterval: 60_000,
  });
  if (!id) return <Empty>No monitor chosen</Empty>;
  if (monitor.error) return <Failed e={monitor.error} />;
  if (!monitor.data) return <Loading />;
  const values = Object.values(monitor.data.state.groups)
    .map((g) => g.value)
    .filter((v): v is number => v !== null);
  const v = values.length ? Math.max(...values) : null;
  return (
    <div
      className="query-value"
      style={{
        color: MONITOR_COLORS[monitor.data.overall_state],
        justifyContent:
          (def.text_align as string) === "left"
            ? "flex-start"
            : (def.text_align as string) === "right"
              ? "flex-end"
              : "center",
      }}
    >
      {fmt(v, def.precision)}
      {def.unit ? (
        <span className="query-value-unit">{def.unit as string}</span>
      ) : null}
    </div>
  );
}

const CHECK_COLORS: Record<string, string> = {
  OK: "#55ab83",
  WARNING: "#e5a24a",
  CRITICAL: "#d8616a",
  UNKNOWN: "#8f8794",
};

function CheckStatus({ def }: { def: WidgetDef }) {
  const res = useQuery({
    queryKey: ["checks"],
    queryFn: api.checks,
    refetchInterval: 30_000,
  });
  if (res.error) return <Failed e={res.error} />;
  if (!res.data) return <Loading />;
  const tags = ((def.tags as string[]) ?? []).filter((t) => t !== "*");
  const group = def.group as string | undefined;
  const runs = res.data.filter(
    (c) =>
      c.check === def.check &&
      tags.every(
        (t) =>
          c.tags.includes(t) ||
          (t.startsWith("host:") && c.host === t.slice(5)),
      ) &&
      (!group || c.tags.includes(group) || `host:${c.host}` === group),
  );
  if (!runs.length)
    return <Empty>No run of {String(def.check ?? "the check")}</Empty>;
  if (def.grouping === "cluster") {
    const counts = new Map<string, number>();
    for (const r of runs) counts.set(r.status, (counts.get(r.status) ?? 0) + 1);
    return (
      <div className="w-status-counts">
        {["CRITICAL", "WARNING", "UNKNOWN", "OK"].map((s) => (
          <div
            key={s}
            className="w-status-count"
            style={{ borderColor: CHECK_COLORS[s] }}
          >
            <strong style={{ color: CHECK_COLORS[s] }}>
              {counts.get(s) ?? 0}
            </strong>
            <span className="faint">{s}</span>
          </div>
        ))}
      </div>
    );
  }
  const order = ["CRITICAL", "WARNING", "UNKNOWN", "OK"];
  const worst = runs.reduce((a, r) =>
    order.indexOf(r.status) < order.indexOf(a.status) ? r : a,
  );
  return (
    <div
      className="query-value"
      style={{ color: CHECK_COLORS[worst.status] }}
      title={worst.message}
    >
      {worst.status}
    </div>
  );
}

function TraceService({
  def,
  ctx,
  height,
}: {
  def: WidgetDef;
  ctx: Ctx;
  height: number;
}) {
  const { time } = ctx;
  const service = def.service as string | undefined;
  const env = (def.env as string | undefined) || undefined;
  const res = useQuery({
    queryKey: ["service", service, time.from, time.to, env],
    queryFn: () => api.service(service!, time.from, time.to, env),
    enabled: !!service,
    placeholderData: keepPreviousData,
    refetchInterval: 60_000,
  });
  if (!service) return <Empty>No service chosen</Empty>;
  if (res.error) return <Failed e={res.error} />;
  if (!res.data) return <Loading />;
  const s = res.data.summary;
  const ts = res.data.timeseries;
  const times = ts.map((p) => p.t);
  const chart = (
    label: string,
    values: number[],
    color: string,
    format?: (v: number) => string,
  ) => (
    <div className="w-trace-chart">
      <div className="faint">{label}</div>
      <TimeChart
        times={times}
        interval={res.data.interval_ms}
        from={time.from}
        to={time.to}
        series={[{ key: label, label, color, values }]}
        height={Math.max(50, height - 90)}
        format={format}
      />
    </div>
  );
  return (
    <div className="w-trace">
      <div className="w-trace-stats">
        <Link to={`/apm/services/${encodeURIComponent(service)}`}>
          <strong>{service}</strong>
        </Link>
        <span>
          {fmt(s.hits)} <span className="faint">requests</span>
        </span>
        <span>
          {(s.error_rate * 100).toFixed(2)} %{" "}
          <span className="faint">errors</span>
        </span>
        <span>
          {nsDuration(s.p50)} <span className="faint">p50</span>
        </span>
        <span>
          {nsDuration(s.p95)} <span className="faint">p95</span>
        </span>
        <span>
          {nsDuration(s.p99)} <span className="faint">p99</span>
        </span>
      </div>
      <div className="w-trace-charts">
        {def.show_hits !== false &&
          chart(
            "Requests",
            ts.map((p) => p.hits),
            "#5a8dd3",
          )}
        {def.show_errors !== false &&
          chart(
            "Errors",
            ts.map((p) => p.errors),
            "#d8616a",
          )}
        {def.show_latency !== false &&
          chart(
            "Latency p95",
            ts.map((p) => p.p95),
            "#9a82c9",
            (v) => nsDuration(v),
          )}
      </div>
    </div>
  );
}

// --- RUM funnel --------------------------------------------------------------------------

function quoteValue(v: string) {
  return /^[\w./:*-]+$/.test(v) ? v : `"${v.replace(/"/g, '\\"')}"`;
}

export function funnelSteps(steps: { facet: string; value: string }[]) {
  return steps.map((s) => ({
    label: s.value,
    search: `${s.facet.startsWith("@view.") ? "@type:view " : s.facet.startsWith("@action.") ? "@type:action " : ""}${s.facet}:${quoteValue(s.value)}`,
  }));
}

function FunnelWidget({ def, ctx }: { def: WidgetDef; ctx: Ctx }) {
  const r = def.requests?.[0] as
    | (WidgetRequest & {
        query?: {
          query_string?: string;
          steps?: { facet: string; value: string }[];
        };
      })
    | undefined;
  const steps = funnelSteps(r?.query?.steps ?? []);
  const { time } = ctx;
  const query = r?.query?.query_string ?? "";
  const res = useQuery({
    queryKey: ["w-funnel", JSON.stringify(steps), query, time.from, time.to],
    queryFn: ({ signal }) =>
      api.rumFunnel({ from: time.from, to: time.to, query, steps }, signal),
    enabled: steps.length > 0,
    placeholderData: keepPreviousData,
    refetchInterval: 120_000,
  });
  if (!steps.length) return <Empty>No step</Empty>;
  if (res.error) return <Failed e={res.error} />;
  if (!res.data) return <Loading />;
  return <FunnelBars steps={res.data.steps} />;
}

export function FunnelBars({
  steps,
}: {
  steps: {
    label: string;
    sessions: number;
    conversion: number;
    dropoff: number;
  }[];
}) {
  return (
    <div className="w-funnel">
      {steps.map((s, i) => (
        <div key={i} className="w-funnel-step">
          <div className="w-funnel-label ellipsis" title={s.label}>
            {i + 1}. {s.label}
          </div>
          <div className="w-funnel-track">
            <div
              className="w-funnel-bar"
              style={{ width: `${Math.max(0.5, s.conversion)}%` }}
            />
          </div>
          <div className="w-funnel-num">
            <strong>{fmt(s.sessions)}</strong>{" "}
            <span className="faint">{s.conversion.toFixed(1)} %</span>
            {i > 0 && s.dropoff > 0 && (
              <span className="w-bad"> −{fmt(s.dropoff)}</span>
            )}
          </div>
        </div>
      ))}
    </div>
  );
}

// --- split graph -------------------------------------------------------------------------

/** `avg:cpu{env:prod} by {host}` with `host:a`: `avg:cpu{env:prod,host:a}`. */
export function withFilter(query: string, tag: string, value: string): string {
  const without = query.replace(/\s+by\s*\{[^}]*\}/, "");
  return without.replace(/\{([^}]*)\}/, (_, f: string) => {
    const parts = f
      .split(",")
      .map((s) => s.trim())
      .filter((s) => s && s !== "*");
    return `{${[...parts, `${tag}:${value}`].join(",")}}`;
  });
}

function SplitGraph({
  def,
  ctx,
  height,
}: {
  def: WidgetDef;
  ctx: Ctx;
  height: number;
}) {
  const source = def.source_widget_definition as WidgetDef | undefined;
  const cfg = def.split_config as
    | {
        split_dimensions?: { one_graph_per: string }[];
        limit?: number;
        static_splits?: { tag_key: string; tag_values: string[] }[][];
      }
    | undefined;
  const tag = cfg?.split_dimensions?.[0]?.one_graph_per;
  const req = source?.requests?.[0];
  const metric = req
    ? normalizeRequest(req).queries.find((q) => q.data_source === "metrics")
    : undefined;
  const probe: WidgetRequest | undefined =
    metric && tag
      ? {
          queries: [
            {
              ...metric,
              name: "a",
              aggregator: "avg",
              query: `${metric.query!.replace(/\s+by\s*\{[^}]*\}/, "")} by {${tag}}`,
            },
          ],
          formulas: [
            { formula: "a", limit: { count: cfg?.limit ?? 24, order: "desc" } },
          ],
        }
      : undefined;
  const res = useScalar(cfg?.static_splits?.length ? undefined : probe, ctx);
  if (!source || !tag)
    return <Empty>Needs a source widget and a tag to split by</Empty>;
  if (res.error) return <Failed e={res.error} />;
  const values = cfg?.static_splits?.length
    ? cfg.static_splits.map((s) => s[0]?.tag_values[0]).filter(Boolean)
    : res.data
      ? table(res.data)
          .map((r) => r.tags[tag])
          .filter(Boolean)
          .slice(0, cfg?.limit ?? 24)
      : null;
  if (!values) return <Loading />;
  if (!values.length) return <Empty />;
  const size = (def.size as string) ?? "md";
  const per = size === "xs" ? 6 : size === "sm" ? 4 : size === "lg" ? 2 : 3;
  const cellH = Math.max(90, Math.min(height, 220) - 20);
  return (
    <div
      className="w-split"
      style={{ gridTemplateColumns: `repeat(${per}, minmax(0, 1fr))` }}
    >
      {values.map((v) => {
        const child: WidgetDef = {
          ...source,
          title: `${tag}:${v}`,
          requests: source.requests?.map((r) => {
            const n = normalizeRequest(r);
            return {
              ...r,
              q: undefined,
              queries: n.queries.map((q) =>
                q.data_source === "metrics"
                  ? { ...q, query: withFilter(q.query ?? "", tag, v!) }
                  : q,
              ),
              formulas: n.formulas,
            };
          }),
        };
        return (
          <div key={v} className="w-split-cell">
            <div className="w-split-title mono ellipsis">
              {tag}:{v}
            </div>
            <ExtraOrBase def={child} ctx={ctx} height={cellH} />
          </div>
        );
      })}
    </div>
  );
}

// The base widgets live in Dashboards.tsx; it registers its renderer here
// so split graphs can draw them.
let baseRenderer:
  ((p: { def: WidgetDef; ctx: Ctx; height: number }) => ReactNode) | null =
  null;
export function registerBaseRenderer(f: typeof baseRenderer) {
  baseRenderer = f;
}

function ExtraOrBase(p: { def: WidgetDef; ctx: Ctx; height: number }) {
  return isExtra(p.def.type) ? (
    <ExtraWidget {...p} />
  ) : (
    <>{baseRenderer?.(p)}</>
  );
}

// --- simple widgets ----------------------------------------------------------------------

function ImageWidget({ def }: { def: WidgetDef }) {
  const url = def.url as string | undefined;
  if (!url) return <Empty>No image URL</Empty>;
  const sizing = (def.sizing as string) ?? "contain";
  const fit =
    sizing === "zoom"
      ? "cover"
      : sizing === "fit"
        ? "contain"
        : sizing === "center"
          ? "none"
          : sizing;
  return (
    <img
      src={url}
      alt=""
      className="w-image"
      style={{
        objectFit: fit as "cover",
        objectPosition: (def.horizontal_align as string) ?? "center",
      }}
    />
  );
}

function IframeWidget({ def }: { def: WidgetDef }) {
  const url = def.url as string | undefined;
  if (!url || !/^https?:\/\//.test(url))
    return <Empty>An http(s) URL is needed</Empty>;
  return (
    <iframe
      src={url}
      className="w-iframe"
      title={def.title ?? url}
      sandbox="allow-scripts allow-forms allow-popups"
      referrerPolicy="no-referrer"
    />
  );
}

function FreeText({ def }: { def: WidgetDef }) {
  const size = def.font_size as string | undefined;
  return (
    <div
      className="w-free-text"
      style={{
        color: (def.color as string) || undefined,
        fontSize: size && size !== "auto" ? `${size}px` : undefined,
        textAlign: ((def.text_align as string) ?? "left") as "left",
      }}
    >
      {def.text as string}
    </div>
  );
}

// --- registry ----------------------------------------------------------------------------

export const EXTRA_TYPES: Record<string, string> = {
  heatmap: "Heatmap",
  distribution: "Distribution",
  sunburst: "Pie Chart",
  treemap: "Treemap",
  scatterplot: "Scatter Plot",
  change: "Change",
  geomap: "Geomap",
  hostmap: "Host Map",
  list_stream: "List",
  event_stream: "Event Stream",
  event_timeline: "Event Timeline",
  slo: "SLO",
  manage_status: "Monitor Summary",
  alert_graph: "Alert Graph",
  alert_value: "Alert Value",
  check_status: "Check Status",
  trace_service: "Service Summary",
  funnel: "Funnel",
  split_graph: "Split Graph",
  free_text: "Free Text",
  image: "Image",
  iframe: "IFrame",
};

/** Widgets drawn without a title bar. */
export const BARE_TYPES = ["free_text", "image", "iframe"];

/** Widgets whose requests are edited with the query rows. */
export const QUERY_TYPES: Record<string, "timeseries" | "scalar"> = {
  heatmap: "timeseries",
  distribution: "scalar",
  sunburst: "scalar",
  treemap: "scalar",
  change: "scalar",
  geomap: "scalar",
};

export const isExtra = (type: string) => type in EXTRA_TYPES;

export function ExtraWidget({
  def,
  ctx,
  height,
}: {
  def: WidgetDef;
  ctx: Ctx;
  height: number;
}) {
  switch (def.type) {
    case "heatmap":
      return <Heatmap def={def} ctx={ctx} height={height} />;
    case "distribution":
      return <Distribution def={def} ctx={ctx} height={height} />;
    case "sunburst":
      return <Sunburst def={def} ctx={ctx} height={height} />;
    case "treemap":
      return <Treemap def={def} ctx={ctx} height={height} />;
    case "scatterplot":
      return <Scatter def={def} ctx={ctx} height={height} />;
    case "change":
      return <Change def={def} ctx={ctx} />;
    case "geomap":
      return <Geomap def={def} ctx={ctx} height={height} />;
    case "hostmap":
      return <Hostmap def={def} ctx={ctx} height={height} />;
    case "list_stream":
      return <ListStream def={def} ctx={ctx} />;
    case "event_stream":
    case "event_timeline":
      return <EventStream def={def} ctx={ctx} height={height} />;
    case "slo":
      return <SloWidget def={def} />;
    case "manage_status":
      return <ManageStatus def={def} />;
    case "alert_graph":
      return <AlertGraph def={def} ctx={ctx} height={height} />;
    case "alert_value":
      return <AlertValue def={def} />;
    case "check_status":
      return <CheckStatus def={def} />;
    case "trace_service":
      return <TraceService def={def} ctx={ctx} height={height} />;
    case "funnel":
      return <FunnelWidget def={def} ctx={ctx} />;
    case "split_graph":
      return <SplitGraph def={def} ctx={ctx} height={height} />;
    case "image":
      return <ImageWidget def={def} />;
    case "iframe":
      return <IframeWidget def={def} />;
    case "free_text":
      return <FreeText def={def} />;
    default:
      return <Empty>The {def.type} widget is not supported yet.</Empty>;
  }
}

const metricReq = (query: string, scalar: boolean): WidgetRequest => ({
  response_format: scalar ? "scalar" : "timeseries",
  queries: [
    {
      data_source: "metrics",
      name: "a",
      query,
      ...(scalar ? { aggregator: "avg" } : {}),
    },
  ],
  formulas: [{ formula: "a" }],
});

/** A new widget's definition. */
export function extraBlank(type: string): WidgetDef {
  switch (type) {
    case "heatmap":
      return {
        type,
        title: "",
        requests: [metricReq("avg:system.cpu.user{*} by {host}", false)],
      };
    case "distribution":
      return {
        type,
        title: "",
        requests: [
          {
            response_format: "scalar",
            queries: [
              {
                data_source: "spans",
                name: "a",
                search: { query: "" },
                compute: { aggregation: "count", metric: "@duration" },
              },
            ],
            formulas: [{ formula: "a" }],
          },
        ],
      };
    case "sunburst":
    case "treemap":
    case "geomap":
      return {
        type,
        title: "",
        requests: [
          type === "geomap"
            ? {
                response_format: "scalar",
                queries: [
                  {
                    data_source: "rum",
                    name: "a",
                    search: { query: "@type:view" },
                    compute: { aggregation: "count" },
                    group_by: [{ facet: "@geo.country_iso_code", limit: 250 }],
                  },
                ],
                formulas: [{ formula: "a" }],
              }
            : metricReq(
                "sum:shop.orders.placed{*} by {payment_method}.as_count()",
                true,
              ),
        ],
        ...(type === "geomap" ? { view: { focus: "WORLD" } } : {}),
      };
    case "change":
      return {
        type,
        title: "",
        requests: [
          {
            ...metricReq("avg:system.cpu.user{*} by {host}", true),
            compare_to: "day_before",
            change_type: "relative",
            increase_good: false,
            order_by: "change",
            order_dir: "desc",
            show_present: true,
          },
        ],
      };
    case "scatterplot":
      return {
        type,
        title: "",
        requests: {
          table: {
            queries: [
              {
                data_source: "metrics",
                name: "a",
                query: "avg:system.cpu.user{*} by {host}",
                aggregator: "avg",
              },
              {
                data_source: "metrics",
                name: "b",
                query: "avg:system.mem.used{*} by {host}",
                aggregator: "avg",
              },
            ],
            formulas: [
              { formula: "a", dimension: "x" },
              { formula: "b", dimension: "y" },
            ],
          },
        } as unknown as WidgetRequest[],
      };
    case "hostmap":
      return {
        type,
        title: "",
        requests: {
          fill: { q: "avg:system.cpu.user{*} by {host}" },
        } as unknown as WidgetRequest[],
        node_type: "host",
        group: [],
        scope: [],
      };
    case "list_stream":
      return {
        type,
        title: "",
        requests: [
          {
            response_format: "event_list",
            columns: [
              { field: "status_line", width: "auto" },
              { field: "timestamp", width: "auto" },
              { field: "service", width: "auto" },
              { field: "content", width: "auto" },
            ],
            query: {
              data_source: "logs_stream",
              query_string: "status:error",
              indexes: [],
            },
          },
        ],
      };
    case "event_stream":
      return { type, title: "", query: "", event_size: "l" };
    case "event_timeline":
      return { type, title: "", query: "" };
    case "slo":
      return {
        type,
        title: "",
        slo_id: "",
        view_type: "detail",
        time_windows: ["7d", "30d"],
        show_error_budget: true,
        view_mode: "overall",
      };
    case "manage_status":
      return {
        type,
        title: "",
        query: "",
        display_format: "countsAndList",
        summary_type: "monitors",
        sort: "status,asc",
        color_preference: "text",
        hide_zero_counts: false,
        show_last_triggered: true,
      };
    case "alert_graph":
      return { type, title: "", alert_id: "", viz_type: "timeseries" };
    case "alert_value":
      return {
        type,
        title: "",
        alert_id: "",
        precision: 2,
        unit: "",
        text_align: "center",
      };
    case "check_status":
      return {
        type,
        title: "",
        check: "datadog.agent.up",
        grouping: "cluster",
        tags: ["*"],
        group_by: [],
      };
    case "trace_service":
      return {
        type,
        title: "",
        service: "",
        env: "",
        span_name: "",
        show_hits: true,
        show_errors: true,
        show_latency: true,
      };
    case "funnel":
      return {
        type,
        title: "",
        requests: [
          {
            request_type: "funnel",
            query: {
              data_source: "rum",
              query_string: "",
              steps: [
                { facet: "@view.name", value: "/" },
                { facet: "@view.name", value: "/cart" },
              ],
            },
          },
        ],
      };
    case "split_graph":
      return {
        type,
        title: "",
        size: "md",
        has_uniform_y_axes: true,
        source_widget_definition: {
          type: "timeseries",
          requests: [metricReq("avg:system.cpu.user{*}", false)],
        },
        split_config: {
          split_dimensions: [{ one_graph_per: "host" }],
          limit: 12,
          sort: {
            order: "desc",
            compute: { aggregation: "avg", metric: "system.cpu.user" },
          },
          static_splits: [],
        },
      };
    case "image":
      return {
        type,
        url: "",
        sizing: "contain",
        has_background: true,
        has_border: true,
      };
    case "iframe":
      return { type, url: "" };
    case "free_text":
      return {
        type,
        text: "Text",
        color: "#4a4350",
        font_size: "auto",
        text_align: "left",
      };
    default:
      return { type, title: "" };
  }
}

// --- editor fields -----------------------------------------------------------------------

function Text({
  label,
  value,
  onChange,
  placeholder,
  mono,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  mono?: boolean;
}) {
  return (
    <label className="form-field">
      <span className="form-label">{label}</span>
      <input
        className={`input${mono ? " mono" : ""}`}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
      />
    </label>
  );
}

function Choice({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string;
  options: [string, string][];
  onChange: (v: string) => void;
}) {
  return (
    <label className="form-field" style={{ flex: "0 0 200px" }}>
      <span className="form-label">{label}</span>
      <select
        className="input"
        value={value}
        onChange={(e) => onChange(e.target.value)}
      >
        {options.map(([v, l]) => (
          <option key={v} value={v}>
            {l}
          </option>
        ))}
      </select>
    </label>
  );
}

/** The settings of the widgets not made of query rows. */
export function ExtraFields({
  def,
  setDef,
}: {
  def: WidgetDef;
  setDef: (patch: Partial<WidgetDef>) => void;
}) {
  const monitors = useQuery({
    queryKey: ["monitors"],
    queryFn: () => api.monitors(),
    enabled: def.type === "alert_graph" || def.type === "alert_value",
  });
  const slos = useQuery({
    queryKey: ["slos"],
    queryFn: api.slos,
    enabled: def.type === "slo",
  });
  const s = (k: string) => (def[k] as string | undefined) ?? "";
  switch (def.type) {
    case "image":
    case "iframe":
      return (
        <>
          <Text
            label="URL"
            value={s("url")}
            onChange={(url) => setDef({ url })}
            placeholder="https://…"
            mono
          />
          {def.type === "image" && (
            <Choice
              label="Sizing"
              value={s("sizing") || "contain"}
              options={[
                ["contain", "Fit"],
                ["cover", "Zoom"],
                ["fill", "Stretch"],
                ["none", "Center"],
              ]}
              onChange={(sizing) => setDef({ sizing })}
            />
          )}
        </>
      );
    case "free_text":
      return (
        <>
          <Text
            label="Text"
            value={s("text")}
            onChange={(text) => setDef({ text })}
          />
          <div className="form-row">
            <Text
              label="Color"
              value={s("color")}
              onChange={(color) => setDef({ color })}
              placeholder="#4a4350"
            />
            <Text
              label="Font size (px or auto)"
              value={s("font_size")}
              onChange={(font_size) => setDef({ font_size })}
            />
            <Choice
              label="Align"
              value={s("text_align") || "left"}
              options={[
                ["left", "Left"],
                ["center", "Center"],
                ["right", "Right"],
              ]}
              onChange={(text_align) => setDef({ text_align })}
            />
          </div>
        </>
      );
    case "event_stream":
    case "event_timeline":
      return (
        <Text
          label="Events matching"
          value={s("query")}
          onChange={(query) => setDef({ query })}
          placeholder="source:alert status:error"
          mono
        />
      );
    case "manage_status":
      return (
        <>
          <Text
            label="Monitors matching"
            value={s("query")}
            onChange={(query) => setDef({ query })}
            placeholder="tag:team:shop status:alert"
            mono
          />
          <Choice
            label="Display"
            value={s("display_format") || "countsAndList"}
            options={[
              ["countsAndList", "Counts and list"],
              ["counts", "Counts"],
              ["list", "List"],
            ]}
            onChange={(display_format) => setDef({ display_format })}
          />
        </>
      );
    case "alert_graph":
    case "alert_value":
      return (
        <Choice
          label="Monitor"
          value={s("alert_id")}
          options={[
            ["", "Choose a monitor"],
            ...(monitors.data ?? []).map(
              (m) => [String(m.id), `${m.id} ${m.name}`] as [string, string],
            ),
          ]}
          onChange={(alert_id) => setDef({ alert_id })}
        />
      );
    case "slo":
      return (
        <>
          <Choice
            label="SLO"
            value={s("slo_id")}
            options={[
              ["", "Choose an SLO"],
              ...(slos.data?.data ?? []).map(
                (x) => [x.id!, x.name] as [string, string],
              ),
            ]}
            onChange={(slo_id) => setDef({ slo_id })}
          />
          <label className="form-check">
            <input
              type="checkbox"
              checked={def.show_error_budget !== false}
              onChange={(e) => setDef({ show_error_budget: e.target.checked })}
            />{" "}
            Show the error budget
          </label>
        </>
      );
    case "check_status":
      return (
        <div className="form-row">
          <Text
            label="Check"
            value={s("check")}
            onChange={(check) => setDef({ check })}
            mono
          />
          <Text
            label="Tags"
            value={((def.tags as string[]) ?? []).join(",")}
            onChange={(v) =>
              setDef({
                tags: v
                  .split(",")
                  .map((t) => t.trim())
                  .filter(Boolean),
              })
            }
            placeholder="env:prod"
            mono
          />
          <Choice
            label="Show"
            value={s("grouping") || "cluster"}
            options={[
              ["cluster", "Counts per status"],
              ["check", "The worst status"],
            ]}
            onChange={(grouping) => setDef({ grouping })}
          />
        </div>
      );
    case "trace_service":
      return (
        <div className="form-row">
          <Text
            label="Service"
            value={s("service")}
            onChange={(service) => setDef({ service })}
            mono
          />
          <Text
            label="Env"
            value={s("env")}
            onChange={(env) => setDef({ env })}
            placeholder="(all)"
            mono
          />
        </div>
      );
    case "hostmap": {
      const reqs = def.requests as unknown as
        { fill?: { q?: string } } | undefined;
      return (
        <>
          <Text
            label="Fill by"
            value={reqs?.fill?.q ?? ""}
            onChange={(q) =>
              setDef({
                requests: { fill: { q } } as unknown as WidgetRequest[],
              })
            }
            mono
          />
          <Text
            label="Group by tags"
            value={((def.group as string[]) ?? []).join(",")}
            onChange={(v) =>
              setDef({
                group: v
                  .split(",")
                  .map((t) => t.trim())
                  .filter(Boolean),
              })
            }
            placeholder="service,availability-zone"
            mono
          />
        </>
      );
    }
    case "list_stream": {
      const r = (def.requests?.[0] ?? {}) as WidgetRequest & {
        query?: StreamQuery;
      };
      const set = (q: StreamQuery) =>
        setDef({ requests: [{ ...r, query: { ...r.query, ...q } }] });
      return (
        <div className="form-row">
          <Choice
            label="Source"
            value={r.query?.data_source ?? "logs_stream"}
            options={[
              ["logs_stream", "Logs"],
              ["trace_stream", "Spans"],
              ["rum_stream", "RUM events"],
              ["event_stream", "Events"],
              ["issue_stream", "Error Tracking issues"],
            ]}
            onChange={(data_source) => set({ data_source })}
          />
          <Text
            label="Query"
            value={r.query?.query_string ?? ""}
            onChange={(query_string) => set({ query_string })}
            mono
          />
        </div>
      );
    }
    case "funnel": {
      const r = (def.requests?.[0] ?? {}) as WidgetRequest & {
        query?: {
          query_string?: string;
          steps?: { facet: string; value: string }[];
        };
      };
      const steps = r.query?.steps ?? [];
      const set = (
        q: Partial<{
          query_string: string;
          steps: { facet: string; value: string }[];
        }>,
      ) =>
        setDef({
          requests: [
            {
              ...r,
              request_type: "funnel",
              query: { data_source: "rum", ...r.query, ...q },
            },
          ],
        });
      return (
        <>
          <Text
            label="Sessions matching"
            value={r.query?.query_string ?? ""}
            onChange={(query_string) => set({ query_string })}
            placeholder="@application.id:… env:prod"
            mono
          />
          <div className="form-label">Steps</div>
          {steps.map((st, i) => (
            <div className="query-row" key={i}>
              <span className="query-letter">{i + 1}</span>
              <select
                className="input"
                value={st.facet}
                onChange={(e) =>
                  set({
                    steps: steps.map((x, j) =>
                      j === i ? { ...x, facet: e.target.value } : x,
                    ),
                  })
                }
              >
                <option value="@view.name">View</option>
                <option value="@action.name">Action</option>
              </select>
              <input
                className="input mono grow"
                value={st.value}
                onChange={(e) =>
                  set({
                    steps: steps.map((x, j) =>
                      j === i ? { ...x, value: e.target.value } : x,
                    ),
                  })
                }
              />
              <button
                className="btn ghost small"
                onClick={() => set({ steps: steps.filter((_, j) => j !== i) })}
              >
                ✕
              </button>
            </div>
          ))}
          <div>
            <button
              className="btn small"
              onClick={() =>
                set({ steps: [...steps, { facet: "@view.name", value: "" }] })
              }
            >
              Add a step
            </button>
          </div>
        </>
      );
    }
    case "split_graph": {
      const cfg = (def.split_config ?? {}) as {
        split_dimensions?: { one_graph_per: string }[];
        limit?: number;
      };
      const src = (def.source_widget_definition ?? {}) as WidgetDef;
      const q = src.requests?.[0]
        ? (normalizeRequest(src.requests[0]).queries[0]?.query ?? "")
        : "";
      return (
        <>
          <Text
            label="Metric query"
            value={q}
            onChange={(query) =>
              setDef({
                source_widget_definition: {
                  type: src.type ?? "timeseries",
                  requests: [metricReq(query, src.type !== "timeseries")],
                },
              })
            }
            mono
          />
          <div className="form-row">
            <Choice
              label="Graph"
              value={src.type ?? "timeseries"}
              options={[
                ["timeseries", "Timeseries"],
                ["query_value", "Query Value"],
                ["toplist", "Top List"],
              ]}
              onChange={(type) =>
                setDef({
                  source_widget_definition: {
                    type,
                    requests: [metricReq(q, type !== "timeseries")],
                  },
                })
              }
            />
            <Text
              label="One graph per"
              value={cfg.split_dimensions?.[0]?.one_graph_per ?? ""}
              onChange={(one_graph_per) =>
                setDef({
                  split_config: {
                    ...cfg,
                    split_dimensions: [{ one_graph_per }],
                  },
                })
              }
              mono
            />
            <Text
              label="At most"
              value={String(cfg.limit ?? 12)}
              onChange={(v) =>
                setDef({ split_config: { ...cfg, limit: Number(v) || 12 } })
              }
            />
          </div>
        </>
      );
    }
    case "change": {
      const r = (def.requests?.[0] ?? {}) as WidgetRequest;
      const set = (patch: Record<string, unknown>) =>
        setDef({
          requests: [{ ...r, ...patch }, ...(def.requests ?? []).slice(1)],
        });
      return (
        <div className="form-row">
          <Choice
            label="Compare with"
            value={(r.compare_to as string) ?? "day_before"}
            options={[
              ["hour_before", "An hour before"],
              ["day_before", "A day before"],
              ["week_before", "A week before"],
              ["month_before", "A month before"],
            ]}
            onChange={(compare_to) => set({ compare_to })}
          />
          <Choice
            label="Change"
            value={(r.change_type as string) ?? "absolute"}
            options={[
              ["absolute", "Absolute"],
              ["relative", "Relative (%)"],
            ]}
            onChange={(change_type) => set({ change_type })}
          />
          <Choice
            label="An increase is"
            value={r.increase_good === false ? "bad" : "good"}
            options={[
              ["good", "Good"],
              ["bad", "Bad"],
            ]}
            onChange={(v) => set({ increase_good: v === "good" })}
          />
        </div>
      );
    }
    case "scatterplot":
      return (
        <div className="form-hint">
          Edit the x and y formulas (`requests.table`) in the JSON tab.
        </div>
      );
    default:
      return null;
  }
}

export type { TemplateVar, Widget };
