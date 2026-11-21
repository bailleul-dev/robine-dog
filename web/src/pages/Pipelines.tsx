import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { Modal } from "../components/Common";
import {
  ChevronDown,
  ChevronRight,
  ChevronUp,
  CopyIcon,
  PlusIcon,
  TrashIcon,
} from "../components/Icons";
import { JsonTree } from "../components/JsonTree";
import { PageHeader } from "../components/Layout";
import {
  api,
  type GrokSample,
  type Pipeline,
  type Processor,
} from "../lib/api";
import { LOG_TABS } from "./LogsExplorer";

type Spec = {
  type: string;
  label: string;
  help: string;
  defaults: Record<string, unknown>;
};

/** Datadog's processors, with the defaults its UI pre-fills. */
const SPECS: Spec[] = [
  {
    type: "grok-parser",
    label: "Grok Parser",
    help: "Parse text into attributes with grok rules.",
    defaults: {
      source: "message",
      samples: [""],
      grok: { match_rules: "", support_rules: "" },
    },
  },
  {
    type: "date-remapper",
    label: "Date Remapper",
    help: "Define an attribute as the official timestamp of the log.",
    defaults: {
      sources: [
        "timestamp",
        "date",
        "_timestamp",
        "Timestamp",
        "eventTime",
        "published_date",
      ],
    },
  },
  {
    type: "status-remapper",
    label: "Status Remapper",
    help: "Define an attribute as the official status of the log.",
    defaults: { sources: ["status", "severity", "level", "syslog.severity"] },
  },
  {
    type: "service-remapper",
    label: "Service Remapper",
    help: "Define an attribute as the official service of the log.",
    defaults: { sources: ["service"] },
  },
  {
    type: "message-remapper",
    label: "Message Remapper",
    help: "Define an attribute as the official message of the log.",
    defaults: { sources: ["message", "msg"] },
  },
  {
    type: "attribute-remapper",
    label: "Remapper",
    help: "Remap an attribute or tag to another attribute or tag.",
    defaults: {
      sources: [],
      source_type: "attribute",
      target: "",
      target_type: "attribute",
      preserve_source: false,
      override_on_conflict: false,
    },
  },
  {
    type: "url-parser",
    label: "URL Parser",
    help: "Extract the host, path and query parameters of a URL.",
    defaults: {
      sources: ["http.url"],
      target: "http.url_details",
      normalize_ending_slashes: false,
    },
  },
  {
    type: "user-agent-parser",
    label: "User-Agent Parser",
    help: "Extract the browser, OS and device of a user agent.",
    defaults: {
      sources: ["http.useragent"],
      target: "http.useragent_details",
      is_encoded: false,
    },
  },
  {
    type: "category-processor",
    label: "Category Processor",
    help: "Add an attribute whose value depends on which query a log matches.",
    defaults: { target: "", categories: [{ filter: { query: "" }, name: "" }] },
  },
  {
    type: "arithmetic-processor",
    label: "Arithmetic Processor",
    help: "Compute a new numeric attribute from others.",
    defaults: { expression: "", target: "", is_replace_missing: false },
  },
  {
    type: "string-builder-processor",
    label: "String Builder Processor",
    help: "Build a string attribute from a template such as %{http.method} %{http.url}.",
    defaults: { template: "", target: "", is_replace_missing: false },
  },
  {
    type: "lookup-processor",
    label: "Lookup Processor",
    help: "Map attribute values to readable ones through a table.",
    defaults: { source: "", target: "", lookup_table: [], default_lookup: "" },
  },
  {
    type: "trace-id-remapper",
    label: "Trace Id Remapper",
    help: "Define an attribute as the trace id of the log.",
    defaults: { sources: ["dd.trace_id", "contextMap.dd.trace_id"] },
  },
  {
    type: "span-id-remapper",
    label: "Span Id Remapper",
    help: "Define an attribute as the span id of the log.",
    defaults: { sources: ["dd.span_id", "contextMap.dd.span_id"] },
  },
  {
    type: "pipeline",
    label: "Nested Pipeline",
    help: "A pipeline inside a pipeline, with its own filter.",
    defaults: { filter: { query: "" }, processors: [] },
  },
];

