//! The API tests' protocols: HTTP, SSL, TCP, DNS, UDP, WebSocket, gRPC
//! health checks and ICMP. Each runs one request and checks it against the
//! test's assertions.

use super::assertions::{
    Assertion, HttpObserved, as_text, check_number, check_text, http_failure, number_holds,
    op_words, text_holds,
};
use super::{Request, TestOptions, dns, x509};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

const MAX_BODY: usize = 256 * 1024;

/// What a request observed.
#[derive(Debug, Clone, Default)]
pub struct Observed {
    pub failure: Option<String>,
    pub duration_ms: f64,
    pub status_code: Option<u16>,
    /// What the protocol tells (certificate, records, latencies…), shown with
    /// the result.
    pub details: Value,
    /// For multistep tests' extracted values.
    pub http: Option<HttpResponse>,
}

#[derive(Debug, Clone, Default)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: String,
}

fn elapsed_ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn failed(started: Instant, why: impl Into<String>) -> Observed {
    Observed {
        failure: Some(why.into()),
        duration_ms: elapsed_ms(started),
        ..Default::default()
    }
}

/// The innermost cause, which says what went wrong.
fn root_cause(e: &(dyn std::error::Error + 'static)) -> String {
    let mut why = e.to_string();
    let mut source = e.source();
    while let Some(s) = source {
        why = s.to_string();
        source = s.source();
    }
    why
}

fn timeout_of(r: &Request) -> Duration {
    Duration::from_secs_f64(r.timeout.unwrap_or(10.0).clamp(1.0, 60.0))
}

fn port_of(r: &Request, default: u16) -> u16 {
    match &r.port {
        Some(Value::Number(n)) => n.as_u64().and_then(|p| u16::try_from(p).ok()),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    }
    .unwrap_or(default)
}

async fn resolve(host: &str, port: u16) -> Result<SocketAddr, String> {
    if host.trim().is_empty() {
        return Err("no host".into());
    }
    tokio::net::lookup_host((host.trim(), port))
        .await
        .map_err(|e| format!("DNS: {e}"))?
        .next()
        .ok_or_else(|| format!("DNS: {host} has no address"))
}

/// The first failed assertion among those the protocol does not handle
/// itself.
fn common(a: &Assertion, duration_ms: f64) -> Option<String> {
    match a.typ.as_str() {
        "responseTime" => check_number(a, "response time (ms)", duration_ms.round()),
        other => Some(format!("unknown assertion {other}")),
    }
}

pub async fn run(
    subtype: &str,
    r: &Request,
    assertions: &[Assertion],
    opts: &TestOptions,
) -> Observed {
    match subtype {
        "" | "http" => http(r, assertions, opts).await,
        "ssl" => ssl(r, assertions, opts).await,
        "tcp" => tcp(r, assertions).await,
        "dns" => dns_test(r, assertions).await,
        "udp" => udp(r, assertions).await,
        "websocket" => websocket(r, assertions).await,
        "grpc" => grpc(r, assertions).await,
        "icmp" => icmp(r, assertions).await,
        other => failed(Instant::now(), format!("unsupported subtype {other}")),
    }
}

// --- HTTP ------------------------------------------------------------------------------

fn client(follow: bool) -> &'static reqwest::Client {
    static FOLLOW: LazyLock<reqwest::Client> = LazyLock::new(|| build(true));
    static STAY: LazyLock<reqwest::Client> = LazyLock::new(|| build(false));
    fn build(follow: bool) -> reqwest::Client {
        reqwest::Client::builder()
            .user_agent("robine-dog-synthetics")
            .redirect(if follow {
                reqwest::redirect::Policy::limited(10)
            } else {
                reqwest::redirect::Policy::none()
            })
            .build()
            .expect("http client")
    }
    if follow { &FOLLOW } else { &STAY }
}

