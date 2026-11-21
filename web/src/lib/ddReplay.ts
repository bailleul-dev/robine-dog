/**
 * Datadog's browser SDK (v7) records Session Replay as "change records": a
 * full snapshot (type 2, `format: 1`) or a change (type 12) is a list of
 * changes, `[changeType, ...items]`, whose strings are ids in a string table
 * the changes themselves fill, and whose nodes get the next id in the order
 * they are added. This turns them into rrweb events, which rrweb-player plays.
 * Older SDKs record rrweb's format already: their records pass through.
 */

type Rec = { type: number; timestamp: number; data?: unknown; format?: number };
export type RrwebEvent = { type: number; timestamp: number; data: unknown };

const Change = {
  AddString: 0,
  AddNode: 1,
  RemoveNode: 2,
  Attribute: 3,
  Text: 4,
  Size: 5,
  ScrollPosition: 6,
  AddStyleSheet: 7,
  AttachedStyleSheets: 8,
  MediaPlaybackState: 9,
  VisualViewport: 10,
  AddRoleAnnotatedStrings: 11,
  InputValue: 12,
  InputSelection: 13,
  ClearStrings: 14,
  ImageContent: 15,
} as const;

/** rrweb's incremental sources. */
const Source = {
  Mutation: 0,
  Scroll: 3,
  Input: 5,
  MediaInteraction: 7,
} as const;

/** Ids past Datadog's, for the nodes added here (adopted style sheets). */
const SYNTHETIC = 1_000_000_000;

type VNode = {
  id: number;
  /** `#document`, `#doctype`, `#text`, `#cdata-section`, `#shadow-root`,
   * `#document-fragment`, or an element's `nodeName` (`svg>` for SVG). */
  name: string;
  attrs: Record<string, string>;
  text?: string;
  doctype?: [string, string, string];
  children: VNode[];
  parent?: VNode;
  /** Adopted style sheets (documents, shadow roots), and the sheet of a
   * `<style>` or `<link>`. */
  sheets?: number[];
};

type Sheet = { css: string; media: string[]; disabled: boolean };

type AddOp = {
  parentId: number;
  nextId: number | null;
  node: VNode;
  shadow: boolean;
};

class Decoder {
  strings: string[] = [];
  nodes = new Map<number, VNode>();
  root: VNode | null = null;
  nextId = 0;
  sheets: Sheet[] = [];
  synthetic = SYNTHETIC;
  documentScroll = { left: 0, top: 0 };

  reset() {
    this.strings = [];
    this.nodes.clear();
    this.root = null;
    this.nextId = 0;
    this.sheets = [];
  }

  str(v: unknown): string {
    return typeof v === "number"
      ? (this.strings[v] ?? "")
      : typeof v === "string"
        ? v
        : "";
  }

  /** rrweb's id of a node: Datadog's start at 0, which rrweb treats as none. */
  static rr(id: number) {
    return id >= SYNTHETIC ? id : id + 1;
  }

  /** The node's parent as rrweb sees it: a shadow root's host. */
  rrParent(n: VNode): { parent: VNode | undefined; shadow: boolean } {
    const p = n.parent;
    if (
      p &&
      (p.name === "#shadow-root" || p.name === "#document-fragment") &&
      p.parent
    )
      return { parent: p.parent, shadow: true };
    return { parent: p, shadow: false };
  }

  nextSibling(n: VNode): VNode | null {
    const siblings = n.parent?.children ?? [];
    const i = siblings.indexOf(n);
    return i >= 0 && i + 1 < siblings.length ? siblings[i + 1] : null;
  }

  insert(n: VNode, point: number | null): void {
    const id = n.id;
    if (point === null) {
      this.root = n;
      return;
    }
    if (point === 0) {
      // Right after the previous node.
      const prev = this.nodes.get(id - 1);
      const parent = prev?.parent;
      if (!prev || !parent) return;
      parent.children.splice(parent.children.indexOf(prev) + 1, 0, n);
      n.parent = parent;
    } else if (point < 0) {
      // Before a sibling.
      const next = this.nodes.get(id + point);
      const parent = next?.parent;
      if (!next || !parent) return;
      parent.children.splice(parent.children.indexOf(next), 0, n);
      n.parent = parent;
    } else {
      // The last child of a parent.
      const parent = this.nodes.get(id - point);
      if (!parent) return;
      parent.children.push(n);
      n.parent = parent;
    }
  }