const specOf = (t: string) => SPECS.find((s) => s.type === t);

function move<T>(xs: T[], i: number, d: number): T[] {
  const j = i + d;
  if (j < 0 || j >= xs.length) return xs;
  const out = [...xs];
  [out[i], out[j]] = [out[j], out[i]];
  return out;
}

function Switch({
  on,
  onChange,
  disabled,
  label,
}: {
  on: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
  label: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      className={`switch${on ? " on" : ""}`}
      disabled={disabled}
      onClick={(e) => {
        e.stopPropagation();
        onChange(!on);
      }}
    />
  );
}

// --- processor editor -------------------------------------------------------

const str = (v: unknown) => (typeof v === "string" ? v : "");
const list = (v: unknown) => (Array.isArray(v) ? (v as string[]) : []);

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <label className="form-field">
      <span className="form-label">{label}</span>
      {children}
      {hint && <span className="form-hint">{hint}</span>}
    </label>
  );
}

function GrokEditor({
  p,
  set,
}: {
  p: Processor;
  set: (patch: Record<string, unknown>) => void;
}) {
  const grok = (p.grok as { match_rules: string; support_rules: string }) ?? {
    match_rules: "",
    support_rules: "",
  };
  const samples = list(p.samples).length ? list(p.samples) : [""];
  const [result, setResult] = useState<{
    error: string | null;
    samples: GrokSample[];
  } | null>(null);
  const [advanced, setAdvanced] = useState(
    !!grok.support_rules || str(p.source) !== "message",
  );

  useEffect(() => {
    const ctl = new AbortController();
    const t = setTimeout(() => {
      if (!grok.match_rules.trim()) return setResult(null);
      api
        .grokTest(grok, samples, ctl.signal)
        .then(setResult)
        .catch(() => {});
    }, 300);
    return () => {
      clearTimeout(t);
      ctl.abort();
    };
  }, [grok.match_rules, grok.support_rules, samples.join("\u0000")]);

  const outcome = (s: string) => result?.samples.find((x) => x.sample === s);
  return (
    <>
      <div className="form-label">Log samples</div>
      {samples.map((s, i) => {
        const o = s ? outcome(s) : undefined;
        return (
          <div key={i} className="grok-sample">
            <div className="grok-sample-head">
              <span className="faint">Sample {i + 1}</span>
              <div className="spacer" />
              {s && result && !result.error && (
                <span className={o?.rule ? "grok-match" : "grok-nomatch"}>
                  {o?.rule ? `match: ${o.rule}` : "No match"}
                </span>
              )}
              {samples.length > 1 && (
                <button
                  className="btn ghost small"
                  title="Remove sample"
                  onClick={() =>
                    set({ samples: samples.filter((_, j) => j !== i) })
                  }
                >
                  <TrashIcon />
                </button>
              )}
            </div>
            <textarea
              className="input mono"
              rows={2}
              value={s}
              onChange={(e) =>
                set({
                  samples: samples.map((x, j) =>
                    j === i ? e.target.value : x,
                  ),
                })
              }
            />
            {o?.rule && (
              <div className="grok-output">
                <JsonTree data={o.attributes} />
              </div>
            )}
          </div>
        );
      })}
      {samples.length < 5 && (
        <button
          className="btn small"
          onClick={() => set({ samples: [...samples, ""] })}
        >
          <PlusIcon /> Add sample
        </button>
      )}
      <Field
        label="Define parsing rules"
        hint="One rule per line: rule_name pattern. The first rule matching the whole text wins."
      >
        <textarea
          className="input mono"
          rows={5}
          value={grok.match_rules}
          placeholder={
            "rule_name %{word:user} logged in from %{ip:network.client.ip}"
          }
          onChange={(e) =>
            set({ grok: { ...grok, match_rules: e.target.value } })
          }
        />
      </Field>
      {result?.error && <div className="error-box">{result.error}</div>}
      <button
        className="btn ghost small"
        onClick={() => setAdvanced(!advanced)}
      >
        {advanced ? <ChevronDown /> : <ChevronRight />} Advanced Settings
      </button>
      {advanced && (
        <>
          <Field
            label="Extract from"
            hint="The attribute to parse (default: message)."
          >
            <input
              className="input mono"
              value={str(p.source)}
              onChange={(e) => set({ source: e.target.value })}
            />
          </Field>
          <Field
            label="Helper rules"
            hint="Rules usable from the parsing rules as %{name}, not matched on their own."
          >
            <textarea
              className="input mono"
              rows={4}
              value={grok.support_rules}
              onChange={(e) =>
                set({ grok: { ...grok, support_rules: e.target.value } })
              }
            />
          </Field>
        </>
      )}
    </>
  );
}

