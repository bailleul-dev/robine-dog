import { keepPreviousData, useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { TimeChart } from "../components/Charts";
import { Modal, SidePanel, StatusPill } from "../components/Common";
import { PlusIcon } from "../components/Icons";
import { JsonTree } from "../components/JsonTree";
import { PageHeader } from "../components/Layout";
import { TimePicker } from "../components/TimePicker";
import { api, ApiError, type Log } from "../lib/api";
import { fullDate, logDate, STATUS_ORDER, statusColor } from "../lib/format";
import { useQueryParam, useTimeRange } from "../lib/time";

const title = (e: Log) => e.message.split("\n", 1)[0];
const body = (e: Log) => e.message.slice(title(e).length + 1);

function PostModal({ onClose }: { onClose: () => void }) {
  const qc = useQueryClient();
  const [t, setT] = useState("");
  const [text, setText] = useState("");
  const [tags, setTags] = useState("");
  const [type, setType] = useState("info");
  const [error, setError] = useState<string | null>(null);
  const save = async () => {
    try {
      await api.postEvent({
        title: t,
        text,
        alert_type: type,
        source_type_name: "user",
        tags: tags
          .split(",")
          .map((x) => x.trim())
          .filter(Boolean),
      });
      // The writer batches for a moment before the event can be read back.
      setTimeout(() => qc.invalidateQueries({ queryKey: ["events"] }), 600);
      onClose();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : String(e));
    }
  };
  return (
    <Modal title="Post an event" onClose={onClose} width={520}>
      <div className="modal-body form">
        <label className="form-field">
          <span className="form-label">Title</span>
          <input className="input" autoFocus value={t} onChange={(e) => setT(e.target.value)} placeholder="Deployed shop 1.4.2" />
        </label>
        <label className="form-field">
          <span className="form-label">Text</span>
          <textarea className="input" rows={3} value={text} onChange={(e) => setText(e.target.value)} />
        </label>
        <div style={{ display: "flex", gap: 12 }}>
          <label className="form-field" style={{ width: 150 }}>
            <span className="form-label">Type</span>
            <select className="input" value={type} onChange={(e) => setType(e.target.value)}>
              {["info", "success", "warning", "error"].map((x) => (
                <option key={x}>{x}</option>
              ))}
            </select>
          </label>
          <label className="form-field" style={{ flex: 1 }}>
            <span className="form-label">Tags</span>
            <input className="input mono" value={tags} onChange={(e) => setTags(e.target.value)} placeholder="service:shop, env:prod" />
          </label>
        </div>
        {error && <div className="form-error">{error}</div>}
      </div>
      <div className="modal-footer">
        <button className="btn" onClick={onClose}>
          Cancel
        </button>
        <button className="btn primary" onClick={save}>
          Post
        </button>
      </div>
    </Modal>
  );
}

export const RUM_TABS = [
  { to: "/rum/performance-monitoring", label: "Performance" },
  { to: "/rum/explorer", label: "Sessions & Events" },
];
const RUM_TYPES = ["view", "action", "error", "resource", "long_task"];

export const EventsExplorer = () => <Explorer mode="events" />;
export const RumExplorer = () => <Explorer mode="rum" />;

