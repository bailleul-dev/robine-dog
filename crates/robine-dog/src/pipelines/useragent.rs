//! `http.useragent_details`, as Datadog's User-Agent parser fills it:
//! `browser` / `os` (family, major, minor, patch) and `device`
//! (family, category: Desktop, Mobile, Tablet, Bot or Other).
//! A compact set of rules covering the common browsers, tools and bots.

use regex::Regex;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;

struct Rule {
    re: Regex,
    family: &'static str,
}

fn rules(list: &[(&str, &'static str)]) -> Vec<Rule> {
    list.iter()
        .map(|(re, family)| Rule {
            re: Regex::new(re).expect("user-agent rule"),
            family,
        })
        .collect()
}

/// Order matters: Edge and Opera also say Chrome, Chrome also says Safari.
static BROWSERS: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    rules(&[
        (r"(?i)(?:googlebot)/(\d+)\.?(\d+)?", "Googlebot"),
        (r"(?i)bingbot/(\d+)\.?(\d+)?", "bingbot"),
        (r"(?i)(?:datadog agent|datadog/synthetics)[/ ]?(\d+)?\.?(\d+)?", "Datadog"),
        (r"Edg(?:e|A|iOS)?/(\d+)\.(\d+)\.?(\d+)?", "Edge"),
        (r"OPR/(\d+)\.(\d+)\.?(\d+)?", "Opera"),
        (r"SamsungBrowser/(\d+)\.(\d+)", "Samsung Internet"),
        (r"(?:Chrome|CriOS)/(\d+)\.(\d+)\.?(\d+)?", "Chrome"),
        (r"(?:Firefox|FxiOS)/(\d+)\.(\d+)\.?(\d+)?", "Firefox"),
        (r"Version/(\d+)\.(\d+)\.?(\d+)?(?: Mobile/\S+)? Safari/", "Safari"),
        (r"MSIE (\d+)\.(\d+)", "IE"),
        (r"Trident/.*rv:(\d+)\.(\d+)", "IE"),
        (r"curl/(\d+)\.(\d+)\.?(\d+)?", "curl"),
        (r"Wget/(\d+)\.(\d+)\.?(\d+)?", "Wget"),
        (r"python-requests/(\d+)\.(\d+)\.?(\d+)?", "Python Requests"),
        (r"(?:aiohttp|httpx)/(\d+)\.(\d+)\.?(\d+)?", "Python HTTP"),
        (r"Go-http-client/(\d+)\.(\d+)", "Go-http-client"),
        (r"okhttp/(\d+)\.(\d+)\.?(\d+)?", "okhttp"),
        (r"Apache-HttpClient/(\d+)\.(\d+)\.?(\d+)?", "Apache-HttpClient"),
        (r"axios/(\d+)\.(\d+)\.?(\d+)?", "axios"),
        (r"node-fetch/(\d+)\.(\d+)\.?(\d+)?", "node-fetch"),
        (r"PostmanRuntime/(\d+)\.(\d+)\.?(\d+)?", "PostmanRuntime"),
        (r"kube-probe/(\d+)\.(\d+)", "kube-probe"),
        (r"ELB-HealthChecker/(\d+)\.(\d+)", "ELB-HealthChecker"),
    ])
});

static OSES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    rules(&[
        (r"Windows NT 10\.0", "Windows"),
        (r"Windows NT (\d+)\.(\d+)", "Windows"),
        (r"(?:iPhone|iPad|iPod).*? OS (\d+)_(\d+)_?(\d+)?", "iOS"),
        (r"Mac OS X (\d+)[_.](\d+)[_.]?(\d+)?", "Mac OS X"),
        (r"Android (\d+)\.?(\d+)?\.?(\d+)?", "Android"),
        (r"CrOS \S+ (\d+)\.(\d+)", "Chrome OS"),
        (r"Ubuntu", "Ubuntu"),
        (r"Linux", "Linux"),
    ])
});

fn version(c: &regex::Captures, i: usize) -> Option<String> {
    c.get(i).map(|m| m.as_str().to_string())
}

fn family(list: &[Rule], ua: &str) -> Map<String, Value> {
    let mut out = Map::new();
    for r in list {
        if let Some(c) = r.re.captures(ua) {
            out.insert("family".into(), json!(r.family));
            for (i, k) in ["major", "minor", "patch"].iter().enumerate() {
                if let Some(v) = version(&c, i + 1) {
                    out.insert((*k).into(), json!(v));
                }
            }
            return out;
        }
    }
    out.insert("family".into(), json!("Other"));
    out
}

pub fn parse(ua: &str) -> Value {
    let browser = family(&BROWSERS, ua);
    let os = family(&OSES, ua);
    let lower = ua.to_lowercase();
    let bot = ["bot", "crawler", "spider", "probe", "healthcheck", "synthetics"]
        .iter()
        .any(|w| lower.contains(w));
    let tool = matches!(
        browser.get("family").and_then(Value::as_str),
        Some(
            "curl"
                | "Wget"
                | "Python Requests"
                | "Python HTTP"
                | "Go-http-client"
                | "okhttp"
                | "Apache-HttpClient"
                | "axios"
                | "node-fetch"
                | "PostmanRuntime"
        )
    );
    let (device, category) = if bot {
        ("Spider", "Bot")
    } else if ua.contains("iPad") {
        ("iPad", "Tablet")
    } else if ua.contains("iPhone") {
        ("iPhone", "Mobile")
    } else if ua.contains("Android") && ua.contains("Mobile") {
        ("Generic Smartphone", "Mobile")
    } else if ua.contains("Android") {
        ("Generic Tablet", "Tablet")
    } else if ua.contains("Macintosh") {
        ("Mac", "Desktop")
    } else if tool {
        ("Other", "Other")
    } else if ua.contains("Windows") || ua.contains("Linux") || ua.contains("X11") {
        ("Other", "Desktop")
    } else {
        ("Other", "Other")
    };
    json!({
        "browser": browser,
        "os": os,
        "device": {"family": device, "category": category},
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_agents() {
        let v = parse("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.6778.86 Safari/537.36");
        assert_eq!(v["browser"]["family"], "Chrome");
        assert_eq!(v["browser"]["major"], "131");
        assert_eq!(v["os"]["family"], "Mac OS X");
        assert_eq!(v["device"]["category"], "Desktop");

        let v = parse("Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1");
        assert_eq!(v["browser"]["family"], "Safari");
        assert_eq!(v["os"]["family"], "iOS");
        assert_eq!(v["device"]["category"], "Mobile");

        let v = parse("curl/8.5.0");
        assert_eq!((v["browser"]["family"].as_str(), v["device"]["category"].as_str()), (Some("curl"), Some("Other")));

        let v = parse("Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)");
        assert_eq!(v["device"]["category"], "Bot");
        let v = parse("kube-probe/1.31");
        assert_eq!(v["device"]["category"], "Bot");
    }
}