function SourcesField({
  p,
  set,
  label = "Set attribute(s) to remap",
}: {
  p: Processor;
  set: (patch: Record<string, unknown>) => void;
  label?: string;
}) {
  return (
    <Field label={label} hint="Comma-separated; the first one present is used.">
      <input
        className="input mono"
        value={list(p.sources).join(", ")}
        onChange={(e) =>
          set({
            sources: e.target.value
              .split(",")
              .map((s) => s.trim())
              .filter(Boolean),
          })
        }
      />
    </Field>
  );
}

function Check({
  label,
  value,
  onChange,
}: {
  label: string;
  value: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className="form-check">
      <input
        type="checkbox"
        checked={value}
        onChange={(e) => onChange(e.target.checked)}
      />{" "}
      {label}
    </label>
  );
}

/** The Reference Table of a lookup processor: the target gets the row of
 * the source's key. */
function RefTableSelect({
  value,
  onChange,
}: {
  value: string;
  onChange: (v: string) => void;
}) {
  const tables = useQuery({
    queryKey: ["reference-tables"],
    queryFn: api.referenceTables,
  });
  return (
    <Field
      label="Reference Table"
      hint="The target attribute gets the row whose primary key is the source's value."
    >
      <select
        className="input"
        value={value}
        onChange={(e) => onChange(e.target.value)}
      >
        <option value="">None: a key,value list</option>
        {(tables.data ?? []).map((t) => (
          <option key={t.id} value={t.attributes.table_name}>
            {t.attributes.table_name}
          </option>
        ))}
      </select>
    </Field>
  );
}

