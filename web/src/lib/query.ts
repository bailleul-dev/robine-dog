/**
 * Edits a Datadog search query as the facet panel does: top-level `key:value`,
 * `key:(a OR b)` and `-key:value` clauses are found, replaced or appended.
 */

type Term = { text: string; start: number; end: number };

/** Splits into top-level terms (outside quotes and parentheses). */
export function topLevelTerms(q: string): Term[] {
  const terms: Term[] = [];
  let depth = 0;
  let inQuote = false;
  let start = -1;
  for (let i = 0; i <= q.length; i++) {
    const c = q[i];
    const atEnd = i === q.length;
    if (!atEnd && c === "\\") {
      if (start < 0) start = i;
      i++;
      continue;
    }
    if (!atEnd && c === '"') inQuote = !inQuote;
    if (!atEnd && !inQuote) {
      if (c === "(") depth++;
      if (c === ")") depth = Math.max(0, depth - 1);
    }
    const isSpace = atEnd || (!inQuote && depth === 0 && /\s/.test(c));
    if (isSpace) {
      if (start >= 0) {
        terms.push({ text: q.slice(start, i), start, end: i });
        start = -1;
      }
    } else if (start < 0) {
      start = i;
    }
  }
  return terms;
}

function unquote(v: string): string {
  if (v.startsWith('"') && v.endsWith('"') && v.length >= 2) return v.slice(1, -1).replace(/\\(.)/g, "$1");
  return v.replace(/\\(.)/g, "$1");
}

export function quoteValue(v: string): string {
  if (v === "") return '""';
  if (/[\s"():\\]/.test(v) || v === "AND" || v === "OR" || v === "NOT") return `"${v.replace(/(["\\])/g, "\\$1")}"`;
  return v;
}

function splitOr(inner: string): string[] {
  const out: string[] = [];
  let cur = "";
  let inQuote = false;
  const toks = inner.match(/"(?:[^"\\]|\\.)*"|\S+/g) ?? [];
  for (const t of toks) {
    if (!inQuote && t === "OR") {
      if (cur) out.push(cur);
      cur = "";
      continue;
    }
    cur = cur ? `${cur} ${t}` : t;
  }
  if (cur) out.push(cur);
  return out.map(unquote);
}

/** Values included for `key` by the top-level clauses (`key:v`, `key:(a OR b)`). */
export function selectedValues(q: string, key: string): string[] {
  const prefix = `${key}:`;
  const out: string[] = [];
  for (const t of topLevelTerms(q)) {
    if (!t.text.startsWith(prefix)) continue;
    const v = t.text.slice(prefix.length);
    if (v.startsWith("(") && v.endsWith(")")) out.push(...splitOr(v.slice(1, -1)));
    else out.push(unquote(v));
  }
  return out;
}

function removeTerms(q: string, pred: (t: string) => boolean): string {
  let out = q;
  const terms = topLevelTerms(q).filter((t) => pred(t.text));
  for (const t of terms.reverse()) out = out.slice(0, t.start) + out.slice(t.end);
  return out.replace(/\s+/g, " ").trim();
}

/** Replaces the inclusion clauses of `key` by exactly `values` (none = no filter). */
export function setFacetValues(q: string, key: string, values: string[]): string {
  const base = removeTerms(q, (t) => t.startsWith(`${key}:`));
  if (values.length === 0) return base;
  const clause =
    values.length === 1 ? `${key}:${quoteValue(values[0])}` : `${key}:(${values.map(quoteValue).join(" OR ")})`;
  return base ? `${base} ${clause}` : clause;
}

export function addClause(q: string, key: string, value: string, negate = false): string {
  const clause = `${negate ? "-" : ""}${key}:${quoteValue(value)}`;
  if (topLevelTerms(q).some((t) => t.text === clause)) return q;
  const opposite = `${negate ? "" : "-"}${key}:${quoteValue(value)}`;
  const base = removeTerms(q, (t) => t === opposite);
  return base ? `${base} ${clause}` : clause;
}

/** Datadog's attribute path for a nested key: `@http.status_code`. */
export function attrPath(path: string[]): string {
  return `@${path.join(".")}`;
}

/** Tokens for the search bar's syntax highlighting. */
export type HighlightToken = { text: string; kind: "key" | "value" | "op" | "text" | "space" | "paren" | "neg" };

export function highlight(q: string): HighlightToken[] {
  const out: HighlightToken[] = [];
  const re = /(\s+)|([()])|(-(?=\S))|("(?:[^"\\]|\\.)*"?)|((?:[^\s():"\\]|\\.)+:)|((?:[^\s()\\]|\\.)+)/g;
  let m: RegExpExecArray | null;
  let afterKey = false;
  while ((m = re.exec(q))) {
    if (m[1]) {
      out.push({ text: m[1], kind: "space" });
      afterKey = false;
    } else if (m[2]) {
      out.push({ text: m[2], kind: "paren" });
    } else if (m[3]) {
      out.push({ text: m[3], kind: "neg" });
    } else if (m[4]) {
      out.push({ text: m[4], kind: afterKey ? "value" : "text" });
      afterKey = false;
    } else if (m[5] && !afterKey) {
      out.push({ text: m[5], kind: "key" });
      afterKey = true;
    } else {
      const t = m[5] ?? m[6];
      const isOp = !afterKey && (t === "AND" || t === "OR" || t === "NOT");
      out.push({ text: t, kind: isOp ? "op" : afterKey ? "value" : "text" });
      afterKey = false;
    }
  }
  return out;
}
