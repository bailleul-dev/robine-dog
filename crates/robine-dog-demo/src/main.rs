//! Demo traffic for robine-dog: a small e-commerce system (storefront, API,
//! cart, payments, Postgres, Redis, Stripe) emitting traces in the ddtrace
//! v0.4 msgpack format to the trace-agent port, and correlated JSON logs to the
//! Datadog logs intake — exactly what real instrumented services would send.
//!
//! Usage: robine-dog-demo [--backfill-minutes 60] [--rate 3] [--no-live]

use rand::Rng;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Write;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Serialize, Clone)]
struct Span {
    trace_id: u64,
    span_id: u64,
    parent_id: u64,
    service: String,
    name: String,
    resource: String,
    #[serde(rename = "type")]
    typ: String,
    start: i64,
    duration: i64,
    error: i32,
    meta: HashMap<String, String>,
    metrics: HashMap<String, f64>,
}

struct LogSpec {
    status: &'static str,
    message: String,
    attrs: Value,
}

struct Node {
    service: &'static str,
    name: &'static str,
    resource: String,
    typ: &'static str,
    self_ns: i64,
    error: bool,
    meta: Vec<(&'static str, String)>,
    children: Vec<Node>,
    logs: Vec<LogSpec>,
}

impl Node {
    fn new(
        service: &'static str,
        name: &'static str,
        resource: impl Into<String>,
        typ: &'static str,
        self_ms: f64,
    ) -> Self {
        Node {
            service,
            name,
            resource: resource.into(),
            typ,
            self_ns: (self_ms * 1e6) as i64,
            error: false,
            meta: Vec::new(),
            children: Vec::new(),
            logs: Vec::new(),
        }
    }
    fn meta(mut self, k: &'static str, v: impl Into<String>) -> Self {
        self.meta.push((k, v.into()));
        self
    }
    fn child(mut self, c: Node) -> Self {
        self.children.push(c);
        self
    }
    fn err(mut self, kind: &str, msg: &str) -> Self {
        self.error = true;
        self.meta.push(("error.type", kind.to_string()));
        self.meta.push(("error.message", msg.to_string()));
        self.meta.push(("error.stack", format!("{kind}: {msg}\n  at handler (app/handlers.py:142)\n  at dispatch (framework/core.py:88)")));
        self
    }
    fn log(mut self, status: &'static str, message: impl Into<String>, attrs: Value) -> Self {
        self.logs.push(LogSpec {
            status,
            message: message.into(),
            attrs,
        });
        self
    }
}

struct ServiceInfo {
    version: &'static str,
    language: &'static str,
    hosts: &'static [&'static str],
    team: &'static str,
}

fn info(service: &str) -> ServiceInfo {
    match service {
        "web-store" => ServiceInfo {
            version: "2.14.1",
            language: "python",
            hosts: &["web-store-7d9f8-x2lkq", "web-store-7d9f8-p9wzt"],
            team: "storefront",
        },
        "shop-api" => ServiceInfo {
            version: "1.31.0",
            language: "go",
            hosts: &[
                "shop-api-5c6b4-h7r2m",
                "shop-api-5c6b4-qq81d",
                "shop-api-5c6b4-z0v3n",
            ],
            team: "core",
        },
        "cart-service" => ServiceInfo {
            version: "0.9.4",
            language: "nodejs",
            hosts: &["cart-service-66f7c-k1t8s"],
            team: "core",
        },
        "payment-service" => ServiceInfo {
            version: "3.2.0",
            language: "java",
            hosts: &["payment-service-8d4e1-w5n7b"],
            team: "payments",
        },
        "order-worker" => ServiceInfo {
            version: "1.31.0",
            language: "go",
            hosts: &["order-worker-77b2a-j3m9c"],
            team: "core",
        },
        _ => ServiceInfo {
            version: "",
            language: "",
            hosts: &["db-primary-01"],
            team: "platform",
        },
    }
}

/// Roughly log-normal latency around `median_ms`.
fn lat(rng: &mut impl Rng, median_ms: f64) -> f64 {
    let n: f64 = (0..4).map(|_| rng.random::<f64>()).sum::<f64>() - 2.0; // ~N(0, 0.58)
    let mut v = median_ms * (n * 0.9).exp();
    if rng.random_bool(0.01) {
        v *= rng.random_range(5.0..20.0); // tail
    }
    v
}

