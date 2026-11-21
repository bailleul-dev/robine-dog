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
mod checks;
mod config;
mod events;
mod hosts;
mod intake;
mod mcp;
mod metrics;
mod monitors;
mod pipelines;
mod query;
mod rum;
mod store;
mod synthetics;
mod watchdog;

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
    pub pipelines: Arc<pipelines::Pipelines>,
    pub generated: Arc<metrics::generated::Generated>,
    pub hosts: Arc<hosts::Hosts>,
    pub cfg: Arc<config::Config>,
    pub auth: Arc<auth::Auth>,
}

fn ui_api(st: AppState) -> Router<AppState> {
    ui_routes().layer(axum::middleware::from_fn_with_state(st, auth::require))
}

/// The API of the UI, before authentication: also what the MCP tools call.
pub(crate) fn ui_routes() -> Router<AppState> {
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
        .route("/apm/map", get(api::apm::map))
        .route(
            "/monitors",
            get(api::monitors::list).post(api::monitors::create),
        )
        .route(
            "/synthetics/tests",
            get(synthetics::list).post(synthetics::create),
        )
        .route(
            "/synthetics/tests/{id}",
            get(synthetics::get)
                .put(synthetics::update)
                .delete(synthetics::delete),
        )
        .route("/synthetics/tests/{id}/trigger", post(synthetics::trigger))
        .route("/slos", get(api::slos::list).post(api::slos::create))
        .route(
            "/slos/{id}",
            axum::routing::put(api::slos::update).delete(api::slos::delete),
        )
        .route(
            "/downtimes",
            get(api::downtimes::list).post(api::downtimes::create),
        )
        .route(
            "/downtimes/{id}",
            axum::routing::put(api::downtimes::update).delete(api::downtimes::cancel),
        )
        .route("/monitors/validate", post(api::monitors::validate))
        .route("/monitors/preview", post(api::monitors::preview))
        .route("/monitors/events", get(api::monitors::all_events))
        .route(
            "/monitors/{id}",
            get(api::monitors::get)
                .put(api::monitors::update)
                .delete(api::monitors::delete),
        )
        .route("/monitors/{id}/mute", post(api::monitors::mute))
        .route("/monitors/{id}/unmute", post(api::monitors::unmute))
        .route("/monitors/{id}/events", get(api::monitors::events))
        .route("/monitors/{id}/history", get(api::monitors::history))
        .route(
            "/webhooks",
            get(api::monitors::webhooks).post(api::monitors::create_webhook),
        )
        .route(
            "/webhooks/{name}",
            get(api::monitors::webhook)
                .put(api::monitors::update_webhook)
                .delete(api::monitors::delete_webhook),
        )
        .route("/webhooks/{name}/test", post(api::monitors::test_webhook))
        .route("/watchdog/stories", get(watchdog::list).post(watchdog::run))
        .route("/rum-config", get(api::notebooks::rum_config))
        .route(
            "/incidents",
            get(api::incidents::list).post(api::incidents::create),
        )
        .route(
            "/incidents/{id}",
            get(api::incidents::get)
                .put(api::incidents::update)
                .delete(api::incidents::delete),
        )
        .route("/incidents/{id}/timeline", post(api::incidents::note))
        .route(
            "/status-page",
            get(api::incidents::status_page).put(api::incidents::put_status_page),
        )
        .route(
            "/notebooks",
            get(api::notebooks::list).post(api::notebooks::create),
        )
        .route(
            "/notebooks/{id}",
            get(api::notebooks::get)
                .put(api::notebooks::update)
                .delete(api::notebooks::delete),
        )
        .route("/rum/search", post(rum::search))
        .route("/rum/histogram", post(rum::histogram))
        .route("/rum/performance", post(rum::performance))
        .route("/rum/sessions", post(rum::sessions))
        .route("/rum/aggregate", post(api::analytics::rum))
        .route("/events/search", post(api::events::search))
        .route("/events/histogram", post(api::events::histogram))
        .route("/events", post(api::events::create))
        .route("/logs/aggregate", post(api::analytics::logs))
        .route("/logs/patterns", post(api::patterns::patterns))
        .route("/spans/aggregate", post(api::analytics::spans))
        .route("/views", get(api::views::list).post(api::views::create))
        .route(
            "/views/{id}",
            axum::routing::put(api::views::update).delete(api::views::delete),
        )
        .route(
            "/logs/config/metrics",
            get(api::generated_metrics::logs_list).post(api::generated_metrics::logs_create),
        )
        .route(
            "/logs/config/metrics/{id}",
            get(api::generated_metrics::logs_get)
                .patch(api::generated_metrics::logs_update)
                .delete(api::generated_metrics::logs_delete),
        )
        .route(
            "/apm/config/metrics",
            get(api::generated_metrics::spans_list).post(api::generated_metrics::spans_create),
        )
        .route(
            "/apm/config/metrics/{id}",
            get(api::generated_metrics::spans_get)
                .patch(api::generated_metrics::spans_update)
                .delete(api::generated_metrics::spans_delete),
        )
        .route("/errors/issues", post(api::errors::issues))
        .route(
            "/errors/issues/{id}",
            get(api::errors::issue).put(api::errors::set_status),
        )
        .route("/query/timeseries", post(api::query::timeseries))
        .route("/query/scalar", post(api::query::scalar))
        .route(
            "/dashboards",
            get(api::dashboards::list).post(api::dashboards::create),
        )
        .route(
            "/dashboards/{id}",
            get(api::dashboards::get)
                .put(api::dashboards::update)
                .delete(api::dashboards::delete),
        )
        .route("/dashboards/{id}/clone", post(api::dashboards::clone))
        .route(
            "/pipelines",
            get(api::pipelines::list).post(api::pipelines::create),
        )
        .route(
            "/pipelines/order",
            get(api::pipelines::order).put(api::pipelines::set_order),
        )
        .route("/pipelines/grok-test", post(api::pipelines::grok_test))
        .route(
            "/pipelines/{id}",
            get(api::pipelines::get)
                .put(api::pipelines::update)
                .delete(api::pipelines::delete),
        )
        .route("/pipelines/{id}/clone", post(api::pipelines::clone))
        .route("/metrics", get(api::metrics::list))
        .route("/hosts", get(api::metrics::hosts))
        .route("/checks", get(checks::list))
        .route("/metrics-volume", get(api::metrics::volume))
        .route("/metrics/query", post(api::metrics::query))
        .route(
            "/metrics/{name}",
            get(api::metrics::detail).patch(api::metrics::update),
        )
}

