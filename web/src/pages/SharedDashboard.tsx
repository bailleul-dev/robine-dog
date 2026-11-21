// A dashboard shared by a public link (`/p/:token`): read-only, without a
// session. Its widgets' queries run on the server from the stored dashboard.
import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import { useParams, useSearchParams } from "react-router-dom";
import { Logo } from "../components/Icons";
import { TimePicker } from "../components/TimePicker";
import { publicApi, type Widget, type WidgetRequest } from "../lib/api";
import { errorText, type Ctx, type Remote, type Vars } from "../lib/dash";
import { useTimeRange } from "../lib/time";
import { FreeCanvas, WidgetCard } from "./Dashboards";

export function SharedDashboard() {
  const { token = "" } = useParams();
  const dash = useQuery({
    queryKey: ["public-dashboard", token],
    queryFn: () => publicApi.dashboard(token),
    retry: false,
  });
  const d = dash.data;
  if (dash.error)
    return (
      <div className="shared-dash">
        <div className="page-padding error-box">
          {errorText(dash.error) === "not found"
            ? "This link does not exist, or was revoked."
            : errorText(dash.error)}
        </div>
      </div>
    );
  if (!d) return <div className="loading">Loading…</div>;
  return <Shared token={token} d={d} />;
}

function Shared({
  token,
  d,
}: {
  token: string;
  d: NonNullable<Awaited<ReturnType<typeof publicApi.dashboard>>>;
}) {
  const time = useTimeRange(d.global_time?.live_span ?? "1h", 60_000);
  const [params, setParams] = useSearchParams();
  const selectable = d.selectable_template_vars ?? [];
  const defs = d.template_variables ?? [];
  const vars: Vars = Object.fromEntries(
    defs.map((v) => [
      v.name,
      (selectable.some((s) => s.name === v.name) &&
        params.get(`tpl_var_${v.name}`)) ||
        v.default ||
        "*",
    ]),
  );
  // Each request of the dashboard, by identity: the server runs it from its
  // widget id and position.
  const where = useMemo(() => {
    const m = new Map<WidgetRequest, [number, number]>();
    const walk = (ws: Widget[]) =>
      ws.forEach((w) => {
        if (w.id !== undefined && Array.isArray(w.definition.requests))
          w.definition.requests.forEach((r, i) => m.set(r, [w.id!, i]));
        walk(w.definition.widgets ?? []);
      });
    walk(d.widgets);
    return m;
  }, [d.widgets]);
  const remote: Remote = {
    run: (r, from, to, scalar, signal) => {
      const at = where.get(r);
      if (!at)
        return Promise.reject(
          new Error("This widget is not available on a public dashboard."),
        );
      const shown = Object.fromEntries(
        selectable.map((s) => [s.name, vars[s.name] ?? "*"]),
      );
      return publicApi.query(
        token,
        { widget_id: at[0], request: at[1], from, to, scalar, vars: shown },
        signal,
      );
    },
  };
  const ctx: Ctx = { time, defs, vars, remote };
  return (
    <div className="shared-dash">
      <header className="shared-dash-header">
        <Logo width={26} height={26} />
        <h1>{d.title}</h1>
        <div className="spacer" />
        {selectable.map((s) => {
          const def = defs.find((v) => v.name === s.name);
          const options = s.visible_tags?.length
            ? s.visible_tags
            : (def?.available_values ?? []);
          const set = (value: string) =>
            setParams((prev) => {
              const p = new URLSearchParams(prev);
              p.set(`tpl_var_${s.name}`, value);
              return p;
            });
          return (
            <label key={s.name} className="tvar">
              <span className="tvar-name">{s.prefix || s.name}</span>
              {options.length ? (
                <select
                  className="input small"
                  value={vars[s.name]}
                  onChange={(e) => set(e.target.value)}
                >
                  {[def?.default ?? "*", ...options]
                    .filter((v, i, a) => a.indexOf(v) === i)
                    .map((v) => (
                      <option key={v} value={v}>
                        {v}
                      </option>
                    ))}
                </select>
              ) : (
                <input
                  className="input small"
                  defaultValue={vars[s.name]}
                  onKeyDown={(e) => {
                    if (e.key === "Enter")
                      set((e.target as HTMLInputElement).value || "*");
                  }}
                />
              )}
            </label>
          );
        })}
        {d.global_time_selectable_enabled ? (
          <TimePicker time={time} allowLive={false} />
        ) : (
          <span className="faint">Past {d.global_time?.live_span ?? "1h"}</span>
        )}
      </header>
      {d.description && <p className="muted page-padding">{d.description}</p>}
      <div className="dash-scroll page-padding">
        {d.layout_type === "free" ? (
          <FreeCanvas widgets={d.widgets} ctx={ctx} />
        ) : (
          <div className="dash-grid">
            {d.widgets.map((w, i) => (
              <WidgetCard key={w.id ?? i} w={w} ctx={ctx} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