pub async fn http(r: &Request, assertions: &[Assertion], opts: &TestOptions) -> Observed {
    let method = reqwest::Method::from_bytes(
        if r.method.is_empty() {
            "GET"
        } else {
            &r.method
        }
        .to_uppercase()
        .as_bytes(),
    )
    .unwrap_or(reqwest::Method::GET);
    let mut req = client(opts.follow_redirects.unwrap_or(true))
        .request(method, &r.url)
        .timeout(timeout_of(r));
    for (k, v) in &r.headers {
        req = req.header(k, as_text(v));
    }
    if let Some(b) = r.body.as_ref().filter(|b| !b.is_empty()) {
        req = req.body(b.clone());
    }
    let started = Instant::now();
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) if e.is_timeout() => return failed(started, "TIMEOUT: no response in time"),
        Err(e) => return failed(started, format!("CONNECTION: {}", root_cause(&e))),
    };
    let status = resp.status().as_u16();
    let headers: HashMap<String, String> = resp
        .headers()
        .iter()
        .filter_map(|(k, v)| Some((k.as_str().to_lowercase(), v.to_str().ok()?.to_string())))
        .collect();
    let body = match resp.bytes().await {
        Ok(b) => String::from_utf8_lossy(&b[..b.len().min(MAX_BODY)]).into_owned(),
        Err(e) => {
            let mut o = failed(started, format!("BODY: {}", root_cause(&e)));
            o.status_code = Some(status);
            return o;
        }
    };
    let duration_ms = elapsed_ms(started);
    let failure = http_failure(
        assertions,
        &HttpObserved {
            status,
            duration_ms,
            headers: &headers,
            body: &body,
        },
    );
    Observed {
        failure,
        duration_ms,
        status_code: Some(status),
        details: json!({"url": r.url, "status_code": status, "body_size": body.len()}),
        http: Some(HttpResponse {
            status,
            headers,
            body,
        }),
    }
}

// --- SSL -------------------------------------------------------------------------------

/// Verifies the chain as usual but lets the handshake go on, so that an
/// invalid certificate can still be described (and accepted when the test
/// accepts self-signed ones).
#[derive(Debug)]
struct Recording {
    inner: Arc<rustls::client::WebPkiServerVerifier>,
    error: Arc<Mutex<Option<String>>>,
}