function ProcessorFields({
  p,
  set,
}: {
  p: Processor;
  set: (patch: Record<string, unknown>) => void;
}) {
  const text = (k: string, label: string, hint?: string, mono = true) => (
    <Field label={label} hint={hint}>
      <input
        className={`input${mono ? " mono" : ""}`}
        value={str(p[k])}
        onChange={(e) => set({ [k]: e.target.value })}
      />
    </Field>
  );
  switch (p.type) {
    case "grok-parser":
      return <GrokEditor p={p} set={set} />;
    case "date-remapper":
    case "status-remapper":
    case "service-remapper":
    case "message-remapper":
    case "trace-id-remapper":
    case "span-id-remapper":
      return <SourcesField p={p} set={set} />;
    case "attribute-remapper":
      return (
        <>
          <div className="form-row">
            <Field label="Remap">
              <select
                className="input"
                value={str(p.source_type)}
                onChange={(e) => set({ source_type: e.target.value })}
              >
                <option value="attribute">Attribute(s)</option>
                <option value="tag">Tag key(s)</option>
              </select>
            </Field>
            <SourcesField p={p} set={set} label="Source(s)" />
          </div>
          <div className="form-row">
            <Field label="To">
              <select
                className="input"
                value={str(p.target_type)}
                onChange={(e) => set({ target_type: e.target.value })}
              >
                <option value="attribute">Attribute</option>
                <option value="tag">Tag key</option>
              </select>
            </Field>
            {text("target", "Target")}
          </div>
          <Field label="Target type">
            <select
              className="input"
              value={str(p.target_format) || "auto"}
              onChange={(e) =>
                set({
                  target_format:
                    e.target.value === "auto" ? undefined : e.target.value,
                })
              }
            >
              <option value="auto">Unchanged</option>
              <option value="string">String</option>
              <option value="integer">Integer</option>
              <option value="double">Double</option>
            </select>
          </Field>
          <Check
            label="Preserve source attribute"
            value={!!p.preserve_source}
            onChange={(v) => set({ preserve_source: v })}
          />
          <Check
            label="Override on conflict"
            value={!!p.override_on_conflict}
            onChange={(v) => set({ override_on_conflict: v })}
          />
        </>
      );
    case "url-parser":
    case "user-agent-parser":
      return (
        <>
          <SourcesField p={p} set={set} label="Source attribute(s)" />
          {text("target", "Target attribute")}
          {p.type === "url-parser" ? (
            <Check
              label="Normalize ending slashes"
              value={!!p.normalize_ending_slashes}
              onChange={(v) => set({ normalize_ending_slashes: v })}
            />
          ) : (
            <Check
              label="Source is URL-encoded"
              value={!!p.is_encoded}
              onChange={(v) => set({ is_encoded: v })}
            />
          )}
        </>
      );
    case "category-processor": {
      const cats =
        (p.categories as { filter: { query: string }; name: string }[]) ?? [];
      const setCat = (
        i: number,
        c: { filter: { query: string }; name: string },
      ) => set({ categories: cats.map((x, j) => (j === i ? c : x)) });
      return (
        <>
          {text(
            "target",
            "Set target category attribute",
            "e.g. http.status_category",
          )}
          <div className="form-label">Populate category</div>
          {cats.map((c, i) => (
            <div className="form-row" key={i}>
              <input
                className="input mono"
                placeholder="All events that match (query)"
                value={c.filter.query}
                onChange={(e) =>
                  setCat(i, { ...c, filter: { query: e.target.value } })
                }
                style={{ flex: 2 }}
              />
              <input
                className="input"
                placeholder="Appear under the value name"
                value={c.name}
                onChange={(e) => setCat(i, { ...c, name: e.target.value })}
                style={{ flex: 1 }}
              />
              <button
                className="btn ghost small"
                onClick={() =>
                  set({ categories: cats.filter((_, j) => j !== i) })
                }
                title="Remove"
              >
                <TrashIcon />
              </button>
            </div>
          ))}
          <button
            className="btn small"
            onClick={() =>
              set({
                categories: [...cats, { filter: { query: "" }, name: "" }],
              })
            }
          >
            <PlusIcon /> Add category
          </button>
        </>
      );
    }
    case "arithmetic-processor":
      return (
        <>
          {text(
            "expression",
            "Arithmetic operation",
            "Attributes, numbers, + - * / %, abs() ceil() floor() round(): (network.bytes_read + network.bytes_written) / 1024",
          )}
          {text("target", "Set target attribute")}
          <Check
            label="Replace missing attributes by 0"
            value={!!p.is_replace_missing}
            onChange={(v) => set({ is_replace_missing: v })}
          />
        </>
      );
    case "string-builder-processor":
      return (
        <>
          {text("template", "Template", "Text with %{attribute} placeholders")}
          {text("target", "Set target attribute")}
          <Check
            label="Replace missing attributes by an empty string"
            value={!!p.is_replace_missing}
            onChange={(v) => set({ is_replace_missing: v })}
          />
        </>
      );
    case "lookup-processor":
      return (
        <>
          {text("source", "Source attribute")}
          {text("target", "Target attribute")}
          <RefTableSelect
            value={str(p.lookup_enrichment_table)}
            onChange={(v) => set({ lookup_enrichment_table: v || undefined })}
          />
          {!p.lookup_enrichment_table && (
            <Field label="Lookup table" hint="One key,value pair per line">
              <textarea
                className="input mono"
                rows={5}
                value={list(p.lookup_table).join("\n")}
                onChange={(e) =>
                  set({
                    lookup_table: e.target.value
                      .split("\n")
                      .filter((l) => l.trim()),
                  })
                }
              />
            </Field>
          )}
          {!p.lookup_enrichment_table &&
            text("default_lookup", "Default value (optional)")}
        </>
      );
    case "pipeline":
      return (
        <Field
          label="Filter"
          hint="Logs matching this query go through the nested pipeline."
        >
          <input
            className="input mono"
            value={(p.filter as { query: string })?.query ?? ""}
            onChange={(e) => set({ filter: { query: e.target.value } })}
          />
        </Field>
      );
    default:
      return <div className="muted">This processor has no settings.</div>;
  }
}

