import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { Link } from "react-router-dom";
import { PageHeader } from "../components/Layout";
import { TimePicker } from "../components/TimePicker";
import { api, type RumPage } from "../lib/api";
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