impl rustls::client::danger::ServerCertVerifier for Recording {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        intermediates: &[rustls::pki_types::CertificateDer<'_>],
        server_name: &rustls::pki_types::ServerName<'_>,
        ocsp: &[u8],
        now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        if let Err(e) =
            self.inner
                .verify_server_cert(end_entity, intermediates, server_name, ocsp, now)
            && let Ok(mut slot) = self.error.lock()
        {
            *slot = Some(e.to_string());
        }
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// `tls1.2`, `TLSv1.3`, `1.2` → 1.2.
fn tls_number(s: &str) -> Option<f64> {
    let digits: String = s
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    digits.parse().ok()
}

pub async fn ssl(r: &Request, assertions: &[Assertion], opts: &TestOptions) -> Observed {
    let started = Instant::now();
    let port = port_of(r, 443);
    let addr = match resolve(&r.host, port).await {
        Ok(a) => a,
        Err(e) => return failed(started, e),
    };
    let limit = timeout_of(r);
    let tcp = match tokio::time::timeout(limit, tokio::net::TcpStream::connect(addr)).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return failed(started, format!("CONNECTION: {e}")),
        Err(_) => return failed(started, "TIMEOUT: no connection in time"),
    };
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let roots = Arc::new(rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    });
    let inner =
        match rustls::client::WebPkiServerVerifier::builder_with_provider(roots, provider.clone())
            .build()
        {
            Ok(v) => v,
            Err(e) => return failed(started, format!("TLS: {e}")),
        };
    let error = Arc::new(Mutex::new(None));
    let config = match rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
    {
        Ok(b) => b
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(Recording {
                inner,
                error: error.clone(),
            }))
            .with_no_client_auth(),
        Err(e) => return failed(started, format!("TLS: {e}")),
    };
    let sni = r.servername.clone().unwrap_or_else(|| r.host.clone());
    let name = match rustls::pki_types::ServerName::try_from(sni.trim().to_string()) {
        Ok(n) => n,
        Err(e) => return failed(started, format!("TLS: bad server name: {e}")),
    };
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let tls = match tokio::time::timeout(limit, connector.connect(name, tcp)).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return failed(started, format!("TLS: {}", root_cause(&e))),
        Err(_) => return failed(started, "TIMEOUT: no TLS handshake in time"),
    };
    let duration_ms = elapsed_ms(started);
    let (_, conn) = tls.get_ref();
    let version = match conn.protocol_version() {
        Some(rustls::ProtocolVersion::TLSv1_3) => "tls1.3",
        Some(rustls::ProtocolVersion::TLSv1_2) => "tls1.2",
        _ => "unknown",
    };
    let chain = conn.peer_certificates().unwrap_or_default();
    let cert = chain.first().map(|c| x509::parse(c.as_ref()));
    let now = chrono::Utc::now().timestamp_millis();
    let invalid = error.lock().ok().and_then(|e| e.clone());
    let mut details = json!({
        "tls_version": version,
        "cipher": conn.negotiated_cipher_suite().map(|c| format!("{:?}", c.suite())),
        "chain_length": chain.len(),
        "valid": invalid.is_none(),
        "error": invalid,
    });
    let cert = match cert {
        Some(Ok(c)) => c,
        Some(Err(e)) => return failed(started, format!("certificate: {e}")),
        None => return failed(started, "no certificate"),
    };
    let days_left = (cert.not_after - now) as f64 / 86_400_000.0;
    details["certificate"] = json!({
        "subject": cert.subject,
        "issuer": cert.issuer,
        "valid_from": cert.not_before,
        "valid_to": cert.not_after,
        "days_left": days_left.floor(),
        "dns_names": cert.dns_names,
    });
    let mut failure = invalid
        .filter(|_| !opts.accept_self_signed.unwrap_or(false))
        .map(|e| format!("certificate: {e}"));
    if failure.is_none() {
        failure = assertions.iter().find_map(|a| match a.typ.as_str() {
            "certificate" => {
                let Some(days) = a
                    .target
                    .as_f64()
                    .or_else(|| as_text(&a.target).parse().ok())
                else {
                    return Some("certificate: the target is not a number of days".into());
                };
                let ok = match a.operator.as_str() {
                    "isInMoreThan" => days_left > days,
                    "isInLessThan" => days_left < days,
                    other => return Some(format!("certificate: unknown operator {other}")),
                };
                (!ok).then(|| {
                    format!(
                        "certificate expires in {:.0} days, should {} {days} days",
                        days_left.floor(),
                        op_words(&a.operator)
                    )
                })
            }
            "tlsVersion" => {
                let (Some(actual), Some(target)) =
                    (tls_number(version), tls_number(&as_text(&a.target)))
                else {
                    return Some(format!("TLS version {version} cannot be compared"));
                };
                match number_holds(&a.operator, actual, target) {
                    Ok(true) => None,
                    Ok(false) => Some(format!(
                        "TLS version is {version}, should {} {}",
                        op_words(&a.operator),
                        as_text(&a.target)
                    )),
                    Err(e) => Some(format!("TLS version: {e}")),
                }
            }
            "property" => {
                let field = a.property.clone().unwrap_or_default();
                let actual = details["certificate"][&field]
                    .as_str()
                    .map(String::from)
                    .or_else(|| details[&field].as_str().map(String::from));
                check_text(a, &format!("certificate {field}"), actual.as_deref())
            }
            _ => common(a, duration_ms),
        });
    }
    Observed {
        failure,
        duration_ms,
        details,
        ..Default::default()
    }
}

// --- TCP -------------------------------------------------------------------------------

pub async fn tcp(r: &Request, assertions: &[Assertion]) -> Observed {
    let started = Instant::now();
    let addr = match resolve(&r.host, port_of(r, 80)).await {
        Ok(a) => a,
        Err(e) => return failed(started, e),
    };
    let state =
        match tokio::time::timeout(timeout_of(r), tokio::net::TcpStream::connect(addr)).await {
            Ok(Ok(_)) => "established",
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => "refused",
            Ok(Err(_)) => "refused",
            Err(_) => "timeout",
        };
    let duration_ms = elapsed_ms(started);
    let checks_connection = assertions.iter().any(|a| a.typ == "connection");
    let failure = if !checks_connection && state != "established" {
        Some(format!("CONNECTION: {state}"))
    } else {
        assertions.iter().find_map(|a| match a.typ.as_str() {
            "connection" => check_text(a, "connection", Some(state)),
            _ => common(a, duration_ms),
        })
    };
    Observed {
        failure,
        duration_ms,
        details: json!({"connection": state, "address": addr.to_string()}),
        ..Default::default()
    }
}

// --- DNS -------------------------------------------------------------------------------

