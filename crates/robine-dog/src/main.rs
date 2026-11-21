//! robine-dog: a personal, self-hosted Datadog-like log and trace explorer.
//!
//! Three listeners:
//! - the main one (UI, UI API, Robine ID sign-in, logs and metrics intakes);
//! - the trace-agent one (default 127.0.0.1:8126) that `ddtrace` libraries report to;
//! - DogStatsD over UDP (default 127.0.0.1:8125).
//!
//! Telemetry is stored in GreptimeDB, the application's state in PostgreSQL.

mod api;
mod audit;
mod auth;
mod cases;
mod catalog;
mod checks;
mod civis;
mod config;
mod dbm;
mod dsm;
mod events;
mod hosts;
mod intake;
mod llmobs;
mod logconfig;
mod mcp;
mod metrics;
mod monitors;
mod oncall;
mod pipelines;
mod processes;
mod profiling;
mod query;
mod rum;
mod security;
mod slo;
mod store;
mod synthetics;
mod watchdog;
mod workflows;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{any, delete, get, patch, post, put};
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
    pub logconfig: Arc<logconfig::LogConfig>,
    pub catalog: Arc<catalog::Catalog>,
}

/// APM retention filters, service definitions, teams and scorecards, on
/// Datadog's paths (`v2`: `/api/v2` for the public API, empty for the UI's).
fn catalog_routes(r: Router<AppState>, v2: &str) -> Router<AppState> {
    use catalog::{definitions, retention, scorecards, teams};
    r.route(
        &format!("{v2}/apm/config/retention-filters"),
        get(retention::list).post(retention::create),
    )
    .route(
        &format!("{v2}/apm/config/retention-filters/{{id}}"),
        get(retention::get)
            .put(retention::update)
            .delete(retention::delete),
    )
    .route(
        &format!("{v2}/apm/config/retention-filters-execution-order"),
        axum::routing::put(retention::reorder),
    )
    .route(
        &format!("{v2}/services/definitions"),
        get(definitions::list).post(definitions::upsert),
    )
    .route(
        &format!("{v2}/services/definitions/{{name}}"),
        get(definitions::get).delete(definitions::delete),
    )
    .route(&format!("{v2}/team"), get(teams::list).post(teams::create))
    .route(
        &format!("{v2}/team/{{id}}"),
        get(teams::get).patch(teams::update).delete(teams::delete),
    )
    .route(
        &format!("{v2}/team/{{id}}/memberships"),
        get(teams::members).post(teams::add_member),
    )
    .route(
        &format!("{v2}/team/{{id}}/memberships/{{user}}"),
        delete(teams::remove_member),
    )
    .route(
        &format!("{v2}/team/{{id}}/links"),
        get(teams::links).post(teams::add_link),
    )
    .route(
        &format!("{v2}/team/{{id}}/links/{{link}}"),
        delete(teams::remove_link),
    )
    .route(
        &format!("{v2}/scorecard/rules"),
        get(scorecards::list_rules).post(scorecards::create_rule),
    )
    .route(
        &format!("{v2}/scorecard/rules/{{id}}"),
        axum::routing::put(scorecards::update_rule).delete(scorecards::delete_rule),
    )
    .route(
        &format!("{v2}/scorecard/outcomes"),
        get(scorecards::list_outcomes).post(scorecards::post_outcomes),
    )
    .route(
        &format!("{v2}/scorecard/outcomes/batch"),
        post(scorecards::post_outcomes),
    )
}