function ProcessorModal({
  initial,
  onSave,
  onClose,
  nested,
  readOnly,
}: {
  initial: Processor | null;
  onSave: (p: Processor) => Promise<void>;
  onClose: () => void;
  nested: boolean;
  readOnly: boolean;
}) {
  const [p, setP] = useState<Processor>(
    initial ?? {
      type: "grok-parser",
      name: "",
      is_enabled: true,
      ...(SPECS[0].defaults as object),
    },
  );
  const [error, setError] = useState<string | null>(null);
  const set = (patch: Record<string, unknown>) =>
    setP((x) => ({ ...x, ...patch }));
  const spec = specOf(p.type);
  return (
    <Modal
      title={
        readOnly
          ? (spec?.label ?? p.type)
          : initial
            ? `Edit ${spec?.label ?? p.type}`
            : "Create processor"
      }
      onClose={onClose}
      width={720}
    >
      <div className="modal-body">
        <fieldset className="form" disabled={readOnly}>
          {!initial && (
            <Field label="Select the processor type">
              <select
                className="input"
                value={p.type}
                onChange={(e) =>
                  setP({
                    type: e.target.value,
                    name: p.name,
                    is_enabled: true,
                    ...(specOf(e.target.value)?.defaults ?? {}),
                  })
                }
              >
                {SPECS.filter((s) => !nested || s.type !== "pipeline").map(
                  (s) => (
                    <option key={s.type} value={s.type}>
                      {s.label}
                    </option>
                  ),
                )}
              </select>
            </Field>
          )}
          {spec && <div className="form-hint">{spec.help}</div>}
          <ProcessorFields p={p} set={set} />
          <Field label="Name the processor">
            <input
              className="input"
              value={p.name}
              onChange={(e) => set({ name: e.target.value })}
              placeholder={spec?.label}
            />
          </Field>
          {error && <div className="error-box">{error}</div>}
        </fieldset>
      </div>
      <div className="modal-footer">
        {readOnly && (
          <span className="muted">
            Integration pipelines are read-only: clone the pipeline to change
            it.
          </span>
        )}
        <div className="spacer" />
        <button className="btn" onClick={onClose}>
          {readOnly ? "Close" : "Cancel"}
        </button>
        {!readOnly && (
          <button
            className="btn primary"
            onClick={async () => {
              try {
                await onSave({ ...p, name: p.name || spec?.label || p.type });
                onClose();
              } catch (e) {
                setError(String((e as Error).message ?? e));
              }
            }}
          >
            {initial ? "Update" : "Create"}
          </button>
        )}
      </div>
    </Modal>
  );
}

