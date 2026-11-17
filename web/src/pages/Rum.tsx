import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { Link, useNavigate } from "react-router-dom";
import { TimeChart } from "../components/Charts";
import { PageHeader } from "../components/Layout";
import { TimePicker } from "../components/TimePicker";
import { api, type RumPage } from "../lib/api";
import { ago } from "../lib/format";
import { thousands } from "../lib/format";
import { useQueryParam, useTimeRange } from "../lib/time";
import { RUM_TABS } from "./Events";

/** Google's Core Web Vitals thresholds: good up to the first, poor past the second. */
const VITALS: { key: keyof RumPage; label: string; limits: [number, number]; ns: boolean }[] = [
  { key: "loading_time", label: "Loading", limits: [2.5e9, 4e9], ns: true },
  { key: "largest_contentful_paint", label: "LCP", limits: [2.5e9, 4e9], ns: true },
  { key: "first_contentful_paint", label: "FCP", limits: [1.8e9, 3e9], ns: true },
  { key: "interaction_to_next_paint", label: "INP", limits: [2e8, 5e8], ns: true },
  { key: "cumulative_layout_shift", label: "CLS", limits: [0.1, 0.25], ns: false },
];

function Vital({ value, limits, ns }: { value: number | null; limits: [number, number]; ns: boolean }) {
  if (value === null) return <span className="muted">–</span>;
  const cls = value <= limits[0] ? "slo-ok" : value <= limits[1] ? "slo-warning" : "slo-breached";
  const text = ns ? (value >= 1e9 ? `${(value / 1e9).toFixed(2)} s` : `${Math.round(value / 1e6)} ms`) : value.toFixed(3);
  return <span className={cls}>{text}</span>;
}

