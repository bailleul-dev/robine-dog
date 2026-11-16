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
    // Configuration objects (pipelines, monitors, dashboards…), one JSON
    // document each, in a user-defined order within their kind.
    r#"
    CREATE TABLE documents (
        kind        TEXT NOT NULL,
        id          TEXT NOT NULL,
        position    INT NOT NULL DEFAULT 0,
        body        JSONB NOT NULL,
        created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
        updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
        PRIMARY KEY (kind, id)
    );
    "#,
];

/// A stored configuration document.
#[derive(Debug, Clone, Serialize)]
pub struct Doc {
    pub id: String,
    pub position: i32,
    pub body: serde_json::Value,
    /// Epoch milliseconds.
    pub created_at: i64,
    pub updated_at: i64,
}

/// A random id like Datadog's (22 URL-safe characters).
pub fn new_id() -> String {
    use base64::Engine;
    let bytes: [u8; 16] = rand::random();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

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

    pub async fn docs(&self, kind: &str) -> Result<Vec<Doc>> {
        self.select_docs(kind, None).await
    }

    pub async fn doc(&self, kind: &str, id: &str) -> Result<Option<Doc>> {
        Ok(self.select_docs(kind, Some(id)).await?.pop())
    }

    async fn select_docs(&self, kind: &str, id: Option<&str>) -> Result<Vec<Doc>> {
        let c = self.pool.get().await?;
        let rows = c
            .query(
                "SELECT id, position, body,
                        (extract(epoch FROM created_at) * 1000)::bigint,
                        (extract(epoch FROM updated_at) * 1000)::bigint
                 FROM documents WHERE kind = $1 AND ($2::text IS NULL OR id = $2)
                 ORDER BY position, created_at",
                &[&kind, &id],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| Doc {
                id: r.get(0),
                position: r.get(1),
                body: r.get(2),
                created_at: r.get(3),
                updated_at: r.get(4),
            })
            .collect())
    }

    /// Creates or replaces a document; a new one goes last.
    pub async fn put_doc(&self, kind: &str, id: &str, body: &serde_json::Value) -> Result<Doc> {
        let c = self.pool.get().await?;
        c.execute(
            "INSERT INTO documents (kind, id, body, position)
             VALUES ($1, $2, $3, (SELECT coalesce(max(position) + 1, 0) FROM documents WHERE kind = $1))
             ON CONFLICT (kind, id) DO UPDATE SET body = excluded.body, updated_at = now()",
            &[&kind, &id, body],
        )
        .await?;
        drop(c);
        self.doc(kind, id)
            .await?
            .context("document vanished after being written")
    }

    pub async fn delete_doc(&self, kind: &str, id: &str) -> Result<bool> {
        let c = self.pool.get().await?;
        Ok(c.execute("DELETE FROM documents WHERE kind = $1 AND id = $2", &[&kind, &id])
            .await?
            > 0)
    }

    /// Puts the listed documents first, in that order.
    pub async fn reorder_docs(&self, kind: &str, ids: &[String]) -> Result<()> {
        let c = self.pool.get().await?;
        c.execute(
            "UPDATE documents d SET position = o.pos - 1
             FROM unnest($2::text[]) WITH ORDINALITY AS o(id, pos)
             WHERE d.kind = $1 AND d.id = o.id",
            &[&kind, &ids],
        )
        .await?;
        Ok(())
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
