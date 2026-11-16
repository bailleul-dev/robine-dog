import { useEffect, useMemo, useRef, useState } from "react";
import { api } from "../lib/api";
import { compact } from "../lib/format";
import { highlight, quoteValue } from "../lib/query";
import { SearchIcon } from "./Icons";

export type KeyHint = { path: string; name: string };

type Suggestion = { kind: "key" | "value"; insert: string; label: string; detail?: string };

function currentToken(text: string, caret: number): { start: number; token: string } {
  let start = caret;
  let inQuote = false;
  for (let i = 0; i < caret; i++) if (text[i] === '"') inQuote = !inQuote;
  while (start > 0) {
    const c = text[start - 1];
    if (!inQuote && (/\s/.test(c) || c === "(" || c === ")")) break;
    start--;
  }
  return { start, token: text.slice(start, caret) };
}

export function SearchBar({
  value,
  onSubmit,
  dataset,
  scope,
  keys,
  error,
  placeholder,
}: {
  value: string;
  onSubmit: (q: string) => void;
  dataset: "logs" | "spans";
  scope: { from: number; to: number };
  keys: KeyHint[];
  error?: string | null;
  placeholder?: string;
}) {
  const [draft, setDraft] = useState(value);
  const [focused, setFocused] = useState(false);
  const [caret, setCaret] = useState(0);
  const [active, setActive] = useState(0);
  const [values, setValues] = useState<Suggestion[]>([]);
  const [dismissed, setDismissed] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const hl = useRef<HTMLDivElement>(null);
  const navigated = useRef(false);

  useEffect(() => setDraft(value), [value]);

  const { start, token } = currentToken(draft, caret);
  const negated = token.startsWith("-");
  const bare = negated ? token.slice(1) : token;
  const colon = bare.indexOf(":");
  const key = colon > 0 ? bare.slice(0, colon) : null;
  const partial = colon > 0 ? bare.slice(colon + 1).replace(/^"/, "") : bare;

  const keySuggestions: Suggestion[] = useMemo(() => {
    if (key) return [];
    const p = partial.toLowerCase();
    return keys
      .filter((k) => !p || k.path.toLowerCase().includes(p) || k.name.toLowerCase().includes(p))
      .slice(0, 12)
      .map((k) => ({ kind: "key", insert: `${k.path}:`, label: k.path, detail: k.name }));
  }, [key, partial, keys]);

  useEffect(() => {
    if (!key || !focused) {
      setValues([]);
      return;
    }
    const ctrl = new AbortController();
    const t = setTimeout(() => {
      api
        .facetValues(dataset, { query: "", from: scope.from, to: scope.to }, [key], 50, ctrl.signal)
        .then((r) => {
          const p = partial.toLowerCase();
          setValues(
            (r.facets[0]?.values ?? [])
              .filter((v) => !p || v.value.toLowerCase().includes(p))
              .slice(0, 12)
              .map((v) => ({ kind: "value", insert: `${key}:${quoteValue(v.value)}`, label: v.value, detail: compact(v.count) })),
          );
        })
        .catch(() => {});
    }, 120);
    return () => {
      clearTimeout(t);
      ctrl.abort();
    };
  }, [key, partial, dataset, scope.from, scope.to, focused]);

  const suggestions = key ? values : token ? keySuggestions : keySuggestions.slice(0, 8);
  const open = focused && !dismissed && suggestions.length > 0;

  useEffect(() => {
    setActive(0);
    navigated.current = false;
  }, [token]);

  const syncScroll = () => {
    if (input.current && hl.current) hl.current.scrollLeft = input.current.scrollLeft;
    if (input.current) setCaret(input.current.selectionStart ?? 0);
  };

  const apply = (s: Suggestion) => {
    const prefix = negated ? "-" : "";
    const before = draft.slice(0, start);
    const after = draft.slice(caret);
    const insert = prefix + s.insert + (s.kind === "value" ? " " : "");
    const next = before + insert + after.replace(/^\S*/, "");
    setDraft(next);
    const pos = (before + insert).length;
    requestAnimationFrame(() => {
      input.current?.setSelectionRange(pos, pos);
      setCaret(pos);
      syncScroll();
    });
    if (s.kind === "value") {
      onSubmit(next.trim());
      setDismissed(true);
    }
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (open && e.key === "ArrowDown") {
      e.preventDefault();
      navigated.current = true;
      setActive((a) => Math.min(a + 1, suggestions.length - 1));
    } else if (open && e.key === "ArrowUp") {
      e.preventDefault();
      navigated.current = true;
      setActive((a) => Math.max(a - 1, 0));
    } else if (open && e.key === "Tab") {
      e.preventDefault();
      apply(suggestions[active]);
    } else if (e.key === "Enter") {
      e.preventDefault();
      // Enter picks a suggestion only once the user moved into the list; otherwise it searches.
      if (open && navigated.current && suggestions[active]) {
        apply(suggestions[active]);
        navigated.current = false;
        return;
      }
      setDismissed(true);
      onSubmit(draft.trim());
    } else if (e.key === "Escape") {
      setDismissed(true);
    } else {
      setDismissed(false);
    }
  };

  return (
    <div className="searchbar">
      <div className={`searchbar-box${focused ? " focused" : ""}${error ? " invalid" : ""}`}>
        <SearchIcon />
        <div className="searchbar-layer">
          <div className="searchbar-highlight" ref={hl} aria-hidden>
            {highlight(draft).map((t, i) => (
              <span key={i} className={t.kind === "text" || t.kind === "space" ? undefined : `hl-${t.kind}`}>
                {t.text}
              </span>
            ))}
          </div>
          <input
            ref={input}
            value={draft}
            spellCheck={false}
            autoComplete="off"
            placeholder={placeholder ?? "Search…"}
            aria-label="Search query"
            onChange={(e) => {
              setDraft(e.target.value);
              setCaret(e.target.selectionStart ?? 0);
              setDismissed(false);
            }}
            onKeyDown={onKeyDown}
            onKeyUp={syncScroll}
            onSelect={syncScroll}
            onScroll={syncScroll}
            onFocus={() => {
              setFocused(true);
              syncScroll();
            }}
            onBlur={() => setTimeout(() => setFocused(false), 150)}
          />
        </div>
      </div>
      {error && <div className="searchbar-error">{error}</div>}
      {open && (
        <div className="suggestions" role="listbox">
          <div className="suggestions-title">{key ? `Values for ${key}` : "Facets & attributes"}</div>
          {suggestions.map((s, i) => (
            <div
              key={s.insert}
              role="option"
              aria-selected={i === active}
              className={`suggestion${i === active ? " active" : ""}`}
              onMouseDown={(e) => {
                e.preventDefault();
                apply(s);
              }}
              onMouseEnter={() => setActive(i)}
            >
              <span className={`mono ${s.kind === "key" ? "hl-key" : "hl-value"}`}>{s.label}</span>
              {s.detail && <span className="faint">{s.detail}</span>}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
