//! Browser sessions, as the RUM SDK reports them: views of a shop journey
//! (home, product, cart, checkout, confirmation) that fewer sessions reach
//! at each step, clicks (some of them rage, dead or error clicks), errors,
//! and users who come back on later days.

use rand::Rng;
use serde_json::{Value, json};

const USERS: usize = 60;
const NAMES: &[&str] = &[
    "ana", "ben", "chloe", "dario", "emma", "farid", "gaia", "hugo", "ines", "jules",
];

fn id(rng: &mut impl Rng) -> String {
    let b: [u8; 16] = rng.random();
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// The users active on a day: the first ones come back more often.
pub fn users_of_day(rng: &mut impl Rng, day: i64) -> Vec<usize> {
    (0..USERS)
        .filter(|u| {
            // Each user starts on some day, then returns less and less.
            let first = (*u as i64 * 5) % 14;
            let since = day - first;
            since >= 0
                && rng.random_bool(if since == 0 {
                    0.9
                } else {
                    0.55 / (1.0 + since as f64 / 3.0)
                })
        })
        .collect()
}

/// One session's events, starting at `start_ms`.
pub fn session(rng: &mut impl Rng, start_ms: i64, user: usize) -> Vec<Value> {
    let sid = id(rng);
    let name = format!("{}{}", NAMES[user % NAMES.len()], user);
    let usr =
        json!({"id": format!("u-{user:03}"), "name": name, "email": format!("{name}@example.com")});
    let mobile = user.is_multiple_of(3);
    let (browser, os, device) = if mobile {
        ("Mobile Safari", "iOS", "mobile")
    } else if user % 3 == 1 {
        ("Chrome", "Windows", "desktop")
    } else {
        ("Firefox", "Mac OS X", "desktop")
    };
    let country = ["France", "Germany", "Spain", "United States"][user % 4];
    let product = rng.random_range(1..40);
    let journey: Vec<(&str, String)> = vec![
        ("home", "/".into()),
        ("product", format!("/products/{product}")),
        ("cart", "/cart".into()),
        ("checkout", "/checkout".into()),
        ("confirmed", "/order/confirmed".into()),
    ];
    // How far the session goes: fewer sessions at each step.
    let mut depth = 1;
    for p in [0.75, 0.55, 0.6, 0.7] {
        if rng.random_bool(p) {
            depth += 1;
        } else {
            break;
        }
    }
    let mut out = Vec::new();
    let mut t = start_ms;
    let base = |typ: &str, t: i64, view: &Value| {
        json!({
            "type": typ, "date": t, "service": "shop-web", "version": "2.4.1", "source": "browser",
            "ddtags": "env:prod,version:2.4.1",
            "application": {"id": "shop-web"},
            "session": {"id": sid, "type": "user", "has_replay": false},
            "view": view, "usr": usr,
            "browser": {"name": browser}, "os": {"name": os}, "device": {"type": device},
            "geo": {"country": country},
        })
    };
    for (step, (label, path)) in journey.iter().take(depth).enumerate() {
        let vid = id(rng);
        let slow = *label == "checkout" && rng.random_bool(0.3);
        let loading = rng.random_range(300.0..1500.0) * if slow { 3.0 } else { 1.0 };
        let mut view = json!({
            "id": vid, "url": format!("https://shop.example{path}"), "name": path,
            "referrer": "", "loading_type": if step == 0 { "initial_load" } else { "route_change" },
        });
        let (mut actions, mut errors, mut frustrations) = (0, 0, 0);
        let view_start = t;
        // The first update, at load.
        out.push(base("view", t, &view));
        t += rng.random_range(800..3000);
        let clicks = rng.random_range(1..5);
        for _ in 0..clicks {
            let target = match *label {
                "home" => "Shop now",
                "product" => "Add to cart",
                "cart" => "Checkout",
                "checkout" => "Pay",
                _ => "Continue shopping",
            };
            let mut action = json!({"id": id(rng), "type": "click", "target": {"name": target}});
            let roll: f64 = rng.random();
            let frustration = if *label == "checkout" && roll < 0.25 {
                Some("rage_click")
            } else if *label == "product" && roll < 0.08 {
                Some("dead_click")
            } else if *label == "cart" && roll < 0.1 {
                Some("error_click")
            } else {
                None
            };
            if let Some(f) = frustration {
                action["frustration"] = json!({"type": [f]});
                frustrations += 1;
                if f == "error_click" {
                    let mut e = base("error", t + 50, &view);
                    e["error"] = json!({"message": "Cannot read properties of undefined (reading 'total')",
                        "type": "TypeError", "source": "source", "handling": "unhandled",
                        "stack": "TypeError: Cannot read properties of undefined (reading 'total')\n    at updateCart (cart.js:42:17)"});
                    out.push(e);
                    errors += 1;
                }
            }
            let mut a = base("action", t, &view);
            a["action"] = action;
            out.push(a);
            actions += 1;
            t += rng.random_range(500..4000);
        }
        if *label == "checkout" && slow && rng.random_bool(0.3) {
            let mut e = base("error", t, &view);
            e["error"] = json!({"message": "Payment provider timeout", "type": "Error", "source": "network"});
            out.push(e);
            errors += 1;
        }
        // The final update: metrics, counts, and the view's end.
        let spent = (t - view_start) * 1_000_000;
        let v = view.as_object_mut().expect("view");
        for (k, val) in [
            ("loading_time", json!((loading * 1e6) as i64)),
            (
                "first_contentful_paint",
                json!((loading * 0.4 * 1e6) as i64),
            ),
            (
                "largest_contentful_paint",
                json!((loading * 0.8 * 1e6) as i64),
            ),
            (
                "interaction_to_next_paint",
                json!(rng.random_range(40..400) * 1_000_000),
            ),
            ("cumulative_layout_shift", json!(rng.random_range(0.0..0.3))),
            ("time_spent", json!(spent)),
            ("action", json!({"count": actions})),
            ("error", json!({"count": errors})),
            ("frustration", json!({"count": frustrations})),
            ("is_active", json!(false)),
        ] {
            v.insert(k.into(), val);
        }
        out.push(base("view", t, &view));
        t += rng.random_range(100..600);
    }
    out
}
