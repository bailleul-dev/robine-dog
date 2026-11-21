//! What happens to logs after their pipelines, as in Datadog: the Sensitive
//! Data Scanner, log-based metrics, archives, then the indexes, which decide
//! what is stored and searchable. Reference Tables enrich logs (the lookup
//! processor) and filter searches.

pub mod archives;
pub mod indexes;
pub mod reftables;
pub mod sds;

use crate::AppState;
use crate::intake::logs::{LogRow, now_ms};
use indexes::Route;

#[derive(Default)]
pub struct LogConfig {
    pub indexes: indexes::Indexes,
    pub archives: archives::Archives,
    pub sds: sds::Scanner,
}

pub fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default()
}

/// Logs out of their pipelines: scanned, counted by log-based metrics,
/// archived, then stored if an index keeps them. Returns how many are stored.
pub async fn ingest(st: &AppState, rows: Vec<LogRow>) -> anyhow::Result<usize> {
    let now = now_ms();
    let mut kept = Vec::with_capacity(rows.len());
    let mut full = false;
    for mut row in rows {
        st.logconfig.sds.scan(&mut row);
        st.generated.on_log(&st.aggregator, &row);
        full |= st.logconfig.archives.add(&row);
        match st.logconfig.indexes.route(&row, now) {
            Route::Keep => kept.push(row),
            Route::Index(name) => {
                row.tags.push(format!("index:{name}"));
                kept.push(row);
            }
            Route::Drop => {}
        }
    }
    if full {
        let st = st.clone();
        tokio::spawn(async move { st.logconfig.archives.flush().await });
    }
    let n = kept.len();
    st.store.logs.send(kept).await?;
    Ok(n)
}

/// Loads the configuration at start, and starts the archive writer.
pub async fn init(st: &AppState) -> anyhow::Result<()> {
    reftables::reload(st).await?;
    sds::reload(st).await?;
    indexes::init(st).await?;
    archives::reload(st).await?;
    archives::reload_views(st).await?;
    archives::spawn(st.clone());
    Ok(())
}
