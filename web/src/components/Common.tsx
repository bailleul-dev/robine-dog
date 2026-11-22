import { useEffect, useRef, useState } from "react";
import { serviceColor, statusColor } from "../lib/format";
import {
  CloseIcon,
  CopyIcon,
  ExcludeIcon,
  FilterIcon,
  PlusIcon,
} from "./Icons";

export function ServiceChip({ name }: { name: string | null }) {
  if (!name) return <span className="faint">—</span>;
  return (
    <span className="service-chip">
      <i style={{ background: serviceColor(name) }} />
      <span>{name}</span>
    </span>
  );
}

export function StatusPill({ status }: { status: string }) {
  return (
    <span className="pill" style={{ background: statusColor(status) }}>
      {status}
    </span>
  );
}

export function HttpCode({
  code,
}: {
  code: string | number | undefined | null;
}) {
  if (code === undefined || code === null || code === "")
    return <span className="faint">—</span>;
  const n = Number(code);
  const cls =
    n >= 500
      ? "code-5xx"
      : n >= 400
        ? "code-4xx"
        : n >= 300
          ? "code-3xx"
          : "code-2xx";
  return <span className={`code-badge ${cls}`}>{code}</span>;
}

/** Right-hand drawer, resizable by its left edge, closed with Escape. */
export function SidePanel({
  onClose,
  header,
  children,
  initialWidth = 0.55,
  storageKey,
}: {
  onClose: () => void;
  header: React.ReactNode;
  children: React.ReactNode;
  initialWidth?: number;
  storageKey: string;
}) {
  const [width, setWidth] = useState(() => {
    try {
      const v = Number(localStorage.getItem(`panel:${storageKey}`));
      if (v > 300) return v;
    } catch {
      /* storage unavailable */
    }
    return Math.round(window.innerWidth * initialWidth);
  });
  const dragging = useRef(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !(e.target instanceof HTMLInputElement))
        onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  useEffect(() => {
    const move = (e: MouseEvent) => {
      if (!dragging.current) return;
      setWidth(
        Math.max(
          480,
          Math.min(window.innerWidth - 120, window.innerWidth - e.clientX),
        ),
      );
    };
    const up = () => {
      if (dragging.current) {
        dragging.current = false;
        document.body.style.cursor = "";
        try {
          localStorage.setItem(`panel:${storageKey}`, String(width));
        } catch {
          /* storage unavailable */
        }
      }
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
    return () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
  }, [storageKey, width]);

  return (
    <>
      <div className="sidepanel-backdrop" onClick={onClose} />
      <section
        className="sidepanel"
        style={{ width }}
        role="dialog"
        aria-modal="true"
      >
        <div
          className="sidepanel-resize"
          onMouseDown={(e) => {
            e.preventDefault();
            dragging.current = true;
            document.body.style.cursor = "col-resize";
          }}
        />
        <div className="sidepanel-header">
          {header}
          <div className="spacer" />
          <button
            className="btn icon ghost"
            onClick={onClose}
            title="Close (Esc)"
          >
            <CloseIcon />
          </button>
        </div>
        <div className="sidepanel-body">{children}</div>
      </section>
    </>
  );
}

export type ValueAction = { path: string; value: string };

/** The menu on a clicked attribute value: filter by, exclude, add facet, copy. */
export function ValueMenu({
  at,
  action,
  onClose,
  onFilter,
  onExclude,
  onAddFacet,
}: {
  at: { x: number; y: number };
  action: ValueAction;
  onClose: () => void;
  onFilter: (a: ValueAction) => void;
  onExclude: (a: ValueAction) => void;
  onAddFacet?: (path: string) => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const close = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    setTimeout(() => document.addEventListener("mousedown", close));
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", esc);
    };
  }, [onClose]);
  const clause = `${action.path}:${action.value}`;
  const left = Math.min(at.x, window.innerWidth - 260);
  const top = Math.min(at.y, window.innerHeight - 180);
  return (
    <div className="menu" ref={ref} style={{ left, top }}>
      <div className="menu-title" title={clause}>
        {clause}
      </div>
      <button onClick={() => (onFilter(action), onClose())}>
        <FilterIcon /> Filter by{" "}
        <span className="mono">
          {clause.length > 34 ? `${clause.slice(0, 34)}…` : clause}
        </span>
      </button>
      <button onClick={() => (onExclude(action), onClose())}>
        <ExcludeIcon /> Exclude <span className="mono">-{action.path}</span>
      </button>
      {onAddFacet && (
        <button onClick={() => (onAddFacet(action.path), onClose())}>
          <PlusIcon /> Create facet for{" "}
          <span className="mono">{action.path}</span>
        </button>
      )}
      <button
        onClick={() => {
          navigator.clipboard?.writeText(action.value);
          onClose();
        }}
      >
        <CopyIcon /> Copy value
      </button>
      <button
        onClick={() => {
          navigator.clipboard?.writeText(clause);
          onClose();
        }}
      >
        <CopyIcon /> Copy as query
      </button>
    </div>
  );
}

export function Modal({
  onClose,
  children,
  title,
  width = 380,
}: {
  onClose: () => void;
  children: React.ReactNode;
  title: string;
  width?: number;
}) {
  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(43,35,51,0.32)",
        zIndex: 90,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
      }}
      onMouseDown={onClose}
    >
      <div
        style={{
          background: "var(--bg)",
          borderRadius: "var(--radius-lg)",
          overflow: "hidden",
          width: `min(${width}px, calc(100vw - 32px))`,
          maxHeight: "calc(100vh - 48px)",
          display: "flex",
          flexDirection: "column",
          boxShadow: "var(--shadow)",
        }}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div
          style={{
            display: "flex",
            alignItems: "center",
            padding: "12px 16px",
            borderBottom: "1px solid var(--border)",
            fontWeight: 600,
          }}
        >
          {title}
          <div className="spacer" />
          <button className="btn icon ghost" onClick={onClose}>
            <CloseIcon />
          </button>
        </div>
        {children}
      </div>
    </div>
  );
}
