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
    // Monitors: per-group state, and the history of state changes.
    r#"
    CREATE SEQUENCE monitor_ids START 1000;
    CREATE TABLE monitor_groups (
        monitor_id      BIGINT NOT NULL,
        grp             TEXT NOT NULL,
        status          TEXT NOT NULL,
        value           DOUBLE PRECISION,
        last_data       TIMESTAMPTZ,
        last_triggered  TIMESTAMPTZ,
        last_notified   TIMESTAMPTZ,
        last_resolved   TIMESTAMPTZ,
        since           TIMESTAMPTZ NOT NULL DEFAULT now(),
        PRIMARY KEY (monitor_id, grp)
    );
    CREATE TABLE monitor_events (
        id           BIGSERIAL PRIMARY KEY,
        monitor_id   BIGINT NOT NULL,
        grp          TEXT NOT NULL,
        ts           TIMESTAMPTZ NOT NULL DEFAULT now(),
        from_status  TEXT NOT NULL,
        to_status    TEXT NOT NULL,
        value        DOUBLE PRECISION,
        title        TEXT NOT NULL,
        message      TEXT NOT NULL,
        notified     TEXT[] NOT NULL DEFAULT '{}'
    );
    CREATE INDEX monitor_events_by_monitor ON monitor_events (monitor_id, ts DESC);
    "#,
    // When each group was first seen (`new_group_delay`).
    r#"
    ALTER TABLE monitor_groups ADD COLUMN first_seen TIMESTAMPTZ NOT NULL DEFAULT now();
    "#,
];

/// A monitor group's state, as last evaluated.
#[derive(Debug, Clone, Serialize)]
pub struct GroupState {
    pub group: String,
    pub status: String,
    pub value: Option<f64>,
    /// Epoch milliseconds.
    pub last_data: Option<i64>,
    pub last_triggered: Option<i64>,
    pub last_notified: Option<i64>,
    pub last_resolved: Option<i64>,
    pub since: i64,
    pub first_seen: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MonitorEvent {
    pub id: i64,
    pub monitor_id: i64,
    pub group: String,
    pub ts: i64,
    pub from_status: String,
    pub to_status: String,
    pub value: Option<f64>,
    pub title: String,
    pub message: String,
    pub notified: Vec<String>,
}

fn ms(t: Option<std::time::SystemTime>) -> Option<i64> {
    t.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
}

fn time(ms: Option<i64>) -> Option<std::time::SystemTime> {
    ms.map(|m| std::time::UNIX_EPOCH + std::time::Duration::from_millis(m.max(0) as u64))
}

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
        Ok(c.execute(
            "DELETE FROM documents WHERE kind = $1 AND id = $2",
            &[&kind, &id],
        )
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

    pub async fn next_monitor_id(&self) -> Result<i64> {
        let c = self.pool.get().await?;
        Ok(c.query_one("SELECT nextval('monitor_ids')", &[])
            .await?
            .get(0))
    }

    pub async fn monitor_groups(&self, monitor_id: i64) -> Result<Vec<GroupState>> {
        let c = self.pool.get().await?;
        let rows = c
            .query(
                "SELECT grp, status, value, last_data, last_triggered, last_notified, last_resolved, since, first_seen
                 FROM monitor_groups WHERE monitor_id = $1 ORDER BY grp",
                &[&monitor_id],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| GroupState {
                group: r.get(0),
                status: r.get(1),
                value: r.get(2),
                last_data: ms(r.get(3)),
                last_triggered: ms(r.get(4)),
                last_notified: ms(r.get(5)),
                last_resolved: ms(r.get(6)),
                since: ms(r.get(7)).unwrap_or(0),
                first_seen: ms(r.get(8)).unwrap_or(0),
            })
            .collect())
    }

    pub async fn put_monitor_group(&self, monitor_id: i64, g: &GroupState) -> Result<()> {
        let c = self.pool.get().await?;
        c.execute(
            "INSERT INTO monitor_groups (monitor_id, grp, status, value, last_data, last_triggered, last_notified, last_resolved, since, first_seen)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT (monitor_id, grp) DO UPDATE SET status = excluded.status, value = excluded.value,
               last_data = excluded.last_data, last_triggered = excluded.last_triggered,
               last_notified = excluded.last_notified, last_resolved = excluded.last_resolved, since = excluded.since",
            &[
                &monitor_id,
                &g.group,
                &g.status,
                &g.value,
                &time(g.last_data),
                &time(g.last_triggered),
                &time(g.last_notified),
                &time(g.last_resolved),
                &time(Some(g.since)),
                &time(Some(g.first_seen)),
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn delete_monitor_groups(&self, monitor_id: i64, keep: &[String]) -> Result<()> {
        let c = self.pool.get().await?;
        c.execute(
            "DELETE FROM monitor_groups WHERE monitor_id = $1 AND NOT (grp = ANY($2))",
            &[&monitor_id, &keep],
        )
        .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn add_monitor_event(
        &self,
        monitor_id: i64,
        group: &str,
        from: &str,
        to: &str,
        value: Option<f64>,
        title: &str,
        message: &str,
        notified: &[String],
    ) -> Result<()> {
        let c = self.pool.get().await?;
        c.execute(
            "INSERT INTO monitor_events (monitor_id, grp, from_status, to_status, value, title, message, notified)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            &[&monitor_id, &group, &from, &to, &value, &title, &message, &notified],
        )
        .await?;
        Ok(())
    }

    /// The latest events, of one monitor or of all.
    pub async fn monitor_events(
        &self,
        monitor_id: Option<i64>,
        limit: i64,
    ) -> Result<Vec<MonitorEvent>> {
        let c = self.pool.get().await?;
        let rows = c
            .query(
                "SELECT id, monitor_id, grp, ts, from_status, to_status, value, title, message, notified
                 FROM monitor_events WHERE $1::bigint IS NULL OR monitor_id = $1
                 ORDER BY ts DESC, id DESC LIMIT $2",
                &[&monitor_id, &limit],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| MonitorEvent {
                id: r.get(0),
                monitor_id: r.get(1),
                group: r.get(2),
                ts: ms(r.get(3)).unwrap_or(0),
                from_status: r.get(4),
                to_status: r.get(5),
                value: r.get(6),
                title: r.get(7),
                message: r.get(8),
                notified: r.get(9),
            })
            .collect())
    }

    /// The state changes of some monitors up to `to_ms`, oldest first: the
    /// history monitor-based SLOs are computed from.
    pub async fn monitor_transitions(
        &self,
        monitor_ids: &[i64],
        to_ms: i64,
    ) -> Result<Vec<(i64, String, i64, String)>> {
        let c = self.pool.get().await?;
        let rows = c
            .query(
                "SELECT monitor_id, grp, ts, to_status FROM monitor_events
                 WHERE monitor_id = ANY($1) AND ts <= to_timestamp($2::double precision / 1000.0)
                 ORDER BY ts, id",
                &[&monitor_ids, &(to_ms as f64)],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| (r.get(0), r.get(1), ms(r.get(2)).unwrap_or(0), r.get(3)))
            .collect())
    }

    pub async fn forget_monitor(&self, monitor_id: i64) -> Result<()> {
        let c = self.pool.get().await?;
        c.execute(
            "DELETE FROM monitor_groups WHERE monitor_id = $1",
            &[&monitor_id],
        )
        .await?;
        c.execute(
            "DELETE FROM monitor_events WHERE monitor_id = $1",
            &[&monitor_id],
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
