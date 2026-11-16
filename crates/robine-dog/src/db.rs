//! SQLite storage: one writer connection, a small pool of readers (WAL mode).

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS logs (
    id          INTEGER PRIMARY KEY,
    ts          INTEGER NOT NULL,              -- epoch milliseconds
    status      TEXT    NOT NULL,
    service     TEXT,
    host        TEXT,
    source      TEXT,
    message     TEXT    NOT NULL DEFAULT '',
    tags        TEXT    NOT NULL DEFAULT '[]', -- JSON array of "key:value"
    attributes  TEXT    NOT NULL DEFAULT '{}', -- nested JSON object
    trace_id    TEXT,
    span_id     TEXT
);
CREATE INDEX IF NOT EXISTS logs_ts ON logs(ts);
CREATE INDEX IF NOT EXISTS logs_service_ts ON logs(service, ts);
CREATE INDEX IF NOT EXISTS logs_trace ON logs(trace_id);

CREATE TABLE IF NOT EXISTS spans (
    id          INTEGER PRIMARY KEY,
    trace_id    TEXT    NOT NULL,              -- decimal, low 64 bits
    span_id     TEXT    NOT NULL,
    parent_id   TEXT    NOT NULL,              -- "0" for a root span
    service     TEXT    NOT NULL,
    name        TEXT    NOT NULL,              -- operation name
    resource    TEXT    NOT NULL,
    type        TEXT,
    env         TEXT,
    version     TEXT,
    start       INTEGER NOT NULL,              -- epoch nanoseconds
    duration    INTEGER NOT NULL,              -- nanoseconds
    error       INTEGER NOT NULL DEFAULT 0,
    top_level   INTEGER NOT NULL DEFAULT 0,    -- service entry span
    meta        TEXT    NOT NULL DEFAULT '{}', -- flat JSON object of strings
    metrics     TEXT    NOT NULL DEFAULT '{}'  -- flat JSON object of numbers
);
CREATE INDEX IF NOT EXISTS spans_start ON spans(start);
CREATE INDEX IF NOT EXISTS spans_trace ON spans(trace_id);
CREATE INDEX IF NOT EXISTS spans_service_start ON spans(service, top_level, start);

CREATE TABLE IF NOT EXISTS facets (
    id       INTEGER PRIMARY KEY,
    dataset  TEXT NOT NULL,                    -- 'logs' | 'spans'
    path     TEXT NOT NULL,                    -- '@http.status_code', 'env', ...
    name     TEXT NOT NULL,
    grp      TEXT NOT NULL DEFAULT 'Attributes',
    UNIQUE(dataset, path)
);
"#;

const DEFAULT_FACETS: &[(&str, &str, &str, &str)] = &[
    ("logs", "env", "Env", "Tags"),
    ("logs", "@http.status_code", "Status Code", "HTTP"),
    ("logs", "@http.method", "Method", "HTTP"),
    ("logs", "@http.url_details.path", "URL Path", "HTTP"),
    ("logs", "@error.kind", "Error Kind", "Error"),
    ("spans", "@http.status_code", "Status Code", "HTTP"),
    ("spans", "@http.method", "Method", "HTTP"),
    ("spans", "@error.type", "Error Type", "Error"),
    ("spans", "@db.system", "Database", "Database"),
    ("spans", "@peer.service", "Peer Service", "Network"),
];

pub struct Db {
    path: PathBuf,
    writer: Mutex<Connection>,
    readers: Mutex<Vec<Connection>>,
}

fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=5000; PRAGMA temp_store=MEMORY;",
    )?;
    Ok(conn)
}

impl Db {
    pub fn open(path: &Path) -> Result<Arc<Self>> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let writer = open(path)?;
        writer.execute_batch(SCHEMA)?;
        {
            let mut stmt = writer.prepare(
                "INSERT OR IGNORE INTO facets (dataset, path, name, grp) VALUES (?, ?, ?, ?)",
            )?;
            let seeded: i64 = writer.query_row("SELECT COUNT(*) FROM facets", [], |r| r.get(0))?;
            if seeded == 0 {
                for f in DEFAULT_FACETS {
                    stmt.execute([f.0, f.1, f.2, f.3])?;
                }
            }
        }
        Ok(Arc::new(Self {
            path: path.to_path_buf(),
            writer: Mutex::new(writer),
            readers: Mutex::new(Vec::new()),
        }))
    }

    /// Runs `f` on the single writer connection, on a blocking thread.
    pub async fn write<T, F>(self: &Arc<Self>, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let db = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = db.writer.lock().unwrap_or_else(|e| e.into_inner());
            f(&mut conn)
        })
        .await?
    }

    /// Runs `f` on a pooled reader connection, on a blocking thread.
    pub async fn read<T, F>(self: &Arc<Self>, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
    {
        let db = self.clone();
        tokio::task::spawn_blocking(move || {
            let conn = match db.readers.lock().unwrap_or_else(|e| e.into_inner()).pop() {
                Some(c) => c,
                None => open(&db.path)?,
            };
            let out = f(&conn);
            let mut pool = db.readers.lock().unwrap_or_else(|e| e.into_inner());
            if pool.len() < 8 {
                pool.push(conn);
            }
            out
        })
        .await?
    }
}