pub async fn dns_test(r: &Request, assertions: &[Assertion]) -> Observed {
    let started = Instant::now();
    let Some(server) = r
        .dns_server
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(dns::system_server)
    else {
        return failed(started, "no DNS server: set one in the test");
    };
    let port = match &r.dns_server_port {
        Some(Value::Number(n)) => n.as_u64().and_then(|p| u16::try_from(p).ok()),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    }
    .unwrap_or(53);
    let addr = match resolve(&server, port).await {
        Ok(a) => a,
        Err(e) => return failed(started, e),
    };
    let limit = timeout_of(r);
    let answers = futures::future::join_all(
        dns::TYPES
            .iter()
            .map(|(_, t)| dns::ask(addr, &r.host, *t, limit)),
    )
    .await;
    let duration_ms = elapsed_ms(started);
    let mut records: HashMap<String, Vec<String>> = HashMap::new();
    let mut rcodes = Vec::new();
    let mut errors = Vec::new();
    for a in answers {
        match a {
            Ok(a) => {
                rcodes.push(a.rcode);
                for (t, v) in a.records {
                    let list = records.entry(t).or_default();
                    if !list.contains(&v) {
                        list.push(v);
                    }
                }
            }
            Err(e) => errors.push(e),
        }
    }
    let details = json!({"server": addr.to_string(), "records": records});
    if rcodes.is_empty() {
        let mut o = failed(
            started,
            format!("DNS: {}", errors.first().cloned().unwrap_or_default()),
        );
        o.details = details;
        return o;
    }
    let failure = if rcodes.iter().all(|c| *c != 0) {
        Some(match rcodes[0] {
            2 => "DNS: SERVFAIL".to_string(),
            3 => format!("DNS: {} does not exist (NXDOMAIN)", r.host),
            5 => "DNS: REFUSED".to_string(),
            c => format!("DNS: error {c}"),
        })
    } else {
        assertions.iter().find_map(|a| match a.typ.as_str() {
            "recordSome" | "recordEvery" => {
                let typ = a.property.clone().unwrap_or_else(|| "A".into());
                let target = as_text(&a.target);
                let values = records.get(&typ).cloned().unwrap_or_default();
                if values.is_empty() {
                    return Some(format!("no {typ} record"));
                }
                let holds = |v: &String| text_holds(&a.operator, v, &target);
                let ok = if a.typ == "recordSome" {
                    values
                        .iter()
                        .map(holds)
                        .collect::<Result<Vec<_>, _>>()
                        .map(|v| v.into_iter().any(|x| x))
                } else {
                    values
                        .iter()
                        .map(holds)
                        .collect::<Result<Vec<_>, _>>()
                        .map(|v| v.into_iter().all(|x| x))
                };
                match ok {
                    Ok(true) => None,
                    Ok(false) => Some(format!(
                        "{} {typ} record should {} {target:?} (got {})",
                        if a.typ == "recordSome" {
                            "some"
                        } else {
                            "every"
                        },
                        op_words(&a.operator),
                        values.join(", ")
                    )),
                    Err(e) => Some(format!("{typ} records: {e}")),
                }
            }
            _ => common(a, duration_ms),
        })
    };
    Observed {
        failure,
        duration_ms,
        details,
        ..Default::default()
    }
}

// --- UDP -------------------------------------------------------------------------------

pub async fn udp(r: &Request, assertions: &[Assertion]) -> Observed {
    let started = Instant::now();
    let addr = match resolve(&r.host, port_of(r, 53)).await {
        Ok(a) => a,
        Err(e) => return failed(started, e),
    };
    let exchange = async {
        let bind: SocketAddr = if addr.is_ipv4() {
            "0.0.0.0:0".parse().unwrap()
        } else {
            "[::]:0".parse().unwrap()
        };
        let sock = tokio::net::UdpSocket::bind(bind)
            .await
            .map_err(|e| e.to_string())?;
        sock.connect(addr).await.map_err(|e| e.to_string())?;
        sock.send(r.message.as_deref().unwrap_or("").as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 65_536];
        let n = sock.recv(&mut buf).await.map_err(|e| e.to_string())?;
        Ok::<_, String>(String::from_utf8_lossy(&buf[..n]).into_owned())
    };
    let received = match tokio::time::timeout(timeout_of(r), exchange).await {
        Ok(Ok(m)) => m,
        Ok(Err(e)) => return failed(started, format!("CONNECTION: {e}")),
        Err(_) => return failed(started, "TIMEOUT: no response in time"),
    };
    let duration_ms = elapsed_ms(started);
    let failure = assertions.iter().find_map(|a| match a.typ.as_str() {
        "receivedMessage" => check_text(a, "received message", Some(&received)),
        _ => common(a, duration_ms),
    });
    Observed {
        failure,
        duration_ms,
        details: json!({"received_message": received}),
        ..Default::default()
    }
}

