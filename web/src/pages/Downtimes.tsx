import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link } from "react-router-dom";
import { Modal } from "../components/Common";
import { PlusIcon } from "../components/Icons";
import { PageHeader } from "../components/Layout";
import { api, ApiError, type Downtime } from "../lib/api";
import { fullDate } from "../lib/format";
import { MONITOR_TABS } from "./Monitors";

const DURATIONS: [string, number | null][] = [
  ["1 hour", 3600],
  ["4 hours", 14_400],
  ["1 day", 86_400],
  ["1 week", 604_800],
  ["Until canceled", null],
];
const STATUS_COLORS: Record<string, string> = {
  active: "var(--purple)",
  scheduled: "var(--blue)",
  ended: "var(--text-faint)",
  canceled: "var(--text-faint)",
};

/** `2026-11-17T14:00` in local time, for a datetime-local input. */
function localInput(ms: number): string {
  const d = new Date(ms - new Date(ms).getTimezoneOffset() * 60_000);
  return d.toISOString().slice(0, 16);
}

function ScheduleModal({ onClose }: { onClose: () => void }) {
  const qc = useQueryClient();
  const monitors = useQuery({ queryKey: ["monitors"], queryFn: api.monitors });
  const [monitor, setMonitor] = useState("");
  const [scope, setScope] = useState("");
  const [start, setStart] = useState(() => localInput(Date.now()));
  const [duration, setDuration] = useState<number | null>(3600);
  const [message, setMessage] = useState("");
  const [error, setError] = useState<string | null>(null);
  const save = async () => {
    const startS = Math.floor(new Date(start).getTime() / 1000);
    try {
      await api.createDowntime({
        monitor_id: monitor ? Number(monitor) : null,
        scope: scope.trim() || "*",
        start: startS,
        end: duration === null ? null : startS + duration,
        message: message.trim() || null,
      });
      await qc.invalidateQueries({ queryKey: ["downtimes"] });
      onClose();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : String(e));
    }
  };
  return (
    <Modal title="Schedule Downtime" onClose={onClose} width={520}>
      <div className="modal-body form">
        <label className="form-field">
          <span className="form-label">Monitor</span>
          <select className="input" value={monitor} onChange={(e) => setMonitor(e.target.value)}>
            <option value="">All monitors</option>
            {(monitors.data ?? []).map((m) => (
              <option key={m.id} value={m.id}>
                {m.name}
              </option>
            ))}
          </select>
        </label>
        <label className="form-field">
          <span className="form-label">Scope</span>
          <input className="input mono" placeholder="* (everything)" value={scope} onChange={(e) => setScope(e.target.value)} />
          <span className="form-hint">
            Tags of the groups to silence, comma separated (<code>host:silo01,env:prod</code>), or tags of the monitors.
          </span>
        </label>
        <label className="form-field">
          <span className="form-label">Start</span>
          <input className="input" type="datetime-local" value={start} onChange={(e) => setStart(e.target.value)} />
        </label>
        <label className="form-field">
          <span className="form-label">Duration</span>
          <select
            className="input"
            value={String(duration)}
            onChange={(e) => setDuration(e.target.value === "null" ? null : Number(e.target.value))}
          >
            {DURATIONS.map(([label, s]) => (
              <option key={label} value={String(s)}>
                {label}
              </option>
            ))}
          </select>
        </label>
        <label className="form-field">
          <span className="form-label">Message</span>
          <input className="input" placeholder="Why (optional)" value={message} onChange={(e) => setMessage(e.target.value)} />
        </label>
        {error && <div className="form-error">{error}</div>}
      </div>
      <div className="modal-footer">
        <button className="btn" onClick={onClose}>
          Cancel
        </button>
        <button className="btn primary" onClick={save}>
          Schedule
        </button>
      </div>
    </Modal>
  );
}

export function Downtimes() {
  const qc = useQueryClient();
  const list = useQuery({ queryKey: ["downtimes"], queryFn: api.downtimes, refetchInterval: 30_000 });
  const monitors = useQuery({ queryKey: ["monitors"], queryFn: api.monitors });
  const [scheduling, setScheduling] = useState(false);
  const name = (d: Downtime) => monitors.data?.find((m) => m.id === d.monitor_id)?.name ?? `#${d.monitor_id}`;
  return (
    <>
      <PageHeader title="Monitors" tabs={MONITOR_TABS}>
        <button className="btn primary" onClick={() => setScheduling(true)}>
          <PlusIcon /> Schedule Downtime
        </button>
      </PageHeader>
      <div className="page-scroll page-padding">
        <p className="muted" style={{ maxWidth: 900, marginTop: 0 }}>
          During a downtime the monitors it covers are still evaluated, and their status still changes, but nothing is
          notified. Same API as Datadog's <code>/api/v1/downtime</code>.
        </p>
        <div className="card" style={{ padding: 0 }}>
          <table className="list">
            <thead>
              <tr>
                <th style={{ width: 110 }}>Status</th>
                <th>Monitor</th>
                <th>Scope</th>
                <th style={{ width: 190 }}>Start</th>
                <th style={{ width: 190 }}>End</th>
                <th>Message</th>
                <th style={{ width: 100 }} />
              </tr>
            </thead>
            <tbody>
              {(list.data ?? []).map((d) => (
                <tr key={d.id}>
                  <td style={{ color: STATUS_COLORS[d.status], fontWeight: 600, textTransform: "capitalize" }}>{d.status}</td>
                  <td className="ellipsis">
                    {d.monitor_id ? <Link to={`/monitors/${d.monitor_id}`}>{name(d)}</Link> : "All monitors"}
                    {d.monitor_tags.length > 0 && <span className="muted"> {d.monitor_tags.join(", ")}</span>}
                  </td>
                  <td className="mono ellipsis">{d.scope}</td>
                  <td>{fullDate(d.start * 1000)}</td>
                  <td>{d.end ? fullDate(d.end * 1000) : "Until canceled"}</td>
                  <td className="ellipsis muted">{d.message}</td>
                  <td>
                    <button
                      className="btn ghost small"
                      onClick={async () => {
                        await api.cancelDowntime(d.id);
                        await qc.invalidateQueries({ queryKey: ["downtimes"] });
                      }}
                    >
                      {d.status === "active" || d.status === "scheduled" ? "Cancel" : "Remove"}
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {list.data?.length === 0 && <div className="empty-state">No downtime scheduled.</div>}
        </div>
      </div>
      {scheduling && <ScheduleModal onClose={() => setScheduling(false)} />}
    </>
  );
}