  addNode(item: unknown[]): VNode {
    const [point, nameRef, ...rest] = item;
    const name = this.str(nameRef);
    const n: VNode = { id: this.nextId++, name, attrs: {}, children: [] };
    if (name === "#text") n.text = this.str(rest[0]);
    else if (name === "#doctype")
      n.doctype = [this.str(rest[0]), this.str(rest[1]), this.str(rest[2])];
    else if (!name.startsWith("#"))
      for (const a of rest) {
        if (Array.isArray(a) && a.length >= 2)
          n.attrs[this.str(a[0])] = this.str(a[1]);
      }
    this.nodes.set(n.id, n);
    this.insert(n, typeof point === "number" ? point : null);
    return n;
  }

  removeNode(id: number): VNode | undefined {
    const n = this.nodes.get(id);
    if (!n) return undefined;
    const drop = (x: VNode) => {
      this.nodes.delete(x.id);
      x.children.forEach(drop);
    };
    drop(n);
    if (n.parent) n.parent.children = n.parent.children.filter((c) => c !== n);
    return n;
  }

  sheetCss(ids: number[]): string {
    return ids
      .map((i) => this.sheets[i])
      .filter((s) => s && !s.disabled)
      .map((s) =>
        s.media.length ? `@media ${s.media.join(", ")} { ${s.css} }` : s.css,
      )
      .join("\n");
  }

  addStyleSheet(item: unknown[]) {
    const [rules, media, disabled] = item;
    const css = Array.isArray(rules)
      ? rules.map((r) => this.str(r)).join("\n")
      : this.str(rules);
    this.sheets.push({
      css,
      media: Array.isArray(media) ? media.map((m) => this.str(m)) : [],
      disabled: disabled === true,
    });
  }

  /** rrweb's serialized node, with its children or not. */
  serialize(n: VNode, deep: boolean): Record<string, unknown> | null {
    const id = Decoder.rr(n.id);
    const children = () => {
      const out: Record<string, unknown>[] = [];
      for (const c of n.children) {
        if (c.name === "#shadow-root" || c.name === "#document-fragment") {
          // rrweb puts a shadow root's children in its host, flagged.
          for (const sc of c.children) {
            const s = this.serialize(sc, true);
            if (s) out.push({ ...s, isShadow: true });
          }
          const css = this.sheetCss(c.sheets ?? []);
          if (css) out.push({ ...this.styleNode(css), isShadow: true });
          continue;
        }
        const s = this.serialize(c, true);
        if (s) out.push(s);
      }
      return out;
    };
    switch (n.name) {
      case "#document": {
        const kids = deep ? children() : [];
        // Adopted style sheets: a <style> at the end of <head>.
        const css = this.sheetCss(n.sheets ?? []);
        if (css && deep) {
          const html = kids.find((k) => k.type === 2) as
            { childNodes: Record<string, unknown>[] } | undefined;
          const head = html?.childNodes.find((k) => k.tagName === "head") as
            { childNodes: Record<string, unknown>[] } | undefined;
          (head?.childNodes ?? html?.childNodes)?.push(this.styleNode(css));
        }
        return { type: 0, id, childNodes: kids };
      }
      case "#doctype":
        return {
          type: 1,
          id,
          name: n.doctype?.[0] ?? "html",
          publicId: n.doctype?.[1] ?? "",
          systemId: n.doctype?.[2] ?? "",
        };
      case "#text":
        return { type: 3, id, textContent: n.text ?? "" };
      case "#cdata-section":
        return { type: 4, id, textContent: "" };
      case "#shadow-root":
      case "#document-fragment":
        return null;
      default: {
        const svg = n.name.startsWith("svg>");
        const tagName = svg ? n.name.slice(4) : n.name.toLowerCase();
        const attributes: Record<string, string | number> = { ...n.attrs };
        const css = this.sheetCss(n.sheets ?? []);
        if (css && (tagName === "style" || tagName === "link"))
          attributes._cssText = css;
        if (n.attrs["rr_scrollLeft"])
          attributes.rr_scrollLeft = Number(n.attrs["rr_scrollLeft"]);
        if (n.attrs["rr_scrollTop"])
          attributes.rr_scrollTop = Number(n.attrs["rr_scrollTop"]);
        const out: Record<string, unknown> = {
          type: 2,
          id,
          tagName,
          attributes,
          childNodes: deep ? children() : [],
        };
        if (svg) out.isSVG = true;
        if (n.children.some((c) => c.name === "#shadow-root"))
          out.isShadowHost = true;
        return out;
      }
    }
  }

