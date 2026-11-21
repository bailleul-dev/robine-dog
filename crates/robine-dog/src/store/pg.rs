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
    // Synthetic test results, and the screenshots of browser tests' steps.
    r#"
    CREATE TABLE synthetics_results (
        result_id TEXT PRIMARY KEY,
        public_id TEXT NOT NULL,
        ts BIGINT NOT NULL,
        passed BOOLEAN NOT NULL,
        batch_id TEXT,
        body JSONB NOT NULL
    );
    CREATE INDEX synthetics_results_by_test ON synthetics_results (public_id, ts DESC);
    CREATE INDEX synthetics_results_by_batch ON synthetics_results (batch_id)
        WHERE batch_id IS NOT NULL;
    CREATE TABLE synthetics_screenshots (
        result_id TEXT NOT NULL REFERENCES synthetics_results ON DELETE CASCADE,
        step INT NOT NULL,
        image BYTEA NOT NULL,
        PRIMARY KEY (result_id, step)
    );
    "#,
    // Session Replay segments, zlib-compressed JSON as the browser SDK sends them.
    r#"
    CREATE TABLE replay_segments (
        id BIGSERIAL PRIMARY KEY,
        session_id TEXT NOT NULL,
        view_id TEXT NOT NULL,
        application_id TEXT,
        start_ms BIGINT NOT NULL,
        end_ms BIGINT NOT NULL,
        records INT NOT NULL,
        full_snapshot BOOLEAN NOT NULL,
        index_in_view INT,
        creation_reason TEXT,
        source TEXT,
        data BYTEA NOT NULL
    );
    CREATE INDEX replay_segments_by_session ON replay_segments (session_id, start_ms);
    CREATE INDEX replay_segments_by_time ON replay_segments (start_ms);
    "#,
    // Continuous Profiler: a profile's tags, its stacks folded per sample type
    // (zstd JSON), and the uploaded pprof files.
    r#"
    CREATE TABLE profiles (
        id BIGSERIAL PRIMARY KEY,
        start_ms BIGINT NOT NULL,
        end_ms BIGINT NOT NULL,
        service TEXT NOT NULL,
        env TEXT,
        version TEXT,
        host TEXT,
        language TEXT,
        runtime_id TEXT,
        tags TEXT[] NOT NULL,
        types TEXT[] NOT NULL,
        folded BYTEA NOT NULL,
        size BIGINT NOT NULL
    );
    CREATE INDEX profiles_by_service ON profiles (service, end_ms);
    CREATE INDEX profiles_by_time ON profiles (end_ms);
    CREATE TABLE profile_files (
        profile_id BIGINT NOT NULL REFERENCES profiles ON DELETE CASCADE,
        name TEXT NOT NULL,
        data BYTEA NOT NULL,
        PRIMARY KEY (profile_id, name)
    );
    "#,
    // LLM Observability: spans of LLM applications and their evaluations.
    r#"
    CREATE TABLE llm_spans (
        trace_id TEXT NOT NULL,
        span_id TEXT NOT NULL,
        parent_id TEXT,
        ml_app TEXT NOT NULL,
        name TEXT NOT NULL,
        kind TEXT NOT NULL,
        start_ns BIGINT NOT NULL,
        duration BIGINT NOT NULL,
        error BOOLEAN NOT NULL,
        model_name TEXT,
        model_provider TEXT,
        input_tokens BIGINT,
        output_tokens BIGINT,
        total_tokens BIGINT,
        cost DOUBLE PRECISION,
        session_id TEXT,
        service TEXT,
        env TEXT,
        tags TEXT[] NOT NULL,
        input TEXT NOT NULL,
        output TEXT NOT NULL,
        body JSONB NOT NULL,
        PRIMARY KEY (trace_id, span_id)
    );
    CREATE INDEX llm_spans_by_app ON llm_spans (ml_app, start_ns);
    CREATE INDEX llm_spans_by_time ON llm_spans (start_ns);
    CREATE INDEX llm_spans_by_session ON llm_spans (session_id);
    CREATE TABLE llm_evals (
        id BIGSERIAL PRIMARY KEY,
        trace_id TEXT NOT NULL,
        span_id TEXT NOT NULL,
        ml_app TEXT NOT NULL,
        label TEXT NOT NULL,
        metric_type TEXT NOT NULL,
        categorical TEXT,
        score DOUBLE PRECISION,
        ts BIGINT NOT NULL,
        tags TEXT[] NOT NULL
    );
    CREATE INDEX llm_evals_by_span ON llm_evals (span_id);
    CREATE INDEX llm_evals_by_time ON llm_evals (ts);
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

