import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "../lib/api";

/** Watchdog Insights: facet values over-represented in the errors of the search. */
export function InsightsBanner({
  dataset,
  query,
  from,
  to,
  onApply,
}: {
  dataset: "logs" | "spans" | "rum";
  query: string;
  from: number;
  to: number;
  onApply: (clause: string) => void;
}) {
  const [hidden, setHidden] = useState(false);
  const q = useQuery({
    queryKey: [
      "insights",
      dataset,
      query,
      Math.round(from / 60000),
      Math.round(to / 60000),
    ],
    queryFn: () => api.insights(dataset, query, from, to),
    enabled: !hidden,
    staleTime: 60_000,
  });
  const items = q.data?.insights ?? [];
  if (hidden || !items.length) return null;
  return (
    <div className="insights-banner">
      <span className="insights-title">Watchdog Insights</span>
      {items.map((i) => (
        <button
          key={i.facet + i.value}
          className="insights-item"
          title={i.text}
          onClick={() => onApply(i.query)}
        >
          <b>
            {i.facet}:{i.value}
          </b>{" "}
          {Math.round(i.share * 100)}% of errors{" "}
          <span className="faint">vs {Math.round(i.baseline * 100)}%</span>
        </button>
      ))}
      <button
        className="btn icon ghost"
        onClick={() => setHidden(true)}
        title="Hide"
      >
        ×
      </button>
    </div>
  );
}
