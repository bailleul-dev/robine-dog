import { useEffect, useRef, useState } from "react";
import { rangeLabel } from "../lib/format";
import { PRESETS, parseRelative, relativeLabel, type TimeRange } from "../lib/time";
import { ChevronDown, PauseIcon, PlayIcon, RefreshIcon } from "./Icons";

function badge(key: string | null): string {
  return key ?? "Fixed";
}

export function TimePicker({ time, allowLive = true }: { time: TimeRange; allowLive?: boolean }) {
  const [open, setOpen] = useState(false);
  const [text, setText] = useState("");
  const [err, setErr] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [open]);

  const apply = () => {
    const t = text.trim();
    if (parseRelative(t) !== null) {
      const m = t.toLowerCase().match(/^(\d+)\s*(s|m|h|d|w)/)!;
      time.setRelative(`${m[1]}${m[2]}`);
      setOpen(false);
      setText("");
      return;
    }
    // "Nov 16 10:00 - Nov 16 12:00" or ISO dates separated by " - ".
    const parts = t.split(/\s+[-–]\s+/);
    if (parts.length === 2) {
      const year = new Date().getFullYear();
      const parse = (s: string) => {
        const d = Date.parse(s);
        return Number.isNaN(d) ? Date.parse(`${s} ${year}`) : d;
      };
      const [a, b] = parts.map(parse);
      if (!Number.isNaN(a) && !Number.isNaN(b) && a < b) {
        time.setAbsolute(a, b);
        setOpen(false);
        setText("");
        return;
      }
    }
    setErr(true);
  };

  const label = time.relative && !time.paused ? relativeLabel(time.relative) : rangeLabel(time.from, time.to);
  const live = !!time.relative && !time.paused;

  return (
    <div className="timepicker" ref={ref}>
      {allowLive && (
        <button
          className="btn icon"
          title={live ? "Pause (freeze the time range)" : "Go live"}
          onClick={() => time.setPaused(live)}
        >
          {live ? <PauseIcon /> : <PlayIcon />}
        </button>
      )}
      <button className="timepicker-button" onClick={() => setOpen((o) => !o)} aria-haspopup="listbox">
        <span className="timepicker-badge">{badge(time.relative && !time.paused ? time.relative : null)}</span>
        <span className="timepicker-label">{label}</span>
        {live && <span className="live-dot" title="Live" />}
        <ChevronDown width={14} height={14} />
      </button>
      <button className="btn icon" title="Refresh" onClick={time.refresh}>
        <RefreshIcon />
      </button>
      {open && (
        <div className="timepicker-menu" role="listbox">
          <input
            className="input"
            autoFocus
            placeholder="e.g. 45m, 3h, 2d or Nov 16 10:00 - Nov 16 12:00"
            value={text}
            style={err ? { borderColor: "var(--red)" } : undefined}
            onChange={(e) => {
              setText(e.target.value);
              setErr(false);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") apply();
              if (e.key === "Escape") setOpen(false);
            }}
          />
          {PRESETS.map((p) => (
            <button
              key={p.key}
              className={`timepicker-option${time.relative === p.key ? " selected" : ""}`}
              onClick={() => {
                time.setRelative(p.key);
                setOpen(false);
              }}
            >
              <span className="timepicker-badge">{p.key}</span>
              {p.label}
            </button>
          ))}
          <div className="timepicker-hint">Drag across a graph to zoom into a time range.</div>
        </div>
      )}
    </div>
  );
}