/// A stored Session Replay segment.
pub struct ReplaySegment {
    pub meta: crate::rum::replay::Meta,
    /// zlib-compressed JSON.
    pub data: Vec<u8>,
}

/// A synthetic test's run.
#[derive(Debug, Clone)]
pub struct SyntheticResult {
    pub result_id: String,
    pub public_id: String,
    /// Epoch milliseconds.
    pub ts: i64,
    pub passed: bool,
    pub batch_id: Option<String>,
    pub body: serde_json::Value,
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
    /// A connection, for modules that own their tables' SQL.
    pub async fn conn(&self) -> Result<deadpool_postgres::Object> {
        Ok(self.pool.get().await?)
    }

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

    // --- synthetic results ---------------------------------------------------------

    /// A run's result, and its screenshots by step.
    pub async fn put_synthetic_result(
        &self,
        r: &SyntheticResult,
        screenshots: &[(i32, Vec<u8>)],
    ) -> Result<()> {
        let mut c = self.pool.get().await?;
        let tx = c.transaction().await?;
        tx.execute(
            "INSERT INTO synthetics_results (result_id, public_id, ts, passed, batch_id, body)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (result_id) DO UPDATE SET ts = $3, passed = $4, body = $6",
            &[
                &r.result_id,
                &r.public_id,
                &r.ts,
                &r.passed,
                &r.batch_id,
                &r.body,
            ],
        )
        .await?;
        for (step, image) in screenshots {
            tx.execute(
                "INSERT INTO synthetics_screenshots (result_id, step, image) VALUES ($1, $2, $3)
                 ON CONFLICT (result_id, step) DO UPDATE SET image = $3",
                &[&r.result_id, step, image],
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    fn synthetic_row(r: &tokio_postgres::Row) -> SyntheticResult {
        SyntheticResult {
            result_id: r.get(0),
            public_id: r.get(1),
            ts: r.get(2),
            passed: r.get(3),
            batch_id: r.get(4),
            body: r.get(5),
        }
    }

    /// A test's results between two times, latest first.
    pub async fn synthetic_results(
        &self,
        public_id: &str,
        from_ms: i64,
        to_ms: i64,
        limit: i64,
    ) -> Result<Vec<SyntheticResult>> {
        let c = self.pool.get().await?;
        let rows = c
            .query(
                "SELECT result_id, public_id, ts, passed, batch_id, body FROM synthetics_results
                 WHERE public_id = $1 AND ts >= $2 AND ts <= $3 ORDER BY ts DESC LIMIT $4",
                &[&public_id, &from_ms, &to_ms, &limit],
            )
            .await?;
        Ok(rows.iter().map(Self::synthetic_row).collect())
    }

    pub async fn synthetic_results_by_id(&self, ids: &[String]) -> Result<Vec<SyntheticResult>> {
        let c = self.pool.get().await?;
        let rows = c
            .query(
                "SELECT result_id, public_id, ts, passed, batch_id, body FROM synthetics_results
                 WHERE result_id = ANY($1)",
                &[&ids],
            )
            .await?;
        Ok(rows.iter().map(Self::synthetic_row).collect())
    }

    /// Each test's latest result.
    pub async fn latest_synthetic_results(&self) -> Result<Vec<SyntheticResult>> {
        let c = self.pool.get().await?;
        let rows = c
            .query(
                "SELECT DISTINCT ON (public_id) result_id, public_id, ts, passed, batch_id, body
                 FROM synthetics_results WHERE batch_id IS NULL ORDER BY public_id, ts DESC",
                &[],
            )
            .await?;
        Ok(rows.iter().map(Self::synthetic_row).collect())
    }

    pub async fn synthetic_screenshot(
        &self,
        result_id: &str,
        step: i32,
    ) -> Result<Option<Vec<u8>>> {
        let c = self.pool.get().await?;
        Ok(c.query_opt(
            "SELECT image FROM synthetics_screenshots WHERE result_id = $1 AND step = $2",
            &[&result_id, &step],
        )
        .await?
        .map(|r| r.get(0)))
    }

    pub async fn delete_synthetic_results(&self, public_id: &str) -> Result<()> {
        let c = self.pool.get().await?;
        c.execute(
            "DELETE FROM synthetics_results WHERE public_id = $1",
            &[&public_id],
        )
        .await?;
        Ok(())
    }

    /// Drops the results older than `before_ms`, and the screenshots of all
    /// but each test's latest `keep_screenshots` runs.
    pub async fn purge_synthetic_results(
        &self,
        before_ms: i64,
        keep_screenshots: i64,
    ) -> Result<u64> {
        let c = self.pool.get().await?;
        let n = c
            .execute(
                "DELETE FROM synthetics_results WHERE ts < $1",
                &[&before_ms],
            )
            .await?;
        c.execute(
            "DELETE FROM synthetics_screenshots WHERE result_id IN (
                SELECT result_id FROM (
                    SELECT result_id, row_number() OVER (PARTITION BY public_id ORDER BY ts DESC) AS n
                    FROM synthetics_results) ranked
                WHERE n > $1)",
            &[&keep_screenshots],
        )
        .await?;
        Ok(n)
    }

    // --- session replay ----------------------------------------------------------

    pub async fn put_replay_segment(
        &self,
        m: &crate::rum::replay::Meta,
        data: &[u8],
    ) -> Result<()> {
        let c = self.pool.get().await?;
        let records = m.records_count as i32;
        let index = m.index_in_view.map(|i| i as i32);
        c.execute(
            "INSERT INTO replay_segments (session_id, view_id, application_id, start_ms, end_ms, records,
                full_snapshot, index_in_view, creation_reason, source, data)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            &[
                &m.session_id,
                &m.view_id,
                &m.application_id,
                &m.start,
                &m.end,
                &records,
                &m.has_full_snapshot,
                &index,
                &m.creation_reason,
                &m.source,
                &data,
            ],
        )
        .await?;
        Ok(())
    }

    /// A session's segments, oldest first.
    pub async fn replay_segments(&self, session_id: &str) -> Result<Vec<ReplaySegment>> {
        let c = self.pool.get().await?;
        let rows = c
            .query(
                "SELECT session_id, view_id, application_id, start_ms, end_ms, records, full_snapshot,
                    index_in_view, creation_reason, source, data
                 FROM replay_segments WHERE session_id = $1 ORDER BY start_ms",
                &[&session_id],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| ReplaySegment {
                meta: crate::rum::replay::Meta {
                    session_id: r.get(0),
                    view_id: r.get(1),
                    application_id: r.get(2),
                    start: r.get(3),
                    end: r.get(4),
                    records_count: r.get::<_, i32>(5) as i64,
                    has_full_snapshot: r.get(6),
                    index_in_view: r.get::<_, Option<i32>>(7).map(i64::from),
                    creation_reason: r.get(8),
                    source: r.get(9),
                },
                data: r.get(10),
            })
            .collect())
    }