/// Datadog's public API, for its clients and Terraform: `DD-API-KEY` auth.
fn public_api(st: AppState) -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/logs/config/pipelines",
            get(api::pipelines::list).post(api::pipelines::create),
        )
        .route(
            "/api/v1/logs/config/pipelines/{id}",
            get(api::pipelines::get)
                .put(api::pipelines::update)
                .delete(api::pipelines::delete),
        )
        .route(
            "/api/v1/monitor",
            get(api::monitors::list).post(api::monitors::create),
        )
        .route(
            "/api/v1/synthetics/tests",
            get(synthetics::list).post(synthetics::create),
        )
        .route("/api/v1/synthetics/tests/api", post(synthetics::create))
        .route(
            "/api/v1/synthetics/tests/api/{id}",
            get(synthetics::get).put(synthetics::update),
        )
        .route(
            "/api/v1/synthetics/tests/{id}",
            get(synthetics::get).delete(synthetics::delete),
        )
        .route("/api/v1/slo", get(api::slos::list).post(api::slos::create))
        .route(
            "/api/v1/slo/{id}",
            axum::routing::put(api::slos::update).delete(api::slos::delete),
        )
        .route(
            "/api/v1/downtime",
            get(api::downtimes::list).post(api::downtimes::create),
        )
        .route(
            "/api/v1/downtime/{id}",
            axum::routing::put(api::downtimes::update).delete(api::downtimes::cancel),
        )
        .route("/api/v1/monitor/validate", post(api::monitors::validate))
        .route("/api/v1/monitor/search", get(api::datadog::monitor_search))
        .route(
            "/api/v1/monitor/{id}",
            get(api::monitors::get)
                .put(api::monitors::update)
                .delete(api::monitors::delete),
        )
        .route("/api/v1/monitor/{id}/mute", post(api::monitors::mute))
        .route("/api/v1/monitor/{id}/unmute", post(api::monitors::unmute))
        .route(
            "/api/v1/integration/webhooks/configuration/webhooks",
            post(api::monitors::create_webhook),
        )
        .route(
            "/api/v1/integration/webhooks/configuration/webhooks/{name}",
            get(api::monitors::webhook)
                .put(api::monitors::update_webhook)
                .delete(api::monitors::delete_webhook),
        )
        .route(
            "/api/v2/logs/config/metrics",
            get(api::generated_metrics::logs_list).post(api::generated_metrics::logs_create),
        )
        .route(
            "/api/v2/logs/config/metrics/{id}",
            get(api::generated_metrics::logs_get)
                .patch(api::generated_metrics::logs_update)
                .delete(api::generated_metrics::logs_delete),
        )
        .route(
            "/api/v2/apm/config/metrics",
            get(api::generated_metrics::spans_list).post(api::generated_metrics::spans_create),
        )
        .route(
            "/api/v2/apm/config/metrics/{id}",
            get(api::generated_metrics::spans_get)
                .patch(api::generated_metrics::spans_update)
                .delete(api::generated_metrics::spans_delete),
        )
        .route("/api/v2/query/timeseries", post(api::query::timeseries))
        .route("/api/v2/query/scalar", post(api::query::scalar))
        .route(
            "/api/v1/dashboard",
            get(api::dashboards::list).post(api::dashboards::create),
        )
        .route(
            "/api/v1/dashboard/{id}",
            get(api::dashboards::get)
                .put(api::dashboards::update)
                .delete(api::dashboards::delete),
        )
        .route(
            "/api/v2/logs/analytics/aggregate",
            post(api::analytics::logs),
        )
        .route(
            "/api/v2/spans/analytics/aggregate",
            post(api::analytics::spans_public),
        )
        .route(
            "/api/v2/logs/events/search",
            post(api::datadog::logs_search),
        )
        .route("/api/v2/logs/events", get(api::datadog::logs_list))
        .route(
            "/api/v2/spans/events/search",
            post(api::datadog::spans_search),
        )
        .route("/api/v2/spans/events", get(api::datadog::spans_list))
        .route("/api/v2/rum/events/search", post(api::datadog::rum_search))
        .route("/api/v2/rum/events", get(api::datadog::rum_list))
        .route("/api/v2/rum/analytics/aggregate", post(api::analytics::rum))
        .route("/api/v2/events/search", post(api::datadog::events_search))
        .route("/api/v2/events", get(api::datadog::events_list))
        .route("/api/v1/events", get(api::datadog::events_v1))
        .route("/api/v1/query", get(api::datadog::query_v1))
        .route("/api/v1/metrics", get(api::datadog::metrics_v1))
        .route(
            "/api/v1/metrics/{name}",
            get(api::datadog::metric_get).put(api::datadog::metric_put),
        )
        .route("/api/v2/metrics", get(api::datadog::metrics_v2))
        .route("/api/v1/search", get(api::datadog::search_v1))
        .route("/api/v1/hosts", get(api::datadog::hosts))
        .route("/api/v1/hosts/totals", get(api::datadog::hosts_totals))
        .route(
            "/api/v2/incidents",
            get(api::datadog::incidents).post(api::datadog::incident_create),
        )
        .route(
            "/api/v2/incidents/{id}",
            get(api::datadog::incident)
                .patch(api::datadog::incident)
                .delete(api::datadog::incident),
        )
        .route(
            "/api/v1/notebooks",
            get(api::datadog::notebooks).post(api::datadog::notebook_create),
        )
        .route(
            "/api/v1/notebooks/{id}",
            get(api::datadog::notebook)
                .put(api::datadog::notebook)
                .delete(api::datadog::notebook),
        )
        .route(
            "/api/v1/logs/config/pipeline-order",
            get(api::pipelines::order).put(api::pipelines::set_order),
        )
        .layer(axum::middleware::from_fn_with_state(
            st,
            intake::logs::require_api_key,
        ))
}