// --- WebSocket -------------------------------------------------------------------------

pub async fn websocket(r: &Request, assertions: &[Assertion]) -> Observed {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
    let started = Instant::now();
    let mut req = match r.url.as_str().into_client_request() {
        Ok(q) => q,
        Err(e) => return failed(started, format!("bad URL: {e}")),
    };
    for (k, v) in &r.headers {
        if let (Ok(name), Ok(value)) = (
            k.parse::<tokio_tungstenite::tungstenite::http::HeaderName>(),
            as_text(v).parse(),
        ) {
            req.headers_mut().insert(name, value);
        }
    }
    let limit = timeout_of(r);
    let exchange = async {
        let (mut ws, resp) = tokio_tungstenite::connect_async(req)
            .await
            .map_err(|e| format!("CONNECTION: {}", root_cause(&e)))?;
        let headers: HashMap<String, String> = resp
            .headers()
            .iter()
            .filter_map(|(k, v)| Some((k.as_str().to_lowercase(), v.to_str().ok()?.to_string())))
            .collect();
        if let Some(m) = r.message.as_deref().filter(|m| !m.is_empty()) {
            ws.send(Message::Text(m.into()))
                .await
                .map_err(|e| format!("SEND: {e}"))?;
        }
        // Some servers greet before they answer: the messages are read
        // until one passes the message assertions, or none comes for a
        // second; the last one counts.
        let wanted = |m: &str| {
            assertions
                .iter()
                .filter(|a| a.typ == "receivedMessage")
                .all(|a| check_text(a, "", Some(m)).is_none())
        };
        let mut received: Option<String> = None;
        loop {
            let next = match &received {
                None => ws.next().await,
                Some(_) => match tokio::time::timeout(Duration::from_secs(1), ws.next()).await {
                    Ok(n) => n,
                    Err(_) => break,
                },
            };
            let m = match next {
                Some(Ok(Message::Text(t))) => t.to_string(),
                Some(Ok(Message::Binary(b))) => String::from_utf8_lossy(&b).into_owned(),
                Some(Ok(Message::Close(_))) | None if received.is_some() => break,
                Some(Ok(Message::Close(_))) | None => {
                    return Err("closed before a message".into());
                }
                Some(Ok(_)) => continue,
                Some(Err(e)) => return Err(format!("RECEIVE: {e}")),
            };
            let done = wanted(&m);
            received = Some(m);
            if done {
                break;
            }
        }
        let received = received.unwrap_or_default();
        let _ = ws.close(None).await;
        Ok::<_, String>((headers, received))
    };
    let (headers, received) = match tokio::time::timeout(limit, exchange).await {
        Ok(Ok(x)) => x,
        Ok(Err(e)) => return failed(started, e),
        Err(_) => return failed(started, "TIMEOUT: no message in time"),
    };
    let duration_ms = elapsed_ms(started);
    let failure = assertions.iter().find_map(|a| match a.typ.as_str() {
        "receivedMessage" => check_text(a, "received message", Some(&received)),
        "header" => {
            let name = a.property.clone().unwrap_or_default().to_lowercase();
            check_text(
                a,
                &format!("header {name}"),
                headers.get(&name).map(String::as_str),
            )
        }
        _ => common(a, duration_ms),
    });
    Observed {
        failure,
        duration_ms,
        details: json!({"received_message": received}),
        ..Default::default()
    }
}

// --- gRPC ------------------------------------------------------------------------------

/// `grpc.health.v1.HealthCheckResponse.ServingStatus`.
const SERVING: &[&str] = &["UNKNOWN", "SERVING", "NOT_SERVING", "SERVICE_UNKNOWN"];

/// The health check's request: `{service}` in protobuf, framed for gRPC.
pub fn health_request(service: &str) -> Vec<u8> {
    let mut msg = Vec::new();
    if !service.is_empty() {
        msg.push(0x0a);
        let mut len = service.len();
        loop {
            let b = (len & 0x7f) as u8;
            len >>= 7;
            if len == 0 {
                msg.push(b);
                break;
            }
            msg.push(b | 0x80);
        }
        msg.extend_from_slice(service.as_bytes());
    }
    let mut framed = vec![0];
    framed.extend_from_slice(&(msg.len() as u32).to_be_bytes());
    framed.extend_from_slice(&msg);
    framed
}

