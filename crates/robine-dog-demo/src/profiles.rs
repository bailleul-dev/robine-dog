//! Profiles, as ddtrace's profiler uploads them every minute: a gzipped pprof
//! with CPU, wall time and allocations, in a multipart form (v4 format).

use rand::Rng;
use std::io::Write;

/// The services that profile, with their stacks (root first) and weights.
/// `(endpoint, stack, weight)`.
type Stack = (&'static str, &'static str, u32);

const STACKS: &[(&str, &str, &[Stack])] = &[
    (
        "shop-api",
        "python",
        &[
            (
                "GET /api/products",
                "main;flask.app.wsgi_app;flask.app.full_dispatch_request;shop.views.list_products;shop.db.fetch_products;psycopg.cursor.execute",
                40,
            ),
            (
                "GET /api/products",
                "main;flask.app.wsgi_app;flask.app.full_dispatch_request;shop.views.list_products;shop.serialize.products_json;json.encoder.encode",
                22,
            ),
            (
                "GET /api/products",
                "main;flask.app.wsgi_app;flask.app.full_dispatch_request;shop.views.list_products;shop.cache.get;redis.client.execute_command",
                8,
            ),
            (
                "POST /api/cart",
                "main;flask.app.wsgi_app;flask.app.full_dispatch_request;shop.views.add_to_cart;shop.db.upsert_cart;psycopg.cursor.execute",
                18,
            ),
            (
                "POST /api/cart",
                "main;flask.app.wsgi_app;flask.app.full_dispatch_request;shop.views.add_to_cart;shop.pricing.compute_total;decimal.Decimal.quantize",
                9,
            ),
            ("", "main;gunicorn.workers.sync.run;selectors.select", 6),
            (
                "",
                "main;threading.Thread.run;ddtrace.profiling.collector.collect",
                3,
            ),
        ],
    ),
    (
        "payment-service",
        "go",
        &[
            (
                "POST /charge",
                "runtime.goexit;net/http.(*conn).serve;main.chargeHandler;main.validateCard;crypto/sha256.block",
                25,
            ),
            (
                "POST /charge",
                "runtime.goexit;net/http.(*conn).serve;main.chargeHandler;main.callProvider;net/http.(*Client).Do;crypto/tls.(*Conn).Handshake",
                35,
            ),
            (
                "POST /charge",
                "runtime.goexit;net/http.(*conn).serve;main.chargeHandler;encoding/json.Marshal",
                10,
            ),
            (
                "",
                "runtime.goexit;runtime.gcBgMarkWorker;runtime.scanobject",
                14,
            ),
        ],
    ),
    (
        "order-worker",
        "python",
        &[
            (
                "",
                "main;worker.loop;worker.process_order;orders.reserve_stock;psycopg.cursor.execute",
                30,
            ),
            (
                "",
                "main;worker.loop;worker.process_order;orders.send_confirmation;smtplib.SMTP.sendmail",
                20,
            ),
            (
                "",
                "main;worker.loop;kombu.Consumer.drain_events;socket.recv",
                25,
            ),
        ],
    ),
];

struct Pb(Vec<u8>);

impl Pb {
    fn varint(&mut self, mut v: u64) {
        while v >= 0x80 {
            self.0.push((v as u8) | 0x80);
            v >>= 7;
        }
        self.0.push(v as u8);
    }
    fn int(&mut self, field: u32, v: u64) {
        self.varint((field as u64) << 3);
        self.varint(v);
    }
    fn bytes(&mut self, field: u32, b: &[u8]) {
        self.varint(((field as u64) << 3) | 2);
        self.varint(b.len() as u64);
        self.0.extend_from_slice(b);
    }
    fn packed(&mut self, field: u32, vs: &[u64]) {
        let mut p = Pb(Vec::new());
        for v in vs {
            p.varint(*v);
        }
        self.bytes(field, &p.0);
    }
}

