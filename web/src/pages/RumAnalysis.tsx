import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { Link, useParams, useSearchParams } from "react-router-dom";
import { PageHeader } from "../components/Layout";
import { TimePicker } from "../components/TimePicker";
import { FunnelBars, funnelSteps } from "../components/Widgets";
import { api, ApiError, type RumFrustrationTally, type RumReplay } from "../lib/api";
import { toRrweb } from "../lib/ddReplay";
import { ago, thousands } from "../lib/format";
import { useQueryParam, useTimeRange } from "../lib/time";
import { RUM_TABS } from "./Events";

const FRUSTRATION_LABELS: Record<string, string> = {
  rage_click: "Rage clicks",
  dead_click: "Dead clicks",
  error_click: "Error clicks",
};

const errorText = (e: unknown) => (e instanceof ApiError ? e.message : e ? String(e) : null);

/** The Explorer, on one session's events. */
export const sessionEvents = (id: string, start: number, end: number) =>
  `/rum/explorer?query=${encodeURIComponent(`@session.id:${id}`)}&from_ts=${start - 60_000}&to_ts=${end + 60_000}&live=false`;

function Player({ replay }: { replay: RumReplay }) {
  const box = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const events = toRrweb(replay.records);
    if (!events.some((e) => e.type === 2)) {
      setError("No full snapshot in this replay: the page can't be rebuilt.");
      return;
    }
    let player: unknown = null;
    let gone = false;
    (async () => {
      const [{ default: RRWebPlayer }] = await Promise.all([
        import("rrweb-player"),
        import("rrweb-player/dist/style.css"),
      ]);
      if (gone) return;
      el.innerHTML = "";
      const width = Math.max(320, el.clientWidth - 8);
      player = new RRWebPlayer({
        target: el,
        props: {
          // Datadog's records follow rrweb's format.
          events: events as never,
          width,
          height: Math.round(width * 0.56),
          autoPlay: false,
          skipInactive: true,
          showController: true,
        },
      });
    })().catch((e) => setError(String(e)));
    return () => {
      gone = true;
      (player as { $destroy?: () => void } | null)?.$destroy?.();
    };
  }, [replay]);
  return (
    <>
      {error && <div className="error-box">{error}</div>}
      <div ref={box} className="rum-replay-player" />
    </>
  );
}

/** A session's replay, view by view, as Datadog's Session Replay shows it. */
export function RumReplayPage() {
  const { id = "" } = useParams();
  const [params, setParams] = useSearchParams();
  const view = params.get("view") ?? undefined;
  const q = useQuery({
    queryKey: ["rum", "replay", id, view],
    queryFn: ({ signal }) => api.rumReplay(id, view, signal),
  });
  const d = q.data;
  const start = d?.views[0]?.start ?? 0;
  const end = d?.views.at(-1)?.end ?? 0;
  return (
    <>
      <PageHeader title="Session Replay" tabs={RUM_TABS} />
      <div className="page-scroll page-padding">
        <div className="row-actions" style={{ marginBottom: 12, gap: 16 }}>
          <Link to="/rum/sessions">← Sessions</Link>
          <span className="mono muted">{id}</span>
          {d && d.views.length > 0 && (
            <Link to={sessionEvents(id, start, end)}>Session events</Link>
          )}
        </div>
        {q.error && <div className="error-box">{errorText(q.error)}</div>}
        {d && d.views.length === 0 && (
          <div className="empty-state">
            <h3>No replay for this session</h3>
            The browser SDK records sessions with <code>sessionReplaySampleRate</code> above 0.
          </div>
        )}
        {d && d.views.length > 0 && (
          <div className="rum-replay">
            <div className="card rum-replay-views">
              <div className="card-title">Views</div>
              <button
                className={`rum-replay-view${view ? "" : " active"}`}
                onClick={() => setParams({})}
              >
                Whole session <span className="muted">· {d.views.length} views</span>
              </button>
              {d.views.map((v, i) => (
                <button
                  key={v.view_id}
                  className={`rum-replay-view${view === v.view_id ? " active" : ""}`}
                  onClick={() => setParams({ view: v.view_id })}
                >
                  {i + 1}. {new Date(v.start).toLocaleTimeString()}{" "}
                  <span className="muted">
                    · {Math.max(1, Math.round((v.end - v.start) / 1000))} s · {thousands(v.records)} records
                  </span>
                </button>
              ))}
            </div>
            <div className="card">
              <Player replay={d} />
            </div>
          </div>
        )}
      </div>
    </>
  );
}

