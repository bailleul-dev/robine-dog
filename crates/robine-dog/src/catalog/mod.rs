//! APM configuration and the Software Catalog: retention filters and
//! ingestion control, service definitions, teams and scorecards.

pub mod definitions;
pub mod retention;
pub mod scorecards;
pub mod teams;

use crate::AppState;

#[derive(Default)]
pub struct Catalog {
    pub retention: retention::Retention,
}

pub async fn init(st: &AppState) -> anyhow::Result<()> {
    retention::init(st).await
}