/// The status of a framed `HealthCheckResponse`.
pub fn health_status(body: &[u8]) -> Result<u64, String> {
    let msg = body.get(5..).ok_or("empty gRPC response")?;
    let mut i = 0;
    while i < msg.len() {
        let key = msg[i];
        i += 1;
        let mut v: u64 = 0;
        let mut shift = 0;
        loop {
            let b = *msg.get(i).ok_or("truncated gRPC response")?;
            i += 1;
            v |= ((b & 0x7f) as u64) << shift;
            shift += 7;
            if b & 0x80 == 0 {
                break;
            }
        }
        if key == 0x08 {
            return Ok(v);
        }
        if key & 7 != 0 {
            return Err("unexpected gRPC response".into());
        }
    }
    // Proto3 omits defaults: no field is UNKNOWN (0).
    Ok(0)
}

pub async fn grpc(r: &Request, assertions: &[Assertion]) -> Observed {
    static H2C: LazyLock<reqwest::Client> = LazyLock::new(|| {
        reqwest::Client::builder()
            .http2_prior_knowledge()
            .user_agent("robine-dog-synthetics")
            .build()
            .expect("h2c client")
    });
    static H2: LazyLock<reqwest::Client> = LazyLock::new(|| {
        reqwest::Client::builder()
            .user_agent("robine-dog-synthetics")
            .build()
            .expect("h2 client")
    });
    let started = Instant::now();
    if r.call_type.as_deref().is_some_and(|c| c != "healthcheck") {
        return failed(started, "only gRPC health checks are supported");
    }
    let port = port_of(r, 443);
    let tls = r
        .extra
        .get("useTls")
        .and_then(Value::as_bool)
        .unwrap_or(port == 443);
    let url = format!(
        "{}://{}:{port}/grpc.health.v1.Health/Check",
        if tls { "https" } else { "http" },
        r.host.trim()
    );
    let mut req = if tls { &H2 } else { &H2C }
        .post(&url)
        .timeout(timeout_of(r))
        .header("content-type", "application/grpc")
        .header("te", "trailers")
        .body(health_request(r.service.as_deref().unwrap_or("")));
    let metadata = r.extra.get("metadata").and_then(Value::as_object).cloned();
    for (k, v) in metadata.iter().flatten() {
        req = req.header(k, as_text(v));
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) if e.is_timeout() => return failed(started, "TIMEOUT: no response in time"),
        Err(e) => return failed(started, format!("CONNECTION: {}", root_cause(&e))),
    };
    let headers: HashMap<String, String> = resp
        .headers()
        .iter()
        .filter_map(|(k, v)| Some((k.as_str().to_lowercase(), v.to_str().ok()?.to_string())))
        .collect();
    let body = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => return failed(started, format!("BODY: {}", root_cause(&e))),
    };
    let duration_ms = elapsed_ms(started);
    if let Some(code) = headers.get("grpc-status").filter(|c| *c != "0") {
        let mut o = failed(
            started,
            format!(
                "gRPC status {code}: {}",
                headers.get("grpc-message").cloned().unwrap_or_default()
            ),
        );
        o.duration_ms = duration_ms;
        return o;
    }
    let status = match health_status(&body) {
        Ok(s) => s,
        Err(e) => return failed(started, e),
    };
    let name = SERVING.get(status as usize).copied().unwrap_or("UNKNOWN");
    let checks_status = assertions.iter().any(|a| a.typ == "grpcHealthcheckStatus");
    let failure = if !checks_status && status != 1 {
        Some(format!("health status is {name}"))
    } else {
        assertions.iter().find_map(|a| match a.typ.as_str() {
            "grpcHealthcheckStatus" => {
                let target = match &a.target {
                    Value::String(s) => SERVING
                        .iter()
                        .position(|x| x.eq_ignore_ascii_case(s))
                        .map(|p| p as f64)
                        .or_else(|| s.parse().ok()),
                    v => v.as_f64(),
                };
                let Some(target) = target else {
                    return Some("health status: bad target".into());
                };
                match number_holds(&a.operator, status as f64, target) {
                    Ok(true) => None,
                    Ok(false) => Some(format!(
                        "health status is {name}, should {} {}",
                        op_words(&a.operator),
                        SERVING.get(target as usize).copied().unwrap_or("?")
                    )),
                    Err(e) => Some(format!("health status: {e}")),
                }
            }
            "grpcMetadata" => {
                let name = a.property.clone().unwrap_or_default().to_lowercase();
                check_text(
                    a,
                    &format!("metadata {name}"),
                    headers.get(&name).map(String::as_str),
                )
            }
            _ => common(a, duration_ms),
        })
    };
    Observed {
        failure,
        duration_ms,
        details: json!({"health_status": name, "url": url}),
        ..Default::default()
    }
}