fn app(st: AppState) -> Router {
    let index = st.cfg.static_dir.join("index.html");
    let spa = ServeDir::new(&st.cfg.static_dir).fallback(ServeFile::new(index));
    let intake = Router::new()
        .route("/api/v2/logs", post(intake::logs::intake))
        .route("/v1/input", post(intake::logs::intake))
        .route("/v1/input/{key}", post(intake::logs::intake))
        .route("/v1/traces", post(intake::otlp::traces_handler))
        .route("/v1/logs", post(intake::otlp::logs_handler))
        .route("/v1/metrics", post(intake::otlp::metrics_handler))
        .route("/api/v1/events", post(api::events::intake))
        .route(
            "/api/v2/rum",
            post(rum::intake).options(intake::sentry::preflight),
        )
        .route(
            "/rum/proxy",
            post(rum::proxy).options(intake::sentry::preflight),
        )
        .route("/api/v1/check_run", post(checks::intake))
        .route("/api/v1/series", post(intake::series::v1))
        .route("/api/v2/series", post(intake::series::v2))
        .route("/api/v1/validate", get(intake::series::validate))
        .route("/api/v2/validate", get(intake::series::validate))
        // Sentry SDKs (Error Tracking).
        // (`/api/{project}/envelope/` and `/store/`, the paths of a DSN
        // without a prefix, are told apart in `agent::acknowledge`.)
        .route(
            "/sentry/{service}/api/{project}/envelope/",
            post(intake::sentry::service_envelope).options(intake::sentry::preflight),
        )
        .route(
            "/sentry/{service}/api/{project}/store/",
            post(intake::sentry::service_store).options(intake::sentry::preflight),
        )
        // A Datadog Agent's forwarders.
        .route("/api/v0.2/traces", post(intake::agent::traces))
        .route("/api/beta/sketches", post(intake::agent::sketches))
        .route("/intake/", post(intake::agent::metadata))
        .route("/api/{*rest}", any(intake::agent::acknowledge))
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024));
    Router::new()
        .merge(public_api(st.clone()))
        .merge(intake)
        .nest("/api/ui", ui_api(st.clone()))
        .route("/auth/login", get(auth::login))
        .route("/auth/callback", get(auth::callback))
        .route("/auth/logout", post(auth::logout).get(auth::logout))
        .route("/health", get(health))
        .route("/status", get(api::incidents::public_page))
        .route("/v1/mcp", post(mcp::post).get(mcp::get))
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
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "robine_dog=info,tower_http=warn".into());
    // JSON, one object per line with `level` and `message` at the top: what
    // a log collector (robine-dog's own intake included) parses unaided.
    if std::env::var("ROBINE_DOG_LOG_FORMAT").as_deref() == Ok("json") {
        tracing_subscriber::fmt()
            .json()
            .flatten_event(true)
            .with_env_filter(filter)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }

    let cfg = Arc::new(config::Config::from_env()?);
    let store = Arc::new(store::Store::open(&cfg).await?);
    tracing::info!(greptime = %cfg.greptime_url, db = %cfg.greptime_db, "storage ready");
    let aggregator = Arc::new(metrics::Aggregator::default());
    aggregator.spawn_flusher(store.clone());
    api::pipelines::ensure_integrations(&store.pg).await?;
    let pipelines = Arc::new(pipelines::Pipelines::default());
    pipelines.load(&api::pipelines::stored(&store.pg).await?);
    let st = AppState {
        store: store.clone(),
        aggregator: aggregator.clone(),
        pipelines,
        generated: Arc::new(metrics::generated::Generated::default()),
        hosts: Arc::new(hosts::Hosts::load(&store.pg).await?),
        cfg: cfg.clone(),
        auth: Arc::new(auth::Auth::new()),
    };
    if let Err(e) = api::dashboards::ensure_default(&st).await {
        tracing::warn!(error = %e, "could not install the default dashboard");
    }
    intake::otlp::init();
    if let Err(e) = api::generated_metrics::reload(&st).await {
        tracing::warn!(error = %e, "could not load the generated metrics");
    }
    monitors::spawn(st.clone());
    synthetics::spawn(st.clone());
    watchdog::spawn(st.clone());
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
        let store_for_sc = st.store.clone();
        tokio::spawn(async move {
            if let Err(e) = metrics::dogstatsd::listen(addr, agg, store_for_sc).await {
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