/// Indexes, archives, the Sensitive Data Scanner and Reference Tables, on
/// Datadog's paths (`v1`, `v2`: `/api/v1`, `/api/v2` for the public API, empty
/// for the UI's).
fn logconfig_routes(r: Router<AppState>, v1: &str, v2: &str) -> Router<AppState> {
    use logconfig::{archives, indexes, reftables, sds};
    r.route(
        &format!("{v1}/logs/config/indexes"),
        get(indexes::list).post(indexes::create),
    )
    .route(
        &format!("{v1}/logs/config/indexes/{{name}}"),
        get(indexes::get)
            .put(indexes::update)
            .delete(indexes::delete),
    )
    .route(
        &format!("{v1}/logs/config/index-order"),
        get(indexes::get_order).put(indexes::set_order),
    )
    .route(
        &format!("{v2}/logs/config/archives"),
        get(archives::list).post(archives::create),
    )
    .route(
        &format!("{v2}/logs/config/archives/{{id}}"),
        get(archives::get)
            .put(archives::update)
            .delete(archives::delete),
    )
    .route(
        &format!("{v2}/logs/config/archive-order"),
        get(archives::get_order).put(archives::set_order),
    )
    .route(
        &format!("{v2}/sensitive-data-scanner/config"),
        get(sds::config).patch(sds::reorder_groups),
    )
    .route(
        &format!("{v2}/sensitive-data-scanner/config/groups"),
        post(sds::create_group),
    )
    .route(
        &format!("{v2}/sensitive-data-scanner/config/groups/{{id}}"),
        axum::routing::patch(sds::update_group).delete(sds::delete_group),
    )
    .route(
        &format!("{v2}/sensitive-data-scanner/config/rules"),
        post(sds::create_rule),
    )
    .route(
        &format!("{v2}/sensitive-data-scanner/config/rules/{{id}}"),
        axum::routing::patch(sds::update_rule).delete(sds::delete_rule),
    )
    .route(
        &format!("{v2}/sensitive-data-scanner/config/standard-patterns"),
        get(sds::standard_patterns),
    )
    .route(
        &format!("{v2}/reference-tables/tables"),
        get(reftables::list).post(reftables::create),
    )
    .route(
        &format!("{v2}/reference-tables/tables/{{id}}"),
        get(reftables::get)
            .patch(reftables::update)
            .delete(reftables::delete),
    )
    .route(
        &format!("{v2}/reference-tables/tables/{{id}}/rows"),
        get(reftables::rows),
    )
}

fn ui_api(st: AppState) -> Router<AppState> {
    ui_routes()
        .layer(axum::middleware::from_fn_with_state(st.clone(), audit::ui))
        .layer(axum::middleware::from_fn_with_state(st, auth::require))
}

