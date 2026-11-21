//! Storage: telemetry (logs, spans, metric points) in GreptimeDB, the
//! application's own state (facets, metric metadata) in PostgreSQL.

pub mod greptime;
pub mod pg;
pub mod writer;

use crate::config::Config;
use crate::intake::{logs::LogRow, traces::SpanRow};
use crate::metrics::Point;
use anyhow::Result;
use greptime::Greptime;
use pg::Pg;
use std::future::Future;
use std::time::Duration;
use writer::Writer;

pub struct Store {
    pub gt: Greptime,
    pub pg: Pg,
    pub logs: Writer<LogRow>,
    pub events: Writer<crate::events::EventRow>,
    pub rum: Writer<crate::rum::RumRow>,
    pub spans: Writer<SpanRow>,
    pub points: Writer<Point>,
    pub checks: Writer<crate::checks::CheckRun>,
}

/// Retries `f` for about two minutes: in a cluster the databases may start
/// after robine-dog does.
async fn patiently<T, F, Fut>(what: &str, mut f: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut delay = Duration::from_millis(500);
    for attempt in 1.. {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) if attempt >= 12 => return Err(e),
            Err(e) => {
                tracing::warn!(error = %format!("{e:#}"), attempt, "{what} not ready, retrying");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(15));
            }
        }
    }
    unreachable!()
}

impl Store {
    pub async fn open(cfg: &Config) -> Result<Self> {
        let gt = Greptime::new(
            &cfg.greptime_url,
            &cfg.greptime_db,
            cfg.greptime_auth.clone(),
        )?;
        patiently("GreptimeDB", || {
            gt.init(cfg.retention_days, cfg.metrics_retention_days)
        })
        .await?;
        let pg = patiently("PostgreSQL", || Pg::connect(&cfg.postgres_url)).await?;
        Ok(Self {
            logs: Writer::spawn(gt.clone()),
            events: Writer::spawn(gt.clone()),
            rum: Writer::spawn(gt.clone()),
            spans: Writer::spawn(gt.clone()),
            points: Writer::spawn(gt.clone()),
            checks: Writer::spawn(gt.clone()),
            gt,
            pg,
        })
    }

    /// Writes out every buffered row (on shutdown).
    pub async fn flush(&self) {
        tokio::join!(
            self.logs.flush(),
            self.events.flush(),
            self.rum.flush(),
            self.spans.flush(),
            self.points.flush(),
            self.checks.flush()
        );
    }
}
