import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { PageHeader } from "../components/Layout";
import { TimePicker } from "../components/TimePicker";
import { api, type FlameNode } from "../lib/api";
import { ago, compact, duration } from "../lib/format";
import { useTimeRange } from "../lib/time";

/** A value in the profile's unit. */
function fmt(v: number, unit: string) {
  if (unit === "nanoseconds") return duration(v);
  if (unit === "bytes")
    return v < 1024
      ? `${v} B`
      : v < 1 << 20
        ? `${(v / 1024).toFixed(1)} KiB`
        : `${(v / (1 << 20)).toFixed(1)} MiB`;
  return compact(v);
}

const TYPE_LABELS: Record<string, string> = {
  "cpu-time": "CPU time",
  cpu: "CPU time",
  "wall-time": "Wall time",
  wall: "Wall time",
  "alloc-space": "Allocated memory",
  "alloc-samples": "Allocations",
  "inuse-space": "Heap live size",
  "inuse-objects": "Heap live objects",
  "lock-acquire-wait": "Lock wait time",
  "lock-release-hold": "Lock hold time",
  "exception-samples": "Thrown exceptions",
};

/** A frame's color, by its package, as flame graphs usually do. */
function frameColor(name: string) {
  const pkg = name.split(/[.(/ ]/)[0];
  let h = 0;
  for (const c of pkg) h = (h * 31 + c.charCodeAt(0)) % 360;
  return `hsl(${20 + (h % 40)}, 75%, ${58 + (h % 12)}%)`;
}

type Row = { node: FlameNode; x: number; w: number; depth: number };

/** Flame graph: the root on top, callees below, width by value. Click to zoom. */
function FlameGraph({
  tree,
  unit,
  highlight,
}: {
  tree: FlameNode;
  unit: string;
  highlight: string;
}) {
  const [path, setPath] = useState<string[]>([]);
  const [hover, setHover] = useState<Row | null>(null);
  // The zoomed node, found again by its path when the tree refreshes.
  const focus = useMemo(() => {
    let n = tree;
    for (const p of path) {
      const c = n.children.find((x) => x.name === p);
      if (!c) return { node: tree, path: [] as string[] };
      n = c;
    }
    return { node: n, path };
  }, [tree, path]);
  const rows = useMemo(() => {
    const out: Row[] = [];
    const walk = (n: FlameNode, x: number, w: number, depth: number) => {
      out.push({ node: n, x, w, depth });
      let cx = x;
      for (const c of n.children) {
        const cw = (c.value / Math.max(1, n.value)) * w;
        if (cw >= 0.0008) walk(c, cx, cw, depth + 1);
        cx += cw;
      }
    };
    walk(focus.node, 0, 1, 0);
    return out;
  }, [focus]);
  const depth = Math.max(...rows.map((r) => r.depth)) + 1;
  const H = 18;
  const total = tree.value || 1;
  return (
    <div className="flamegraph">
      <div className="flame-crumbs">
        <button className="link-button" onClick={() => setPath([])}>
          all
        </button>
        {focus.path.map((p, i) => (
          <span key={i}>
            {" › "}
            <button
              className="link-button mono"
              onClick={() => setPath(focus.path.slice(0, i + 1))}
            >
              {p}
            </button>
          </span>
        ))}
      </div>
      <div
        className="flame-canvas"
        style={{ height: depth * H }}
        onMouseLeave={() => setHover(null)}
      >
        {rows.map((r, i) => {
          const match =
            highlight &&
            r.node.name.toLowerCase().includes(highlight.toLowerCase());
          return (
            <div
              key={i}
              className={`flame-frame${match ? " match" : ""}`}
              style={{
                left: `${r.x * 100}%`,
                width: `calc(${r.w * 100}% - 1px)`,
                top: r.depth * H,
                height: H - 1,
                background:
                  r.depth === 0
                    ? "var(--purple-soft)"
                    : frameColor(r.node.name),
                opacity: highlight && !match ? 0.35 : 1,
              }}
              onMouseEnter={() => setHover(r)}
              onClick={() => {
                if (r.depth === 0) setPath(focus.path.slice(0, -1));
                else {
                  // The frame's path from the focus.
                  const names: string[] = [];
                  const find = (n: FlameNode, trail: string[]): boolean => {
                    if (n === r.node) {
                      names.push(...trail);
                      return true;
                    }
                    return n.children.some((c) => find(c, [...trail, c.name]));
                  };
                  find(focus.node, []);
                  setPath([...focus.path, ...names]);
                }
              }}
            >
              {r.w > 0.03 && <span>{r.node.name}</span>}
            </div>
          );
        })}
      </div>
      <div className="flame-hover">
        {hover ? (
          <>
            <span className="mono">{hover.node.name}</span> · total{" "}
            {fmt(hover.node.value, unit)} (
            {((hover.node.value / total) * 100).toFixed(1)}%) · self{" "}
            {fmt(hover.node.self, unit)}
          </>
        ) : (
          <span className="faint">
            Hover a frame for its values; click to zoom in.
          </span>
        )}
      </div>
    </div>
  );
}

export function Profiler() {
  const [params, setParams] = useSearchParams();
  const time = useTimeRange("1h", 60_000);
  const service = params.get("service") ?? "";
  const type = params.get("type") ?? "";
  const endpoint = params.get("endpoint") ?? "";
  const version = params.get("version") ?? "";
  const [highlight, setHighlight] = useState("");
  const set = (k: string, v: string) => {
    const p = new URLSearchParams(params);
    if (v) p.set(k, v);
    else p.delete(k);
    if (k === "service") {
      p.delete("endpoint");
      p.delete("version");
    }
    setParams(p, { replace: true });
  };
  const services = useQuery({
    queryKey: ["profiled-services", time.from, time.to],
    queryFn: () => api.profiledServices({ from: time.from, to: time.to }),
    placeholderData: keepPreviousData,
  });
  const current = service || services.data?.[0]?.service || "";
  const filter = {
    service: current,
    version: version || undefined,
    from: time.from,
    to: time.to,
  };
  const flame = useQuery({
    queryKey: ["flamegraph", filter, type, endpoint],
    queryFn: () =>
      api.flamegraph({
        ...filter,
        type: type || undefined,
        endpoint: endpoint || undefined,
      }),
    enabled: !!current,
    placeholderData: keepPreviousData,
  });
  const list = useQuery({
    queryKey: ["profiles", filter],
    queryFn: () => api.profiles(filter),
    enabled: !!current,
  });
  const f = flame.data;
  const info = services.data?.find((s) => s.service === current);
  return (
    <>
      <PageHeader title="Profiles">
        <TimePicker time={time} />
      </PageHeader>
      <div className="filters-row">
        <label className="muted">
          Service{" "}
          <select
            className="input"
            value={current}
            onChange={(e) => set("service", e.target.value)}
          >
            {(services.data ?? []).map((s) => (
              <option key={s.service} value={s.service}>
                {s.service} ({s.languages.join(", ") || "?"})
              </option>
            ))}
          </select>
        </label>
        <label className="muted">
          Version{" "}
          <select
            className="input"
            value={version}
            onChange={(e) => set("version", e.target.value)}
          >
            <option value="">all</option>
            {(info?.versions ?? []).map((v) => (
              <option key={v}>{v}</option>
            ))}
          </select>
        </label>
        <label className="muted">
          Show{" "}
          <select
            className="input"
            value={f?.type ?? type}
            onChange={(e) => set("type", e.target.value)}
          >
            {Object.keys(f?.types ?? {}).map((t) => (
              <option key={t} value={t}>
                {TYPE_LABELS[t] ?? t}
              </option>
            ))}
          </select>
        </label>
        <label className="muted">
          Endpoint{" "}
          <select
            className="input"
            value={endpoint}
            onChange={(e) => set("endpoint", e.target.value)}
          >
            <option value="">all</option>
            {(f?.endpoints ?? []).map((e) => (
              <option key={e.endpoint} value={e.endpoint}>
                {e.endpoint}
              </option>
            ))}
          </select>
        </label>
        <input
          className="input"
          placeholder="Highlight a function"
          value={highlight}
          onChange={(e) => setHighlight(e.target.value)}
          style={{ width: 200 }}
        />
        <div className="spacer" />
        {f && (
          <span className="muted">
            {f.profiles} profiles merged · {fmt(f.tree.value, f.unit)}
          </span>
        )}
      </div>
      <div className="page-scroll page-padding">
        {services.isSuccess && !services.data.length && (
          <div className="card muted">
            No profile in this period. Enable the profiler in your services (
            <code>DD_PROFILING_ENABLED=true</code>): profiles go through the
            Agent to <code>/api/v2/profile</code>, or straight to the trace port
            at <code>/profiling/v1/input</code>.
          </div>
        )}
        {f && f.tree.value > 0 && (
          <>
            <div className="card">
              <div className="card-title">
                {TYPE_LABELS[f.type] ?? f.type}{" "}
                {endpoint && <span className="muted">· {endpoint}</span>}
              </div>
              <FlameGraph tree={f.tree} unit={f.unit} highlight={highlight} />
            </div>
            <div className="cards" style={{ marginTop: 12 }}>
              <div className="card">
                <div className="card-title">Top functions (self)</div>
                <table className="list">
                  <thead>
                    <tr>
                      <th>Function</th>
                      <th className="num">Self</th>
                      <th className="num">Total</th>
                    </tr>
                  </thead>
                  <tbody>
                    {f.top.slice(0, 20).map((t) => (
                      <tr
                        key={t.name}
                        className="profile-top-row"
                        onClick={() => setHighlight(t.name)}
                      >
                        <td className="mono" style={{ fontSize: 12 }}>
                          <span
                            className="errbar"
                            style={{
                              width: Math.max(
                                2,
                                (t.self / (f.top[0]?.self || 1)) * 50,
                              ),
                              background: "var(--orange, #e3a63c)",
                            }}
                          />
                          {t.name}
                        </td>
                        <td className="num">{fmt(t.self, f.unit)}</td>
                        <td className="num">{fmt(t.total, f.unit)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              <div className="card">
                <div className="card-title">Endpoints</div>
                <table className="list">
                  <tbody>
                    {f.endpoints.map((e) => (
                      <tr
                        key={e.endpoint}
                        onClick={() =>
                          set(
                            "endpoint",
                            e.endpoint === endpoint ? "" : e.endpoint,
                          )
                        }
                      >
                        <td
                          className="mono"
                          style={{
                            fontWeight: e.endpoint === endpoint ? 700 : 400,
                          }}
                        >
                          {e.endpoint}
                        </td>
                        <td className="num">{fmt(e.value, f.unit)}</td>
                      </tr>
                    ))}
                    {!f.endpoints.length && (
                      <tr>
                        <td className="faint">
                          No sample was taken in a traced request.
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </div>
            </div>
          </>
        )}
        {(list.data?.length ?? 0) > 0 && (
          <>
            <div className="section-title">Profiles</div>
            <div className="card" style={{ padding: 0 }}>
              <table className="list">
                <thead>
                  <tr>
                    <th>Received</th>
                    <th>Host</th>
                    <th>Version</th>
                    <th>Env</th>
                    <th>Types</th>
                    <th className="num">Size</th>
                    <th>Files</th>
                  </tr>
                </thead>
                <tbody>
                  {list.data!.slice(0, 50).map((p) => (
                    <tr key={p.id}>
                      <td className="muted">{ago(p.end)}</td>
                      <td>{p.host ?? "—"}</td>
                      <td>{p.version ?? "—"}</td>
                      <td>{p.env ?? "—"}</td>
                      <td style={{ fontSize: 12 }}>
                        {p.types.map((t) => TYPE_LABELS[t] ?? t).join(", ")}
                      </td>
                      <td className="num">{fmt(p.size, "bytes")}</td>
                      <td>
                        {p.files.map((n) => (
                          <a
                            key={n}
                            className="mono"
                            style={{ marginRight: 8 }}
                            href={`/api/ui/profiling/profiles/${p.id}/files/${encodeURIComponent(n)}`}
                          >
                            {n}
                          </a>
                        ))}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </>
        )}
      </div>
    </>
  );
}
