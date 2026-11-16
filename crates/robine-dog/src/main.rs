//! robine-dog: a personal, self-hosted Datadog-like log and trace explorer.
//!
//! Three listeners:
//! - the main one (UI, UI API, Robine ID sign-in, logs and metrics intakes);
//! - the trace-agent one (default 127.0.0.1:8126) that `ddtrace` libraries report to;
//! - DogStatsD over UDP (default 127.0.0.1:8125).
//!
//! Telemetry is stored in GreptimeDB, the application's state in PostgreSQL.

mod api;
mod auth;
mod config;
mod intake;
mod metrics;
mod query;
mod store;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{any, delete, get, post};
use std::sync::Arc;
use tower_http::compression::CompressionLayer;
use tower_http::services::{ServeDir, ServeFile};

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<store::Store>,
    pub aggregator: Arc<metrics::Aggregator>,
    pub cfg: Arc<config::Config>,
    pub auth: Arc<auth::Auth>,
}

fn ui_api(st: AppState) -> Router<AppState> {
    Router::new()
        .route("/me", get(auth::me))
        .route("/logs/search", post(api::logs::search))
        .route("/logs/histogram", post(api::logs::histogram))
        .route("/logs/{id}", get(api::logs::get))
        .route("/spans/search", post(api::spans::search))
        .route("/spans/histogram", post(api::spans::histogram))
        .route("/traces/{trace_id}", get(api::spans::trace))
        .route("/facets/values", post(api::facets::values))
        .route("/facets", get(api::facets::list).post(api::facets::create))
        .route("/facets/{id}", delete(api::facets::delete))
        .route("/apm/services", get(api::apm::services))
        .route("/apm/services/{name}", get(api::apm::service))
        .route("/apm/envs", get(api::apm::envs))
        .route("/metrics", get(api::metrics::list))
        .route("/metrics/query", post(api::metrics::query))
        .route(
            "/metrics/{name}",
            get(api::metrics::detail).patch(api::metrics::update),
        )
        .layer(axum::middleware::from_fn_with_state(st, auth::require))
}

fn app(st: AppState) -> Router {
    let index = st.cfg.static_dir.join("index.html");
    let spa = ServeDir::new(&st.cfg.static_dir).fallback(ServeFile::new(index));
    let intake = Router::new()
        .route("/api/v2/logs", post(intake::logs::intake))
        .route("/v1/input", post(intake::logs::intake))
        .route("/v1/input/{key}", post(intake::logs::intake))
        .route("/api/v1/series", post(intake::series::v1))
        .route("/api/v2/series", post(intake::series::v2))
        .route("/api/v1/validate", get(intake::series::validate))
        .route("/api/v2/validate", get(intake::series::validate))
        // A Datadog Agent's forwarders.
        .route("/api/v0.2/traces", post(intake::agent::traces))
        .route("/api/beta/sketches", post(intake::agent::sketches))
        .route("/intake/", post(intake::agent::acknowledge))
        .route("/api/{*rest}", any(intake::agent::acknowledge))
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024));
    Router::new()
        .merge(intake)
        .nest("/api/ui", ui_api(st.clone()))
        .route("/auth/login", get(auth::login))
        .route("/auth/callback", get(auth::callback))
        .route("/auth/logout", post(auth::logout).get(auth::logout))
        .route("/health", get(health))
        .fallback_service(spa)
        .layer(CompressionLayer::new())
        .with_state(st)
}

/// Liveness, and the deployed version (`ROBINE_DOG_VERSION`, set by the image build).
async fn health() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok",
        "version": option_env!("ROBINE_DOG_VERSION").unwrap_or(env!("CARGO_PKG_VERSION")),
    }))
}

fn trace_app(st: AppState) -> Router {
    intake::traces::router()
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
        .with_state(st)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "robine_dog=info,tower_http=warn".into()),
        )
        .init();

    let cfg = Arc::new(config::Config::from_env()?);
    let store = Arc::new(store::Store::open(&cfg).await?);
    tracing::info!(greptime = %cfg.greptime_url, db = %cfg.greptime_db, "storage ready");
    let aggregator = Arc::new(metrics::Aggregator::default());
    aggregator.spawn_flusher(store.clone());
    let st = AppState {
        store: store.clone(),
        aggregator: aggregator.clone(),
        cfg: cfg.clone(),
        auth: Arc::new(auth::Auth::new()),
    };
    if cfg.auth_disabled {
        tracing::warn!(
            "authentication is DISABLED (ROBINE_DOG_AUTH=disabled): loopback development only"
        );
    }
    if cfg.api_key.is_none() {
        tracing::warn!("ROBINE_DOG_API_KEY is not set: the logs intake accepts any API key");
    }

    if let Some(addr) = cfg.dogstatsd_listen {
        let agg = aggregator.clone();
        tokio::spawn(async move {
            if let Err(e) = metrics::dogstatsd::listen(addr, agg).await {
                tracing::error!(error = %e, "DogStatsD listener stopped");
            }
        });
    }

    if let Some(addr) = cfg.trace_listen {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        tracing::info!(%addr, "trace agent API listening");
        let router = trace_app(st.clone());
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, router).await {
                tracing::error!(error = %e, "trace listener stopped");
            }
        });
    }

    let listener = tokio::net::TcpListener::bind(cfg.listen).await?;
    tracing::info!(addr = %cfg.listen, base_url = %cfg.base_url, "robine-dog listening");
    axum::serve(listener, app(st))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    // Nothing buffered is lost on a restart: the aggregator, then the writers.
    aggregator.flush(&store).await;
    store.flush().await;
    tracing::info!("stopped");
    Ok(())
}
