import { useEffect, useMemo, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { ServiceChip } from "../components/Common";
import { PauseIcon, PlayIcon } from "../components/Icons";
import { PageHeader } from "../components/Layout";
import { SearchBar, type KeyHint } from "../components/SearchBar";
import { api, ApiError, type Log } from "../lib/api";
import { logDate, statusColor } from "../lib/format";
import { useQueryParam } from "../lib/time";
import { LOG_CORE_FACETS, LOG_TABS, LogPanel, logContent } from "./LogsExplorer";

const MAX = 1000;

/** Live Tail: every new log matching the query, as it arrives. */
export function LiveTail() {
  const [query, setQuery] = useQueryParam("query");
  const [params, setParams] = useSearchParams();
  const [logs, setLogs] = useState<Log[]>([]);
  const [paused, setPaused] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [rate, setRate] = useState(0);
  const lastId = useRef<number | null>(null);
  const selectedId = Number(params.get("event")) || null;

  useEffect(() => {
    setLogs([]);
    lastId.current = null;
  }, [query]);

  useEffect(() => {
    if (paused) return;
    let stop = false;
    const tick = async () => {
      const now = Date.now();
      try {
        const r = await api.searchLogs({
          query,
          from: now - 15 * 60_000,
          to: now + 60_000,
          limit: lastId.current === null ? 50 : 500,
          after_id: lastId.current ?? undefined,
        });
        setError(null);
        if (stop) return;
        if (r.logs.length) {
          lastId.current = Math.max(lastId.current ?? 0, ...r.logs.map((l) => l.id));
          setLogs((prev) => [...r.logs, ...prev].slice(0, MAX));
        } else if (lastId.current === null) {
          lastId.current = 0;
        }
        setRate(r.logs.length);
      } catch (e) {
        if (e instanceof ApiError && e.status === 400) setError(e.message);
      }
    };
    tick();
    const id = setInterval(tick, 1000);
    return () => {
      stop = true;
      clearInterval(id);
    };
  }, [query, paused]);

  const keys: KeyHint[] = useMemo(() => LOG_CORE_FACETS.map((f) => ({ path: f.path, name: f.name })), []);
  const selected = logs.find((l) => l.id === selectedId) ?? null;
  const select = (id: number | null) =>
    setParams((prev) => {
      const p = new URLSearchParams(prev);
      if (id) p.set("event", String(id));
      else p.delete("event");
      return p;
    });

  return (
    <>
      <PageHeader title="Logs" tabs={LOG_TABS}>
        <span className="muted">{paused ? "Paused" : `${rate} logs/s`}</span>
        <button className={`btn${paused ? " primary" : ""}`} onClick={() => setPaused((p) => !p)}>
          {paused ? <PlayIcon /> : <PauseIcon />} {paused ? "Resume" : "Pause"}
        </button>
      </PageHeader>
      <div className="searchbar-row">
        <SearchBar value={query} onSubmit={setQuery} dataset="logs" scope={{ from: Date.now() - 900_000, to: Date.now() }} keys={keys} error={error} placeholder="Filter the live stream…" />
      </div>
      <div className="content">
        <div className="table-wrap">
          <table className="list">
            <colgroup>
              <col style={{ width: 6 }} />
              <col style={{ width: 172 }} />
              <col style={{ width: 170 }} />
              <col style={{ width: 150 }} />
              <col />
            </colgroup>
            <thead>
              <tr>
                <th />
                <th>Date</th>
                <th>Host</th>
                <th>Service</th>
                <th>Content</th>
              </tr>
            </thead>
            <tbody>
              {logs.map((l) => (
                <tr key={l.id} className={l.id === selectedId ? "selected" : undefined} onClick={() => select(l.id)}>
                  <td className="status-cell">
                    <span className="status-bar" style={{ background: statusColor(l.status) }} />
                  </td>
                  <td className="date">{logDate(l.timestamp)}</td>
                  <td>{l.host}</td>
                  <td>
                    <ServiceChip name={l.service} />
                  </td>
                  <td className="content-cell">{logContent(l)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {logs.length === 0 && (
            <div className="empty-state">
              <h3>Waiting for logs…</h3>
              New logs matching the query appear here as they are received.
            </div>
          )}
        </div>
      </div>
      {selectedId && <LogPanel id={selectedId} initial={selected} query={query} setQuery={setQuery} onClose={() => select(null)} />}
    </>
  );
}