fn http_client(
    from: &'static str,
    to: &'static str,
    method: &str,
    path: &str,
    status: u16,
    rng: &mut impl Rng,
) -> Node {
    let lang = info(from).language;
    let name = match lang {
        "go" => "http.request",
        "nodejs" => "http.request",
        "java" => "okhttp.request",
        _ => "requests.request",
    };
    Node::new(
        from,
        name,
        format!("{method} {path}"),
        "http",
        lat(rng, 0.4),
    )
    .meta("http.method", method)
    .meta("http.url", format!("http://{to}{path}"))
    .meta("http.status_code", status.to_string())
    .meta("span.kind", "client")
    .meta("peer.service", to)
    .meta("out.host", to)
}

fn server(
    service: &'static str,
    method: &str,
    route: &str,
    path: &str,
    status: u16,
    self_ms: f64,
) -> Node {
    let name = match info(service).language {
        "go" => "http.request",
        "nodejs" => "express.request",
        "java" => "servlet.request",
        _ => "flask.request",
    };
    Node::new(service, name, format!("{method} {route}"), "web", self_ms)
        .meta("http.method", method)
        .meta("http.route", route)
        .meta("http.url", format!("http://{service}{path}"))
        .meta("http.status_code", status.to_string())
        .meta("span.kind", "server")
        .meta("component", name.split('.').next().unwrap_or(name))
}

fn pg(rng: &mut impl Rng, from: &'static str, sql: &str, median: f64) -> Node {
    let mut n = Node::new("postgres", "postgresql.query", sql, "sql", lat(rng, median))
        .meta("db.system", "postgresql")
        .meta("db.name", "shop")
        .meta("db.user", "shop_app")
        .meta("span.kind", "client")
        .meta("out.host", "db-primary-01")
        .meta("db.statement", sql)
        .meta("peer.service", "postgres");
    if rng.random_bool(0.004) {
        n = n.err(
            "psycopg2.errors.QueryCanceled",
            "canceling statement due to statement timeout",
        );
        n.self_ns += 2_000_000_000;
    }
    let _ = from;
    n
}

fn redis(rng: &mut impl Rng, cmd: &str, key: &str) -> Node {
    Node::new(
        "redis",
        "redis.command",
        format!("{cmd} {key}"),
        "redis",
        lat(rng, 0.25),
    )
    .meta("db.system", "redis")
    .meta("redis.raw_command", format!("{cmd} {key}"))
    .meta("out.host", "redis-cache-01")
    .meta("span.kind", "client")
    .meta("peer.service", "redis")
}

fn access_log(status: u16, method: &str, path: &str, ms: f64, user: &str) -> LogSpec {
    LogSpec {
        status: if status >= 500 {
            "error"
        } else if status >= 400 {
            "warn"
        } else {
            "info"
        },
        message: format!("{method} {path} {status} {ms:.1}ms"),
        attrs: json!({
            "http": {"method": method, "status_code": status, "url": path, "url_details": {"path": path}, "useragent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5) AppleWebKit/605.1.15"},
            "network": {"client": {"ip": format!("81.{}.{}.{}", ms as u32 % 250, (ms * 7.0) as u32 % 250, (ms * 13.0) as u32 % 250)}},
            "usr": {"id": user},
            "duration": (ms * 1e6) as i64
        }),
    }
}

const PRODUCTS: &[&str] = &[
    "espresso-machine",
    "grinder-pro",
    "milk-jug",
    "tamper-58mm",
    "scale-v2",
    "beans-ethiopia",
];
const USERS: &[&str] = &[
    "u-1042", "u-2291", "u-3310", "u-4123", "u-5007", "u-6654", "u-7781", "u-8020",
];