// --- ICMP ------------------------------------------------------------------------------

/// Echo requests: the round trips, `None` for each lost packet.
fn ping(addr: IpAddr, count: u32, wait: Duration) -> Result<Vec<Option<f64>>, String> {
    use socket2::{Domain, Protocol, SockAddr, Socket, Type};
    use std::io::Read;
    let v4 = addr.is_ipv4();
    let (domain, proto) = if v4 {
        (Domain::IPV4, Protocol::ICMPV4)
    } else {
        (Domain::IPV6, Protocol::ICMPV6)
    };
    // Unprivileged ping sockets first (net.ipv4.ping_group_range), raw ones
    // with CAP_NET_RAW.
    let (sock, raw) = match Socket::new(domain, Type::DGRAM, Some(proto)) {
        Ok(s) => (s, false),
        Err(_) => match Socket::new(domain, Type::RAW, Some(proto)) {
            Ok(s) => (s, true),
            Err(e) => {
                return Err(format!(
                    "ICMP: robine-dog may not send pings ({e}): give it the NET_RAW \
                     capability, or allow its group in net.ipv4.ping_group_range"
                ));
            }
        },
    };
    sock.connect(&SockAddr::from(SocketAddr::new(addr, 0)))
        .map_err(|e| format!("ICMP: {e}"))?;
    let ident: u16 = rand::random();
    let mut out = Vec::new();
    for seq in 0..count as u16 {
        let mut pkt = vec![if v4 { 8 } else { 128 }, 0, 0, 0];
        pkt.extend_from_slice(&ident.to_be_bytes());
        pkt.extend_from_slice(&seq.to_be_bytes());
        pkt.extend_from_slice(&[0x61; 32]);
        if v4 {
            let sum = checksum(&pkt);
            pkt[2..4].copy_from_slice(&sum.to_be_bytes());
        }
        let sent = Instant::now();
        sock.send(&pkt).map_err(|e| format!("ICMP: {e}"))?;
        let mut rtt = None;
        let mut buf = [0u8; 1500];
        while sent.elapsed() < wait {
            let left = wait
                .saturating_sub(sent.elapsed())
                .max(Duration::from_millis(1));
            sock.set_read_timeout(Some(left)).ok();
            let n = match (&sock).read(&mut buf) {
                Ok(n) => n,
                Err(_) => break,
            };
            let data = if raw && v4 {
                let ihl = ((buf[0] & 0x0f) as usize) * 4;
                &buf[ihl.min(n)..n]
            } else {
                &buf[..n]
            };
            let reply = if v4 { 0 } else { 129 };
            if data.len() >= 8
                && data[0] == reply
                && data[6..8] == seq.to_be_bytes()
                && (!raw || data[4..6] == ident.to_be_bytes())
            {
                rtt = Some(elapsed_ms(sent));
                break;
            }
        }
        out.push(rtt);
        if (seq as u32) + 1 < count {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    Ok(out)
}

fn checksum(b: &[u8]) -> u16 {
    let mut sum: u32 = b
        .chunks(2)
        .map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]) as u32)
        .sum();
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

