use anyhow::{Result, bail};
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Oidc {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    /// Immutable Robine ID `sub` values allowed in.
    pub subjects: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub listen: SocketAddr,
    pub trace_listen: Option<SocketAddr>,
    /// DogStatsD (UDP), as a local Datadog Agent's port 8125.
    pub dogstatsd_listen: Option<SocketAddr>,
    pub greptime_url: String,
    pub greptime_db: String,
    pub greptime_auth: Option<(String, String)>,
    pub postgres_url: String,
    pub static_dir: PathBuf,
    pub base_url: String,
    /// Logs and spans.
    pub retention_days: i64,
    pub metrics_retention_days: i64,
    pub api_key: Option<String>,
    /// The key of Sentry DSNs: public, as Datadog's client tokens are.
    pub client_token: Option<String>,
    pub trace_require_key: bool,
    pub session_secret: Vec<u8>,
    pub oidc: Option<Oidc>,
    pub auth_disabled: bool,
}

fn var(k: &str) -> Option<String> {
    std::env::var(k)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let listen: SocketAddr = var("ROBINE_DOG_LISTEN")
            .unwrap_or_else(|| "127.0.0.1:8080".into())
            .parse()?;
        let trace_listen = match var("ROBINE_DOG_TRACE_LISTEN").as_deref() {
            Some("off") => None,
            Some(v) => Some(v.parse()?),
            None => Some("127.0.0.1:8126".parse()?),
        };
        let dogstatsd_listen = match var("ROBINE_DOG_DOGSTATSD_LISTEN").as_deref() {
            Some("off") => None,
            Some(v) => Some(v.parse()?),
            None => Some("127.0.0.1:8125".parse()?),
        };
        let auth_disabled = var("ROBINE_DOG_AUTH").as_deref() == Some("disabled");
        if auth_disabled && !listen.ip().is_loopback() {
            bail!(
                "ROBINE_DOG_AUTH=disabled is only allowed when ROBINE_DOG_LISTEN is a loopback address"
            );
        }
        let oidc = match (
            var("ROBINE_DOG_OIDC_ISSUER"),
            var("ROBINE_DOG_OIDC_CLIENT_ID"),
            var("ROBINE_DOG_OIDC_CLIENT_SECRET"),
        ) {
            (Some(issuer), Some(client_id), Some(client_secret)) => Some(Oidc {
                issuer: issuer.trim_end_matches('/').to_string(),
                client_id,
                client_secret,
                subjects: var("ROBINE_DOG_OIDC_SUBJECTS")
                    .unwrap_or_default()
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
            }),
            _ => None,
        };
        if oidc.is_none() && !auth_disabled {
            bail!(
                "Robine ID is not configured: set ROBINE_DOG_OIDC_ISSUER, ROBINE_DOG_OIDC_CLIENT_ID and \
                 ROBINE_DOG_OIDC_CLIENT_SECRET (or ROBINE_DOG_AUTH=disabled for a loopback-only dev server)"
            );
        }
        let session_secret = match var("ROBINE_DOG_SESSION_SECRET") {
            Some(s) if s.len() >= 32 => s.into_bytes(),
            Some(_) => bail!("ROBINE_DOG_SESSION_SECRET must be at least 32 characters"),
            None if auth_disabled => b"robine-dog-local-development-only-secret".to_vec(),
            None => bail!("ROBINE_DOG_SESSION_SECRET is required"),
        };
        Ok(Self {
            listen,
            trace_listen,
            dogstatsd_listen,
            greptime_url: var("ROBINE_DOG_GREPTIME_URL")
                .unwrap_or_else(|| "http://127.0.0.1:14000".into()),
            greptime_db: var("ROBINE_DOG_GREPTIME_DB").unwrap_or_else(|| "robine_dog".into()),
            greptime_auth: var("ROBINE_DOG_GREPTIME_USER")
                .map(|u| (u, var("ROBINE_DOG_GREPTIME_PASSWORD").unwrap_or_default())),
            postgres_url: var("ROBINE_DOG_POSTGRES_URL").unwrap_or_else(|| {
                "postgres://robine_dog:robine_dog@127.0.0.1:15432/robine_dog".into()
            }),
            static_dir: var("ROBINE_DOG_STATIC_DIR")
                .unwrap_or_else(|| "web/dist".into())
                .into(),
            base_url: var("ROBINE_DOG_BASE_URL")
                .unwrap_or_else(|| format!("http://{listen}"))
                .trim_end_matches('/')
                .to_string(),
            retention_days: var("ROBINE_DOG_RETENTION_DAYS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(15),
            metrics_retention_days: var("ROBINE_DOG_METRICS_RETENTION_DAYS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(455),
            api_key: var("ROBINE_DOG_API_KEY"),
            client_token: var("ROBINE_DOG_CLIENT_TOKEN"),
            trace_require_key: var("ROBINE_DOG_TRACE_REQUIRE_KEY")
                .is_some_and(|v| v == "1" || v == "true"),
            session_secret,
            oidc,
            auth_disabled,
        })
    }
}
