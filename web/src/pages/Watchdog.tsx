import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link } from "react-router-dom";
import { TimeChart } from "../components/Charts";
import { PageHeader } from "../components/Layout";
import { api, type WatchdogStory } from "../lib/api";
import { fullDate, statusColor, STATUS_ORDER } from "../lib/format";

const KINDS: Record<string, string> = { error_logs: "Error logs", apm_errors: "APM errors", apm_latency: "APM latency" };

/** The error logs of the service around the story. */
function LogsChart({ s }: { s: WatchdogStory }) {
  const from = s.start - 6 * 3_600_000;
  const to = Math.min(Date.now(), (s.end ?? s.last_seen) + 3_600_000);
  const q = useQuery({
    queryKey: ["watchdog-chart", s.id, s.last_seen],
    queryFn: ({ signal }) => api.logsHistogram({ query: `service:${s.service} status:error`, from, to }, signal),
  });
  if (!q.data) return null;
  const statuses = STATUS_ORDER.filter((x) => q.data.buckets.some((b) => b.counts[x]));
  return (
    <TimeChart
      times={q.data.buckets.map((b) => b.t)}
      interval={q.data.interval_ms}
      from={from}
      to={to}
      height={90}
      series={statuses.map((x) => ({ key: x, color: statusColor(x), values: q.data.buckets.map((b) => b.counts[x] ?? 0) }))}
    />
  );
}

function Analysis({ s, what }: { s: WatchdogStory; what: "rca" | "impact" }) {
  const rca = useQuery({ queryKey: ["rca", s.id], queryFn: () => api.storyRca(s.id), enabled: what === "rca" });
  const impact = useQuery({ queryKey: ["impact", s.id], queryFn: () => api.storyImpact(s.id), enabled: what === "impact" });
  if (what === "rca") {
    const d = rca.data;
    if (!d) return <div className="muted">Looking at the dependencies…</div>;
    return (
      <div className="watchdog-analysis">
        {d.root_cause ? (
          <p>
            Likely root cause: <b>{d.root_cause}</b>, a downstream dependency with its own anomaly at the same time.
          </p>
        ) : (
          <p className="muted">
            No anomalous dependency downstream of {s.service}
            {d.dependencies.length ? ` (${d.dependencies.map((x) => x.service).join(", ")})` : " (it calls no other service)"}:
            the cause is likely in {s.service} itself.
          </p>
        )}
        {d.anomalous_dependencies.map((a) => (
          <div key={a.service}>
            {"→ ".repeat(a.depth)}
            <Link to={`/apm/services/${encodeURIComponent(a.service)}`}>{a.service}</Link>:{" "}
            {a.stories.map((x) => x.title).join("; ")}
          </div>
        ))}
        {d.impacted_upstream.length > 0 && (
          <p className="muted">
            Called by: {d.impacted_upstream.map((u) => u.service + (u.stories.length ? " (anomalous)" : "")).join(", ")}
          </p>
        )}
      </div>
    );
  }
  const d = impact.data;
  if (!d) return <div className="muted">Looking for RUM sessions…</div>;
  return (
    <div className="watchdog-analysis">
      {d.sessions ? (
        <>
          <p>
            <b>{d.sessions}</b> RUM sessions and <b>{d.users}</b> identified users made requests through {d.traces}{" "}
            affected traces.
          </p>
          {d.views.map((v) => (
            <div key={v.view}>
              <span className="mono">{v.view}</span> · {v.sessions} sessions, {v.requests} requests
            </div>
          ))}
        </>
      ) : (
        <p className="muted">
          No RUM session linked to the {d.traces} affected traces (RUM links requests to traces with{" "}
          <code>allowedTracingUrls</code>).
        </p>
      )}
    </div>
  );
}