    /// Which of these sessions have a replay.
    pub async fn sessions_with_replay(
        &self,
        ids: &[String],
    ) -> Result<std::collections::HashSet<String>> {
        let c = self.pool.get().await?;
        let rows = c
            .query(
                "SELECT DISTINCT session_id FROM replay_segments WHERE session_id = ANY($1)",
                &[&ids],
            )
            .await?;
        Ok(rows.iter().map(|r| r.get(0)).collect())
    }

    pub async fn purge_replay_segments(&self, before_ms: i64) -> Result<u64> {
        let c = self.pool.get().await?;
        Ok(c.execute(
            "DELETE FROM replay_segments WHERE end_ms < $1",
            &[&before_ms],
        )
        .await?)
    }

    pub async fn put_profile(
        &self,
        m: &crate::profiling::Meta,
        types: &[String],
        folded: &[u8],
        files: &[(String, Vec<u8>)],
    ) -> Result<i64> {
        let mut c = self.pool.get().await?;
        let tx = c.transaction().await?;
        let size: i64 = files.iter().map(|f| f.1.len() as i64).sum();
        let id: i64 = tx
            .query_one(
                "INSERT INTO profiles (start_ms, end_ms, service, env, version, host, language,
                    runtime_id, tags, types, folded, size)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) RETURNING id",
                &[
                    &m.start,
                    &m.end,
                    &m.service,
                    &m.env,
                    &m.version,
                    &m.host,
                    &m.language,
                    &m.runtime_id,
                    &m.tags,
                    &types,
                    &folded,
                    &size,
                ],
            )
            .await?
            .get(0);
        let mut seen = std::collections::HashSet::new();
        for (name, data) in files {
            if seen.insert(name.as_str()) {
                tx.execute(
                    "INSERT INTO profile_files (profile_id, name, data) VALUES ($1, $2, $3)",
                    &[&id, name, data],
                )
                .await?;
            }
        }
        tx.commit().await?;
        Ok(id)
    }

    /// Profiles overlapping a range, newest first.
    pub async fn profiles(
        &self,
        f: &crate::profiling::Filter,
        limit: i64,
    ) -> Result<Vec<crate::profiling::ProfileRow>> {
        let c = self.pool.get().await?;
        let rows = c
            .query(
                "SELECT p.id, p.start_ms, p.end_ms, p.service, p.env, p.version, p.host, p.language,
                    p.runtime_id, p.tags, p.types, p.size,
                    ARRAY(SELECT name FROM profile_files WHERE profile_id = p.id ORDER BY name)
                 FROM profiles p
                 WHERE p.end_ms >= $1 AND p.start_ms <= $2
                   AND ($3::TEXT IS NULL OR p.service = $3) AND ($4::TEXT IS NULL OR p.env = $4)
                   AND ($5::TEXT IS NULL OR p.version = $5) AND ($6::TEXT IS NULL OR p.host = $6)
                 ORDER BY p.end_ms DESC LIMIT $7",
                &[&f.from, &f.to, &f.service, &f.env, &f.version, &f.host, &limit],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| crate::profiling::ProfileRow {
                id: r.get(0),
                meta: crate::profiling::Meta {
                    start: r.get(1),
                    end: r.get(2),
                    service: r.get(3),
                    env: r.get(4),
                    version: r.get(5),
                    host: r.get(6),
                    language: r.get(7),
                    runtime_id: r.get(8),
                    tags: r.get(9),
                },
                types: r.get(10),
                size: r.get(11),
                files: r.get(12),
            })
            .collect())
    }

    pub async fn profile_folded(&self, ids: &[i64]) -> Result<Vec<Vec<u8>>> {
        let c = self.pool.get().await?;
        let rows = c
            .query("SELECT folded FROM profiles WHERE id = ANY($1)", &[&ids])
            .await?;
        Ok(rows.iter().map(|r| r.get(0)).collect())
    }

    pub async fn profile_file(&self, id: i64, name: &str) -> Result<Option<Vec<u8>>> {
        let c = self.pool.get().await?;
        Ok(c.query_opt(
            "SELECT data FROM profile_files WHERE profile_id = $1 AND name = $2",
            &[&id, &name],
        )
        .await?
        .map(|r| r.get(0)))
    }

    pub async fn purge_profiles(&self, before_ms: i64) -> Result<u64> {
        let c = self.pool.get().await?;
        Ok(
            c.execute("DELETE FROM profiles WHERE end_ms < $1", &[&before_ms])
                .await?,
        )
    }
}