function TypeBadges({ t }: { t: Record<string, number> }) {
  return (
    <>
      {Object.entries(t).map(([k, n]) => (
        <span key={k} className={`rum-frustration ${k}`} title={FRUSTRATION_LABELS[k]}>
          {FRUSTRATION_LABELS[k]?.replace(" clicks", "") ?? k} {n}
        </span>
      ))}
    </>
  );
}

/** Rage, dead and error clicks: where users get stuck. */
export function RumFrustrations() {
  const time = useTimeRange("1w");
  const [query] = useQueryParam("query");
  const scope = { query, from: time.from, to: time.to };
  const q = useQuery({
    queryKey: ["rum", "frustrations", scope],
    queryFn: ({ signal }) => api.rumFrustrations(scope, signal),
    placeholderData: keepPreviousData,
  });
  const d = q.data;
  const explore = (extra: string) =>
    `/rum/explorer?query=${encodeURIComponent(`source:action @action.frustration.type:* ${extra}`.trim())}&from_ts=${time.from}&to_ts=${time.to}&live=false`;
  const row = (t: RumFrustrationTally, link: string, label: React.ReactNode) => (
    <tr key={`${t.view}|${t.target ?? ""}`}>
      <td className="mono ellipsis">
        <Link to={link}>{label}</Link>
      </td>
      <td>
        <TypeBadges t={t.by_type} />
      </td>
      <td className="num">{thousands(t.count)}</td>
      <td className="num">{thousands(t.sessions)}</td>
    </tr>
  );
  return (
    <>
      <PageHeader title="Real User Monitoring" tabs={RUM_TABS}>
        <TimePicker time={time} />
      </PageHeader>
      <div className="page-scroll page-padding">
        {q.error && <div className="error-box">{errorText(q.error)}</div>}
        <div className="slo-tiles" style={{ gridTemplateColumns: "repeat(5, 1fr)" }}>
          <div className="card">
            <div className="card-title">Frustrated sessions</div>
            <div className="slo-big">
              {d ? `${d.sessions ? ((d.total.sessions / d.sessions) * 100).toFixed(1) : "0"} %` : "–"}
            </div>
            <div className="muted">
              {d ? `${thousands(d.total.sessions)} of ${thousands(d.sessions)}` : ""}
            </div>
          </div>
          {Object.entries(FRUSTRATION_LABELS).map(([k, label]) => (
            <div className="card" key={k}>
              <div className="card-title">{label}</div>
              <div className="slo-big">
                <Link to={explore(`@action.frustration.type:${k}`)}>
                  {d ? thousands(d.total.by_type[k] ?? 0) : "–"}
                </Link>
              </div>
            </div>
          ))}
          <div className="card">
            <div className="card-title">All frustrations</div>
            <div className="slo-big">{d ? thousands(d.total.count) : "–"}</div>
          </div>
        </div>
        <div className="rum-grid" style={{ gridTemplateColumns: "repeat(2, 1fr)" }}>
          <div className="card" style={{ padding: 0 }}>
            <div className="card-title" style={{ padding: "12px 12px 0" }}>
              Pages
            </div>
            <table className="list">
              <thead>
                <tr>
                  <th>View</th>
                  <th>Signals</th>
                  <th className="num" style={{ width: 80 }}>
                    Clicks
                  </th>
                  <th className="num" style={{ width: 80 }}>
                    Sessions
                  </th>
                </tr>
              </thead>
              <tbody>
                {(d?.pages ?? []).map((p) =>
                  row(p, explore(`@view.url_path:"${p.view}"`), p.view),
                )}
              </tbody>
            </table>
          </div>
          <div className="card" style={{ padding: 0 }}>
            <div className="card-title" style={{ padding: "12px 12px 0" }}>
              Elements
            </div>
            <table className="list">
              <thead>
                <tr>
                  <th>Element</th>
                  <th>Signals</th>
                  <th className="num" style={{ width: 80 }}>
                    Clicks
                  </th>
                  <th className="num" style={{ width: 80 }}>
                    Sessions
                  </th>
                </tr>
              </thead>
              <tbody>
                {(d?.targets ?? []).map((t) =>
                  row(
                    t,
                    explore(`@view.url_path:"${t.view}" @action.target.name:"${t.target}"`),
                    <>
                      {t.target} <span className="muted">on {t.view}</span>
                    </>,
                  ),
                )}
              </tbody>
            </table>
          </div>
        </div>
        <div className="card" style={{ padding: 0 }}>
          <div className="card-title" style={{ padding: "12px 12px 0" }}>
            Latest
          </div>
          <table className="list">
            <thead>
              <tr>
                <th style={{ width: 120 }}>When</th>
                <th style={{ width: 200 }}>Signals</th>
                <th>Element</th>
                <th>View</th>
                <th style={{ width: 160 }}>User</th>
                <th style={{ width: 110 }} />
              </tr>
            </thead>
            <tbody>
              {(d?.latest ?? []).map((f, i) => (
                <tr key={i}>
                  <td className="muted" title={new Date(f.ts).toLocaleString()}>
                    {ago(f.ts)}
                  </td>
                  <td>
                    <TypeBadges t={Object.fromEntries(f.types.map((t) => [t, 1]))} />
                  </td>
                  <td className="ellipsis">{f.target}</td>
                  <td className="mono ellipsis">{f.view}</td>
                  <td className="muted ellipsis">{f.user ?? "anonymous"}</td>
                  <td>
                    {f.session_id && (
                      <Link to={sessionEvents(f.session_id, f.ts - 600_000, f.ts + 600_000)}>Session</Link>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {d && d.total.count === 0 && (
            <div className="empty-state">
              No frustration signal in this range. The browser SDK flags them with{" "}
              <code>trackUserInteractions: true</code>.
            </div>
          )}
        </div>
      </div>
    </>
  );
}

const DEFAULT_STEPS = ["/", "/products/*", "/cart", "/checkout", "/order/confirmed"];

/** Conversion through a series of views or actions, by session. */
export function RumFunnels() {
  const time = useTimeRange("1w");
  const [query, setQuery] = useQueryParam("query");
  const [raw, setRaw] = useQueryParam("steps");
  const stored = raw ? raw.split("\n") : DEFAULT_STEPS;
  const [draft, setDraft] = useState(stored);
  const steps = funnelSteps(
    stored.filter((s) => s.trim()).map((s) => {
      const m = /^(@[\w.]+):(.*)$/.exec(s.trim());
      return m ? { facet: m[1], value: m[2] } : { facet: "@view.url_path", value: s.trim() };
    }),
  );
  const q = useQuery({
    queryKey: ["rum", "funnel", JSON.stringify(steps), query, time.from, time.to],
    queryFn: ({ signal }) => api.rumFunnel({ from: time.from, to: time.to, query, steps }, signal),
    enabled: steps.length > 0,
    placeholderData: keepPreviousData,
  });
  const apply = () => setRaw(draft.filter((s) => s.trim()).join("\n"));
  return (
    <>
      <PageHeader title="Real User Monitoring" tabs={RUM_TABS}>
        <TimePicker time={time} />
      </PageHeader>
      <div className="page-scroll page-padding">
        <div className="rum-grid" style={{ gridTemplateColumns: "340px 1fr" }}>
          <div className="card form">
            <div className="card-title">Steps</div>
            {draft.map((s, i) => (
              <div className="row-actions" key={i}>
                <span className="muted" style={{ width: 18 }}>
                  {i + 1}.
                </span>
                <input
                  className="input mono"
                  value={s}
                  onChange={(e) => setDraft(draft.map((x, j) => (j === i ? e.target.value : x)))}
                  onKeyDown={(e) => e.key === "Enter" && apply()}
                />
                <button
                  className="btn ghost small"
                  title="Remove"
                  onClick={() => setDraft(draft.filter((_, j) => j !== i))}
                >
                  ×
                </button>
              </div>
            ))}
            <div className="row-actions">
              <button className="btn small" onClick={() => setDraft([...draft, ""])}>
                Add step
              </button>
              <button className="btn primary small" onClick={apply}>
                Apply
              </button>
            </div>
            <span className="form-hint">
              A view path (wildcards allowed), or <code>@facet:value</code>, like{" "}
              <code>@action.target.name:Pay</code>. A session counts at a step if it went through
              the steps before, in order.
            </span>
            <label className="form-field">
              <span className="form-label">Sessions matching</span>
              <input
                className="input mono"
                defaultValue={query}
                placeholder="@geo.country:France"
                onKeyDown={(e) => e.key === "Enter" && setQuery(e.currentTarget.value)}
                onBlur={(e) => setQuery(e.currentTarget.value)}
              />
            </label>
          </div>
          <div className="card">
            <div className="card-title">Conversion</div>
            {q.error && <div className="error-box">{errorText(q.error)}</div>}
            {q.data && <FunnelBars steps={q.data.steps} />}
            {q.data && q.data.steps.length > 1 && (
              <p className="muted">
                {q.data.steps.at(-1)!.conversion.toFixed(1)} % of the sessions at step 1 reached the last
                step. The biggest drop is after step{" "}
                {q.data.steps
                  .slice(1)
                  .reduce((b, s, i, a) => (s.dropoff > a[b].dropoff ? i : b), 0) + 1}
                .
              </p>
            )}
          </div>
        </div>
      </div>
    </>
  );
}

function cohortLabel(ms: number, week: boolean) {
  const d = new Date(ms);
  const day = d.toLocaleDateString(undefined, { month: "short", day: "numeric", timeZone: "UTC" });
  return week ? `Week of ${day}` : day;
}

/** Of the users who first came in a day or week, how many came back after. */
export function RumRetention() {
  const time = useTimeRange("2w");
  const [period, setPeriod] = useQueryParam("period");
  const [start, setStart] = useQueryParam("start");
  const [ret, setRet] = useQueryParam("return");
  const [by, setBy] = useQueryParam("by");
  const week = period === "week";
  const req = {
    start_query: start,
    return_query: ret || undefined,
    period: (week ? "week" : "day") as "day" | "week",
    by: by || "@usr.id",
    from: time.from,
    to: time.to,
  };
  const q = useQuery({
    queryKey: ["rum", "retention", req],
    queryFn: ({ signal }) => api.rumRetention(req, signal),
    placeholderData: keepPreviousData,
  });
  const cohorts = q.data?.cohorts ?? [];
  const width = Math.max(0, ...cohorts.map((c) => c.rates.length));
  const total = cohorts.reduce((n, c) => n + c.users, 0);
  // Every cohort's rate k periods later, weighted by size.
  const average = Array.from({ length: width }, (_, k) => {
    const these = cohorts.filter((c) => c.rates.length > k);
    const n = these.reduce((s, c) => s + c.users, 0);
    return n ? (these.reduce((s, c) => s + c.returned[k], 0) / n) * 100 : 0;
  });
  const cell = (r: number) => ({
    background: `color-mix(in srgb, var(--accent, #7c5cff) ${Math.round(Math.min(100, r) * 0.9)}%, transparent)`,
  });
  const field = (label: string, value: string, set: (v: string) => void, placeholder: string) => (
    <label className="form-field">
      <span className="form-label">{label}</span>
      <input
        className="input mono"
        defaultValue={value}
        placeholder={placeholder}
        onKeyDown={(e) => e.key === "Enter" && set(e.currentTarget.value)}
        onBlur={(e) => set(e.currentTarget.value)}
      />
    </label>
  );
  return (
    <>
      <PageHeader title="Real User Monitoring" tabs={RUM_TABS}>
        <TimePicker time={time} />
      </PageHeader>
      <div className="page-scroll page-padding">
        <div className="card form">
          <div className="form-row">
            {field("Users who did", start, setStart, "*  (any event)")}
            {field("And came back to", ret, setRet, "the same")}
            {field("User", by, setBy, "@usr.id")}
            <label className="form-field" style={{ flex: "0 0 140px" }}>
              <span className="form-label">By</span>
              <select className="input" value={week ? "week" : "day"} onChange={(e) => setPeriod(e.target.value)}>
                <option value="day">Day</option>
                <option value="week">Week</option>
              </select>
            </label>
          </div>
        </div>
        {q.error && <div className="error-box">{errorText(q.error)}</div>}
        <div className="card" style={{ padding: 0, overflowX: "auto" }}>
          <table className="list rum-retention">
            <thead>
              <tr>
                <th style={{ width: 150 }}>Cohort</th>
                <th className="num" style={{ width: 80 }}>
                  Users
                </th>
                {Array.from({ length: width }, (_, k) => (
                  <th key={k} className="num">
                    {week ? "W" : "D"}
                    {k + 1}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {cohorts.length > 0 && (
                <tr className="rum-retention-avg">
                  <td>All users</td>
                  <td className="num">{thousands(total)}</td>
                  {average.map((r, k) => (
                    <td key={k} className="num" style={cell(r)}>
                      {r.toFixed(0)} %
                    </td>
                  ))}
                </tr>
              )}
              {cohorts.map((c) => (
                <tr key={c.start}>
                  <td>{cohortLabel(c.start, week)}</td>
                  <td className="num">{thousands(c.users)}</td>
                  {Array.from({ length: width }, (_, k) => (
                    <td
                      key={k}
                      className="num"
                      style={k < c.rates.length ? cell(c.rates[k]) : undefined}
                      title={k < c.rates.length ? `${c.returned[k]} of ${c.users}` : undefined}
                    >
                      {k < c.rates.length ? `${c.rates[k].toFixed(0)} %` : ""}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
          {q.data && cohorts.length === 0 && (
            <div className="empty-state">
              No user in this range. Retention follows <code>{by || "@usr.id"}</code>, set by the SDK's{" "}
              <code>setUser</code>.
            </div>
          )}
        </div>
      </div>
    </>
  );
}