function StoryCard({ s }: { s: WatchdogStory }) {
  const [analysis, setAnalysis] = useState<"rca" | "impact" | null>(null);
  const range = `from_ts=${s.start - 3_600_000}&to_ts=${(s.end ?? s.last_seen) + 3_600_000}&live=false`;
  return (
    <div className="card watchdog-story">
      <div className="watchdog-head">
        <span className={`synth-state ${s.end ? "paused" : "alert"}`}>{s.end ? "Resolved" : "Ongoing"}</span>
        <strong>{s.title}</strong>
        <span className="tag">{KINDS[s.kind] ?? s.kind}</span>
      </div>
      <div className="muted">
        {fullDate(s.start)} → {s.end ? fullDate(s.end) : "now"} · peaked at{" "}
        <strong>
          {Math.round(s.value)} {s.unit}
        </strong>
        , against {s.baseline.toFixed(1)} usually
      </div>
      {s.kind === "error_logs" && <LogsChart s={s} />}
      {analysis && <Analysis s={s} what={analysis} />}
      <div className="toolbar" style={{ marginTop: 8 }}>
        {s.kind !== "error_logs" && (
          <>
            <button className="btn small" onClick={() => setAnalysis(analysis === "rca" ? null : "rca")}>
              Root cause
            </button>
            <button className="btn small" onClick={() => setAnalysis(analysis === "impact" ? null : "impact")}>
              Impact
            </button>
          </>
        )}
        {!s.end && (
          <Link className="btn small" to={`/incidents?declare=${encodeURIComponent(s.title)}`}>
            Declare Incident
          </Link>
        )}
        {s.kind === "error_logs" ? (
          <Link className="btn small" to={`/logs?query=${encodeURIComponent(`service:${s.service} status:error`)}&${range}`}>
            View error logs
          </Link>
        ) : (
          <>
            <Link className="btn small" to={`/apm/services/${encodeURIComponent(s.service)}`}>
              Service page
            </Link>
            <Link
              className="btn small"
              to={`/apm/traces?query=${encodeURIComponent(`service:${s.service}${s.kind === "apm_errors" ? " status:error" : ""}`)}&${range}`}
            >
              View traces
            </Link>
          </>
        )}
      </div>
    </div>
  );
}

export function Watchdog() {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ["watchdog"], queryFn: api.watchdog, refetchInterval: 60_000 });
  const [running, setRunning] = useState(false);
  const stories = q.data?.stories ?? [];
  const ongoing = stories.filter((s) => !s.end);
  return (
    <>
      <PageHeader title="Watchdog">
        <button
          className="btn"
          disabled={running}
          onClick={async () => {
            setRunning(true);
            try {
              qc.setQueryData(["watchdog"], await api.runWatchdog());
            } finally {
              setRunning(false);
            }
          }}
        >
          {running ? "Looking…" : "Run detection now"}
        </button>
      </PageHeader>
      <div className="page-scroll page-padding">
        <p className="muted" style={{ maxWidth: 900, marginTop: 0 }}>
          Every five minutes, the last quarter of an hour of each service is compared with its previous day: error logs,
          failed requests and latency far above usual open a story here and post an event (<code>source:watchdog</code>).
          No configuration.
        </p>
        <div className="section-title">Ongoing ({ongoing.length})</div>
        {ongoing.map((s) => (
          <StoryCard key={s.id} s={s} />
        ))}
        {q.data && ongoing.length === 0 && <div className="card muted">Nothing out of the ordinary right now.</div>}
        <div className="section-title" style={{ marginTop: 18 }}>
          Past 30 days
        </div>
        {stories
          .filter((s) => s.end)
          .map((s) => (
            <StoryCard key={s.id} s={s} />
          ))}
        {q.data && stories.every((s) => !s.end) && <div className="card muted">No past story.</div>}
      </div>
    </>
  );
}

/** What stands out in the errors of the last hour, in logs, traces and RUM. */
export function WatchdogInsights() {
  const [to] = useState(() => Date.now());
  const from = to - 3_600_000;
  const sets = [
    { ds: "logs" as const, label: "Logs", link: "/logs" },
    { ds: "spans" as const, label: "Traces", link: "/apm/traces" },
    { ds: "rum" as const, label: "RUM", link: "/rum/explorer" },
  ];
  return (
    <>
      <PageHeader title="Watchdog Insights" />
      <div className="page-scroll page-padding">
        <p className="muted" style={{ marginTop: 0 }}>
          Values over-represented in the errors of the last hour, against all events. The same insights show above the
          explorers&apos; results for their search.
        </p>
        {sets.map((x) => (
          <InsightList key={x.ds} {...x} from={from} to={to} />
        ))}
      </div>
    </>
  );
}

function InsightList({
  ds,
  label,
  link,
  from,
  to,
}: {
  ds: "logs" | "spans" | "rum";
  label: string;
  link: string;
  from: number;
  to: number;
}) {
  const q = useQuery({ queryKey: ["insights-page", ds], queryFn: () => api.insights(ds, "", from, to) });
  return (
    <>
      <div className="section-title">{label}</div>
      <div className="card">
        {!q.data && <span className="muted">Looking…</span>}
        {q.data && !q.data.insights.length && <span className="muted">Nothing stands out.</span>}
        {q.data?.insights.map((i) => (
          <div key={i.facet + i.value}>
            <Link to={`${link}?query=${encodeURIComponent(i.query)}&from_ts=${from}&to_ts=${to}&live=false`}>
              {i.facet}:{i.value}
            </Link>{" "}
            — {i.text}
          </div>
        ))}
      </div>
    </>
  );
}