pub async fn icmp(r: &Request, assertions: &[Assertion]) -> Observed {
    let started = Instant::now();
    let addr = match resolve(&r.host, 0).await {
        Ok(a) => a.ip(),
        Err(e) => return failed(started, e),
    };
    let count = r.number_of_packets.unwrap_or(4).clamp(1, 10);
    let wait = timeout_of(r).min(Duration::from_secs(5));
    let rtts = match tokio::task::spawn_blocking(move || ping(addr, count, wait)).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return failed(started, e),
        Err(e) => return failed(started, format!("ICMP: {e}")),
    };
    let got: Vec<f64> = rtts.iter().flatten().copied().collect();
    let received = got.len() as f64;
    let loss = (1.0 - received / count as f64) * 100.0;
    let (min, max) = got
        .iter()
        .fold((f64::INFINITY, 0.0f64), |(a, b), x| (a.min(*x), b.max(*x)));
    let avg = if got.is_empty() {
        0.0
    } else {
        got.iter().sum::<f64>() / received
    };
    let latency = |p: &str| match p {
        "min" if !got.is_empty() => Some(min),
        "max" if !got.is_empty() => Some(max),
        "avg" if !got.is_empty() => Some(avg),
        _ => None,
    };
    let duration_ms = elapsed_ms(started);
    let checks_packets = assertions
        .iter()
        .any(|a| a.typ == "packetsReceived" || a.typ == "packetLossPercentage");
    let failure = if !checks_packets && got.is_empty() {
        Some(format!("no reply from {addr}"))
    } else {
        assertions.iter().find_map(|a| match a.typ.as_str() {
            "latency" => {
                let p = a.property.clone().unwrap_or_else(|| "avg".into());
                match latency(&p) {
                    Some(v) => {
                        check_number(a, &format!("{p} latency (ms)"), (v * 10.0).round() / 10.0)
                    }
                    None => Some("no reply to measure the latency".into()),
                }
            }
            "packetLossPercentage" => check_number(a, "packet loss (%)", loss.round()),
            "packetsReceived" => check_number(a, "packets received", received),
            _ => common(a, duration_ms),
        })
    };
    Observed {
        failure,
        duration_ms,
        details: json!({
            "address": addr.to_string(),
            "packets_sent": count,
            "packets_received": received,
            "packet_loss_percentage": loss,
            "latency": if got.is_empty() { Value::Null } else { json!({"min": min, "avg": avg, "max": max}) },
        }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grpc_health_frames() {
        assert_eq!(health_request(""), vec![0, 0, 0, 0, 0]);
        assert_eq!(
            health_request("shop"),
            vec![0, 0, 0, 0, 6, 0x0a, 4, b's', b'h', b'o', b'p']
        );
        assert_eq!(health_status(&[0, 0, 0, 0, 2, 0x08, 1]), Ok(1));
        assert_eq!(health_status(&[0, 0, 0, 0, 0]), Ok(0));
        assert!(health_status(&[0, 0]).is_err());
    }

    #[test]
    fn icmp_checksum_and_tls_versions() {
        // The echo request of RFC 1071's example layout: the checksum of a
        // packet carrying its checksum is zero.
        let mut pkt = vec![8, 0, 0, 0, 0x12, 0x34, 0, 1, 0x61, 0x62];
        let sum = checksum(&pkt);
        pkt[2..4].copy_from_slice(&sum.to_be_bytes());
        assert_eq!(checksum(&pkt), 0);
        assert_eq!(tls_number("tls1.2"), Some(1.2));
        assert_eq!(tls_number("TLSv1.3"), Some(1.3));
        assert_eq!(tls_number("x"), None);
    }

    #[tokio::test]
    async fn tcp_and_udp() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let req = |port: u16, msg: Option<&str>| Request {
            host: "127.0.0.1".into(),
            port: Some(json!(port)),
            message: msg.map(String::from),
            timeout: Some(2.0),
            ..Default::default()
        };
        let o = tcp(&req(port, None), &[]).await;
        assert_eq!(o.failure, None);
        assert_eq!(o.details["connection"], "established");
        drop(listener);
        let refused: Assertion = serde_json::from_value(
            json!({"type": "connection", "operator": "is", "target": "refused"}),
        )
        .unwrap();
        assert_eq!(tcp(&req(port, None), &[refused]).await.failure, None);

        let echo = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let uport = echo.local_addr().unwrap().port();
        tokio::spawn(async move {
            let mut buf = [0u8; 64];
            let (n, from) = echo.recv_from(&mut buf).await.unwrap();
            echo.send_to(&buf[..n], from).await.unwrap();
        });
        let a: Assertion = serde_json::from_value(
            json!({"type": "receivedMessage", "operator": "is", "target": "ping"}),
        )
        .unwrap();
        let o = udp(&req(uport, Some("ping")), &[a]).await;
        assert_eq!(o.failure, None);
    }
}
