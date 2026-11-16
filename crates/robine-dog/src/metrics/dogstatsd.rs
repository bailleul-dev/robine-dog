//! DogStatsD over UDP (port 8125), as a local Datadog Agent receives it:
//! `<name>:<value>[:<value>…]|<type>[|@<rate>][|#<tag>,…][|T<unix ts>]`, one
//! metric per line. Events (`_e{`) and service checks (`_sc|`) are ignored.

use super::{Aggregator, Sample, normalize_tags};
use std::net::SocketAddr;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub name: String,
    pub samples: Vec<Sample>,
    pub tags: Vec<String>,
    pub host: Option<String>,
    /// Epoch milliseconds, when the client set one.
    pub ts: Option<i64>,
}

pub fn parse_line(line: &str) -> Option<Parsed> {
    let line = line.trim();
    if line.is_empty() || line.starts_with("_e{") || line.starts_with("_sc|") {
        return None;
    }
    let mut parts = line.split('|');
    let head = parts.next()?;
    let (name, values) = head.split_once(':')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let typ = parts.next()?.trim();
    let mut rate = 1.0f64;
    let mut tags = Vec::new();
    let mut ts = None;
    for p in parts {
        if let Some(r) = p.strip_prefix('@') {
            rate = r.parse().ok().filter(|r: &f64| *r > 0.0 && *r <= 1.0)?;
        } else if let Some(t) = p.strip_prefix('#') {
            tags.extend(t.split(',').map(str::to_string));
        } else if let Some(t) = p.strip_prefix('T') {
            ts = t.parse::<i64>().ok().map(|s| s * 1000);
        }
        // `c:` (container id) and unknown fields are ignored.
    }
    let mut host = None;
    tags.retain(|t| match t.strip_prefix("host:") {
        Some(h) => {
            host = Some(h.to_string());
            false
        }
        None => true,
    });
    let mut samples = Vec::new();
    for v in values.split(':') {
        let s = match typ {
            "s" => Sample::Set(v.to_string()),
            _ => {
                let x: f64 = v.trim().parse().ok().filter(|x: &f64| x.is_finite())?;
                match typ {
                    // A sampled counter stands for 1/rate occurrences.
                    "c" => Sample::Count(x / rate),
                    "g" => Sample::Gauge(x),
                    "h" | "ms" => Sample::Histogram(x),
                    "d" => Sample::Distribution(x),
                    _ => return None,
                }
            }
        };
        samples.push(s);
    }
    Some(Parsed {
        name: name.to_string(),
        samples,
        tags: normalize_tags(tags),
        host,
        ts,
    })
}

pub fn ingest(agg: &Aggregator, packet: &str, now_ms: i64) -> usize {
    let mut n = 0;
    for line in packet.lines() {
        if let Some(p) = parse_line(line) {
            for s in p.samples {
                agg.add(
                    p.ts.unwrap_or(now_ms),
                    &p.name,
                    p.host.as_deref(),
                    p.tags.clone(),
                    s,
                );
                n += 1;
            }
        }
    }
    n
}

pub async fn listen(addr: SocketAddr, agg: Arc<Aggregator>) -> anyhow::Result<()> {
    let sock = tokio::net::UdpSocket::bind(addr).await?;
    tracing::info!(%addr, "DogStatsD listening (UDP)");
    let mut buf = vec![0u8; 65_535];
    loop {
        let (len, _) = sock.recv_from(&mut buf).await?;
        let packet = String::from_utf8_lossy(&buf[..len]);
        ingest(&agg, &packet, crate::intake::logs::now_ms());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_protocol() {
        let p = parse_line("page.views:1|c|@0.5|#env:prod,host:web-1,team:a").unwrap();
        assert_eq!(p.name, "page.views");
        assert_eq!(p.samples, vec![Sample::Count(2.0)]);
        assert_eq!(p.tags, vec!["env:prod", "team:a"]);
        assert_eq!(p.host.as_deref(), Some("web-1"));

        let p = parse_line("lat:1:2.5:3|d|T1700000000").unwrap();
        assert_eq!(p.samples.len(), 3);
        assert_eq!(p.ts, Some(1_700_000_000_000));

        assert_eq!(
            parse_line("users:alice|s").unwrap().samples,
            vec![Sample::Set("alice".into())]
        );
        assert!(parse_line("_e{5,4}:title|text").is_none());
        assert!(parse_line("_sc|check|0").is_none());
        assert!(parse_line("bad:x|g").is_none());
        assert!(parse_line("noval|g").is_none());
        assert!(parse_line("x:1|zz").is_none());
    }

    #[test]
    fn ingests_a_packet_of_lines() {
        let agg = Aggregator::default();
        assert_eq!(ingest(&agg, "a:1|c\nb:2|g\n\nc:1:2|h", 0), 4);
    }
}
