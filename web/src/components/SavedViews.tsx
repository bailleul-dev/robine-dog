import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { api, ApiError } from "../lib/api";
import { ChevronDown, TrashIcon } from "./Icons";

/** Datadog's "Views" menu: save the explorer's state (query, facets, columns, visualization, time) and reopen it. */
export function SavedViews({ page }: { page: "logs" | "traces" }) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  const qc = useQueryClient();
  const navigate = useNavigate();
  const { pathname, search } = useLocation();
  const views = useQuery({
    queryKey: ["views", page],
    queryFn: () => api.views(page),
  });
  const current = (views.data ?? []).find(
    (v) => v.params === search.replace(/^\?/, ""),
  );

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node))
        setOpen(false);
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [open]);

  const save = async () => {
    setError(null);
    try {
      await api.createView({
        name: name.trim(),
        page,
        params: search.replace(/^\?/, ""),
      });
      setName("");
      await qc.invalidateQueries({ queryKey: ["views", page] });
    } catch (e) {
      setError(e instanceof ApiError ? e.message : String(e));
    }
  };

  return (
    <div className="saved-views" ref={ref}>
      <button
        className="btn"
        onClick={() => setOpen(!open)}
        title="Saved views"
      >
        <span className="saved-views-label">
          {current ? current.name : "Views"}
        </span>
        <ChevronDown />
      </button>
      {open && (
        <div className="saved-views-menu">
          <div className="saved-views-title">Saved views</div>
          {(views.data ?? []).map((v) => (
            <div
              key={v.id}
              className={`saved-view${v.id === current?.id ? " active" : ""}`}
            >
              <button
                className="saved-view-open"
                onClick={() => {
                  navigate(`${pathname}?${v.params}`);
                  setOpen(false);
                }}
              >
                {v.name}
              </button>
              <button
                className="btn ghost small"
                title="Delete view"
                onClick={async () => {
                  await api.deleteView(v.id);
                  await qc.invalidateQueries({ queryKey: ["views", page] });
                }}
              >
                <TrashIcon />
              </button>
            </div>
          ))}
          {views.data && !views.data.length && (
            <div className="faint saved-view-empty">No saved view yet.</div>
          )}
          <form
            className="saved-views-save"
            onSubmit={(e) => {
              e.preventDefault();
              if (name.trim()) save();
            }}
          >
            <input
              className="input"
              placeholder="Save the current view as…"
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
            <button className="btn primary small" disabled={!name.trim()}>
              Save
            </button>
          </form>
          {error && <div className="error-text">{error}</div>}
        </div>
      )}
    </div>
  );
}