/// The API of the UI, before authentication: also what the MCP tools call.
pub(crate) fn ui_routes() -> Router<AppState> {
    let r = Router::new()
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
        .route("/synthetics/tests/{id}/status", put(synthetics::set_status))
        .route("/synthetics/tests/{id}/results", get(synthetics::results))
        .route("/synthetics/results/{result_id}", get(synthetics::result))
        .route(
            "/synthetics/results/{result_id}/screenshots/{step}",
            get(synthetics::screenshot),
        )
        .route("/synthetics/ci/batches", get(synthetics::ci::batches))
        .route(
            "/synthetics/variables",
            get(synthetics::variables::list).post(synthetics::variables::create),
        )
        .route(
            "/synthetics/variables/{id}",
            get(synthetics::variables::get)
                .put(synthetics::variables::update)
                .delete(synthetics::variables::delete),
        )
        .route("/slos", get(api::slos::list).post(api::slos::create))
        .route("/slos/search", get(api::slos::search))
        .route("/slos/can_delete", get(api::slos::can_delete))
        .route(
            "/slo-corrections",
            get(api::slos::list_corrections).post(api::slos::create_correction),
        )
        .route(
            "/slo-corrections/{id}",
            get(api::slos::get_correction)
                .patch(api::slos::update_correction)
                .delete(api::slos::delete_correction),
        )
        .route(
            "/slos/{id}",
            get(api::slos::get)
                .put(api::slos::update)
                .delete(api::slos::delete),
        )
        .route("/slos/{id}/history", get(api::slos::history))
        .route("/slos/{id}/corrections", get(api::slos::slo_corrections))
        .route("/slos/{id}/status", get(api::slos::status))
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
        .route("/rum/replay/{session_id}", get(rum::replay::get))
        .route("/rum/frustrations", post(rum::analytics::frustrations))
        .route("/rum/retention", post(rum::analytics::retention))
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
        .route(
            "/rum/config/metrics",
            get(api::generated_metrics::rum_list).post(api::generated_metrics::rum_create),
        )
        .route(
            "/rum/config/metrics/{id}",
            get(api::generated_metrics::rum_get)
                .patch(api::generated_metrics::rum_update)
                .delete(api::generated_metrics::rum_delete),
        )
        .route("/errors/issues", post(api::errors::issues))
        .route(
            "/errors/issues/{id}",
            get(api::errors::issue).put(api::errors::set_status),
        )
        .route("/query/timeseries", post(api::query::timeseries))
        .route("/query/distribution", post(api::widgets::distribution))
        .route("/rum/funnel", post(api::widgets::funnel))
        .route("/monitors/search", get(api::datadog::monitor_search))
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
            "/dashboards/{id}/shares",
            get(api::shared_dashboards::of_dashboard),
        )
        .route(
            "/dashboards/{id}/favorite",
            put(api::dashboard_lists::star).delete(api::dashboard_lists::unstar),
        )
        .route("/dashboard-favorites", get(api::dashboard_lists::favorites))
        .route(
            "/dashboard-lists",
            get(api::dashboard_lists::ui_lists).post(api::dashboard_lists::create),
        )
        .route(
            "/dashboard-lists/{id}",
            put(api::dashboard_lists::update).delete(api::dashboard_lists::delete),
        )
        .route(
            "/dashboard-lists/{id}/dashboards",
            post(api::dashboard_lists::add)
                .delete(api::dashboard_lists::remove)
                .put(api::dashboard_lists::replace),
        )
        .route("/dashboard-shares", post(api::shared_dashboards::create))
        .route(
            "/dashboard-shares/{token}",
            put(api::shared_dashboards::update).delete(api::shared_dashboards::delete),
        )
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
        .route(
            "/logs/rehydrations",
            get(logconfig::archives::list_views).post(logconfig::archives::create_view),
        )
        .route(
            "/logs/rehydrations/{id}",
            delete(logconfig::archives::delete_view),
        )
        .route("/sensitive-data-scanner/test", post(logconfig::sds::test))
        .route("/metrics", get(api::metrics::list))
        .route("/hosts", get(api::metrics::hosts))
        .route("/checks", get(checks::list))
        .route("/metrics-volume", get(api::metrics::volume))
        .route("/metrics/query", post(api::metrics::query))
        .route(
            "/metrics/{name}",
            get(api::metrics::detail).patch(api::metrics::update),
        );
    let r = r
        .route("/apm/ingestion", get(catalog::retention::ingestion))
        .route("/ci/tests", get(civis::tests))
        .route("/processes", get(processes::list))
        .route("/data-streams", get(dsm::overview))
        .route("/databases", get(dbm::overview))
        .route("/databases/queries/{signature}", get(dbm::query))
        .route(
            "/security/rules",
            get(security::list_rules).post(security::create_rule),
        )
        .route(
            "/security/rules/{id}",
            get(security::get_rule)
                .put(security::update_rule)
                .delete(security::delete_rule),
        )
        .route("/security/signals", get(security::signals))
        .route(
            "/security/signals/{id}/{what}",
            axum::routing::patch(security::triage),
        )
        .route("/audit/events", get(audit::events))
        .route("/workflows", get(workflows::list).post(workflows::create))
        .route("/workflows/runs", get(workflows::runs))
        .route("/workflows/runs/{id}/{what}", post(workflows::decide))
        .route(
            "/workflows/{id}",
            get(workflows::get)
                .put(workflows::update)
                .delete(workflows::delete),
        )
        .route("/workflows/{id}/instances", post(workflows::run))
        .route("/apps", get(workflows::apps).post(workflows::create_app))
        .route(
            "/apps/{id}",
            put(workflows::update_app).delete(workflows::delete_app),
        )
        .route("/apps/{id}/run", post(workflows::run_app))
        .route("/workflows/runs/{id}", get(workflows::run_one))
        .route("/cases", get(cases::list).post(cases::create))
        .route("/cases/{id}", get(cases::get))
        .route("/cases/{id}/{what}", post(cases::action))
        .route(
            "/on-call/pages",
            get(oncall::pages).post(oncall::create_page),
        )
        .route("/on-call/pages/{id}/{action}", post(oncall::page_action))
        .route("/on-call/teams/{team}/on-call", get(oncall::team_on_call))
        .route("/on-call/{what}", get(oncall::list).post(oncall::create))
        .route(
            "/on-call/{what}/{id}",
            axum::routing::put(oncall::update).delete(oncall::delete),
        )
        .route("/ci/pipelines", get(civis::pipelines))
        .route("/ci/pipelines/{provider}/{id}/jobs", get(civis::jobs))
        .route("/llm-obs/traces", get(llmobs::traces))
        .route("/llm-obs/traces/{id}", get(llmobs::trace))
        .route("/llm-obs/overview", get(llmobs::overview))
        .route("/profiling/profiles", get(profiling::list))
        .route("/profiling/services", get(profiling::services))
        .route("/profiling/flamegraph", post(profiling::flamegraph))
        .route(
            "/profiling/profiles/{id}/files/{name}",
            get(profiling::download),
        )
        .route("/apm/resource", get(api::apm::resource))
        .route(
            "/apm/definition-template/{name}",
            get(api::apm::definition_template),
        )
        .route("/teams", get(catalog::teams::ui_list))
        .route("/scorecards", get(catalog::scorecards::ui));
    catalog_routes(logconfig_routes(r, "", ""), "")
}