  styleNode(css: string) {
    return {
      type: 2,
      id: this.synthetic++,
      tagName: "style",
      attributes: { _cssText: css },
      childNodes: [],
    };
  }

  /** Applies a record's changes; returns rrweb's events for them. */
  apply(changes: unknown[], ts: number, full: boolean): RrwebEvent[] {
    const adds: AddOp[] = [];
    const removes: { parentId: number; id: number }[] = [];
    const texts: { id: number; value: string }[] = [];
    const attributes = new Map<number, Record<string, string | null>>();
    const extra: RrwebEvent[] = [];
    const incremental = (source: number, data: Record<string, unknown>) =>
      extra.push({ type: 3, timestamp: ts, data: { source, ...data } });
    const setAttr = (n: VNode, name: string, value: string | null) => {
      if (value === null) delete n.attrs[name];
      else n.attrs[name] = value;
      if (!full)
        attributes.set(n.id, {
          ...(attributes.get(n.id) ?? {}),
          [name]: value,
        });
    };
    for (const change of changes) {
      if (!Array.isArray(change) || typeof change[0] !== "number") continue;
      const [type, ...items] = change as [number, ...unknown[]];
      switch (type) {
        case Change.ClearStrings:
          this.strings = [];
          break;
        case Change.AddString:
          for (const s of items) this.strings.push(String(s));
          break;
        case Change.AddRoleAnnotatedStrings:
          for (const group of items)
            if (Array.isArray(group))
              for (const s of group.slice(1)) this.strings.push(String(s));
          break;
        case Change.AddNode:
          for (const item of items) {
            if (!Array.isArray(item)) continue;
            const n = this.addNode(item);
            if (full || !n.parent) continue;
            if (n.name === "#shadow-root" || n.name === "#document-fragment")
              continue;
            const { parent, shadow } = this.rrParent(n);
            if (!parent) continue;
            let next = this.nextSibling(n);
            if (
              next &&
              (next.name === "#shadow-root" ||
                next.name === "#document-fragment")
            )
              next = null;
            adds.push({
              parentId: Decoder.rr(parent.id),
              nextId: next ? Decoder.rr(next.id) : null,
              node: n,
              shadow,
            });
          }
          break;
        case Change.RemoveNode:
          for (const id of items) {
            if (typeof id !== "number") continue;
            const n = this.nodes.get(id);
            const parent = n && this.rrParent(n).parent;
            if (this.removeNode(id) && parent && !full)
              removes.push({
                parentId: Decoder.rr(parent.id),
                id: Decoder.rr(id),
              });
          }
          break;
        case Change.Attribute:
          for (const item of items) {
            if (!Array.isArray(item)) continue;
            const n = this.nodes.get(item[0] as number);
            if (!n) continue;
            for (const a of item.slice(1)) {
              if (!Array.isArray(a)) continue;
              setAttr(n, this.str(a[0]), a.length > 1 ? this.str(a[1]) : null);
            }
          }
          break;
        case Change.Text:
          for (const item of items) {
            if (!Array.isArray(item)) continue;
            const n = this.nodes.get(item[0] as number);
            if (!n) continue;
            n.text = this.str(item[1]);
            if (!full) texts.push({ id: Decoder.rr(n.id), value: n.text });
          }
          break;
        case Change.Size:
          for (const item of items) {
            if (!Array.isArray(item)) continue;
            const n = this.nodes.get(item[0] as number);
            if (n)
              setAttr(n, "style", `width: ${item[1]}px; height: ${item[2]}px;`);
          }
          break;
        case Change.ScrollPosition:
          for (const item of items) {
            if (!Array.isArray(item)) continue;
            const [id, x, y] = item as number[];
            const n = this.nodes.get(id);
            if (!n) continue;
            if (full) {
              if (n.name === "#document")
                this.documentScroll = { left: x, top: y };
              else {
                n.attrs.rr_scrollLeft = String(x);
                n.attrs.rr_scrollTop = String(y);
              }
            } else incremental(Source.Scroll, { id: Decoder.rr(id), x, y });
          }
          break;
        case Change.AddStyleSheet:
          for (const item of items)
            if (Array.isArray(item)) this.addStyleSheet(item);
          break;
        case Change.AttachedStyleSheets:
          for (const item of items) {
            if (!Array.isArray(item)) continue;
            const n = this.nodes.get(item[0] as number);
            if (!n) continue;
            n.sheets = item.slice(1) as number[];
            const isNew = adds.some((a) => a.node === n);
            if (!full && !isNew && !n.name.startsWith("#"))
              attributes.set(n.id, {
                ...(attributes.get(n.id) ?? {}),
                _cssText: this.sheetCss(n.sheets),
              });
          }
          break;
        case Change.MediaPlaybackState:
          for (const item of items) {
            if (!Array.isArray(item)) continue;
            if (!full)
              incremental(Source.MediaInteraction, {
                id: Decoder.rr(item[0] as number),
                type: item[1],
              });
          }
          break;
        case Change.InputValue:
          for (const item of items) {
            if (!Array.isArray(item)) continue;
            const n = this.nodes.get(item[0] as number);
            if (!n) continue;
            n.attrs.value = this.str(item[1]);
            if (!full)
              incremental(Source.Input, {
                id: Decoder.rr(n.id),
                text: n.attrs.value,
                isChecked: false,
              });
          }
          break;
        case Change.InputSelection:
          for (const item of items) {
            if (!Array.isArray(item)) continue;
            const [state, ...ids] = item as number[];
            for (const id of ids) {
              const n = this.nodes.get(id);
              if (!n) continue;
              const checked = state === 0;
              const attr = n.name === "OPTION" ? "selected" : "checked";
              if (checked) n.attrs[attr] = "";
              else delete n.attrs[attr];
              if (!full)
                incremental(Source.Input, {
                  id: Decoder.rr(id),
                  text: "",
                  isChecked: checked,
                });
            }
          }
          break;
        default:
          // Visual viewport, canvas images: nothing rrweb replays.
          break;
      }
    }
    if (full) return [];
    const events: RrwebEvent[] = [];
    if (adds.length || removes.length || texts.length || attributes.size)
      events.push({
        type: 3,
        timestamp: ts,
        data: {
          source: Source.Mutation,
          removes,
          adds: adds
            .map((a) => {
              const node = this.serialize(a.node, false);
              return (
                node && {
                  parentId: a.parentId,
                  nextId: a.nextId,
                  node: a.shadow ? { ...node, isShadow: true } : node,
                }
              );
            })
            .filter(Boolean),
          texts,
          attributes: [...attributes].map(([id, attrs]) => ({
            id: Decoder.rr(id),
            attributes: attrs,
          })),
        },
      });
    return [...events, ...extra];
  }
}

