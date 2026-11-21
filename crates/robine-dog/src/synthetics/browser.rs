//! Browser tests: a headless Chromium driven over the Chrome DevTools
//! Protocol, one fresh profile per run. Steps have the shape of Datadog's
//! (`goToUrl`, `click`, `typeText`, `assertElementContent`…), elements are
//! found by their `userLocator` (CSS or XPath), and every step ends with a
//! screenshot.

use super::assertions::text_holds;
use super::variables::{Vars, render, render_value};
use super::{StepResult, Test};
use base64::Engine;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Semaphore;
use tokio_tungstenite::tungstenite::Message;

/// Browsers are heavy: two runs at a time.
static SLOTS: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(2));

const DEFAULT_STEP_TIMEOUT: f64 = 20.0;
const MAX_RUN: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrowserStep {
    #[serde(default)]
    pub name: String,
    #[serde(rename = "type")]
    pub typ: String,
    #[serde(default)]
    pub params: Value,
    #[serde(rename = "allowFailure", default)]
    pub allow_failure: bool,
    /// Seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

pub const STEP_TYPES: &[&str] = &[
    "goToUrl",
    "refresh",
    "click",
    "hover",
    "typeText",
    "pressKey",
    "selectOption",
    "scroll",
    "wait",
    "assertElementPresent",
    "assertElementContent",
    "assertElementAttribute",
    "assertPageContains",
    "assertPageDoesNotContain",
    "assertCurrentUrl",
    "assertFromJavascript",
    "extractFromJavascript",
    "extractVariable",
];

/// Datadog's devices: their viewport, and whether they are mobile.
pub fn device(id: &str) -> (u32, u32, bool) {
    match id {
        "chrome.tablet" | "firefox.tablet" | "edge.tablet" => (1024, 1366, false),
        "chrome.mobile_small" | "firefox.mobile_small" | "edge.mobile_small" => (390, 844, true),
        "chrome.laptop_small" => (1024, 768, false),
        _ => (1440, 1100, false),
    }
}

fn chromium() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ROBINE_DOG_CHROMIUM") {
        return Some(PathBuf::from(p));
    }
    let path = std::env::var_os("PATH")?;
    for name in [
        "chromium",
        "chromium-browser",
        "headless_shell",
        "google-chrome",
        "google-chrome-stable",
    ] {
        for dir in std::env::split_paths(&path) {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// A running browser and the page the steps drive.
struct Cdp {
    child: tokio::process::Child,
    profile: PathBuf,
    ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    next: u64,
    session: Option<String>,
}

impl Cdp {
    async fn launch() -> Result<Self, String> {
        let exe = chromium()
            .ok_or("no Chromium: install it, or set ROBINE_DOG_CHROMIUM to its path".to_string())?;
        let profile = std::env::temp_dir().join(format!(
            "robine-dog-chromium-{}",
            crate::store::pg::new_id().to_lowercase()
        ));
        let mut cmd = tokio::process::Command::new(&exe);
        cmd.args([
            "--headless=new",
            "--disable-gpu",
            "--disable-dev-shm-usage",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            "--disable-background-networking",
            "--mute-audio",
            "--hide-scrollbars",
            "--remote-debugging-port=0",
        ])
        .arg(format!("--user-data-dir={}", profile.display()));
        // `--no-sandbox` in containers where the sandbox cannot start.
        if let Ok(extra) = std::env::var("ROBINE_DOG_CHROMIUM_ARGS") {
            cmd.args(extra.split_whitespace());
        }
        cmd.arg("about:blank")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("starting {}: {e}", exe.display()))?;
        let stderr = child.stderr.take().ok_or("no stderr")?;
        let mut lines = BufReader::new(stderr).lines();
        let find = async {
            let mut seen = Vec::new();
            while let Ok(Some(l)) = lines.next_line().await {
                if let Some(url) = l.split("DevTools listening on ").nth(1) {
                    return Ok(url.trim().to_string());
                }
                seen.push(l);
            }
            Err(format!(
                "Chromium exited: {}",
                seen.into_iter()
                    .rev()
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(" | ")
            ))
        };
        let url = tokio::time::timeout(Duration::from_secs(20), find)
            .await
            .map_err(|_| "Chromium did not start in time".to_string())??;
        // Keep reading its output, or it blocks once the pipe is full.
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
        let (ws, _) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .map_err(|e| format!("DevTools: {e}"))?;
        let mut cdp = Cdp {
            child,
            profile,
            ws,
            next: 0,
            session: None,
        };
        let target = cdp
            .call("Target.createTarget", json!({"url": "about:blank"}))
            .await?;
        let attached = cdp
            .call(
                "Target.attachToTarget",
                json!({"targetId": target["targetId"], "flatten": true}),
            )
            .await?;
        cdp.session = attached["sessionId"].as_str().map(String::from);
        Ok(cdp)
    }

    /// A command, to the page once attached; its result.
    async fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next += 1;
        let id = self.next;
        let mut msg = json!({"id": id, "method": method, "params": params});
        if let Some(s) = &self.session {
            msg["sessionId"] = json!(s);
        }
        self.ws
            .send(Message::Text(msg.to_string().into()))
            .await
            .map_err(|e| format!("DevTools: {e}"))?;
        let wait = async {
            while let Some(m) = self.ws.next().await {
                let Ok(Message::Text(t)) = m else { continue };
                let Ok(v) = serde_json::from_str::<Value>(&t) else {
                    continue;
                };
                if v["id"].as_u64() != Some(id) {
                    continue;
                }
                if let Some(e) = v.get("error") {
                    return Err(e["message"]
                        .as_str()
                        .unwrap_or("DevTools error")
                        .to_string());
                }
                return Ok(v["result"].clone());
            }
            Err("the browser closed".to_string())
        };
        tokio::time::timeout(Duration::from_secs(60), wait)
            .await
            .map_err(|_| format!("{method}: no answer from the browser"))?
    }

    /// A JavaScript expression's value.
    async fn eval(&mut self, expr: &str) -> Result<Value, String> {
        let r = self
            .call(
                "Runtime.evaluate",
                json!({"expression": expr, "returnByValue": true, "awaitPromise": true}),
            )
            .await?;
        if let Some(ex) = r.get("exceptionDetails") {
            return Err(ex["exception"]["description"]
                .as_str()
                .or(ex["text"].as_str())
                .unwrap_or("JavaScript error")
                .lines()
                .next()
                .unwrap_or_default()
                .to_string());
        }
        Ok(r["result"]["value"].clone())
    }

    /// Until the page has loaded (or the time is up).
    async fn settle(&mut self, limit: Duration) {
        let started = Instant::now();
        while started.elapsed() < limit {
            if let Ok(v) = self.eval("document.readyState").await
                && v == "complete"
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn navigate(&mut self, url: &str, limit: Duration) -> Result<(), String> {
        let r = self.call("Page.navigate", json!({"url": url})).await?;
        if let Some(e) = r["errorText"].as_str().filter(|e| !e.is_empty()) {
            return Err(format!("cannot load {url}: {e}"));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        self.settle(limit).await;
        Ok(())
    }

    async fn screenshot(&mut self) -> Option<Vec<u8>> {
        let r = self
            .call(
                "Page.captureScreenshot",
                json!({"format": "jpeg", "quality": 55}),
            )
            .await
            .ok()?;
        base64::engine::general_purpose::STANDARD
            .decode(r["data"].as_str()?)
            .ok()
    }

    async fn close(mut self) {
        self.session = None;
        let _ = self.call("Browser.close", json!({})).await;
        let _ = tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await;
        let _ = self.child.kill().await;
        let _ = tokio::fs::remove_dir_all(&self.profile).await;
    }
}

/// The element a step targets: its first CSS or XPath locator.
fn locator(params: &Value) -> Option<(String, String)> {
    let el = &params["element"];
    let from = |v: &Value| {
        Some((
            v["type"].as_str().unwrap_or("css").to_string(),
            v["value"].as_str()?.to_string(),
        ))
    };
    el["userLocator"]["values"]
        .as_array()
        .and_then(|a| a.iter().find_map(from))
        .or_else(|| el["selector"].as_str().map(|s| ("css".into(), s.into())))
}

/// A JavaScript expression evaluating to the element, or `null`.
fn find_js(kind: &str, value: &str) -> String {
    let v = serde_json::to_string(value).unwrap_or_default();
    if kind == "xpath" {
        format!(
            "document.evaluate({v}, document, null, XPathResult.FIRST_ORDERED_NODE_TYPE, null).singleNodeValue"
        )
    } else {
        format!("document.querySelector({v})")
    }
}

/// Retries a JavaScript check (`{ok, actual}`) until it passes or the time
/// is up; the last value seen.
async fn until(cdp: &mut Cdp, js: &str, limit: Duration) -> Result<Value, (String, Value)> {
    let started = Instant::now();
    loop {
        let last = match cdp.eval(js).await {
            Ok(v) if v["ok"] == true => return Ok(v),
            Ok(v) => (v["why"].as_str().unwrap_or("no").to_string(), v),
            Err(e) => (e, Value::Null),
        };
        if started.elapsed() >= limit {
            return Err(last);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// The element's center, scrolled into view.
async fn element_at(cdp: &mut Cdp, params: &Value, limit: Duration) -> Result<(f64, f64), String> {
    let (kind, value) = locator(params).ok_or("the step has no element locator")?;
    let js = format!(
        "(() => {{ const el = {}; if (!el) return {{ok: false, why: 'cannot locate the element ' + {}}};
           el.scrollIntoView({{block: 'center', inline: 'center'}});
           const r = el.getBoundingClientRect();
           if (!r.width && !r.height) return {{ok: false, why: 'the element is not visible'}};
           return {{ok: true, x: r.x + r.width / 2, y: r.y + r.height / 2}}; }})()",
        find_js(&kind, &value),
        serde_json::to_string(&value).unwrap_or_default()
    );
    let v = until(cdp, &js, limit).await.map_err(|(why, _)| why)?;
    Ok((
        v["x"].as_f64().unwrap_or(0.0),
        v["y"].as_f64().unwrap_or(0.0),
    ))
}

/// `Enter`, `Tab`, `a`… as key events.
fn key(name: &str) -> (String, String, i64, Option<String>) {
    let (code, vk, text) = match name {
        "Enter" => ("Enter", 13, Some("\r")),
        "Tab" => ("Tab", 9, None),
        "Escape" => ("Escape", 27, None),
        "Backspace" => ("Backspace", 8, None),
        "Delete" => ("Delete", 46, None),
        "ArrowUp" => ("ArrowUp", 38, None),
        "ArrowDown" => ("ArrowDown", 40, None),
        "ArrowLeft" => ("ArrowLeft", 37, None),
        "ArrowRight" => ("ArrowRight", 39, None),
        "Space" | " " => ("Space", 32, Some(" ")),
        other => {
            let c = other.chars().next().unwrap_or(' ');
            let vk = c.to_ascii_uppercase() as i64;
            return (
                other.to_string(),
                format!("Key{}", c.to_ascii_uppercase()),
                vk,
                Some(other.to_string()),
            );
        }
    };
    (
        if name == "Space" {
            " ".into()
        } else {
            name.to_string()
        },
        code.to_string(),
        vk,
        text.map(String::from),
    )
}

async fn mouse(
    cdp: &mut Cdp,
    kind: &str,
    x: f64,
    y: f64,
    button: &str,
    count: i64,
) -> Result<(), String> {
    cdp.call(
        "Input.dispatchMouseEvent",
        json!({"type": kind, "x": x, "y": y, "button": button, "clickCount": count}),
    )
    .await
    .map(|_| ())
}

/// One step: what it extracted, if anything.
async fn step(
    cdp: &mut Cdp,
    s: &BrowserStep,
    vars: &Vars,
    limit: Duration,
) -> Result<Option<(String, String)>, String> {
    let p = render_value(&s.params, vars);
    let text = |k: &str| {
        p[k].as_str()
            .map(String::from)
            .unwrap_or_else(|| match &p[k] {
                Value::Null => String::new(),
                v => v.to_string(),
            })
    };
    let check = |actual: &str, what: &str| -> Result<(), String> {
        let op = p["check"].as_str().unwrap_or("contains");
        let expected = text("value");
        match text_holds(op, actual, &expected) {
            Ok(true) => Ok(()),
            Ok(false) => Err(format!("{what} is {actual:?}, should {op} {expected:?}")),
            Err(e) => Err(e),
        }
    };
    match s.typ.as_str() {
        "goToUrl" => cdp.navigate(&text("value"), limit).await?,
        "refresh" => {
            cdp.call("Page.reload", json!({})).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;
            cdp.settle(limit).await;
        }
        "click" | "hover" => {
            let (x, y) = element_at(cdp, &p, limit).await?;
            mouse(cdp, "mouseMoved", x, y, "none", 0).await?;
            if s.typ == "click" {
                let (button, count) = match p["clickType"].as_str() {
                    Some("contextual") => ("right", 1),
                    Some("double") => ("left", 2),
                    _ => ("left", 1),
                };
                for c in 1..=count {
                    mouse(cdp, "mousePressed", x, y, button, c).await?;
                    mouse(cdp, "mouseReleased", x, y, button, c).await?;
                }
                // A click may navigate: let the next page load.
                tokio::time::sleep(Duration::from_millis(300)).await;
                cdp.settle(limit).await;
            }
        }
        "typeText" => {
            let (kind, value) = locator(&p).ok_or("the step has no element locator")?;
            let js = format!(
                "(() => {{ const el = {}; if (!el) return {{ok: false, why: 'cannot locate the element'}};
                   el.focus(); if (el.select) el.select(); return {{ok: true}}; }})()",
                find_js(&kind, &value)
            );
            until(cdp, &js, limit).await.map_err(|(why, _)| why)?;
            cdp.call("Input.insertText", json!({"text": text("value")}))
                .await?;
        }
        "pressKey" => {
            let (k, code, vk, txt) = key(&text("value"));
            let modifiers = p["modifiers"].as_array().map_or(0, |m| {
                m.iter().fold(0, |acc, x| {
                    acc | match x.as_str() {
                        Some("Alt") => 1,
                        Some("Control") => 2,
                        Some("Meta") => 4,
                        Some("Shift") => 8,
                        _ => 0,
                    }
                })
            });
            let mut down = json!({"type": "keyDown", "key": k, "code": code,
                "windowsVirtualKeyCode": vk, "modifiers": modifiers});
            if let Some(t) = txt.filter(|_| modifiers == 0) {
                down["text"] = json!(t);
            }
            cdp.call("Input.dispatchKeyEvent", down).await?;
            cdp.call(
                "Input.dispatchKeyEvent",
                json!({"type": "keyUp", "key": k, "code": code, "windowsVirtualKeyCode": vk, "modifiers": modifiers}),
            )
            .await?;
            tokio::time::sleep(Duration::from_millis(200)).await;
            cdp.settle(limit).await;
        }
        "selectOption" => {
            let (kind, value) = locator(&p).ok_or("the step has no element locator")?;
            let js = format!(
                "(() => {{ const el = {}; if (!el) return {{ok: false, why: 'cannot locate the element'}};
                   const v = {}; const o = [...(el.options || [])].find(o => o.value === v || o.text === v);
                   if (!o) return {{ok: false, why: 'no option ' + v}};
                   el.value = o.value;
                   el.dispatchEvent(new Event('input', {{bubbles: true}}));
                   el.dispatchEvent(new Event('change', {{bubbles: true}}));
                   return {{ok: true}}; }})()",
                find_js(&kind, &value),
                serde_json::to_string(&text("value")).unwrap_or_default()
            );
            until(cdp, &js, limit).await.map_err(|(why, _)| why)?;
        }
        "scroll" => {
            if locator(&p).is_some() {
                element_at(cdp, &p, limit).await?;
            } else {
                let (x, y) = (
                    p["x"].as_f64().unwrap_or(0.0),
                    p["y"].as_f64().unwrap_or(0.0),
                );
                cdp.eval(&format!("window.scrollBy({x}, {y})")).await?;
            }
        }
        "wait" => {
            let secs = p["value"]
                .as_f64()
                .or_else(|| text("value").parse().ok())
                .unwrap_or(1.0)
                .clamp(0.0, 300.0);
            tokio::time::sleep(Duration::from_secs_f64(secs)).await;
        }
        "assertElementPresent" => {
            element_at(cdp, &p, limit).await?;
        }
        "assertElementContent" | "assertElementAttribute" | "extractVariable" => {
            let (kind, value) = locator(&p).ok_or("the step has no element locator")?;
            let read = if s.typ == "assertElementAttribute" {
                format!(
                    "el.getAttribute({}) ?? ''",
                    serde_json::to_string(&text("attribute")).unwrap_or_default()
                )
            } else {
                "('value' in el && el.tagName !== 'BUTTON' && el.tagName !== 'LI' ? String(el.value) : el.innerText)".into()
            };
            let js = format!(
                "(() => {{ const el = {}; if (!el) return {{ok: false, why: 'cannot locate the element'}};
                   return {{ok: true, actual: {read}}}; }})()",
                find_js(&kind, &value)
            );
            if s.typ == "extractVariable" {
                let v = until(cdp, &js, limit).await.map_err(|(why, _)| why)?;
                let name = p["variable"]["name"].as_str().unwrap_or("").to_string();
                return Ok(Some((name, v["actual"].as_str().unwrap_or("").to_string())));
            }
            // The content may still change: checked until it holds.
            let started = Instant::now();
            loop {
                let v = until(cdp, &js, limit).await.map_err(|(why, _)| why)?;
                let actual = v["actual"].as_str().unwrap_or("").trim().to_string();
                let what = if s.typ == "assertElementAttribute" {
                    format!("attribute {}", text("attribute"))
                } else {
                    "the element's content".into()
                };
                match check(&actual, &what) {
                    Ok(()) => break,
                    Err(e) if started.elapsed() >= limit => return Err(e),
                    Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
                }
            }
        }
        "assertPageContains" | "assertPageDoesNotContain" => {
            let want = text("value");
            let present = s.typ == "assertPageContains";
            let js = format!(
                "(() => {{ const t = document.body ? document.body.innerText : '';
                   const found = t.includes({});
                   return {{ok: found === {present}, why: found ? 'the page contains it' : 'the page does not contain it'}}; }})()",
                serde_json::to_string(&want).unwrap_or_default()
            );
            until(cdp, &js, limit)
                .await
                .map_err(|(why, _)| format!("{why}: {want:?}"))?;
        }
        "assertCurrentUrl" => {
            let started = Instant::now();
            loop {
                let url = cdp.eval("location.href").await?;
                match check(url.as_str().unwrap_or(""), "the URL") {
                    Ok(()) => break,
                    Err(e) if started.elapsed() >= limit => return Err(e),
                    Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
                }
            }
        }
        "assertFromJavascript" | "extractFromJavascript" => {
            let code = p["code"].as_str().unwrap_or("return true");
            let element = locator(&p)
                .map(|(k, v)| find_js(&k, &v))
                .unwrap_or_else(|| "null".into());
            let js = format!(
                "(async function(vars, element) {{ {code} }})({}, {element})",
                serde_json::to_string(vars).unwrap_or_else(|_| "{}".into())
            );
            if s.typ == "extractFromJavascript" {
                let v = cdp.eval(&js).await?;
                let name = p["variable"]["name"].as_str().unwrap_or("").to_string();
                let value = match v {
                    Value::String(s) => s,
                    Value::Null => String::new(),
                    other => other.to_string(),
                };
                return Ok(Some((name, value)));
            }
            let started = Instant::now();
            loop {
                let v = cdp.eval(&js).await?;
                let truthy = !matches!(v, Value::Null | Value::Bool(false))
                    && v != json!(0)
                    && v != json!("");
                if truthy {
                    break;
                }
                if started.elapsed() >= limit {
                    return Err(format!("the JavaScript assertion returned {v}"));
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
        other => return Err(format!("the step type {other} is not supported")),
    }
    Ok(None)
}

/// A browser run: its steps (the first loads the start URL), their
/// screenshots by step, and why it failed.
pub struct BrowserRun {
    pub steps: Vec<StepResult>,
    pub screenshots: Vec<(i32, Vec<u8>)>,
    pub failure: Option<String>,
    pub details: Value,
}

pub async fn run(
    t: &Test,
    start_url: &str,
    device_id: &str,
    default_timeout: Option<f64>,
    vars: &mut Vars,
) -> BrowserRun {
    let _slot = SLOTS.acquire().await;
    let started = Instant::now();
    let fail = |why: String| BrowserRun {
        steps: vec![],
        screenshots: vec![],
        failure: Some(why),
        details: Value::Null,
    };
    let mut cdp = match Cdp::launch().await {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let (w, h, mobile) = device(device_id);
    let _ = cdp
        .call(
            "Emulation.setDeviceMetricsOverride",
            json!({"width": w, "height": h, "deviceScaleFactor": 1, "mobile": mobile}),
        )
        .await;
    let mut out = BrowserRun {
        steps: vec![],
        screenshots: vec![],
        failure: None,
        details: json!({"device": {"id": device_id, "width": w, "height": h}, "start_url": start_url}),
    };
    let all = async {
        let first = BrowserStep {
            name: format!("Navigate to start URL {start_url}"),
            typ: "goToUrl".into(),
            params: json!({"value": start_url}),
            ..Default::default()
        };
        for (i, s) in std::iter::once(&first).chain(&t.steps).enumerate() {
            let limit = Duration::from_secs_f64(
                s.timeout
                    .or(default_timeout)
                    .unwrap_or(DEFAULT_STEP_TIMEOUT)
                    .clamp(1.0, 300.0),
            );
            let t0 = Instant::now();
            let res = step(&mut cdp, s, vars, limit).await;
            let duration_ms = t0.elapsed().as_secs_f64() * 1000.0;
            let mut extracted = Map::new();
            if let Ok(Some((name, value))) = &res
                && !name.is_empty()
            {
                vars.insert(name.clone(), value.clone());
                extracted.insert(name.clone(), json!(value));
            }
            let url = cdp
                .eval("location.href")
                .await
                .ok()
                .and_then(|v| v.as_str().map(String::from));
            let shot = cdp.screenshot().await;
            let failure = res.err();
            if let Some(png) = shot {
                out.screenshots.push((i as i32, png));
            }
            out.steps.push(StepResult {
                name: if s.name.is_empty() {
                    describe(s, vars)
                } else {
                    render(&s.name, vars)
                },
                typ: s.typ.clone(),
                passed: failure.is_none(),
                duration_ms,
                failure: failure.clone(),
                allow_failure: s.allow_failure,
                url,
                extracted,
                screenshot: out.screenshots.last().is_some_and(|(n, _)| *n == i as i32),
                ..Default::default()
            });
            if let Some(f) = failure {
                // A step allowed to fail fails the test only if critical.
                let critical = !s.allow_failure
                    || s.extra.get("isCritical").and_then(Value::as_bool) == Some(true);
                if critical && out.failure.is_none() {
                    out.failure = Some(format!("step {} ({}): {f}", i, out.steps[i].name));
                }
                if !s.allow_failure {
                    break;
                }
            }
        }
    };
    if tokio::time::timeout(MAX_RUN.saturating_sub(started.elapsed()), all)
        .await
        .is_err()
    {
        out.failure
            .get_or_insert_with(|| "the test took more than 5 minutes".into());
    }
    cdp.close().await;
    out
}

/// A step's default name: `Click on #buy`, `Type "x" in #q`.
/// A page printed to PDF (A3 landscape, with backgrounds), once it has
/// settled and `wait` more has passed for charts to draw. Dashboard reports.
pub async fn pdf(url: &str, wait: Duration) -> Result<Vec<u8>, String> {
    let mut cdp = Cdp::launch().await?;
    let r = async {
        let _ = cdp
            .call(
                "Emulation.setDeviceMetricsOverride",
                json!({"width": 1600, "height": 1000, "deviceScaleFactor": 1, "mobile": false}),
            )
            .await;
        cdp.navigate(url, Duration::from_secs(30)).await?;
        tokio::time::sleep(wait).await;
        let r = cdp
            .call(
                "Page.printToPDF",
                json!({"landscape": false, "printBackground": true, "preferCSSPageSize": false,
                    "paperWidth": 16.5, "paperHeight": 11.7, "marginTop": 0.3, "marginBottom": 0.3,
                    "marginLeft": 0.3, "marginRight": 0.3}),
            )
            .await?;
        base64::engine::general_purpose::STANDARD
            .decode(r["data"].as_str().unwrap_or(""))
            .map_err(|e| e.to_string())
    }
    .await;
    cdp.close().await;
    r
}

pub fn describe(s: &BrowserStep, vars: &Vars) -> String {
    let p = render_value(&s.params, vars);
    let el = locator(&p).map(|(_, v)| v).unwrap_or_default();
    let value = p["value"]
        .as_str()
        .map(String::from)
        .unwrap_or_else(|| p["value"].to_string());
    match s.typ.as_str() {
        "goToUrl" => format!("Navigate to {value}"),
        "click" => format!("Click on {el}"),
        "hover" => format!("Hover over {el}"),
        "typeText" => format!("Type \"{value}\" in {el}"),
        "pressKey" => format!("Press {value}"),
        "selectOption" => format!("Select {value} in {el}"),
        "wait" => format!("Wait {value} s"),
        "assertElementPresent" => format!("Test that {el} is present"),
        "assertElementContent" => format!(
            "Test that {el} content {} \"{value}\"",
            p["check"].as_str().unwrap_or("contains")
        ),
        "assertPageContains" => format!("Test that the page contains \"{value}\""),
        "assertPageDoesNotContain" => format!("Test that the page does not contain \"{value}\""),
        "assertElementAttribute" => format!(
            "Test that {el} attribute {} {} \"{value}\"",
            p["attribute"].as_str().unwrap_or(""),
            p["check"].as_str().unwrap_or("contains")
        ),
        "extractVariable" => format!(
            "Extract the content of {el} into {}",
            p["variable"]["name"].as_str().unwrap_or("")
        ),
        "extractFromJavascript" => format!(
            "Extract a value with JavaScript into {}",
            p["variable"]["name"].as_str().unwrap_or("")
        ),
        "assertFromJavascript" => "Test with JavaScript".into(),
        "refresh" => "Refresh the page".into(),
        "scroll" => "Scroll".into(),
        "assertCurrentUrl" => format!(
            "Test that the URL {} \"{value}\"",
            p["check"].as_str().unwrap_or("contains")
        ),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locators_and_keys() {
        let p = json!({"element": {"userLocator": {"values": [{"type": "xpath", "value": "//button"}]}}});
        assert_eq!(locator(&p), Some(("xpath".into(), "//button".into())));
        assert_eq!(
            locator(&json!({"element": {"selector": "#buy"}})),
            Some(("css".into(), "#buy".into()))
        );
        assert_eq!(locator(&json!({})), None);
        assert!(find_js("css", "a[href=\"x\"]").contains(r#"querySelector("a[href=\"x\"]")"#));
        assert_eq!(key("Enter").3.as_deref(), Some("\r"));
        assert_eq!(key("a").2, 65);
        let s = BrowserStep {
            typ: "typeText".into(),
            params: json!({"value": "{{ USER }}", "element": {"selector": "#login"}}),
            ..Default::default()
        };
        let vars = Vars::from([("USER".to_string(), "ana".to_string())]);
        assert_eq!(describe(&s, &vars), "Type \"ana\" in #login");
        assert_eq!(device("chrome.mobile_small"), (390, 844, true));
    }
}