/// A pprof profile of one service over one minute.
fn pprof(rng: &mut impl Rng, lang: &str, stacks: &[(&str, &str, u32)], start_ns: i64) -> Vec<u8> {
    let mut strings: Vec<String> = vec![String::new()];
    let idx = |s: &str, strings: &mut Vec<String>| -> u64 {
        match strings.iter().position(|x| x == s) {
            Some(i) => i as u64,
            None => {
                strings.push(s.to_string());
                (strings.len() - 1) as u64
            }
        }
    };
    let mut p = Pb(Vec::new());
    for (t, u) in [
        ("cpu-time", "nanoseconds"),
        ("wall-time", "nanoseconds"),
        ("alloc-space", "bytes"),
    ] {
        let mut vt = Pb(Vec::new());
        vt.int(1, idx(t, &mut strings));
        vt.int(2, idx(u, &mut strings));
        p.bytes(1, &vt.0);
    }
    let endpoint_key = idx("trace endpoint", &mut strings);
    let mut funcs: Vec<String> = Vec::new();
    for (endpoint, stack, weight) in stacks {
        // Locations are leaf first; one location (and function) per frame.
        let ids: Vec<u64> = stack
            .split(';')
            .rev()
            .map(|f| match funcs.iter().position(|x| x == f) {
                Some(i) => i as u64 + 1,
                None => {
                    funcs.push(f.to_string());
                    funcs.len() as u64
                }
            })
            .collect();
        let jitter = rng.random_range(0.7..1.3);
        let cpu = (*weight as f64 * 10_000_000.0 * jitter) as u64;
        let mut s = Pb(Vec::new());
        s.packed(1, &ids);
        s.packed(
            2,
            &[cpu, cpu * 3, (*weight as f64 * 40_000.0 * jitter) as u64],
        );
        if !endpoint.is_empty() {
            let mut l = Pb(Vec::new());
            l.int(1, endpoint_key);
            l.int(2, idx(endpoint, &mut strings));
            s.bytes(3, &l.0);
        }
        p.bytes(2, &s.0);
    }
    for (i, f) in funcs.iter().enumerate() {
        let id = i as u64 + 1;
        let mut loc = Pb(Vec::new());
        loc.int(1, id);
        let mut line = Pb(Vec::new());
        line.int(1, id);
        line.int(2, 10 + i as u64);
        loc.bytes(4, &line.0);
        p.bytes(4, &loc.0);
        let file = match (lang, f.rsplit_once('.')) {
            ("python", Some((m, _))) => m.replace('.', "/") + ".py",
            _ => String::new(),
        };
        let mut func = Pb(Vec::new());
        func.int(1, id);
        func.int(2, idx(f, &mut strings));
        func.int(4, idx(&file, &mut strings));
        p.bytes(5, &func.0);
    }
    for s in &strings {
        p.bytes(6, s.as_bytes());
    }
    p.int(9, start_ns as u64);
    p.int(10, 60_000_000_000);
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let _ = gz.write_all(&p.0);
    gz.finish().unwrap_or_default()
}

fn rfc3339(ms: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp_nanos(ms as i128 * 1_000_000)
        .ok()
        .and_then(|t| {
            t.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_default()
}

/// The multipart uploads (content type, body) of every service for the
/// minute ending at `end_ms`.
pub fn uploads(rng: &mut impl Rng, end_ms: i64, env: &str) -> Vec<(String, Vec<u8>)> {
    STACKS
        .iter()
        .map(|(service, lang, stacks)| {
            let start = end_ms - 60_000;
            let version = if *service == "shop-api" { "1.4.2" } else { "2.0.1" };
            let event = serde_json::json!({
                "attachments": ["auto.pprof"],
                "tags_profiler": format!("service:{service},env:{env},version:{version},host:demo-host,language:{lang},runtime-id:demo-{service}"),
                "start": rfc3339(start), "end": rfc3339(end_ms), "family": lang, "version": "4",
            });
            let data = pprof(rng, lang, stacks, start * 1_000_000);
            let mut body = Vec::new();
            for (name, file, ct, bytes) in [
                ("event", "event.json", "application/json", event.to_string().into_bytes()),
                ("auto.pprof", "auto.pprof", "application/octet-stream", data),
            ] {
                body.extend(
                    format!(
                        "--DEMOPROFILE\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{file}\"\r\nContent-Type: {ct}\r\n\r\n"
                    )
                    .as_bytes(),
                );
                body.extend(bytes);
                body.extend(b"\r\n");
            }
            body.extend(b"--DEMOPROFILE--\r\n");
            ("multipart/form-data; boundary=DEMOPROFILE".to_string(), body)
        })
        .collect()
}