/// Datadog's public API, for its clients and Terraform: `DD-API-KEY` auth.
fn public_api(st: AppState) -> Router<AppState> {
    let r = Router::new()
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
        .route("/api/v1/synthetics/tests/api", post(synthetics::create_api))
        .route(
            "/api/v1/synthetics/tests/browser",
            post(synthetics::create_browser),
        )
        .route(
            "/api/v1/synthetics/tests/{typ}/{id}",
            get(synthetics::get_typed).put(synthetics::update_typed),
        )
        .route(
            "/api/v1/synthetics/tests/{id}",
            get(synthetics::get).delete(synthetics::delete),
        )
        .route(
            "/api/v1/synthetics/tests/delete",
            post(synthetics::delete_many),
        )
        .route("/api/v1/synthetics/tests/search", get(synthetics::search))
        .route(
            "/api/v1/synthetics/tests/{id}/status",
            put(synthetics::set_status),
        )
        .route(
            "/api/v1/synthetics/tests/{id}/results",
            get(synthetics::results),
        )
        .route(
            "/api/v1/synthetics/tests/trigger/ci",
            post(synthetics::ci::trigger),
        )
        .route(
            "/api/v1/synthetics/tests/trigger",
            post(synthetics::ci::trigger),
        )
        .route(
            "/api/v1/synthetics/ci/batch/{id}",
            get(synthetics::ci::batch),
        )
        .route(
            "/api/v2/synthetics/tests/poll_results",
            get(synthetics::ci::poll_results),
        )
        .route("/api/v1/synthetics/settings", get(synthetics::ci::settings))
        .route(
            "/api/v1/synthetics/locations",
            get(synthetics::ci::locations),
        )
        .route(
            "/api/v1/synthetics/variables",
            get(synthetics::variables::list).post(synthetics::variables::create),
        )
        .route(
            "/api/v1/synthetics/variables/{id}",
            get(synthetics::variables::get)
                .put(synthetics::variables::update)
                .delete(synthetics::variables::delete),
        )
        .route("/api/v1/slo", get(api::slos::list).post(api::slos::create))
        .route("/api/v1/slo/search", get(api::slos::search))
        .route("/api/v1/slo/can_delete", get(api::slos::can_delete))
        .route(
            "/api/v1/slo/correction",
            get(api::slos::list_corrections).post(api::slos::create_correction),
        )
        .route(
            "/api/v1/slo/correction/{id}",
            get(api::slos::get_correction)
                .patch(api::slos::update_correction)
                .delete(api::slos::delete_correction),
        )
        .route(
            "/api/v1/slo/{id}",
            get(api::slos::get)
                .put(api::slos::update)
                .delete(api::slos::delete),
        )
        .route("/api/v1/slo/{id}/history", get(api::slos::history))
        .route(
            "/api/v1/slo/{id}/corrections",
            get(api::slos::slo_corrections),
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
        .route(
            "/api/v2/rum/config/metrics",
            get(api::generated_metrics::rum_list).post(api::generated_metrics::rum_create),
        )
        .route(
            "/api/v2/rum/config/metrics/{id}",
            get(api::generated_metrics::rum_get)
                .patch(api::generated_metrics::rum_update)
                .delete(api::generated_metrics::rum_delete),
        )
        .route("/api/v2/query/timeseries", post(api::query::timeseries))
        .route("/api/v2/query/scalar", post(api::query::scalar))
        .route(
            "/api/v1/dashboard",
            get(api::dashboards::list).post(api::dashboards::create),
        )
        .route(
            "/api/v1/dashboard/lists/manual",
            get(api::dashboard_lists::list).post(api::dashboard_lists::create),
        )
        .route(
            "/api/v1/dashboard/lists/manual/{id}",
            get(api::dashboard_lists::get)
                .put(api::dashboard_lists::update)
                .delete(api::dashboard_lists::delete),
        )
        .route(
            "/api/v2/dashboard/lists/manual/{id}/dashboards",
            get(api::dashboard_lists::dashboards)
                .post(api::dashboard_lists::add)
                .delete(api::dashboard_lists::remove)
                .put(api::dashboard_lists::replace),
        )
        .route(
            "/api/v1/dashboard/public",
            post(api::shared_dashboards::create),
        )
        .route(
            "/api/v1/dashboard/public/{token}",
            get(api::shared_dashboards::get)
                .put(api::shared_dashboards::update)
                .delete(api::shared_dashboards::delete),
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
            "/api/v2/security_monitoring/rules",
            get(security::list_rules).post(security::create_rule),
        )
        .route(
            "/api/v2/security_monitoring/rules/{id}",
            get(security::get_rule)
                .put(security::update_rule)
                .delete(security::delete_rule),
        )
        .route(
            "/api/v2/security_monitoring/signals",
            get(security::signals),
        )
        .route(
            "/api/v2/security_monitoring/signals/{id}/{what}",
            axum::routing::patch(security::triage),
        )
        .route("/api/v2/audit/events", get(audit::events))
        .route(
            "/api/v2/workflows",
            get(workflows::list).post(workflows::create),
        )
        .route(
            "/api/v2/workflows/{id}",
            get(workflows::get)
                .patch(workflows::update)
                .put(workflows::update)
                .delete(workflows::delete),
        )
        .route(
            "/api/v2/workflows/{id}/instances",
            get(workflows::instances).post(workflows::run),
        )
        .route(
            "/api/v2/workflows/{id}/instances/{instance}",
            get(workflows::instance),
        )
        .route(
            "/api/v2/app-builder/apps",
            get(workflows::apps).post(workflows::create_app),
        )
        .route(
            "/api/v2/app-builder/apps/{id}",
            patch(workflows::update_app).delete(workflows::delete_app),
        )
        .route(
            "/api/v2/app-builder/apps/{id}/run",
            post(workflows::run_app),
        )
        .route("/api/v2/cases", get(cases::list).post(cases::create))
        .route("/api/v2/cases/{id}", get(cases::get))
        .route("/api/v2/cases/{id}/{what}", post(cases::action))
        .route(
            "/api/v2/on-call/pages",
            get(oncall::pages).post(oncall::create_page),
        )
        .route(
            "/api/v2/on-call/pages/{id}/{action}",
            post(oncall::page_action),
        )
        .route(
            "/api/v2/on-call/teams/{team}/on-call",
            get(oncall::team_on_call),
        )
        .route(
            "/api/v2/on-call/{what}",
            get(oncall::list).post(oncall::create),
        )
        .route(
            "/api/v2/on-call/{what}/{id}",
            axum::routing::put(oncall::update).delete(oncall::delete),
        )
        .route(
            "/api/v1/logs/config/pipeline-order",
            get(api::pipelines::order).put(api::pipelines::set_order),
        );
    catalog_routes(logconfig_routes(r, "/api/v1", "/api/v2"), "/api/v2")
        .layer(axum::middleware::from_fn_with_state(st.clone(), audit::api))
        .layer(axum::middleware::from_fn_with_state(
            st,
            intake::logs::require_api_key,
        ))
}

fn app(st: AppState) -> Router {
    let index = st.cfg.static_dir.join("index.html");
    let spa = ServeDir::new(&st.cfg.static_dir).fallback(ServeFile::new(index));
    // index.html names the hashed bundles: always revalidated, so a deploy
    // shows up on the next load.
    let spa = Router::new()
        .fallback_service(spa)
        .layer(axum::middleware::map_response(spa_cache));
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
            "/api/v2/replay",
            post(rum::replay_intake).options(intake::sentry::preflight),
        )
        .route(
            "/rum/proxy",
            post(rum::proxy).options(intake::sentry::preflight),
        )
        .route("/api/v1/check_run", post(checks::intake))
        .route("/profiling/v1/input", post(profiling::intake))
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
    // Intakes added for Datadog parity: a valid API key (header, or
    // `?dd-api-key=` for webhooks), like the Agent's.
    let keyed = Router::new()
        .route("/api/v2/profile", post(profiling::intake))
        .route("/api/v2/llmobs", post(llmobs::spans_intake))
        .route("/api/v2/citestcycle", post(civis::citestcycle))
        .route("/api/v2/cireport", post(civis::junit))
        .route("/api/v2/ci/pipeline", post(civis::pipeline_api))
        .route("/api/v2/webhook", post(civis::webhook))
        .route(
            "/api/intake/llm-obs/v1/trace/spans",
            post(llmobs::spans_intake),
        )
        .route(
            "/api/intake/llm-obs/v1/eval-metric",
            post(llmobs::evals_intake),
        )
        .route(
            "/api/intake/llm-obs/v2/eval-metric",
            post(llmobs::evals_intake),
        )
        .route("/api/v1/collector", post(processes::intake))
        .route("/api/v0.1/pipeline_stats", post(dsm::intake))
        .route("/api/v2/databasequery", post(dbm::samples_intake))
        .route("/api/v2/dbmmetrics", post(dbm::metrics_intake))
        .layer(axum::middleware::from_fn_with_state(
            st.clone(),
            intake::logs::require_api_key,
        ))
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024));
    Router::new()
        .merge(public_api(st.clone()))
        .merge(keyed)
        .merge(intake)
        .nest("/api/ui", ui_api(st.clone()))
        .route("/auth/login", get(auth::login))
        .route("/auth/callback", get(auth::callback))
        .route("/auth/logout", post(auth::logout).get(auth::logout))
        .route("/health", get(health))
        .route("/status", get(api::incidents::public_page))
        .route(
            "/api/public/dashboard/{token}",
            get(api::shared_dashboards::public_dashboard),
        )
        .route(
            "/api/public/dashboard/{token}/query",
            post(api::shared_dashboards::public_query),
        )
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
        logconfig: Arc::new(logconfig::LogConfig::default()),
        catalog: Arc::new(catalog::Catalog::default()),
    };
    if let Err(e) = catalog::init(&st).await {
        tracing::warn!(error = %e, "could not load the retention filters");
    }
    if let Err(e) = logconfig::init(&st).await {
        tracing::warn!(error = %e, "could not load the log configuration");
    }
    if let Err(e) = api::dashboards::ensure_default(&st).await {
        tracing::warn!(error = %e, "could not install the default dashboard");
    }
    intake::otlp::init();
    if let Err(e) = api::generated_metrics::reload(&st).await {
        tracing::warn!(error = %e, "could not load the generated metrics");
    }
    monitors::spawn(st.clone());
    synthetics::spawn(st.clone());
    rum::replay::spawn_purge(st.clone());
    profiling::spawn_purge(st.clone());
    llmobs::spawn_purge(st.clone());
    civis::spawn_purge(st.clone());
    audit::spawn_purge(st.clone());
    security::spawn(st.clone());
    oncall::spawn(st.clone());
    workflows::spawn(st.clone());
    {
        // RUM-based metrics counted at a session's or view's end.
        let st = st.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                tick.tick().await;
                st.generated
                    .flush_rum(&st.aggregator, intake::logs::now_ms());
            }
        });
    }
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
    let logconf = st.logconfig.clone();
    axum::serve(listener, app(st))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    // Nothing buffered is lost on a restart: the aggregator, the archives,
    // then the writers.
    aggregator.flush(&store).await;
    logconf.archives.flush().await;
    store.flush().await;
    tracing::info!("stopped");
    Ok(())
}

async fn spa_cache(mut res: axum::response::Response) -> axum::response::Response {
    use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, HeaderValue};
    let html = res
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/html"));
    if html {
        res.headers_mut()
            .insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    }
    res
}
