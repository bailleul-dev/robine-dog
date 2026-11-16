//! robine-dog: a personal, self-hosted Datadog-like log and trace explorer.
//!
//! Two listeners:
//! - the main one (UI, UI API, Robine ID sign-in, logs intake `/api/v2/logs`);
//! - the trace-agent one (default 127.0.0.1:8126) that `ddtrace` libraries report to.

mod api;
mod auth;
mod config;
mod db;
mod intake;
mod query;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post};
use std::sync::Arc;
use tower_http::compression::CompressionLayer;
use tower_http::services::{ServeDir, ServeFile};

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<db::Db>,
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
        .layer(axum::middleware::from_fn_with_state(st, auth::require))
}

fn app(st: AppState) -> Router {
    let index = st.cfg.static_dir.join("index.html");
    let spa = ServeDir::new(&st.cfg.static_dir).fallback(ServeFile::new(index));
    let intake = Router::new()
        .route("/api/v2/logs", post(intake::logs::intake))
        .route("/v1/input", post(intake::logs::intake))
        .route("/v1/input/{key}", post(intake::logs::intake))
        .layer(DefaultBodyLimit::max(16 * 1024 * 1024));
    Router::new()
        .merge(intake)
        .nest("/api/ui", ui_api(st.clone()))
        .route("/auth/login", get(auth::login))
        .route("/auth/callback", get(auth::callback))
        .route("/auth/logout", post(auth::logout).get(auth::logout))
        .route("/health", get(|| async { "ok" }))
        .fallback_service(spa)
        .layer(CompressionLayer::new())
        .with_state(st)
}

fn trace_app(st: AppState) -> Router {
    intake::traces::router()
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
        .with_state(st)
}

async fn retention(st: AppState) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(600));
    loop {
        tick.tick().await;
        let cutoff_ms = intake::logs::now_ms() - st.cfg.retention_days * 86_400_000;
        let res = st
            .db
            .write(move |c| {
                let logs = c.execute("DELETE FROM logs WHERE ts < ?", [cutoff_ms])?;
                let spans =
                    c.execute("DELETE FROM spans WHERE start < ?", [cutoff_ms * 1_000_000])?;
                Ok((logs, spans))
            })
            .await;
        match res {
            Ok((0, 0)) => {}
            Ok((logs, spans)) => tracing::info!(logs, spans, "retention purge"),
            Err(e) => tracing::error!(error = %e, "retention purge failed"),
        }
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
    let st = AppState {
        db: db::Db::open(&cfg.db_path)?,
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

    tokio::spawn(retention(st.clone()));

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
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
        })
        .await?;
    Ok(())
}
