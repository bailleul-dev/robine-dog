//! PostgreSQL: the application's own state (facets, metric metadata), as
//! opposed to the telemetry in GreptimeDB.

use anyhow::{Context, Result};
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use serde::{Deserialize, Serialize};
use tokio_postgres::NoTls;

/// Applied in order, once each; the applied version is kept in `schema_version`.
const MIGRATIONS: &[&str] = &[
    r#"
    CREATE TABLE facets (
        id       BIGSERIAL PRIMARY KEY,
        dataset  TEXT NOT NULL CHECK (dataset IN ('logs', 'spans')),
        path     TEXT NOT NULL,
        name     TEXT NOT NULL,
        grp      TEXT NOT NULL DEFAULT 'Attributes',
        UNIQUE (dataset, path)
    );
    INSERT INTO facets (dataset, path, name, grp) VALUES
        ('logs', 'env', 'Env', 'Tags'),
        ('logs', '@http.status_code', 'Status Code', 'HTTP'),
        ('logs', '@http.method', 'Method', 'HTTP'),
        ('logs', '@http.url_details.path', 'URL Path', 'HTTP'),
        ('logs', '@error.kind', 'Error Kind', 'Error'),
        ('spans', '@http.status_code', 'Status Code', 'HTTP'),
        ('spans', '@http.method', 'Method', 'HTTP'),
        ('spans', '@error.type', 'Error Type', 'Error'),
        ('spans', '@db.system', 'Database', 'Database'),
        ('spans', '@peer.service', 'Peer Service', 'Network');
    "#,
    r#"
    CREATE TABLE metric_metadata (
        name         TEXT PRIMARY KEY,
        type         TEXT NOT NULL,
        unit         TEXT,
        description  TEXT,
        first_seen   TIMESTAMPTZ NOT NULL DEFAULT now(),
        last_seen    TIMESTAMPTZ NOT NULL DEFAULT now()
    );
    "#,
];

#[derive(Clone)]
pub struct Pg {
    pool: Pool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FacetDef {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub dataset: String,
    pub path: String,
    pub name: String,
    #[serde(default = "default_group")]
    pub group: String,
}

fn default_group() -> String {
    "Attributes".into()
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricMeta {
    pub name: String,
    #[serde(rename = "type")]
    pub typ: String,
    pub unit: Option<String>,
    pub description: Option<String>,
    /// Epoch milliseconds.
    pub last_seen: i64,
}

impl Pg {
    pub async fn connect(url: &str) -> Result<Self> {
        let cfg: tokio_postgres::Config = url.parse().context("ROBINE_DOG_POSTGRES_URL")?;
        let mgr = Manager::from_config(
            cfg,
            NoTls,
            ManagerConfig {
                recycling_method: RecyclingMethod::Fast,
            },
        );
        let pool = Pool::builder(mgr).max_size(8).build()?;
        let pg = Self { pool };
        pg.migrate().await?;
        Ok(pg)
    }

    async fn migrate(&self) -> Result<()> {
        let mut c = self.pool.get().await.context("PostgreSQL unreachable")?;
        c.batch_execute("CREATE TABLE IF NOT EXISTS schema_version (version INT NOT NULL)")
            .await?;
        let tx = c.transaction().await?;
        // One migrator at a time, should two instances start together.
        tx.batch_execute("LOCK TABLE schema_version IN EXCLUSIVE MODE")
            .await?;
        let current: i32 = tx
            .query_opt("SELECT version FROM schema_version", &[])
            .await?
            .map(|r| r.get(0))
            .unwrap_or(0);
        for (i, m) in MIGRATIONS.iter().enumerate().skip(current as usize) {
            tx.batch_execute(m)
                .await
                .with_context(|| format!("migration {}", i + 1))?;
        }
        tx.execute("DELETE FROM schema_version", &[]).await?;
        tx.execute(
            "INSERT INTO schema_version (version) VALUES ($1)",
            &[&(MIGRATIONS.len() as i32)],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn facets(&self, dataset: &str) -> Result<Vec<FacetDef>> {
        let c = self.pool.get().await?;
        let rows = c
            .query(
                "SELECT id, dataset, path, name, grp FROM facets WHERE dataset = $1 ORDER BY grp, name",
                &[&dataset],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| FacetDef {
                id: r.get(0),
                dataset: r.get(1),
                path: r.get(2),
                name: r.get(3),
                group: r.get(4),
            })
            .collect())
    }

    pub async fn upsert_facet(&self, mut def: FacetDef) -> Result<FacetDef> {
        let c = self.pool.get().await?;
        let row = c
            .query_one(
                "INSERT INTO facets (dataset, path, name, grp) VALUES ($1, $2, $3, $4)
                 ON CONFLICT (dataset, path) DO UPDATE SET name = excluded.name, grp = excluded.grp
                 RETURNING id",
                &[&def.dataset, &def.path, &def.name, &def.group],
            )
            .await?;
        def.id = row.get(0);
        Ok(def)
    }

    pub async fn delete_facet(&self, id: i64) -> Result<()> {
        let c = self.pool.get().await?;
        c.execute("DELETE FROM facets WHERE id = $1", &[&id])
            .await?;
        Ok(())
    }

    /// Records metrics seen in a flush: new ones are created, known ones touched.
    pub async fn touch_metrics(&self, metrics: &[(String, String)]) -> Result<()> {
        if metrics.is_empty() {
            return Ok(());
        }
        let names: Vec<&str> = metrics.iter().map(|m| m.0.as_str()).collect();
        let types: Vec<&str> = metrics.iter().map(|m| m.1.as_str()).collect();
        let c = self.pool.get().await?;
        c.execute(
            "INSERT INTO metric_metadata (name, type)
             SELECT * FROM unnest($1::text[], $2::text[])
             ON CONFLICT (name) DO UPDATE SET last_seen = now()",
            &[&names, &types],
        )
        .await?;
        Ok(())
    }

    async fn select_metrics(&self, name: Option<&str>) -> Result<Vec<MetricMeta>> {
        let c = self.pool.get().await?;
        let rows = c
            .query(
                "SELECT name, type, unit, description, (extract(epoch FROM last_seen) * 1000)::bigint
                 FROM metric_metadata WHERE $1::text IS NULL OR name = $1 ORDER BY name",
                &[&name],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| MetricMeta {
                name: r.get(0),
                typ: r.get(1),
                unit: r.get(2),
                description: r.get(3),
                last_seen: r.get(4),
            })
            .collect())
    }

    pub async fn metrics(&self) -> Result<Vec<MetricMeta>> {
        self.select_metrics(None).await
    }

    pub async fn metric(&self, name: &str) -> Result<Option<MetricMeta>> {
        Ok(self.select_metrics(Some(name)).await?.pop())
    }

    pub async fn update_metric(
        &self,
        name: &str,
        unit: Option<&str>,
        description: Option<&str>,
    ) -> Result<bool> {
        let c = self.pool.get().await?;
        let n = c
            .execute(
                "UPDATE metric_metadata SET unit = $2, description = $3 WHERE name = $1",
                &[&name, &unit, &description],
            )
            .await?;
        Ok(n > 0)
    }
}