fn scenario(rng: &mut impl Rng) -> Node {
    let user = USERS[rng.random_range(0..USERS.len())];
    let roll = rng.random::<f64>();
    if roll < 0.40 {
        // Home page
        let api = server(
            "shop-api",
            "GET",
            "/api/products",
            "/api/products?limit=24",
            200,
            lat(rng, 2.0),
        )
        .child(redis(rng, "GET", "products:featured"))
        .child(pg(
            rng,
            "shop-api",
            "SELECT id, name, price FROM products WHERE featured = ? ORDER BY rank LIMIT ?",
            4.0,
        ))
        .log(
            "info",
            "listed featured products",
            json!({"count": 24, "cache": "miss"}),
        );
        server("web-store", "GET", "/", "/", 200, lat(rng, 6.0))
            .child(
                http_client("web-store", "shop-api", "GET", "/api/products", 200, rng).child(api),
            )
            .child(Node::new(
                "web-store",
                "flask.render_template",
                "home.html",
                "template",
                lat(rng, 3.0),
            ))
    } else if roll < 0.75 {
        // Product page
        let p = PRODUCTS[rng.random_range(0..PRODUCTS.len())];
        let not_found = rng.random_bool(0.03);
        let status = if not_found { 404 } else { 200 };
        let mut api = server(
            "shop-api",
            "GET",
            "/api/products/{id}",
            &format!("/api/products/{p}"),
            status,
            lat(rng, 1.0),
        )
        .child(redis(rng, "GET", &format!("product:{p}")));
        if rng.random_bool(0.3) || not_found {
            api = api.child(pg(
                rng,
                "shop-api",
                "SELECT * FROM products WHERE slug = ?",
                2.0,
            ));
        }
        if not_found {
            api = api.log(
                "warn",
                format!("product {p}-old not found"),
                json!({"product": {"slug": format!("{p}-old")}}),
            );
        }
        server(
            "web-store",
            "GET",
            "/products/<slug>",
            &format!("/products/{p}"),
            status,
            lat(rng, 5.0),
        )
        .child(
            http_client(
                "web-store",
                "shop-api",
                "GET",
                &format!("/api/products/{p}"),
                status,
                rng,
            )
            .child(api),
        )
        .child(Node::new(
            "web-store",
            "flask.render_template",
            "product.html",
            "template",
            lat(rng, 2.5),
        ))
    } else if roll < 0.92 {
        // Add to cart
        let p = PRODUCTS[rng.random_range(0..PRODUCTS.len())];
        let cart = server(
            "cart-service",
            "POST",
            "/cart/:userId/items",
            &format!("/cart/{user}/items"),
            201,
            lat(rng, 1.5),
        )
        .child(redis(rng, "HINCRBY", &format!("cart:{user}")))
        .child(pg(
            rng,
            "cart-service",
            "INSERT INTO cart_events (user_id, product, qty) VALUES (?, ?, ?)",
            3.0,
        ))
        .log(
            "info",
            format!("added {p} to cart"),
            json!({"product": {"slug": p}, "cart": {"user_id": user}}),
        );
        server("web-store", "POST", "/cart", "/cart", 201, lat(rng, 3.0)).child(
            http_client(
                "web-store",
                "cart-service",
                "POST",
                &format!("/cart/{user}/items"),
                201,
                rng,
            )
            .child(cart),
        )
    } else {
        // Checkout
        let amount = rng.random_range(1500..45000) as f64 / 100.0;
        let fate = rng.random::<f64>();
        let (declined, timeout) = (fate < 0.08, (0.08..0.11).contains(&fate));
        let stripe_status = if declined {
            402
        } else if timeout {
            504
        } else {
            200
        };
        let mut stripe = Node::new(
            "payment-service",
            "okhttp.request",
            "POST /v1/payment_intents",
            "http",
            if timeout { 10_000.0 } else { lat(rng, 180.0) },
        )
        .meta("http.method", "POST")
        .meta("http.url", "https://api.stripe.com/v1/payment_intents")
        .meta("http.status_code", stripe_status.to_string())
        .meta("peer.service", "stripe-api")
        .meta("span.kind", "client");
        if timeout {
            stripe = stripe.err(
                "java.net.SocketTimeoutException",
                "Read timed out after 10000ms",
            );
        }
        let pay_status = if declined {
            402
        } else if timeout {
            502
        } else {
            201
        };
        let mut pay = server(
            "payment-service",
            "POST",
            "/payments",
            "/payments",
            pay_status,
            lat(rng, 4.0),
        )
        .child(pg(
            rng,
            "payment-service",
            "SELECT * FROM payment_methods WHERE user_id = ?",
            2.0,
        ))
        .child(stripe);
        pay = if declined {
            pay.err("com.shop.payments.CardDeclinedException", "card_declined: insufficient_funds")
                .log("warn", "payment declined by issuer", json!({"payment": {"amount": amount, "currency": "EUR", "decline_code": "insufficient_funds"}, "usr": {"id": user}}))
        } else if timeout {
            pay.err("com.shop.payments.ProviderUnavailableException", "Stripe did not answer in time")
                .log("error", "payment provider timeout", json!({"payment": {"amount": amount, "currency": "EUR", "provider": "stripe"}, "error": {"kind": "ProviderUnavailableException", "message": "Stripe did not answer in time"}}))
        } else {
            pay.log(
                "info",
                "payment captured",
                json!({"payment": {"amount": amount, "currency": "EUR", "provider": "stripe"}}),
            )
        };
        let ok = !declined && !timeout;
        let order_status = if declined {
            402
        } else if timeout {
            502
        } else {
            201
        };
        let mut api = server(
            "shop-api",
            "POST",
            "/api/orders",
            "/api/orders",
            order_status,
            lat(rng, 3.0),
        )
        .child(
            http_client(
                "shop-api",
                "cart-service",
                "GET",
                &format!("/cart/{user}"),
                200,
                rng,
            )
            .child(
                server(
                    "cart-service",
                    "GET",
                    "/cart/:userId",
                    &format!("/cart/{user}"),
                    200,
                    lat(rng, 1.0),
                )
                .child(redis(rng, "HGETALL", &format!("cart:{user}"))),
            ),
        )
        .child(
            http_client(
                "shop-api",
                "payment-service",
                "POST",
                "/payments",
                pay_status,
                rng,
            )
            .child(pay),
        );
        if ok {
            api = api
                .child(pg(rng, "shop-api", "INSERT INTO orders (user_id, total, status) VALUES (?, ?, ?) RETURNING id", 5.0))
                .log("info", "order created", json!({"order": {"total": amount, "items": rng.random_range(1..5)}, "usr": {"id": user}}));
        } else if timeout {
            api = api.err("upstream.Error", "payment-service returned 502").log(
                "error",
                "checkout failed: payment-service returned 502",
                json!({"error": {"kind": "UpstreamError", "message": "payment-service returned 502"}, "usr": {"id": user}}),
            );
        }
        let mut web = server(
            "web-store",
            "POST",
            "/checkout",
            "/checkout",
            if timeout {
                500
            } else if declined {
                402
            } else {
                200
            },
            lat(rng, 4.0),
        )
        .child(
            http_client(
                "web-store",
                "shop-api",
                "POST",
                "/api/orders",
                order_status,
                rng,
            )
            .child(api),
        );
        if timeout {
            web = web.err(
                "requests.exceptions.HTTPError",
                "500 Server Error: Internal Server Error for url: http://shop-api/api/orders",
            );
        }
        web
    }
}

