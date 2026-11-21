//! LLM Observability: the spans ddtrace's LLMObs SDK sends (agentless to
//! `/api/v2/llmobs`, or through the Agent's EVP proxy), and the evaluations
//! posted to `/api/intake/llm-obs/v{1,2}/eval-metric`. Spans are kept with
//! their prompts and answers, an estimated cost, and `ml_obs.*` metrics.

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use crate::metrics::Sample;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::Read;

/// A span, as stored.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LlmSpan {
    pub trace_id: String,
    pub span_id: String,
    pub parent_id: Option<String>,
    pub ml_app: String,
    pub name: String,
    pub kind: String,
    pub start_ns: i64,
    pub duration: i64,
    pub error: bool,
    pub model_name: Option<String>,
    pub model_provider: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub cost: Option<f64>,
    pub session_id: Option<String>,
    pub service: Option<String>,
    pub env: Option<String>,
    pub tags: Vec<String>,
    pub input: String,
    pub output: String,
    pub body: Value,
}

/// USD per million input and output tokens, by model name prefix (most
/// specific first).
const PRICES: &[(&str, f64, f64)] = &[
    ("gpt-4o-mini", 0.15, 0.6),
    ("gpt-4o", 2.5, 10.0),
    ("gpt-4.1-nano", 0.1, 0.4),
    ("gpt-4.1-mini", 0.4, 1.6),
    ("gpt-4.1", 2.0, 8.0),
    ("gpt-4-turbo", 10.0, 30.0),
    ("gpt-4", 30.0, 60.0),
    ("gpt-3.5-turbo", 0.5, 1.5),
    ("o1-mini", 1.1, 4.4),
    ("o3-mini", 1.1, 4.4),
    ("o4-mini", 1.1, 4.4),
    ("o1", 15.0, 60.0),
    ("o3", 2.0, 8.0),
    ("claude-3-haiku", 0.25, 1.25),
    ("claude-3-5-haiku", 0.8, 4.0),
    ("claude-haiku", 1.0, 5.0),
    ("claude-3-opus", 15.0, 75.0),
    ("claude-opus", 15.0, 75.0),
    ("claude-3-5-sonnet", 3.0, 15.0),
    ("claude-3-7-sonnet", 3.0, 15.0),
    ("claude-sonnet", 3.0, 15.0),
    ("gemini-1.5-flash", 0.075, 0.3),
    ("gemini-1.5-pro", 1.25, 5.0),
    ("gemini-2.0-flash", 0.1, 0.4),
    ("gemini-2.5-flash", 0.3, 2.5),
    ("gemini-2.5-pro", 1.25, 10.0),
    ("mistral-large", 2.0, 6.0),
    ("mistral-small", 0.2, 0.6),
    ("text-embedding-3-small", 0.02, 0.0),
    ("text-embedding-3-large", 0.13, 0.0),
];

/// Estimated cost in USD, when the model is known.
pub fn cost(model: &str, input: i64, output: i64) -> Option<f64> {
    let m = model.to_lowercase();
    let m = m.rsplit('/').next().unwrap_or(&m);
    PRICES
        .iter()
        .find(|(p, ..)| m.starts_with(p))
        .map(|(_, i, o)| (input as f64 * i + output as f64 * o) / 1e6)
}

fn tag<'a>(tags: &'a [String], key: &str) -> Option<&'a str> {
    tags.iter()
        .find_map(|t| t.strip_prefix(key)?.strip_prefix(':'))
        .filter(|v| !v.is_empty())
}

fn id_str(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() && s != "undefined" && s != "0" => Some(s.clone()),
        Value::Number(n) if n.as_u64() != Some(0) => Some(n.to_string()),
        _ => None,
    }
}

