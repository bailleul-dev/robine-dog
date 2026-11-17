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

function StoryCard({ s }: { s: WatchdogStory }) {
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
      <div className="toolbar" style={{ marginTop: 8 }}>
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