export function RumPerformancePage() {
  const time = useTimeRange("1d");
  const [query] = useQueryParam("query");
  const scope = { query, from: time.from, to: time.to };
  const q = useQuery({
    queryKey: ["rum", "performance", scope],
    queryFn: ({ signal }) => api.rumPerformance(scope, signal),
    placeholderData: keepPreviousData,
  });
  const d = q.data;
  return (
    <>
      <PageHeader title="Real User Monitoring" tabs={RUM_TABS}>
        <TimePicker time={time} />
      </PageHeader>
      <div className="page-scroll page-padding">
        <div className="slo-tiles" style={{ gridTemplateColumns: "repeat(4, 1fr)" }}>
          {(
            [
              ["Sessions", d?.sessions],
              ["Page views", d?.views],
              ["Actions", d?.actions],
              ["Errors", d?.errors],
            ] as const
          ).map(([label, n]) => (
            <div className="card" key={label}>
              <div className="card-title">{label}</div>
              <div className="slo-big">{n === undefined ? "–" : thousands(n)}</div>
            </div>
          ))}
        </div>
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Page</th>
                <th style={{ width: 150 }}>Service</th>
                <th className="num" style={{ width: 90 }}>
                  Views
                </th>
                {VITALS.map((v) => (
                  <th key={v.key} className="num" style={{ width: 110 }} title="75th percentile">
                    {v.label} p75
                  </th>
                ))}
                <th className="num" style={{ width: 80 }}>
                  Errors
                </th>
              </tr>
            </thead>
            <tbody>
              {(d?.pages ?? []).map((p) => (
                <tr key={`${p.service}|${p.name}`}>
                  <td className="mono ellipsis">
                    <Link to={`/rum/explorer?query=${encodeURIComponent(`source:view "${p.name}"`)}`}>{p.name}</Link>
                  </td>
                  <td className="muted ellipsis">{p.service}</td>
                  <td className="num">{thousands(p.views)}</td>
                  {VITALS.map((v) => (
                    <td key={v.key} className="num">
                      <Vital value={p[v.key] as number | null} limits={v.limits} ns={v.ns} />
                    </td>
                  ))}
                  <td className={`num${p.errors > 0 ? " infra-bad" : ""}`}>{p.errors}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {d && d.pages.length === 0 && (
            <div className="empty-state">
              <h3>No page view in this time range</h3>
              Add Datadog's browser SDK to a site, as Datadog documents it, with <code>proxy</code> pointing here:
              <pre className="event-text" style={{ textAlign: "left", maxWidth: 760, margin: "12px auto 0" }}>
                {`<script>
  (function(h,o,u,n,d) {
    h=h[d]=h[d]||{q:[],onReady:function(c){h.q.push(c)}}
    d=o.createElement(u);d.async=1;d.src=n
    n=o.getElementsByTagName(u)[0];n.parentNode.insertBefore(d,n)
  })(window,document,'script','https://www.datadoghq-browser-agent.com/us1/v6/datadog-rum.js','DD_RUM')
  window.DD_RUM.onReady(function() {
    window.DD_RUM.init({
      clientToken: '<ROBINE_DOG_CLIENT_TOKEN>',
      applicationId: 'my-app',
      proxy: '${window.location.origin}/rum/proxy',
      service: 'my-app-web',
      env: 'prod',
      sessionSampleRate: 100,
    });
  })
</script>`}
              </pre>
            </div>
          )}
        </div>
      </div>
    </>
  );
}

function length(ms: number) {
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s} s`;
  if (s < 3600) return `${Math.floor(s / 60)} min ${s % 60} s`;
  return `${Math.floor(s / 3600)} h ${Math.floor((s % 3600) / 60)} min`;
}

/** What each visitor did: a session is the events sharing a `session.id`. */
export function RumSessions() {
  const navigate = useNavigate();
  const time = useTimeRange("1d");
  const [query] = useQueryParam("query");
  const scope = { query, from: time.from, to: time.to };
  const q = useQuery({
    queryKey: ["rum", "sessions", scope],
    queryFn: ({ signal }) => api.rumSessions(scope, signal),
    placeholderData: keepPreviousData,
  });
  const rows = q.data?.sessions ?? [];
  const open = (id: string, start: number, end: number) =>
    `/rum/explorer?query=${encodeURIComponent(`@session.id:${id}`)}&from_ts=${start - 60_000}&to_ts=${end + 60_000}&live=false`;
  return (
    <>
      <PageHeader title="Real User Monitoring" tabs={RUM_TABS}>
        <TimePicker time={time} />
      </PageHeader>
      <div className="page-scroll page-padding">
        <div className="card" style={{ padding: 0 }}>
          <table className="list infra-table">
            <thead>
              <tr>
                <th style={{ width: 130 }}>Started</th>
                <th style={{ width: 170 }}>Service</th>
                <th>Entered at</th>
                <th style={{ width: 170 }}>User</th>
                <th className="num" style={{ width: 110 }}>
                  Duration
                </th>
                <th className="num" style={{ width: 70 }}>
                  Views
                </th>
                <th className="num" style={{ width: 80 }}>
                  Actions
                </th>
                <th className="num" style={{ width: 70 }}>
                  Errors
                </th>
              </tr>
            </thead>
            <tbody>
              {rows.map((s) => (
                <tr key={s.id} onClick={() => navigate(open(s.id, s.start, s.end))}>
                  <td className="muted" title={new Date(s.start).toLocaleString()}>
                    {ago(s.start)}
                  </td>
                  <td className="ellipsis">{s.service}</td>
                  <td className="mono ellipsis">
                    <Link to={open(s.id, s.start, s.end)}>{s.first_view ?? "–"}</Link>
                  </td>
                  <td className="muted ellipsis">{s.user ?? "anonymous"}</td>
                  <td className="num">{length(s.end - s.start)}</td>
                  <td className="num">{s.views}</td>
                  <td className="num">{s.actions}</td>
                  <td className={`num${s.errors > 0 ? " infra-bad" : ""}`}>{s.errors}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {q.data && rows.length === 0 && (
            <div className="empty-state">
              <h3>No session in this time range</h3>
              Sessions appear as soon as a site with Datadog's browser SDK is visited.
            </div>
          )}
        </div>
      </div>
    </>
  );
}

const BREAKDOWNS: { title: string; facet: string; metric: string; unit: string }[] = [
  { title: "Top pages", facet: "@view.url_path", metric: "@view.id", unit: "views" },
  { title: "Sites", facet: "@view.url_host", metric: "@session.id", unit: "sessions" },
  { title: "Browsers", facet: "@browser.name", metric: "@session.id", unit: "sessions" },
  { title: "Operating systems", facet: "@os.name", metric: "@session.id", unit: "sessions" },
  { title: "Devices", facet: "@device.type", metric: "@session.id", unit: "sessions" },
  { title: "Referrers", facet: "@view.referrer", metric: "@session.id", unit: "sessions" },
];

function Breakdown({ b, query, from, to }: { b: (typeof BREAKDOWNS)[number]; query: string; from: number; to: number }) {
  const q = useQuery({
    queryKey: ["rum", "breakdown", b.facet, query, from, to],
    queryFn: ({ signal }) =>
      api.aggregate(
        "rum",
        {
          compute: [{ aggregation: "cardinality", metric: b.metric }],
          filter: { query: `source:view ${query}`.trim(), from, to },
          group_by: [{ facet: b.facet, limit: 10 }],
        },
        signal,
      ),
    placeholderData: keepPreviousData,
  });
  const rows = (q.data?.data.buckets ?? [])
    .map((r) => ({ label: r.by[b.facet] ?? "", n: typeof r.computes.c0 === "number" ? r.computes.c0 : 0 }))
    .sort((x, y) => y.n - x.n);
  const max = Math.max(1, ...rows.map((r) => r.n));
  return (
    <div className="card">
      <div className="card-title">
        {b.title} <span className="muted">· {b.unit}</span>
      </div>
      <div className="widget-toplist">
        {rows.map((r) => (
          <Link
            className="widget-toplist-row"
            key={r.label}
            to={`/rum/explorer?query=${encodeURIComponent(`source:view ${r.label ? `${b.facet}:"${r.label}" ` : ""}${query}`.trim())}&from_ts=${from}&to_ts=${to}&live=false`}
          >
            <span className="widget-toplist-label mono ellipsis" title={r.label}>
              {r.label || "(none)"}
            </span>
            <span className="widget-toplist-bar">
              <i style={{ width: `${(r.n / max) * 100}%` }} />
            </span>
            <span className="widget-toplist-value">{thousands(r.n)}</span>
          </Link>
        ))}
        {q.data && rows.length === 0 && <div className="muted">No data</div>}
      </div>
    </div>
  );
}

/** Who visits: sessions over time, then pages, browsers, systems, devices and referrers. */
export function RumAnalytics() {
  const time = useTimeRange("1w");
  const [query] = useQueryParam("query");
  const interval = Math.max(60_000, Math.ceil((time.to - time.from) / 60 / 60_000) * 60_000);
  const series = useQuery({
    queryKey: ["rum", "analytics", query, time.from, time.to],
    queryFn: ({ signal }) =>
      api.aggregate(
        "rum",
        {
          compute: [
            { aggregation: "cardinality", metric: "@session.id", type: "timeseries", interval: String(interval) },
            { aggregation: "cardinality", metric: "@view.id", type: "timeseries", interval: String(interval) },
          ],
          filter: { query: `source:view ${query}`.trim(), from: time.from, to: time.to },
          group_by: [],
        },
        signal,
      ),
    placeholderData: keepPreviousData,
  });
  const bucket = series.data?.data.buckets[0];
  const points = (k: string) => (Array.isArray(bucket?.computes[k]) ? (bucket!.computes[k] as { time: string; value: number | null }[]) : []);
  const step = series.data?.meta.interval_ms ?? interval;
  const chart = (title: string, k: string, color: string) => (
    <div className="card">
      <div className="card-title">{title}</div>
      <TimeChart
        times={points(k).map((p) => Date.parse(p.time))}
        interval={step}
        from={time.from}
        to={time.to}
        height={120}
        series={[{ key: k, label: title, color, values: points(k).map((p) => p.value ?? 0) }]}
        onZoom={(a, b) => time.setAbsolute(Math.round(a), Math.round(b))}
      />
    </div>
  );
  return (
    <>
      <PageHeader title="Real User Monitoring" tabs={RUM_TABS}>
        <TimePicker time={time} />
      </PageHeader>
      <div className="page-scroll page-padding">
        <div className="rum-grid" style={{ gridTemplateColumns: "repeat(2, 1fr)" }}>
          {chart("Sessions", "c0", "#8b6fd0")}
          {chart("Page views", "c1", "#3d9bd9")}
        </div>
        <div className="rum-grid">
          {BREAKDOWNS.map((b) => (
            <Breakdown key={b.facet} b={b} query={query} from={time.from} to={time.to} />
          ))}
        </div>
      </div>
    </>
  );
}