function PipelineModal({
  initial,
  onSave,
  onClose,
}: {
  initial: Pipeline | null;
  onSave: (p: Pipeline) => Promise<void>;
  onClose: () => void;
}) {
  const [name, setName] = useState(initial?.name ?? "");
  const [query, setQuery] = useState(initial?.filter.query ?? "");
  const [description, setDescription] = useState(initial?.description ?? "");
  const [error, setError] = useState<string | null>(null);
  return (
    <Modal
      title={initial ? "Edit pipeline" : "New pipeline"}
      onClose={onClose}
      width={560}
    >
      <div className="modal-body form">
        <Field
          label="Filter"
          hint="Only logs matching this query go through the pipeline, e.g. source:nginx service:web"
        >
          <input
            className="input mono"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            autoFocus
          />
        </Field>
        <Field label="Name">
          <input
            className="input"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </Field>
        <Field label="Description">
          <input
            className="input"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />
        </Field>
        {error && <div className="error-box">{error}</div>}
      </div>
      <div className="modal-footer">
        <button className="btn" onClick={onClose}>
          Cancel
        </button>
        <button
          className="btn primary"
          disabled={!name.trim()}
          onClick={async () => {
            try {
              await onSave({
                ...(initial ?? {
                  type: "pipeline",
                  is_enabled: true,
                  processors: [],
                  is_read_only: false,
                }),
                name: name.trim(),
                filter: { query },
                description: description || undefined,
              } as Pipeline);
              onClose();
            } catch (e) {
              setError(String((e as Error).message ?? e));
            }
          }}
        >
          {initial ? "Update" : "Create"}
        </button>
      </div>
    </Modal>
  );
}

// --- lists ------------------------------------------------------------------------

type Editing = { path: number[]; processor: Processor | null; nested: boolean };

function ProcessorList({
  processors,
  readOnly,
  onChange,
  onEdit,
  path,
}: {
  processors: Processor[];
  readOnly: boolean;
  onChange: (ps: Processor[]) => void;
  onEdit: (e: Editing) => void;
  path: number[];
}) {
  return (
    <ol className="processor-list">
      {processors.map((p, i) => {
        const spec = specOf(p.type);
        const nestedList =
          p.type === "pipeline" ? ((p.processors as Processor[]) ?? []) : null;
        return (
          <li key={i} className={`processor${p.is_enabled ? "" : " disabled"}`}>
            <div className="processor-head">
              <span className="processor-index">{i + 1}</span>
              <Switch
                label="Enable processor"
                on={p.is_enabled}
                disabled={readOnly}
                onChange={(v) =>
                  onChange(
                    processors.map((x, j) =>
                      j === i ? { ...x, is_enabled: v } : x,
                    ),
                  )
                }
              />
              <span className="processor-type">{spec?.label ?? p.type}</span>
              <span className="processor-name">{p.name}</span>
              {p.type === "pipeline" && (
                <span className="mono faint">
                  {(p.filter as { query: string })?.query}
                </span>
              )}
              <div className="spacer" />
              {!readOnly && (
                <span className="row-actions">
                  <button
                    className="btn ghost small"
                    title="Move up"
                    onClick={() => onChange(move(processors, i, -1))}
                  >
                    <ChevronUp />
                  </button>
                  <button
                    className="btn ghost small"
                    title="Move down"
                    onClick={() => onChange(move(processors, i, 1))}
                  >
                    <ChevronDown />
                  </button>
                  <button
                    className="btn ghost small"
                    onClick={() =>
                      onEdit({
                        path: [...path, i],
                        processor: p,
                        nested: path.length > 0,
                      })
                    }
                  >
                    Edit
                  </button>
                  <button
                    className="btn ghost small"
                    title="Delete"
                    onClick={() =>
                      onChange(processors.filter((_, j) => j !== i))
                    }
                  >
                    <TrashIcon />
                  </button>
                </span>
              )}
              {readOnly && (
                <button
                  className="btn ghost small"
                  onClick={() =>
                    onEdit({
                      path: [...path, i],
                      processor: p,
                      nested: path.length > 0,
                    })
                  }
                >
                  View
                </button>
              )}
            </div>
            {nestedList && (
              <div className="nested-pipeline">
                <ProcessorList
                  processors={nestedList}
                  readOnly={readOnly}
                  path={[...path, i]}
                  onEdit={onEdit}
                  onChange={(ps) =>
                    onChange(
                      processors.map((x, j) =>
                        j === i ? { ...x, processors: ps } : x,
                      ),
                    )
                  }
                />
                {!readOnly && (
                  <button
                    className="btn small"
                    onClick={() =>
                      onEdit({
                        path: [...path, i, nestedList.length],
                        processor: null,
                        nested: true,
                      })
                    }
                  >
                    <PlusIcon /> Add Processor
                  </button>
                )}
              </div>
            )}
          </li>
        );
      })}
      {!processors.length && (
        <li className="muted processor-empty">No processors yet.</li>
      )}
    </ol>
  );
}

