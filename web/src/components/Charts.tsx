import { useLayoutEffect, useRef, useState } from "react";
import { axisTime, compact, fullDate } from "../lib/format";

export function useWidth<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const [width, setWidth] = useState(600);
  useLayoutEffect(() => {
    if (!ref.current) return;
    const ro = new ResizeObserver((entries) => setWidth(entries[0].contentRect.width));
    ro.observe(ref.current);
    setWidth(ref.current.getBoundingClientRect().width);
    return () => ro.disconnect();
  }, []);
  return [ref, width] as const;
}

export type Series = { key: string; label?: string; color: string; values: number[] };

function niceMax(v: number): number {
  if (v <= 0) return 1;
  const exp = Math.pow(10, Math.floor(Math.log10(v)));
  const f = v / exp;
  const nf = f <= 1 ? 1 : f <= 2 ? 2 : f <= 2.5 ? 2.5 : f <= 5 ? 5 : 10;
  return nf * exp;
}

/**
 * A time series chart: stacked bars (log volume, hits) or lines (latency).
 * Dragging across it selects a time range (`onZoom`).
 */
export function TimeChart({
  times,
  interval,
  from,
  to,
  series,
  kind = "bars",
  height = 96,
  format = compact,
  onZoom,
  onBarClick,
  markers = [],
}: {
  times: number[];
  interval: number;
  from: number;
  to: number;
  series: Series[];
  kind?: "bars" | "lines";
  height?: number;
  format?: (v: number) => string;
  onZoom?: (from: number, to: number) => void;
  onBarClick?: (t: number) => void;
  /** Horizontal reference lines (monitor thresholds). */
  markers?: { value: number; color: string; label?: string }[];
}) {
  const [ref, width] = useWidth<HTMLDivElement>();
  const [hover, setHover] = useState<number | null>(null);
  const [drag, setDrag] = useState<{ x0: number; x1: number } | null>(null);
  const left = 44;
  const right = 8;
  const top = 6;
  const bottom = 20;
  const plotW = Math.max(10, width - left - right);
  const plotH = height - top - bottom;
  const span = Math.max(1, to - from);
  const x = (t: number) => left + ((t - from) / span) * plotW;

  // A NaN value is a gap: no line segment, nothing stacked.
  const val = (v: number | undefined) => (v === undefined || Number.isNaN(v) ? 0 : v);
  const totals = times.map((_, i) =>
    kind === "bars" ? series.reduce((s, se) => s + val(se.values[i]), 0) : Math.max(0, ...series.map((se) => val(se.values[i]))),
  );
  const max = niceMax(Math.max(0, ...totals, ...markers.map((m) => m.value * 1.1)));
  const y = (v: number) => top + plotH - (v / max) * plotH;
  const barW = Math.max(1, (interval / span) * plotW - (interval / span) * plotW * 0.18);

  const ticks = Math.max(2, Math.min(8, Math.floor(plotW / 110)));
  const xTicks = Array.from({ length: ticks }, (_, i) => from + (span * (i + 0.5)) / ticks);

  const idxAt = (px: number) => {
    const t = from + ((px - left) / plotW) * span;
    let best = -1;
    let bestD = Infinity;
    times.forEach((tt, i) => {
      const d = Math.abs(tt + interval / 2 - t);
      if (d < bestD) {
        bestD = d;
        best = i;
      }
    });
    return best;
  };

  const localX = (e: React.MouseEvent) => e.clientX - (ref.current?.getBoundingClientRect().left ?? 0);

  return (
    <div className="chart" ref={ref} style={{ position: "relative" }}>
      <svg
        width={width}
        height={height}
        onMouseMove={(e) => {
          const px = localX(e);
          setHover(px >= left && px <= left + plotW ? idxAt(px) : null);
          if (drag) setDrag({ ...drag, x1: Math.max(left, Math.min(left + plotW, px)) });
        }}
        onMouseLeave={() => {
          setHover(null);
          setDrag(null);
        }}
        onMouseDown={(e) => {
          if (!onZoom) return;
          const px = localX(e);
          if (px >= left) setDrag({ x0: px, x1: px });
        }}
        onMouseUp={(e) => {
          if (drag && Math.abs(drag.x1 - drag.x0) > 4 && onZoom) {
            const a = from + ((Math.min(drag.x0, drag.x1) - left) / plotW) * span;
            const b = from + ((Math.max(drag.x0, drag.x1) - left) / plotW) * span;
            onZoom(a, b);
          } else if (onBarClick) {
            const i = idxAt(localX(e));
            if (i >= 0) onBarClick(times[i]);
          }
          setDrag(null);
        }}
        style={{ cursor: onZoom ? "crosshair" : undefined }}
      >
        <g className="chart-grid">
          {[0, 0.5, 1].map((f) => (
            <line key={f} x1={left} x2={left + plotW} y1={y(max * f)} y2={y(max * f)} />
          ))}
        </g>
        <g className="chart-axis">
          {[0, 0.5, 1].map((f) => (
            <text key={f} x={left - 6} y={y(max * f) + 3.5} textAnchor="end">
              {format(max * f)}
            </text>
          ))}
          {xTicks.map((t) => (
            <text key={t} x={x(t)} y={height - 5} textAnchor="middle">
              {axisTime(t, span)}
            </text>
          ))}
        </g>
        {kind === "bars" &&
          times.map((t, i) => {
            let acc = 0;
            const bx = x(t) + ((interval / span) * plotW * 0.09);
            return (
              <g key={t} opacity={hover === null || hover === i ? 1 : 0.55}>
                {series.map((s) => {
                  const v = val(s.values[i]);
                  if (!v) return null;
                  const y0 = y(acc);
                  acc += v;
                  const y1 = y(acc);
                  return <rect key={s.key} x={bx} y={y1} width={barW} height={Math.max(1, y0 - y1)} fill={s.color} />;
                })}
              </g>
            );
          })}
        {kind === "lines" &&
          series.map((s) => (
            <path
              key={s.key}
              d={times
                .map((t, i) => {
                  const v = s.values[i];
                  if (v === undefined || Number.isNaN(v)) return "";
                  const prev = s.values[i - 1];
                  const move = i === 0 || prev === undefined || Number.isNaN(prev);
                  return `${move ? "M" : "L"}${x(t + interval / 2).toFixed(1)},${y(v).toFixed(1)}`;
                })
                .join("")}
              fill="none"
              stroke={s.color}
              strokeWidth={1.6}
              strokeLinejoin="round"
            />
          ))}
        {kind === "lines" && hover !== null && (
          <g>
            <line x1={x(times[hover] + interval / 2)} x2={x(times[hover] + interval / 2)} y1={top} y2={top + plotH} stroke="#c9ccd6" />
            {series
              .filter((s) => !Number.isNaN(s.values[hover] ?? NaN))
              .map((s) => (
                <circle key={s.key} cx={x(times[hover] + interval / 2)} cy={y(s.values[hover])} r={3} fill={s.color} />
              ))}
          </g>
        )}
        {markers.map((m) => (
          <g key={`${m.value}-${m.color}`}>
            <line x1={left} x2={left + plotW} y1={y(m.value)} y2={y(m.value)} stroke={m.color} strokeDasharray="4 3" strokeWidth={1.2} />
            {m.label && (
              <text x={left + plotW - 4} y={y(m.value) - 4} textAnchor="end" fontSize={10} fill={m.color}>
                {m.label}
              </text>
            )}
          </g>
        ))}
        {drag && Math.abs(drag.x1 - drag.x0) > 1 && (
          <rect className="brush" x={Math.min(drag.x0, drag.x1)} y={top} width={Math.abs(drag.x1 - drag.x0)} height={plotH} />
        )}
      </svg>
      {hover !== null && !drag && times[hover] !== undefined && (
        <div
          className="chart-tooltip"
          style={{
            left: Math.min(Math.max(0, x(times[hover]) + 14), width - 200),
            top: 0,
          }}
        >
          <div className="chart-tooltip-title">{fullDate(times[hover]).replace(/\.\d+$/, "")}</div>
          {[...series].reverse().map((s) => (
            <div className="chart-tooltip-row" key={s.key}>
              <i style={{ width: 8, height: 8, background: s.color, borderRadius: 2, display: "inline-block" }} />
              <span>{s.label ?? s.key}</span>
              <span>{Number.isNaN(s.values[hover] ?? NaN) ? "–" : format(s.values[hover])}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export function Sparkline({ values, color = "#8b6fd0", width = 110, height = 22 }: { values: number[]; color?: string; width?: number; height?: number }) {
  const max = Math.max(1, ...values);
  const step = width / Math.max(1, values.length);
  return (
    <svg width={width} height={height} style={{ display: "block" }}>
      {values.map((v, i) => {
        const h = Math.max(v > 0 ? 1.5 : 0, (v / max) * (height - 2));
        return <rect key={i} x={i * step + 0.5} y={height - h} width={Math.max(1, step - 1.2)} height={h} fill={color} rx={0.5} />;
      })}
    </svg>
  );
}
