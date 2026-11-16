import { useCallback, useEffect, useMemo, useState } from "react";
import { useSearchParams } from "react-router-dom";

export type Preset = { key: string; label: string; ms: number };

export const PRESETS: Preset[] = [
  { key: "5m", label: "Past 5 Minutes", ms: 5 * 60_000 },
  { key: "15m", label: "Past 15 Minutes", ms: 15 * 60_000 },
  { key: "1h", label: "Past 1 Hour", ms: 3_600_000 },
  { key: "4h", label: "Past 4 Hours", ms: 4 * 3_600_000 },
  { key: "1d", label: "Past 1 Day", ms: 86_400_000 },
  { key: "2d", label: "Past 2 Days", ms: 2 * 86_400_000 },
  { key: "3d", label: "Past 3 Days", ms: 3 * 86_400_000 },
  { key: "7d", label: "Past 7 Days", ms: 7 * 86_400_000 },
  { key: "15d", label: "Past 15 Days", ms: 15 * 86_400_000 },
];

const UNIT_MS: Record<string, number> = { s: 1000, m: 60_000, h: 3_600_000, d: 86_400_000, w: 7 * 86_400_000 };
const UNIT_LABEL: Record<string, string> = { s: "Second", m: "Minute", h: "Hour", d: "Day", w: "Week" };

/** `"15m"` → 900000; also accepts `"2 hours"`. */
export function parseRelative(s: string): number | null {
  const m = s.trim().toLowerCase().match(/^(\d+)\s*(s|m|h|d|w)[a-z]*$/);
  if (!m) return null;
  return Number(m[1]) * UNIT_MS[m[2]];
}

export function relativeLabel(key: string): string {
  const p = PRESETS.find((x) => x.key === key);
  if (p) return p.label;
  const m = key.match(/^(\d+)(s|m|h|d|w)$/);
  if (!m) return key;
  const n = Number(m[1]);
  return `Past ${n} ${UNIT_LABEL[m[2]]}${n > 1 ? "s" : ""}`;
}

export type TimeState = {
  from: number;
  to: number;
  /** Relative key ("15m") when the range follows the clock. */
  relative: string | null;
  paused: boolean;
};

/**
 * Time range in the URL, like Datadog: `?from_ts=…&to_ts=…&live=true`.
 * A live range moves with the clock (refreshed every `tickMs`).
 */
export function useTimeRange(defaultKey = "15m", tickMs = 10_000) {
  const [params, setParams] = useSearchParams();
  const fromTs = Number(params.get("from_ts")) || 0;
  const toTs = Number(params.get("to_ts")) || 0;
  const live = params.get("live") !== "false";
  const relKey = params.get("range") ?? (fromTs && toTs ? null : defaultKey);
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    if (!relKey || !live) return;
    const id = setInterval(() => setNow(Date.now()), tickMs);
    return () => clearInterval(id);
  }, [relKey, live, tickMs]);

  const state: TimeState = useMemo(() => {
    if (relKey) {
      const ms = parseRelative(relKey) ?? 15 * 60_000;
      const end = live ? now : toTs || now;
      return { from: end - ms, to: end, relative: relKey, paused: !live };
    }
    return { from: fromTs, to: toTs, relative: null, paused: true };
  }, [relKey, live, now, fromTs, toTs]);

  const update = useCallback(
    (fn: (p: URLSearchParams) => void) => {
      setParams(
        (prev) => {
          const p = new URLSearchParams(prev);
          fn(p);
          return p;
        },
        { replace: false },
      );
    },
    [setParams],
  );

  const setRelative = useCallback(
    (key: string) => {
      setNow(Date.now());
      update((p) => {
        p.set("range", key);
        p.delete("from_ts");
        p.delete("to_ts");
        p.delete("live");
      });
    },
    [update],
  );

  const setAbsolute = useCallback(
    (from: number, to: number) =>
      update((p) => {
        p.delete("range");
        p.set("from_ts", String(Math.round(from)));
        p.set("to_ts", String(Math.round(to)));
        p.set("live", "false");
      }),
    [update],
  );

  const setPaused = useCallback(
    (paused: boolean) => {
      setNow(Date.now());
      update((p) => {
        if (paused) {
          p.set("live", "false");
          p.set("to_ts", String(Date.now()));
        } else {
          p.delete("live");
          p.delete("to_ts");
          if (!p.get("range")) {
            p.set("range", "15m");
            p.delete("from_ts");
          }
        }
      });
    },
    [update],
  );

  const refresh = useCallback(() => setNow(Date.now()), []);

  return { ...state, setRelative, setAbsolute, setPaused, refresh };
}

export type TimeRange = ReturnType<typeof useTimeRange>;

/** Query string in the URL (`?query=`), like Datadog. */
export function useQueryParam(name = "query"): [string, (v: string) => void] {
  const [params, setParams] = useSearchParams();
  const value = params.get(name) ?? "";
  const set = useCallback(
    (v: string) =>
      setParams((prev) => {
        const p = new URLSearchParams(prev);
        if (v) p.set(name, v);
        else p.delete(name);
        return p;
      }),
    [name, setParams],
  );
  return [value, set];
}
