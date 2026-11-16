import { useState } from "react";
import { ChevronDown, ChevronRight } from "./Icons";

type OnValue = (path: string[], value: string, e: React.MouseEvent) => void;

function Leaf({ v }: { v: unknown }) {
  if (v === null) return <span className="json-null">null</span>;
  if (typeof v === "string") return <span className="json-string">"{v}"</span>;
  if (typeof v === "number") return <span className="json-number">{v}</span>;
  if (typeof v === "boolean") return <span className="json-bool">{String(v)}</span>;
  return <span>{JSON.stringify(v)}</span>;
}

function Node({
  name,
  value,
  path,
  depth,
  onValue,
  facets,
  prefix,
}: {
  name: string;
  value: unknown;
  path: string[];
  depth: number;
  onValue?: OnValue;
  facets?: Set<string>;
  prefix: string;
}) {
  const [open, setOpen] = useState(depth < 3);
  const isObj = value !== null && typeof value === "object";
  const entries = isObj ? (Array.isArray(value) ? value.map((v, i) => [String(i), v] as const) : Object.entries(value as object)) : [];
  const full = prefix + path.join(".");
  return (
    <div>
      <div className="json-row" style={{ paddingLeft: depth * 16 }}>
        {isObj ? (
          <button className="json-toggle" onClick={() => setOpen((o) => !o)} aria-label={open ? "Collapse" : "Expand"}>
            {open ? <ChevronDown /> : <ChevronRight />}
          </button>
        ) : (
          <span className="json-toggle" />
        )}
        <span className="json-key">{name}</span>
        <span className="faint">:</span>{" "}
        {isObj ? (
          <span className="faint" style={{ cursor: "pointer" }} onClick={() => setOpen((o) => !o)}>
            {Array.isArray(value) ? `[${entries.length}]` : open ? "" : `{ ${entries.length} }`}
          </span>
        ) : (
          <span
            className="json-value"
            onClick={(e) => onValue?.(path, value === null ? "null" : String(value), e)}
            title="Click for actions"
          >
            <Leaf v={value} />
          </span>
        )}
        {facets?.has(full) && <span className="json-facet">facet</span>}
      </div>
      {isObj &&
        open &&
        entries.map(([k, v]) => (
          <Node
            key={k}
            name={k}
            value={v}
            path={Array.isArray(value) ? path : [...path, k]}
            depth={depth + 1}
            onValue={onValue}
            facets={facets}
            prefix={prefix}
          />
        ))}
    </div>
  );
}

/** Expandable JSON with clickable values (filter/exclude/facet actions). */
export function JsonTree({
  data,
  onValue,
  facets,
  prefix = "@",
}: {
  data: Record<string, unknown>;
  onValue?: OnValue;
  facets?: Set<string>;
  prefix?: string;
}) {
  const entries = Object.entries(data ?? {});
  if (!entries.length) return <div className="faint">No attributes</div>;
  return (
    <div className="json-tree">
      {entries.map(([k, v]) => (
        <Node key={k} name={k} value={v} path={[k]} depth={0} onValue={onValue} facets={facets} prefix={prefix} />
      ))}
    </div>
  );
}
