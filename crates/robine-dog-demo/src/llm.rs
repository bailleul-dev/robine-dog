//! An LLM application, as ddtrace's LLMObs SDK reports it: a support bot
//! whose workflow retrieves documents, calls a model (sometimes a tool), and
//! gets evaluated.

use rand::Rng;
use serde_json::{Value, json};

const QUESTIONS: &[(&str, &str)] = &[
    (
        "Where is my order #4512?",
        "Your order #4512 left our warehouse this morning and should arrive Thursday.",
    ),
    (
        "Can I return a product after 30 days?",
        "Returns are accepted within 30 days; after that we can offer store credit.",
    ),
    (
        "Do you ship to Canada?",
        "Yes, we ship to Canada in 5 to 8 business days.",
    ),
    (
        "My payment failed twice, what should I do?",
        "Please check your card's 3-D Secure confirmation, then retry; your cart is kept.",
    ),
    (
        "How do I change my delivery address?",
        "Open the order in your account and choose \"Change address\" before it ships.",
    ),
    (
        "Is the blue jacket available in XL?",
        "The blue jacket is back in stock in XL as of today.",
    ),
];

fn hex(rng: &mut impl Rng, n: usize) -> String {
    (0..n)
        .map(|_| format!("{:x}", rng.random_range(0..16u8)))
        .collect()
}

/// A trace's spans and evaluations, starting at `start_ns`.
pub fn trace(rng: &mut impl Rng, start_ns: i64) -> (Value, Value) {
    let trace_id = hex(rng, 32);
    let session = format!("session-{}", rng.random_range(0..40));
    let (q, a) = QUESTIONS[rng.random_range(0..QUESTIONS.len())];
    let (model, provider) = if rng.random_bool(0.7) {
        ("gpt-4o-mini", "openai")
    } else {
        ("claude-sonnet-4-20250514", "anthropic")
    };
    let failed = rng.random_bool(0.04);
    let tool = rng.random_bool(0.3);
    let ids: Vec<String> = (0..5)
        .map(|_| rng.random_range(1u64..u64::MAX / 2).to_string())
        .collect();
    let ms = 1_000_000i64;
    let retrieval = rng.random_range(40..180) * ms;
    let llm = rng.random_range(600..3500) * ms;
    let tool_d = if tool {
        rng.random_range(50..400) * ms
    } else {
        0
    };
    let total = retrieval + llm + tool_d + 20 * ms;
    let input_tokens = rng.random_range(400..2400);
    let output_tokens = if failed { 0 } else { rng.random_range(30..300) };
    let base_tags = |extra: &[&str]| {
        let mut t = vec![
            "ml_app:support-bot".to_string(),
            "service:support-bot".into(),
            "env:prod".into(),
            format!("session_id:{session}"),
        ];
        t.extend(extra.iter().map(|s| s.to_string()));
        json!(t)
    };
    let mut spans = vec![
        json!({"trace_id": trace_id, "span_id": ids[0], "parent_id": "undefined", "name": "answer_customer",
            "start_ns": start_ns, "duration": total, "status": if failed { "error" } else { "ok" },
            "session_id": session, "tags": base_tags(&[]),
            "meta": {"span.kind": "workflow", "input": {"value": q}, "output": {"value": if failed { "" } else { a }},
                "error.message": if failed { "RateLimitError: Rate limit reached for requests" } else { "" }},
            "metrics": {}}),
        json!({"trace_id": trace_id, "span_id": ids[1], "parent_id": ids[0], "name": "search_help_center",
            "start_ns": start_ns + 5 * ms, "duration": retrieval, "status": "ok", "tags": base_tags(&[]),
            "meta": {"span.kind": "retrieval", "input": {"value": q},
                "output": {"documents": [{"text": "Shipping and delivery times", "name": "shipping.md", "score": 0.82},
                    {"text": "Returns and refunds policy", "name": "returns.md", "score": 0.61}]}},
            "metrics": {}}),
        json!({"trace_id": trace_id, "span_id": ids[2], "parent_id": ids[0], "name": format!("{provider}.chat"),
            "start_ns": start_ns + retrieval + 10 * ms, "duration": llm, "status": if failed { "error" } else { "ok" },
            "tags": base_tags(&[]),
            "meta": {"span.kind": "llm", "model_name": model, "model_provider": provider,
                "input": {"messages": [{"role": "system", "content": "You are the shop's support assistant. Answer briefly, from the documents."},
                    {"role": "user", "content": q}]},
                "output": {"messages": [{"role": "assistant", "content": if failed { "" } else { a }}]},
                "metadata": {"temperature": 0.2, "max_tokens": 512}},
            "metrics": {"input_tokens": input_tokens, "output_tokens": output_tokens, "total_tokens": input_tokens + output_tokens}}),
    ];
    if tool {
        spans.push(json!({"trace_id": trace_id, "span_id": ids[3], "parent_id": ids[0], "name": "lookup_order",
            "start_ns": start_ns + retrieval + llm + 15 * ms, "duration": tool_d, "status": "ok", "tags": base_tags(&[]),
            "meta": {"span.kind": "tool", "input": {"value": "{\"order_id\": 4512}"}, "output": {"value": "{\"status\": \"shipped\"}"}},
            "metrics": {}}));
    }
    let event =
        json!({"_dd.stage": "raw", "event_type": "span", "ml_app": "support-bot", "spans": spans});
    let now_ms = (start_ns + total) / 1_000_000;
    let mut metrics = vec![];
    if !failed {
        let sentiment = ["positive", "neutral", "negative"][rng.random_range(0..3)];
        metrics.push(json!({"span_id": ids[0], "trace_id": trace_id, "label": "sentiment", "metric_type": "categorical",
            "categorical_value": sentiment, "ml_app": "support-bot", "timestamp_ms": now_ms}));
        metrics.push(json!({"span_id": ids[0], "trace_id": trace_id, "label": "faithfulness", "metric_type": "score",
            "score_value": (rng.random_range(0.55..1.0f64) * 100.0).round() / 100.0, "ml_app": "support-bot", "timestamp_ms": now_ms}));
    }
    let evals = json!({"data": {"type": "evaluation_metric", "attributes": {"metrics": metrics}}});
    (event, evals)
}