fn worker_job(rng: &mut impl Rng) -> Node {
    let n = rng.random_range(0..40);
    let mut job = Node::new(
        "order-worker",
        "worker.job",
        "sync_shipments",
        "worker",
        lat(rng, 20.0),
    )
    .child(pg(
        rng,
        "order-worker",
        "SELECT id FROM orders WHERE status = ? AND shipped_at IS NULL LIMIT ?",
        6.0,
    ))
    .log(
        "info",
        format!("synced {n} shipments"),
        json!({"job": {"name": "sync_shipments", "processed": n}}),
    );
    if rng.random_bool(0.05) {
        job = job.err("context.DeadlineExceeded", "carrier API deadline exceeded").log(
            "error",
            "shipment sync failed",
            json!({"job": {"name": "sync_shipments"}, "error": {"kind": "DeadlineExceeded", "message": "carrier API deadline exceeded"}}),
        );
    }
    job
}

struct Out {
    spans: Vec<Span>,
    logs: Vec<Value>,
}

#[allow(clippy::too_many_arguments)]
fn emit(
    node: Node,
    trace_id: u64,
    parent: u64,
    start: i64,
    env: &str,
    hosts: &mut HashMap<&'static str, &'static str>,
    rng: &mut impl Rng,
    out: &mut Out,
) -> i64 {
    let span_id: u64 = rng.random_range(1..u64::MAX / 2);
    let mut t = start + node.self_ns / 3;
    for c in node.children {
        t += rng.random_range(50_000..400_000);
        let d = emit(c, trace_id, span_id, t, env, hosts, rng, out);
        t += d;
    }
    let duration = (t - start) + node.self_ns * 2 / 3;
    let si = info(node.service);
    let host = *hosts
        .entry(node.service)
        .or_insert_with(|| si.hosts[rng.random_range(0..si.hosts.len())]);
    let mut meta: HashMap<String, String> = node
        .meta
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    meta.insert("env".into(), env.into());
    if !si.version.is_empty() {
        meta.insert("version".into(), si.version.into());
        meta.insert("language".into(), si.language.into());
    }
    meta.insert("_dd.hostname".into(), host.into());
    let mut metrics = HashMap::new();
    if parent == 0 {
        metrics.insert("_sampling_priority_v1".into(), 1.0);
    }
    for l in node.logs {
        let ts_ms = (start + duration) / 1_000_000;
        let mut inner = json!({
            "level": l.status.to_uppercase(),
            "msg": l.message,
            "logger": {"name": format!("{}.handlers", node.service.replace('-', "_"))},
            "dd": {"trace_id": trace_id.to_string(), "span_id": span_id.to_string(), "service": node.service, "env": env, "version": si.version},
        });
        if let (Value::Object(i), Value::Object(a)) = (&mut inner, l.attrs) {
            i.extend(a);
        }
        out.logs.push(json!({
            "ddsource": si.language,
            "ddtags": format!("env:{env},version:{},team:{},kube_namespace:shop", si.version, si.team),
            "hostname": host,
            "service": node.service,
            "timestamp": ts_ms,
            "message": inner.to_string(),
        }));
    }
    if parent == 0 || node.typ == "web" {
        let status: u16 = meta
            .get("http.status_code")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if node.typ == "web" {
            let mut l = access_log(
                status,
                meta.get("http.method").map(String::as_str).unwrap_or("GET"),
                meta.get("http.url")
                    .map(|u| u.split_once("//").map(|x| x.1).unwrap_or(u))
                    .and_then(|u| u.find('/').map(|i| &u[i..]))
                    .unwrap_or("/"),
                duration as f64 / 1e6,
                "anonymous",
            );
            if let Value::Object(a) = &mut l.attrs {
                a.insert(
                    "dd".into(),
                    json!({"trace_id": trace_id.to_string(), "span_id": span_id.to_string()}),
                );
            }
            out.logs.push(json!({
                "ddsource": si.language,
                "ddtags": format!("env:{env},version:{},team:{}", si.version, si.team),
                "hostname": host,
                "service": node.service,
                "status": l.status,
                "timestamp": (start + duration) / 1_000_000,
                "message": l.message,
                "http": l.attrs["http"], "network": l.attrs["network"], "duration": l.attrs["duration"], "dd": l.attrs["dd"],
                "logger": {"name": "access"},
            }));
        }
    }
    out.spans.push(Span {
        trace_id,
        span_id,
        parent_id: parent,
        service: node.service.into(),
        name: node.name.into(),
        resource: node.resource,
        typ: node.typ.into(),
        start,
        duration,
        error: node.error as i32,
        meta,
        metrics,
    });
    duration
}