/** The explorer of events, and of RUM events: both are stored like logs. */
function Explorer({ mode }: { mode: "events" | "rum" }) {
  const rum = mode === "rum";
  const time = useTimeRange(rum ? "1h" : "1d");
  const [query, setQuery] = useQueryParam("query");
  const [draft, setDraft] = useState(query);
  useEffect(() => setDraft(query), [query]);
  const [selected, setSelected] = useState<Log | null>(null);
  const [posting, setPosting] = useState(false);
  const scope = { query, from: time.from, to: time.to };

  const histogram = useQuery({
    queryKey: [mode, "histogram", scope],
    queryFn: ({ signal }) => (rum ? api.rumHistogram : api.eventsHistogram)(scope, signal),
    placeholderData: keepPreviousData,
  });
  const list = useInfiniteQuery({
    queryKey: [mode, "list", scope],
    queryFn: ({ pageParam, signal }) => (rum ? api.searchRum : api.searchEvents)({ ...scope, limit: 100, cursor: pageParam ?? undefined }, signal),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.next_cursor,
    placeholderData: keepPreviousData,
  });
  const events = list.data?.pages.flatMap((p) => p.logs) ?? [];
  const total = list.data?.pages[0]?.total;
  const error = list.error instanceof ApiError ? list.error.message : null;
  const statuses = STATUS_ORDER.filter((s) => histogram.data?.buckets.some((b) => b.counts[s]));

  return (
    <>
      <PageHeader title={rum ? "Real User Monitoring" : "Events"} tabs={rum ? RUM_TABS : undefined}>
        {!rum && (
          <button className="btn" onClick={() => setPosting(true)}>
            <PlusIcon /> Post Event
          </button>
        )}
        <TimePicker time={time} />
      </PageHeader>
      <form
        className="searchbar-row"
        onSubmit={(e) => {
          e.preventDefault();
          setQuery(draft.trim());
        }}
      >
        <input
          className="input mono"
          style={{ flex: 1 }}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          placeholder={
            rum
              ? "Search RUM events: source:error service:shop-web @view.url_path:/cart @session.id:…"
              : "Search events: source:alert status:error service:shop @monitor.id:12"
          }
          aria-label="Search events"
        />
      </form>
      {error && <div className="form-error" style={{ padding: "0 16px 8px" }}>{error}</div>}
      <div className="page-scroll">
        {histogram.data && (
          <div className="histogram" style={{ padding: "0 16px" }}>
            <TimeChart
              times={histogram.data.buckets.map((b) => b.t)}
              interval={histogram.data.interval_ms}
              from={time.from}
              to={time.to}
              height={80}
              series={statuses.map((s) => ({
                key: s,
                color: statusColor(s),
                values: histogram.data!.buckets.map((b) => b.counts[s] ?? 0),
              }))}
              onZoom={(a, b) => time.setAbsolute(Math.round(a), Math.round(b))}
            />
          </div>
        )}
        <div className="filters-row">
          {rum &&
            RUM_TYPES.map((t) => (
              <button
                key={t}
                className={`btn small${query.includes(`source:${t}`) ? " selected" : ""}`}
                onClick={() => {
                  const rest = query.replace(/\s*source:\w+/g, "").trim();
                  setQuery(query.includes(`source:${t}`) ? rest : `source:${t} ${rest}`.trim());
                }}
              >
                {t.replace("_", " ")}s
              </button>
            ))}
          <div className="spacer" />
          <span className="muted">{total ?? events.length} events</span>
        </div>
        <table className="list infra-table">
          <thead>
            <tr>
              <th style={{ width: 150 }}>Date</th>
              <th style={{ width: 80 }}>{rum ? "Status" : "Type"}</th>
              <th style={{ width: 130 }}>{rum ? "Type" : "Source"}</th>
              {rum && <th style={{ width: 150 }}>Service</th>}
              <th>Event</th>
              <th style={{ width: "24%" }}>Tags</th>
            </tr>
          </thead>
          <tbody>
            {events.map((e) => (
              <tr key={e.id} className={selected?.id === e.id ? "selected" : ""} onClick={() => setSelected(e)}>
                <td className="date">{logDate(e.timestamp)}</td>
                <td>
                  <StatusPill status={e.status} />
                </td>
                <td className="muted ellipsis">{e.source}</td>
                {rum && <td className="muted ellipsis">{e.service}</td>}
                <td className="ellipsis">
                  <strong>{title(e)}</strong> <span className="muted">{body(e).replace(/\s+/g, " ").slice(0, 200)}</span>
                </td>
                <td className="muted ellipsis mono">{e.tags.join(" ")}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {list.data && events.length === 0 && (
          <div className="empty-state">
            <h3>No events in this time range</h3>
            {rum ? (
              <>
                Point Datadog's browser SDK here: <code>proxy: "{window.location.origin}/rum/proxy"</code> in{" "}
                <code>datadogRum.init</code>, with the client token.
              </>
            ) : (
              <>
                Monitors post one when they change state, Agents when something happens on a host, and anything can
                through <code>POST /api/v1/events</code>.
              </>
            )}
          </div>
        )}
        {list.hasNextPage && (
          <div className="list-footer">
            <button className="btn" onClick={() => list.fetchNextPage()} disabled={list.isFetchingNextPage}>
              Load more
            </button>
          </div>
        )}
      </div>
      {selected && (
        <SidePanel
          onClose={() => setSelected(null)}
          storageKey="event"
          header={
            <>
              <StatusPill status={selected.status} /> <strong style={{ marginLeft: 8 }}>{title(selected)}</strong>
            </>
          }
        >
          <div className="infra-detail">
            <div className="muted" style={{ marginBottom: 10 }}>
              {fullDate(selected.timestamp)} · {selected.source}
              {selected.host ? ` · ${selected.host}` : ""}
            </div>
            {body(selected) && <pre className="event-text">{body(selected)}</pre>}
            <div className="infra-facts" style={{ margin: "12px 0" }}>
              {selected.tags.map((t) => (
                <span className="tag" key={t} onClick={() => setQuery(t)} title="Search this tag">
                  {t}
                </span>
              ))}
            </div>
            {rum && typeof (selected.attributes.session as { id?: string } | undefined)?.id === "string" && (
              <button
                className="btn small"
                style={{ marginBottom: 12 }}
                onClick={() => {
                  setQuery(`@session.id:${(selected.attributes.session as { id: string }).id}`);
                  setSelected(null);
                }}
              >
                Show the whole session
              </button>
            )}
            <div className="section-title">Attributes</div>
            <JsonTree data={selected.attributes} />
          </div>
        </SidePanel>
      )}
      {posting && <PostModal onClose={() => setPosting(false)} />}
    </>
  );
}