/// The text of an input or output: its messages (`role: content`), or its
/// value, or its documents.
fn io_text(v: &Value) -> String {
    if let Some(msgs) = v["messages"].as_array() {
        return msgs
            .iter()
            .map(|m| {
                let content = m["content"].as_str().map(String::from).unwrap_or_else(|| {
                    if m["tool_calls"].is_array() {
                        format!("[tool calls] {}", m["tool_calls"])
                    } else {
                        m["content"].to_string()
                    }
                });
                match m["role"].as_str() {
                    Some(r) if !r.is_empty() => format!("{r}: {content}"),
                    _ => content,
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    match &v["value"] {
        Value::String(s) => return s.clone(),
        Value::Null => {}
        other => return other.to_string(),
    }
    if let Some(docs) = v["documents"].as_array() {
        return docs
            .iter()
            .map(|d| {
                d["text"]
                    .as_str()
                    .or(d["name"].as_str())
                    .unwrap_or("")
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    String::new()
}

/// One span of a payload. `event_tags` and `event_app` come from the event
/// around it.
pub fn parse_span(s: &Value, event_tags: &[String], event_app: Option<&str>) -> Option<LlmSpan> {
    let mut tags: Vec<String> = s["tags"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| t.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    for t in event_tags {
        if !tags.contains(t) {
            tags.push(t.clone());
        }
    }
    let meta = &s["meta"];
    let kind = meta["span.kind"]
        .as_str()
        .or(meta["span"]["kind"].as_str())
        .or(meta["kind"].as_str())
        .unwrap_or("workflow")
        .to_string();
    let metrics = &s["metrics"];
    let tokens = |k: &str| metrics[k].as_f64().map(|v| v as i64);
    let (input_tokens, output_tokens) = (tokens("input_tokens"), tokens("output_tokens"));
    let total_tokens = tokens("total_tokens").or(match (input_tokens, output_tokens) {
        (None, None) => None,
        (i, o) => Some(i.unwrap_or(0) + o.unwrap_or(0)),
    });
    let model_name = meta["model_name"]
        .as_str()
        .filter(|m| !m.is_empty())
        .map(String::from);
    let cost = model_name
        .as_deref()
        .filter(|_| input_tokens.is_some() || output_tokens.is_some())
        .and_then(|m| cost(m, input_tokens.unwrap_or(0), output_tokens.unwrap_or(0)));
    let ml_app = tag(&tags, "ml_app")
        .or(event_app)
        .or(s["ml_app"].as_str())
        .unwrap_or("unknown")
        .to_string();
    let error = s["status"].as_str() == Some("error") || tag(&tags, "error") == Some("1");
    Some(LlmSpan {
        trace_id: id_str(&s["trace_id"])?,
        span_id: id_str(&s["span_id"])?,
        parent_id: id_str(&s["parent_id"]),
        name: s["name"].as_str().unwrap_or(&kind).to_string(),
        start_ns: s["start_ns"].as_i64()?,
        duration: s["duration"].as_i64().unwrap_or(0),
        error,
        model_provider: meta["model_provider"]
            .as_str()
            .filter(|m| !m.is_empty())
            .map(String::from),
        session_id: s["session_id"]
            .as_str()
            .map(String::from)
            .or(tag(&tags, "session_id").map(String::from)),
        service: tag(&tags, "service").map(String::from),
        env: tag(&tags, "env").map(String::from),
        input: io_text(&meta["input"]),
        output: io_text(&meta["output"]),
        body: s.clone(),
        ml_app,
        kind,
        model_name,
        input_tokens,
        output_tokens,
        total_tokens,
        cost,
        tags,
    })
}

/// The spans of a payload: an event (`{ml_app, tags, spans}`), a list of
/// events, or `{data: …}` around either.
pub fn parse_payload(v: &Value) -> Vec<LlmSpan> {
    let v = if v["data"].is_object() || v["data"].is_array() {
        &v["data"]
    } else {
        v
    };
    let events: Vec<&Value> = match v {
        Value::Array(a) => a.iter().collect(),
        other => vec![other],
    };
    let mut out = Vec::new();
    for ev in events {
        let ev = if ev["attributes"].is_object() {
            &ev["attributes"]
        } else {
            ev
        };
        let tags: Vec<String> = ev["tags"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|t| t.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let app = ev["ml_app"].as_str();
        match ev["spans"].as_array() {
            Some(spans) => out.extend(spans.iter().filter_map(|s| parse_span(s, &tags, app))),
            None => out.extend(parse_span(ev, &tags, app)),
        }
    }
    out
}

/// An evaluation of a span.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Eval {
    pub trace_id: String,
    pub span_id: String,
    pub ml_app: String,
    pub label: String,
    pub metric_type: String,
    pub categorical: Option<String>,
    pub score: Option<f64>,
    pub ts: i64,
    pub tags: Vec<String>,
}

pub fn parse_evals(v: &Value, now: i64) -> Vec<Eval> {
    let attrs = &v["data"]["attributes"];
    let metrics = attrs["metrics"].as_array().cloned().unwrap_or_default();
    metrics
        .iter()
        .filter_map(|m| {
            // v1: span_id / trace_id; v2: join_on.span.{span_id, trace_id}.
            let join = &m["join_on"]["span"];
            let span_id = id_str(&m["span_id"]).or(id_str(&join["span_id"]))?;
            let trace_id = id_str(&m["trace_id"]).or(id_str(&join["trace_id"]))?;
            let tags: Vec<String> = m["tags"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|t| t.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let metric_type = m["metric_type"].as_str().unwrap_or("score").to_string();
            Some(Eval {
                ml_app: m["ml_app"]
                    .as_str()
                    .or(tag(&tags, "ml_app"))
                    .unwrap_or("unknown")
                    .to_string(),
                label: m["label"].as_str()?.to_string(),
                categorical: m["categorical_value"]
                    .as_str()
                    .map(String::from)
                    .or(m["boolean_value"].as_bool().map(|b| b.to_string())),
                score: m["score_value"].as_f64(),
                ts: m["timestamp_ms"].as_i64().unwrap_or(now),
                metric_type,
                span_id,
                trace_id,
                tags,
            })
        })
        .collect()
}

fn body_json(headers: &HeaderMap, body: &[u8]) -> Result<Value, String> {
    let gz = headers
        .get(header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("gzip"))
        || body.starts_with(&[0x1f, 0x8b]);
    let raw = if gz {
        let mut out = Vec::new();
        flate2::read::MultiGzDecoder::new(body)
            .read_to_end(&mut out)
            .map_err(|e| format!("gzip: {e}"))?;
        out
    } else {
        body.to_vec()
    };
    serde_json::from_slice(&raw).map_err(|e| format!("JSON: {e}"))
}

/// The `ml_obs.*` metrics of a span.
fn record_metrics(st: &AppState, s: &LlmSpan) {
    let ts = s.start_ns / 1_000_000;
    let mut tags = vec![
        format!("ml_app:{}", s.ml_app),
        format!("span_kind:{}", s.kind),
        format!("error:{}", s.error as u8),
    ];
    for (k, v) in [
        ("model_name", &s.model_name),
        ("model_provider", &s.model_provider),
        ("service", &s.service),
        ("env", &s.env),
    ] {
        if let Some(v) = v {
            tags.push(format!("{k}:{v}"));
        }
    }
    let agg = &st.aggregator;
    agg.add(ts, "ml_obs.span", None, tags.clone(), Sample::Count(1.0));
    agg.add(
        ts,
        "ml_obs.span.duration",
        None,
        tags.clone(),
        Sample::Distribution(s.duration as f64 / 1e9),
    );
    if s.error {
        agg.add(
            ts,
            "ml_obs.span.error",
            None,
            tags.clone(),
            Sample::Count(1.0),
        );
    }
    if s.parent_id.is_none() {
        agg.add(ts, "ml_obs.trace", None, tags.clone(), Sample::Count(1.0));
        agg.add(
            ts,
            "ml_obs.trace.duration",
            None,
            tags.clone(),
            Sample::Distribution(s.duration as f64 / 1e9),
        );
    }
    for (name, v) in [
        ("ml_obs.span.llm.input.tokens", s.input_tokens),
        ("ml_obs.span.llm.output.tokens", s.output_tokens),
        ("ml_obs.span.llm.total.tokens", s.total_tokens),
    ] {
        if let Some(v) = v {
            agg.add(ts, name, None, tags.clone(), Sample::Count(v as f64));
        }
    }
    if let Some(c) = s.cost {
        agg.add(
            ts,
            "ml_obs.span.llm.estimated_cost",
            None,
            tags,
            Sample::Count(c),
        );
    }
}

pub async fn store_spans(st: &AppState, spans: &[LlmSpan]) -> anyhow::Result<()> {
    let c = st.store.pg.conn().await?;
    let stmt = c
        .prepare(
            "INSERT INTO llm_spans (trace_id, span_id, parent_id, ml_app, name, kind, start_ns, duration,
                error, model_name, model_provider, input_tokens, output_tokens, total_tokens, cost,
                session_id, service, env, tags, input, output, body)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22)
             ON CONFLICT (trace_id, span_id) DO NOTHING",
        )
        .await?;
    for s in spans {
        c.execute(
            &stmt,
            &[
                &s.trace_id,
                &s.span_id,
                &s.parent_id,
                &s.ml_app,
                &s.name,
                &s.kind,
                &s.start_ns,
                &s.duration,
                &s.error,
                &s.model_name,
                &s.model_provider,
                &s.input_tokens,
                &s.output_tokens,
                &s.total_tokens,
                &s.cost,
                &s.session_id,
                &s.service,
                &s.env,
                &s.tags,
                &s.input,
                &s.output,
                &s.body,
            ],
        )
        .await?;
        record_metrics(st, s);
    }
    Ok(())
}

async fn store_evals(st: &AppState, evals: &[Eval]) -> anyhow::Result<()> {
    let c = st.store.pg.conn().await?;
    for e in evals {
        c.execute(
            "INSERT INTO llm_evals (trace_id, span_id, ml_app, label, metric_type, categorical, score, ts, tags)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
            &[&e.trace_id, &e.span_id, &e.ml_app, &e.label, &e.metric_type, &e.categorical, &e.score, &e.ts, &e.tags],
        )
        .await?;
        let mut tags = vec![format!("ml_app:{}", e.ml_app), format!("label:{}", e.label)];
        if let Some(v) = &e.categorical {
            tags.push(format!("value:{v}"));
        }
        st.aggregator.add(
            e.ts,
            "ml_obs.evaluation",
            None,
            tags.clone(),
            Sample::Count(1.0),
        );
        if let Some(v) = e.score {
            st.aggregator.add(
                e.ts,
                "ml_obs.evaluation.score",
                None,
                tags,
                Sample::Distribution(v),
            );
        }
    }
    Ok(())
}

fn reply(r: Result<usize, String>) -> Response {
    match r {
        Ok(n) => (StatusCode::ACCEPTED, Json(json!({"accepted": n}))).into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "LLM Observability payload rejected");
            (StatusCode::BAD_REQUEST, Json(json!({"errors": [e]}))).into_response()
        }
    }
}

async fn take_spans(st: &AppState, headers: &HeaderMap, body: &[u8]) -> Result<usize, String> {
    let v = body_json(headers, body)?;
    let spans = parse_payload(&v);
    store_spans(st, &spans).await.map_err(|e| e.to_string())?;
    Ok(spans.len())
}

async fn take_evals(st: &AppState, headers: &HeaderMap, body: &[u8]) -> Result<usize, String> {
    let v = body_json(headers, body)?;
    let evals = parse_evals(&v, crate::intake::logs::now_ms());
    store_evals(st, &evals).await.map_err(|e| e.to_string())?;
    Ok(evals.len())
}

/// `POST /api/v2/llmobs`, `/api/intake/llm-obs/v1/trace/spans`.
pub async fn spans_intake(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    reply(take_spans(&st, &headers, &body).await)
}

/// `POST /api/intake/llm-obs/v{1,2}/eval-metric`.
pub async fn evals_intake(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    reply(take_evals(&st, &headers, &body).await)
}

/// The Agent's EVP proxy on the trace port (`/evp_proxy/v{2,4}/…`): LLM
/// Observability here, CI Visibility in `civis`; anything else is accepted
/// and dropped.
pub async fn evp_proxy(
    State(st): State<AppState>,
    Path(rest): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = rest.split_once('/').map(|(_, p)| p).unwrap_or(&rest);
    if path.ends_with("api/v2/llmobs") || path.contains("llm-obs/v1/trace/spans") {
        reply(take_spans(&st, &headers, &body).await)
    } else if path.contains("eval-metric") {
        reply(take_evals(&st, &headers, &body).await)
    } else {
        StatusCode::ACCEPTED.into_response()
    }
}

/// Removes spans and evaluations past the retention.
pub fn spawn_purge(st: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            tick.tick().await;
            let before = crate::intake::logs::now_ms() - st.cfg.retention_days * 86_400_000;
            let r = async {
                let c = st.store.pg.conn().await?;
                c.execute(
                    "DELETE FROM llm_spans WHERE start_ns < $1",
                    &[&(before * 1_000_000)],
                )
                .await?;
                c.execute("DELETE FROM llm_evals WHERE ts < $1", &[&before])
                    .await?;
                anyhow::Ok(())
            }
            .await;
            if let Err(e) = r {
                tracing::warn!(error = %e, "purging LLM Observability data");
            }
        }
    });
}

// ---------------------------------------------------------------- queries

#[derive(Debug, Deserialize)]
pub struct Filter {
    pub ml_app: Option<String>,
    pub query: Option<String>,
    pub kind: Option<String>,
    #[serde(default)]
    pub errors: bool,
    pub from: i64,
    pub to: i64,
}

const SPAN_COLS: &str = "trace_id, span_id, parent_id, ml_app, name, kind, start_ns, duration, error,
    model_name, model_provider, input_tokens, output_tokens, total_tokens, cost, session_id, service,
    env, tags, input, output, body";

fn row_span(r: &tokio_postgres::Row) -> LlmSpan {
    LlmSpan {
        trace_id: r.get(0),
        span_id: r.get(1),
        parent_id: r.get(2),
        ml_app: r.get(3),
        name: r.get(4),
        kind: r.get(5),
        start_ns: r.get(6),
        duration: r.get(7),
        error: r.get(8),
        model_name: r.get(9),
        model_provider: r.get(10),
        input_tokens: r.get(11),
        output_tokens: r.get(12),
        total_tokens: r.get(13),
        cost: r.get(14),
        session_id: r.get(15),
        service: r.get(16),
        env: r.get(17),
        tags: r.get(18),
        input: r.get(19),
        output: r.get(20),
        body: r.get(21),
    }
}

/// A trace in the list: its root span and totals over its spans.
#[derive(Debug, Serialize)]
pub struct TraceRow {
    #[serde(flatten)]
    pub root: LlmSpan,
    pub spans: i64,
    pub llm_calls: i64,
    pub tokens: i64,
    pub cost: f64,
    pub errors: i64,
    pub evals: Vec<Value>,
}

/// `GET /api/ui/llm-obs/traces`: root spans (or spans of a kind) matching.
pub async fn traces(
    State(st): State<AppState>,
    Query(f): Query<Filter>,
) -> ApiResult<Vec<TraceRow>> {
    let c = st.store.pg.conn().await?;
    let q = f.query.as_deref().map(str::trim).filter(|q| !q.is_empty());
    let pattern = q.map(|q| format!("%{}%", q.replace('%', "\\%").replace('_', "\\_")));
    let roots_only = f.kind.is_none();
    let rows = c
        .query(
            &format!(
                "SELECT {SPAN_COLS} FROM llm_spans
                 WHERE start_ns >= $1 AND start_ns <= $2
                   AND ($3::TEXT IS NULL OR ml_app = $3)
                   AND ($4::TEXT IS NULL OR kind = $4)
                   AND (NOT $5 OR parent_id IS NULL)
                   AND (NOT $6 OR error)
                   AND ($7::TEXT IS NULL OR name ILIKE $7 OR input ILIKE $7 OR output ILIKE $7)
                 ORDER BY start_ns DESC LIMIT 200"
            ),
            &[
                &(f.from * 1_000_000),
                &(f.to * 1_000_000),
                &f.ml_app,
                &f.kind,
                &roots_only,
                &f.errors,
                &pattern,
            ],
        )
        .await
        .map_err(anyhow::Error::from)?;
    let spans: Vec<LlmSpan> = rows.iter().map(row_span).collect();
    let ids: Vec<String> = spans.iter().map(|s| s.trace_id.clone()).collect();
    let totals = c
        .query(
            "SELECT trace_id, count(*), count(*) FILTER (WHERE kind = 'llm'),
                coalesce(sum(total_tokens), 0)::BIGINT, coalesce(sum(cost), 0), count(*) FILTER (WHERE error)
             FROM llm_spans WHERE trace_id = ANY($1) GROUP BY trace_id",
            &[&ids],
        )
        .await
        .map_err(anyhow::Error::from)?;
    let evals = c
        .query(
            "SELECT span_id, label, metric_type, categorical, score FROM llm_evals WHERE trace_id = ANY($1)
             ORDER BY ts",
            &[&ids],
        )
        .await
        .map_err(anyhow::Error::from)?;
    let out = spans
        .into_iter()
        .map(|s| {
            let t = totals.iter().find(|r| r.get::<_, String>(0) == s.trace_id);
            let get = |i: usize| t.map(|r| r.get::<_, i64>(i)).unwrap_or(0);
            TraceRow {
                spans: get(1),
                llm_calls: get(2),
                tokens: get(3),
                cost: t.map(|r| r.get::<_, f64>(4)).unwrap_or(0.0),
                errors: get(5),
                evals: evals
                    .iter()
                    .filter(|e| e.get::<_, String>(0) == s.span_id)
                    .map(|e| {
                        json!({"label": e.get::<_, String>(1), "type": e.get::<_, String>(2),
                            "categorical": e.get::<_, Option<String>>(3), "score": e.get::<_, Option<f64>>(4)})
                    })
                    .collect(),
                root: s,
            }
        })
        .collect();
    Ok(Json(out))
}

/// `GET /api/ui/llm-obs/traces/{id}`: a trace's spans and evaluations.
pub async fn trace(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let c = st.store.pg.conn().await?;
    let rows = c
        .query(
            &format!("SELECT {SPAN_COLS} FROM llm_spans WHERE trace_id = $1 ORDER BY start_ns"),
            &[&id],
        )
        .await
        .map_err(anyhow::Error::from)?;
    if rows.is_empty() {
        return Err(ApiError::NotFound);
    }
    let evals = c
        .query(
            "SELECT span_id, label, metric_type, categorical, score, ts, tags FROM llm_evals
             WHERE trace_id = $1 ORDER BY ts",
            &[&id],
        )
        .await
        .map_err(anyhow::Error::from)?;
    Ok(Json(json!({
        "spans": rows.iter().map(row_span).collect::<Vec<_>>(),
        "evals": evals.iter().map(|e| json!({
            "span_id": e.get::<_, String>(0), "label": e.get::<_, String>(1), "type": e.get::<_, String>(2),
            "categorical": e.get::<_, Option<String>>(3), "score": e.get::<_, Option<f64>>(4),
            "ts": e.get::<_, i64>(5), "tags": e.get::<_, Vec<String>>(6),
        })).collect::<Vec<_>>(),
    })))
}

/// `GET /api/ui/llm-obs/overview`: totals, by model, by minute, evaluations.
pub async fn overview(State(st): State<AppState>, Query(f): Query<Filter>) -> ApiResult<Value> {
    let c = st.store.pg.conn().await?;
    let (from, to) = (f.from * 1_000_000, f.to * 1_000_000);
    let err = |e: tokio_postgres::Error| ApiError::Internal(e.into());
    let apps = c
        .query(
            "SELECT ml_app, count(*) FILTER (WHERE parent_id IS NULL) FROM llm_spans
             WHERE start_ns >= $1 AND start_ns <= $2 GROUP BY ml_app ORDER BY ml_app",
            &[&from, &to],
        )
        .await
        .map_err(err)?;
    let totals = c
        .query_one(
            "SELECT count(*) FILTER (WHERE parent_id IS NULL), count(*) FILTER (WHERE parent_id IS NULL AND error),
                count(*) FILTER (WHERE kind = 'llm'), coalesce(sum(input_tokens), 0)::BIGINT,
                coalesce(sum(output_tokens), 0)::BIGINT, coalesce(sum(cost), 0),
                coalesce(percentile_cont(0.5) WITHIN GROUP (ORDER BY duration) FILTER (WHERE parent_id IS NULL), 0)::BIGINT,
                coalesce(percentile_cont(0.95) WITHIN GROUP (ORDER BY duration) FILTER (WHERE parent_id IS NULL), 0)::BIGINT
             FROM llm_spans WHERE start_ns >= $1 AND start_ns <= $2 AND ($3::TEXT IS NULL OR ml_app = $3)",
            &[&from, &to, &f.ml_app],
        )
        .await
        .map_err(err)?;
    let models = c
        .query(
            "SELECT coalesce(model_provider, ''), model_name, count(*), count(*) FILTER (WHERE error),
                coalesce(sum(input_tokens), 0)::BIGINT, coalesce(sum(output_tokens), 0)::BIGINT,
                coalesce(sum(cost), 0), coalesce(percentile_cont(0.95) WITHIN GROUP (ORDER BY duration), 0)::BIGINT
             FROM llm_spans WHERE kind IN ('llm', 'embedding') AND model_name IS NOT NULL
               AND start_ns >= $1 AND start_ns <= $2 AND ($3::TEXT IS NULL OR ml_app = $3)
             GROUP BY 1, 2 ORDER BY 3 DESC",
            &[&from, &to, &f.ml_app],
        )
        .await
        .map_err(err)?;
    let step = ((to - from) / 60).max(60_000_000_000);
    let series = c
        .query(
            "SELECT (start_ns / $4) * $4 / 1000000, count(*) FILTER (WHERE parent_id IS NULL),
                count(*) FILTER (WHERE parent_id IS NULL AND error), coalesce(sum(total_tokens), 0)::BIGINT,
                coalesce(sum(cost), 0)
             FROM llm_spans WHERE start_ns >= $1 AND start_ns <= $2 AND ($3::TEXT IS NULL OR ml_app = $3)
             GROUP BY 1 ORDER BY 1",
            &[&from, &to, &f.ml_app, &step],
        )
        .await
        .map_err(err)?;
    let evals = c
        .query(
            "SELECT label, metric_type, count(*), avg(score), mode() WITHIN GROUP (ORDER BY categorical),
                count(*) FILTER (WHERE categorical IN ('false', 'negative', 'fail', 'failed', 'bad', 'toxic', 'yes'))
             FROM llm_evals WHERE ts >= $1 AND ts <= $2 AND ($3::TEXT IS NULL OR ml_app = $3)
             GROUP BY 1, 2 ORDER BY 1",
            &[&f.from, &f.to, &f.ml_app],
        )
        .await
        .map_err(err)?;
    Ok(Json(json!({
        "apps": apps.iter().map(|r| json!({"ml_app": r.get::<_, String>(0), "traces": r.get::<_, i64>(1)})).collect::<Vec<_>>(),
        "traces": totals.get::<_, i64>(0), "errors": totals.get::<_, i64>(1), "llm_calls": totals.get::<_, i64>(2),
        "input_tokens": totals.get::<_, i64>(3), "output_tokens": totals.get::<_, i64>(4),
        "cost": totals.get::<_, f64>(5), "p50": totals.get::<_, i64>(6), "p95": totals.get::<_, i64>(7),
        "interval_ms": step / 1_000_000,
        "series": series.iter().map(|r| json!({"t": r.get::<_, i64>(0), "traces": r.get::<_, i64>(1),
            "errors": r.get::<_, i64>(2), "tokens": r.get::<_, i64>(3), "cost": r.get::<_, f64>(4)})).collect::<Vec<_>>(),
        "models": models.iter().map(|r| json!({"provider": r.get::<_, String>(0), "model": r.get::<_, String>(1),
            "calls": r.get::<_, i64>(2), "errors": r.get::<_, i64>(3), "input_tokens": r.get::<_, i64>(4),
            "output_tokens": r.get::<_, i64>(5), "cost": r.get::<_, f64>(6), "p95": r.get::<_, i64>(7)})).collect::<Vec<_>>(),
        "evaluations": evals.iter().map(|r| json!({"label": r.get::<_, String>(0), "type": r.get::<_, String>(1),
            "count": r.get::<_, i64>(2), "avg_score": r.get::<_, Option<f64>>(3),
            "top_value": r.get::<_, Option<String>>(4), "flagged": r.get::<_, i64>(5)})).collect::<Vec<_>>(),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_of_an_event() {
        let ev = json!({"_dd.stage": "raw", "ml_app": "support-bot", "tags": ["env:prod", "service:bot"],
        "spans": [
            {"trace_id": "6789abcdef", "span_id": "1", "parent_id": "undefined", "name": "answer",
             "start_ns": 1_700_000_000_000_000_000i64, "duration": 2_000_000_000, "status": "ok",
             "meta": {"span.kind": "workflow", "input": {"value": "Where is my order?"}, "output": {"value": "It ships today."}},
             "metrics": {}, "tags": ["ml_app:support-bot", "session_id:s1"]},
            {"trace_id": "6789abcdef", "span_id": "2", "parent_id": "1", "name": "openai.request",
             "start_ns": 1_700_000_000_100_000_000i64, "duration": 1_500_000_000, "status": "error",
             "meta": {"span": {"kind": "llm"}, "model_name": "gpt-4o-mini-2024-07-18", "model_provider": "openai",
               "input": {"messages": [{"role": "system", "content": "Be brief"}, {"role": "user", "content": "Where is my order?"}]},
               "output": {"messages": [{"role": "assistant", "content": "It ships today."}]}},
             "metrics": {"input_tokens": 1000, "output_tokens": 500}}
        ]});
        let spans = parse_payload(&json!([ev]));
        assert_eq!(spans.len(), 2);
        let (root, llm) = (&spans[0], &spans[1]);
        assert_eq!(root.parent_id, None);
        assert_eq!(root.kind, "workflow");
        assert_eq!(root.input, "Where is my order?");
        assert_eq!(root.session_id.as_deref(), Some("s1"));
        assert_eq!(root.service.as_deref(), Some("bot"));
        assert_eq!(llm.kind, "llm");
        assert!(llm.error);
        assert_eq!(llm.ml_app, "support-bot");
        assert_eq!(llm.total_tokens, Some(1500));
        assert_eq!(llm.input, "system: Be brief\nuser: Where is my order?");
        let c = llm.cost.unwrap();
        assert!((c - (1000.0 * 0.15 + 500.0 * 0.6) / 1e6).abs() < 1e-12);
        assert_eq!(
            cost("anthropic/claude-sonnet-4-20250514", 1_000_000, 0),
            Some(3.0)
        );
        assert_eq!(cost("my-own-model", 1, 1), None);
    }

    #[test]
    fn evaluations_v1_and_v2() {
        let v = json!({"data": {"type": "evaluation_metric", "attributes": {"metrics": [
            {"span_id": "2", "trace_id": "6789abcdef", "label": "sentiment", "metric_type": "categorical",
             "categorical_value": "positive", "ml_app": "support-bot", "timestamp_ms": 5},
            {"join_on": {"span": {"span_id": "1", "trace_id": "6789abcdef"}}, "label": "faithfulness",
             "metric_type": "score", "score_value": 0.8, "tags": ["ml_app:support-bot"]}
        ]}}});
        let e = parse_evals(&v, 10);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].categorical.as_deref(), Some("positive"));
        assert_eq!(e[0].ts, 5);
        assert_eq!(e[1].score, Some(0.8));
        assert_eq!(e[1].ml_app, "support-bot");
        assert_eq!(e[1].ts, 10);
    }
}