fn noise_logs(rng: &mut impl Rng, ts_ms: i64, env: &str, out: &mut Out) {
    let r = rng.random::<f64>();
    let (service, source, host, status, message) = if r < 0.3 {
        (
            "nginx",
            "nginx",
            "edge-lb-01",
            "info",
            format!(
                "10.0.{}.{} - - \"GET /static/app.{}.js HTTP/1.1\" 200 48211 \"-\" \"Mozilla/5.0\"",
                rng.random_range(0..255),
                rng.random_range(0..255),
                rng.random_range(1000..9999)
            ),
        )
    } else if r < 0.45 {
        (
            "redis",
            "redis",
            "redis-cache-01",
            "info",
            format!("{}:M * DB saved on disk", rng.random_range(1..9)),
        )
    } else if r < 0.6 {
        (
            "postgres",
            "postgresql",
            "db-primary-01",
            if rng.random_bool(0.1) { "warn" } else { "info" },
            format!(
                "duration: {:.3} ms  statement: SELECT count(*) FROM orders WHERE created_at > now() - interval '1 hour'",
                rng.random_range(1.0..900.0)
            ),
        )
    } else if r < 0.75 {
        (
            "kubelet",
            "kubernetes",
            "node-pool-a-3",
            "debug",
            format!(
                "Probe succeeded probeType=\"Readiness\" pod=\"shop/shop-api-5c6b4-{}\"",
                ["h7r2m", "qq81d", "z0v3n"][rng.random_range(0..3)]
            ),
        )
    } else if r < 0.85 {
        (
            "shop-api",
            "go",
            "shop-api-5c6b4-qq81d",
            "debug",
            "connection pool stats".into(),
        )
    } else if r < 0.97 {
        (
            "web-store",
            "python",
            "web-store-7d9f8-x2lkq",
            "info",
            "cache warmup complete".into(),
        )
    } else {
        ("postgres", "postgresql", "db-primary-01", "error", "FATAL:  remaining connection slots are reserved for non-replication superuser connections".into())
    };
    out.logs.push(json!({
        "ddsource": source, "ddtags": format!("env:{env}"), "hostname": host, "service": service,
        "status": status, "timestamp": ts_ms, "message": message,
        "pool": if service == "shop-api" { json!({"open": rng.random_range(5..50), "idle": rng.random_range(0..10), "wait_count": rng.random_range(0..3)}) } else { Value::Null },
    }));
}

