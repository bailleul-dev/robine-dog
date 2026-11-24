import { Marked, type Tokens } from "marked";

/**
 * The documentation: the Markdown files of the repository's `docs/`, bundled
 * at build time. Each starts with a front matter:
 *
 *     ---
 *     title: Log Explorer
 *     section: Logs
 *     order: 10
 *     app: /logs          (optional: the page of Robine Dog it documents)
 *     description: …      (optional: one sentence for the index and search)
 *     ---
 *
 * `docs/README.md` is the home page. Links between pages are relative
 * (`log-explorer.md#facets`) so the files read on GitHub too.
 */
const FILES = import.meta.glob("../../../docs/*.md", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

/** The sections of the navigation, in Datadog's documentation order. */
export const DOC_SECTIONS = [
  "Getting Started",
  "Send Data",
  "Search",
  "Logs",
  "APM",
  "Metrics",
  "Infrastructure",
  "Digital Experience",
  "Monitors and Alerting",
  "Dashboards",
  "Service Management",
  "Security",
  "Software Delivery",
  "LLM Observability",
  "Cloud Cost",
  "Reference",
];

export type DocPage = {
  slug: string;
  title: string;
  section: string;
  order: number;
  app?: string;
  description?: string;
  body: string;
};

function parse(path: string, raw: string): DocPage {
  const file = path.split("/").pop()!.replace(/\.md$/, "");
  const slug = file === "README" ? "" : file;
  const meta: Record<string, string> = {};
  let body = raw;
  const m = raw.match(/^---\n([\s\S]*?)\n---\n?/);
  if (m) {
    for (const line of m[1].split("\n")) {
      const i = line.indexOf(":");
      if (i > 0) meta[line.slice(0, i).trim()] = line.slice(i + 1).trim();
    }
    body = raw.slice(m[0].length);
  }
  // The page's title is shown by the layout, not twice.
  body = body.replace(/^\s*# .*\n/, "");
  return {
    slug,
    title: meta.title ?? file,
    section: meta.section ?? "Reference",
    order: Number(meta.order ?? 100),
    app: meta.app,
    description: meta.description,
    body,
  };
}

const rank = (s: string) => {
  const i = DOC_SECTIONS.indexOf(s);
  return i < 0 ? DOC_SECTIONS.length : i;
};

/** Every page, in reading order: section, then order, then title. */
export const DOCS: DocPage[] = Object.entries(FILES)
  .map(([path, raw]) => parse(path, raw))
  .sort(
    (a, b) =>
      rank(a.section) - rank(b.section) ||
      a.order - b.order ||
      a.title.localeCompare(b.title),
  );

export const docBySlug = (slug: string) => DOCS.find((d) => d.slug === slug);

export const docPath = (slug: string) => (slug ? `/docs/${slug}` : "/docs");

export const slugify = (s: string) =>
  s
    .toLowerCase()
    .replace(/<[^>]+>/g, "")
    .replace(/&[a-z]+;/g, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");

/** A link of a page, made a route of the app: `x.md#y` → `/docs/x#y`. */
function route(href: string) {
  if (/^[a-z]+:/i.test(href) || href.startsWith("#") || href.startsWith("/"))
    return href;
  const [file, hash] = href.split("#");
  const name = file.replace(/^\.\//, "").replace(/\.md$/, "");
  return docPath(name === "README" ? "" : name) + (hash ? `#${hash}` : "");
}

const escape = (s: string) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

export type Rendered = { html: string; toc: { id: string; text: string; depth: number }[] };

/**
 * A page as HTML, with its headings for the table of contents. The examples
 * name `http://localhost:8080`; served elsewhere, they show this address.
 */
export function render(page: DocPage): Rendered {
  const toc: Rendered["toc"] = [];
  const origin = window.location.origin;
  const local = /^https?:\/\/(localhost|127\.0\.0\.1)(:\d+)?$/.test(origin);
  const marked = new Marked({
    gfm: true,
    renderer: {
      heading({ tokens, depth }: Tokens.Heading) {
        const inner = this.parser.parseInline(tokens);
        const id = slugify(inner);
        if (depth === 2 || depth === 3) toc.push({ id, text: inner.replace(/<[^>]+>/g, ""), depth });
        return `<h${depth} id="${id}"><a class="doc-anchor" href="#${id}" aria-hidden="true">#</a>${inner}</h${depth}>\n`;
      },
      link({ href, title, tokens }: Tokens.Link) {
        const inner = this.parser.parseInline(tokens);
        const to = route(href);
        const external = /^https?:/.test(to);
        return `<a href="${escape(to)}"${title ? ` title="${escape(title)}"` : ""}${
          external ? ' target="_blank" rel="noreferrer" class="doc-external"' : ""
        }>${inner}</a>`;
      },
      code({ text, lang }: Tokens.Code) {
        const shown = local ? text : text.replaceAll("http://localhost:8080", origin);
        return `<div class="doc-code"><div class="doc-code-head"><span>${escape(lang ?? "")}</span><button class="btn small doc-copy" type="button">Copy</button></div><pre><code>${escape(shown)}</code></pre></div>\n`;
      },
      blockquote({ tokens }: Tokens.Blockquote) {
        const inner = this.parser.parse(tokens);
        const kind = /^<p><strong>(Warning|Caution)/.test(inner) ? "warning" : "note";
        return `<div class="doc-callout ${kind}">${inner}</div>\n`;
      },
      table(token: Tokens.Table) {
        const cell = (c: Tokens.TableCell, tag: string) =>
          `<${tag}${c.align ? ` style="text-align:${c.align}"` : ""}>${this.parser.parseInline(c.tokens)}</${tag}>`;
        return `<div class="doc-table"><table><thead><tr>${token.header
          .map((c) => cell(c, "th"))
          .join("")}</tr></thead><tbody>${token.rows
          .map((r) => `<tr>${r.map((c) => cell(c, "td")).join("")}</tr>`)
          .join("")}</tbody></table></div>\n`;
      },
    },
  });
  return { html: marked.parse(page.body, { async: false }) as string, toc };
}

/** The page's text without Markdown, for the search. */
const plain = (s: string) =>
  s
    .replace(/```[\s\S]*?```/g, " ")
    .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
    .replace(/[`*#>|]+/g, " ")
    .replace(/(^|\s)[-_]+(?=\s)/g, " ")
    .replace(/\s+/g, " ");

const TEXT = new Map(DOCS.map((d) => [d.slug, plain(d.body)]));

export type Hit = { page: DocPage; snippet?: string; score: number };

/** Pages matching every word of a query, titles first, with a snippet. */
export function search(query: string): Hit[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return [];
  const hits: Hit[] = [];
  for (const page of DOCS) {
    const title = page.title.toLowerCase();
    const text = TEXT.get(page.slug)!;
    const lower = text.toLowerCase();
    const raw = page.body.toLowerCase();
    if (!words.every((w) => title.includes(w) || lower.includes(w) || raw.includes(w))) continue;
    let score = 0;
    for (const w of words) {
      if (title.includes(w)) score += 10;
      if (page.description?.toLowerCase().includes(w)) score += 3;
      score += Math.min(5, lower.split(w).length - 1);
    }
    const at = lower.indexOf(words[0]);
    const snippet =
      at >= 0
        ? (at > 60 ? "…" : "") + text.slice(Math.max(0, at - 60), at + 110).trim() + "…"
        : page.description;
    hits.push({ page, snippet, score });
  }
  return hits.sort((a, b) => b.score - a.score);
}