/** rrweb's incremental records carry node ids, shifted like the nodes. */
function shiftIds(data: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = { ...data };
  if (typeof out.id === "number") out.id = Decoder.rr(out.id);
  if (Array.isArray(out.positions))
    out.positions = out.positions.map((p: Record<string, unknown>) =>
      typeof p.id === "number" ? { ...p, id: Decoder.rr(p.id) } : p,
    );
  return out;
}

/** Whether a session was recorded as change records. */
export function usesChanges(records: Rec[]): boolean {
  return records.some(
    (r) =>
      r.type === 12 ||
      (r.type === 2 && (r.format === 1 || Array.isArray(r.data))),
  );
}

/** rrweb events for Datadog's records, in either format. */
export function toRrweb(records: Rec[]): RrwebEvent[] {
  if (!usesChanges(records))
    return records.filter(
      (r) => r.type === 2 || r.type === 3 || r.type === 4,
    ) as RrwebEvent[];
  const d = new Decoder();
  const out: RrwebEvent[] = [];
  for (const r of records) {
    switch (r.type) {
      case 4:
        out.push(r as RrwebEvent);
        break;
      case 2:
        if (!Array.isArray(r.data)) {
          out.push(r as RrwebEvent);
          break;
        }
        d.reset();
        d.documentScroll = { left: 0, top: 0 };
        d.apply(r.data, r.timestamp, true);
        if (d.root) {
          const node = d.serialize(d.root, true);
          if (node)
            out.push({
              type: 2,
              timestamp: r.timestamp,
              data: { node, initialOffset: d.documentScroll },
            });
        }
        break;
      case 12:
        if (Array.isArray(r.data) && d.root)
          out.push(...d.apply(r.data, r.timestamp, false));
        break;
      case 3:
        if (r.data && typeof r.data === "object")
          out.push({
            type: 3,
            timestamp: r.timestamp,
            data: shiftIds(r.data as Record<string, unknown>),
          });
        break;
      default:
        break;
    }
  }
  return out;
}