fn now_ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as i64
}

struct Client {
    http: reqwest::blocking::Client,
    app_url: String,
    agent_url: String,
    api_key: String,
}

impl Client {
    fn send(&self, out: Out) -> anyhow::Result<()> {
        if !out.spans.is_empty() {
            let mut traces: HashMap<u64, Vec<Span>> = HashMap::new();
            for s in out.spans {
                traces.entry(s.trace_id).or_default().push(s);
            }
            let payload: Vec<Vec<Span>> = traces.into_values().collect();
            let body = rmp_serde::to_vec_named(&payload)?;
            self.http
                .put(format!("{}/v0.4/traces", self.agent_url))
                .header("Content-Type", "application/msgpack")
                .header("Datadog-Meta-Lang", "rust")
                .header("X-Datadog-Trace-Count", payload.len().to_string())
                .body(body)
                .send()?
                .error_for_status()?;
        }
        for chunk in out.logs.chunks(500) {
            let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            gz.write_all(&serde_json::to_vec(chunk)?)?;
            self.http
                .post(format!("{}/api/v2/logs", self.app_url))
                .header("Content-Type", "application/json")
                .header("Content-Encoding", "gzip")
                .header("DD-API-KEY", &self.api_key)
                .body(gz.finish()?)
                .send()?
                .error_for_status()?;
        }
        Ok(())
    }
}

fn generate(rng: &mut impl Rng, start_ns: i64, env: &str, out: &mut Out) {
    let mut hosts = HashMap::new();
    let root = if rng.random_bool(0.03) {
        worker_job(rng)
    } else {
        scenario(rng)
    };
    let trace_id: u64 = rng.random_range(1..u64::MAX / 2);
    emit(root, trace_id, 0, start_ns, env, &mut hosts, rng, out);
    if rng.random_bool(0.6) {
        noise_logs(rng, start_ns / 1_000_000, env, out);
    }
}

fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let backfill_min: i64 = arg("--backfill-minutes")
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let rate: f64 = arg("--rate").and_then(|v| v.parse().ok()).unwrap_or(3.0);
    let live = !args.iter().any(|a| a == "--no-live");
    let client = Client {
        http: reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?,
        app_url: std::env::var("ROBINE_DOG_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".into()),
        agent_url: std::env::var("DD_TRACE_AGENT_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8126".into()),
        api_key: std::env::var("DD_API_KEY")
            .or_else(|_| std::env::var("ROBINE_DOG_API_KEY"))
            .unwrap_or_default(),
    };
    let mut rng = rand::rng();

    if backfill_min > 0 {
        let end = now_ns();
        let begin = end - backfill_min * 60 * 1_000_000_000;
        let total = (backfill_min as f64 * 60.0 * rate) as i64;
        println!("backfilling {total} requests over the last {backfill_min} minutes…");
        let mut out = Out {
            spans: Vec::new(),
            logs: Vec::new(),
        };
        for i in 0..total {
            let t = begin + (end - begin) / total.max(1) * i + rng.random_range(0..1_000_000_000);
            let env = if rng.random_bool(0.85) {
                "prod"
            } else {
                "staging"
            };
            generate(&mut rng, t, env, &mut out);
            if out.spans.len() > 3000 {
                client.send(std::mem::replace(
                    &mut out,
                    Out {
                        spans: Vec::new(),
                        logs: Vec::new(),
                    },
                ))?;
            }
        }
        client.send(out)?;
        println!("backfill done");
    }
    if live {
        println!("sending live traffic at ~{rate} req/s (Ctrl-C to stop)");
        loop {
            let mut out = Out {
                spans: Vec::new(),
                logs: Vec::new(),
            };
            let n = (rate + rng.random_range(-0.5..0.5) * rate).round().max(0.0) as i64;
            for _ in 0..n {
                let env = if rng.random_bool(0.85) {
                    "prod"
                } else {
                    "staging"
                };
                let t = now_ns() - rng.random_range(0..900_000_000);
                generate(&mut rng, t, env, &mut out);
            }
            if let Err(e) = client.send(out) {
                eprintln!("send failed: {e}");
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    Ok(())
}