/** Replaces (or inserts, at the end of a list) the processor at a path. */
function setAt(ps: Processor[], path: number[], p: Processor): Processor[] {
  const [i, ...rest] = path;
  if (!rest.length)
    return i >= ps.length ? [...ps, p] : ps.map((x, j) => (j === i ? p : x));
  return ps.map((x, j) =>
    j === i
      ? {
          ...x,
          processors: setAt((x.processors as Processor[]) ?? [], rest, p),
        }
      : x,
  );
}

export function Pipelines() {
  const qc = useQueryClient();
  const pipelines = useQuery({
    queryKey: ["pipelines"],
    queryFn: api.pipelines,
  });
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [filter, setFilter] = useState("");
  const [pipelineModal, setPipelineModal] = useState<{
    pipeline: Pipeline | null;
  } | null>(null);
  const [editing, setEditing] = useState<
    (Editing & { pipeline: Pipeline }) | null
  >(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = () => qc.invalidateQueries({ queryKey: ["pipelines"] });
  const run = async (f: () => Promise<unknown>) => {
    setError(null);
    try {
      await f();
    } catch (e) {
      setError(String((e as Error).message ?? e));
    }
    await refresh();
  };
  const save = (p: Pipeline) => run(() => api.updatePipeline(p.id!, p));
  const all = pipelines.data ?? [];
  const shown = useMemo(
    () =>
      all.filter(
        (p) =>
          !filter ||
          `${p.name} ${p.filter.query}`
            .toLowerCase()
            .includes(filter.toLowerCase()),
      ),
    [all, filter],
  );

  return (
    <>
      <PageHeader title="Logs" tabs={LOG_TABS} />
      <div className="page-scroll page-padding">
        <p className="muted pipelines-intro">
          Pipelines run in order on every incoming log that matches their
          filter, after the JSON and reserved-attribute preprocessing. Within a
          pipeline, processors run in order. Integration pipelines are
          read-only: clone one to change it.
        </p>
        <div className="toolbar">
          <input
            className="input"
            placeholder="Filter pipelines"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            style={{ width: 280 }}
          />
          <div className="spacer" />
          <button
            className="btn primary"
            onClick={() => setPipelineModal({ pipeline: null })}
          >
            <PlusIcon /> New Pipeline
          </button>
        </div>
        {error && <div className="error-box">{error}</div>}
        <div className="pipelines">
          {shown.map((p) => {
            const idx = all.findIndex((x) => x.id === p.id);
            const isOpen = open.has(p.id!);
            const toggle = () => {
              const s = new Set(open);
              if (isOpen) s.delete(p.id!);
              else s.add(p.id!);
              setOpen(s);
            };
            return (
              <div
                key={p.id}
                className={`pipeline${p.is_enabled ? "" : " disabled"}`}
              >
                <div className="pipeline-head" onClick={toggle}>
                  {isOpen ? <ChevronDown /> : <ChevronRight />}
                  <Switch
                    label="Enable pipeline"
                    on={p.is_enabled}
                    onChange={(v) => save({ ...p, is_enabled: v })}
                  />
                  <span className="pipeline-name">{p.name}</span>
                  {p.is_read_only && (
                    <span className="type-badge">Integration</span>
                  )}
                  <span className="mono pipeline-filter">
                    {p.filter.query || "*"}
                  </span>
                  <div className="spacer" />
                  <span className="muted">
                    {p.processors.length} processor
                    {p.processors.length === 1 ? "" : "s"}
                  </span>
                  <span
                    className="row-actions"
                    onClick={(e) => e.stopPropagation()}
                  >
                    <button
                      className="btn ghost small"
                      title="Move up"
                      disabled={!!filter}
                      onClick={() =>
                        run(() =>
                          api.orderPipelines(
                            move(all, idx, -1).map((x) => x.id!),
                          ),
                        )
                      }
                    >
                      <ChevronUp />
                    </button>
                    <button
                      className="btn ghost small"
                      title="Move down"
                      disabled={!!filter}
                      onClick={() =>
                        run(() =>
                          api.orderPipelines(
                            move(all, idx, 1).map((x) => x.id!),
                          ),
                        )
                      }
                    >
                      <ChevronDown />
                    </button>
                    <button
                      className="btn ghost small"
                      title="Clone"
                      onClick={() => run(() => api.clonePipeline(p.id!))}
                    >
                      <CopyIcon />
                    </button>
                    {!p.is_read_only && (
                      <>
                        <button
                          className="btn ghost small"
                          onClick={() => setPipelineModal({ pipeline: p })}
                        >
                          Edit
                        </button>
                        <button
                          className="btn ghost small"
                          title="Delete"
                          onClick={() => {
                            if (
                              window.confirm(`Delete the pipeline "${p.name}"?`)
                            )
                              run(() => api.deletePipeline(p.id!));
                          }}
                        >
                          <TrashIcon />
                        </button>
                      </>
                    )}
                  </span>
                </div>
                {isOpen && (
                  <div className="pipeline-body">
                    {p.description && <p className="muted">{p.description}</p>}
                    <ProcessorList
                      processors={p.processors}
                      readOnly={p.is_read_only}
                      path={[]}
                      onEdit={(e) => setEditing({ ...e, pipeline: p })}
                      onChange={(ps) => save({ ...p, processors: ps })}
                    />
                    {!p.is_read_only && (
                      <button
                        className="btn small"
                        onClick={() =>
                          setEditing({
                            pipeline: p,
                            path: [p.processors.length],
                            processor: null,
                            nested: false,
                          })
                        }
                      >
                        <PlusIcon /> Add Processor
                      </button>
                    )}
                  </div>
                )}
              </div>
            );
          })}
          {!shown.length && !pipelines.isLoading && (
            <div className="empty-state">No pipeline matches.</div>
          )}
        </div>
      </div>
      {pipelineModal && (
        <PipelineModal
          initial={pipelineModal.pipeline}
          onClose={() => setPipelineModal(null)}
          onSave={async (p) => {
            if (p.id) await api.updatePipeline(p.id, p);
            else {
              const created = await api.createPipeline(p);
              setOpen(new Set([...open, created.id!]));
            }
            await refresh();
          }}
        />
      )}
      {editing && (
        <ProcessorModal
          initial={editing.processor}
          nested={editing.nested}
          readOnly={editing.pipeline.is_read_only}
          onClose={() => setEditing(null)}
          onSave={async (proc) => {
            if (editing.pipeline.is_read_only) return;
            const p = {
              ...editing.pipeline,
              processors: setAt(
                editing.pipeline.processors,
                editing.path,
                proc,
              ),
            };
            await api.updatePipeline(p.id!, p);
            await refresh();
          }}
        />
      )}
    </>
  );
}
