//! Batched inserts: the intakes hand rows over and answer at once; one task per
//! table gathers them for up to [`LINGER`] and writes them in a few large
//! `INSERT`s, which GreptimeDB ingests far better than many small ones.

use super::greptime::Greptime;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

const LINGER: Duration = Duration::from_millis(200);
const MAX_ROWS: usize = 5_000;
const MAX_STATEMENT_BYTES: usize = 8 << 20;

pub trait Insertable: Send + Sync + 'static {
    const TABLE: &'static str;
    /// Column list, e.g. `("ts", "id", "message")`.
    const COLUMNS: &'static str;
    /// Appends this row's `(…)` tuple of literals.
    fn values(&self, out: &mut String);
}

enum Msg<T> {
    Rows(Vec<T>),
    Flush(oneshot::Sender<()>),
}

pub struct Writer<T> {
    tx: mpsc::Sender<Msg<T>>,
}

impl<T> Clone for Writer<T> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

impl<T: Insertable> Writer<T> {
    pub fn spawn(gt: Greptime) -> Self {
        // Bounded: when GreptimeDB falls behind, the intakes wait instead of
        // piling rows up in memory.
        let (tx, rx) = mpsc::channel(256);
        tokio::spawn(run(gt, rx));
        Self { tx }
    }

    pub async fn send(&self, rows: Vec<T>) -> anyhow::Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        self.tx
            .send(Msg::Rows(rows))
            .await
            .map_err(|_| anyhow::anyhow!("{} writer stopped", T::TABLE))
    }

    /// Waits until everything sent so far is written.
    pub async fn flush(&self) {
        let (ack, done) = oneshot::channel();
        if self.tx.send(Msg::Flush(ack)).await.is_ok() {
            done.await.ok();
        }
    }
}

async fn run<T: Insertable>(gt: Greptime, mut rx: mpsc::Receiver<Msg<T>>) {
    let mut buf: Vec<T> = Vec::new();
    loop {
        let msg = if buf.is_empty() {
            rx.recv().await
        } else {
            match tokio::time::timeout(LINGER, rx.recv()).await {
                Ok(m) => m,
                Err(_) => {
                    write(&gt, std::mem::take(&mut buf)).await;
                    continue;
                }
            }
        };
        match msg {
            Some(Msg::Rows(rows)) => {
                buf.extend(rows);
                if buf.len() >= MAX_ROWS {
                    write(&gt, std::mem::take(&mut buf)).await;
                }
            }
            Some(Msg::Flush(ack)) => {
                write(&gt, std::mem::take(&mut buf)).await;
                ack.send(()).ok();
            }
            None => {
                write(&gt, std::mem::take(&mut buf)).await;
                return;
            }
        }
    }
}

async fn write<T: Insertable>(gt: &Greptime, rows: Vec<T>) {
    if rows.is_empty() {
        return;
    }
    let head = format!("INSERT INTO {} {} VALUES ", T::TABLE, T::COLUMNS);
    let mut sql = head.clone();
    let mut in_stmt = 0;
    for (i, r) in rows.iter().enumerate() {
        if in_stmt > 0 {
            sql.push(',');
        }
        r.values(&mut sql);
        in_stmt += 1;
        if sql.len() >= MAX_STATEMENT_BYTES || i == rows.len() - 1 {
            exec_with_retry(gt, &sql, in_stmt, T::TABLE).await;
            sql.clear();
            sql.push_str(&head);
            in_stmt = 0;
        }
    }
}

async fn exec_with_retry(gt: &Greptime, sql: &str, rows: usize, table: &str) {
    let mut delay = Duration::from_millis(500);
    for attempt in 1..=6 {
        match gt.exec(sql).await {
            Ok(()) => return,
            Err(e) if attempt == 6 => {
                tracing::error!(error = %e, rows, table, "dropping rows after repeated insert failures");
            }
            Err(e) => {
                tracing::warn!(error = %e, rows, table, attempt, "insert failed, retrying");
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
        }
    }
}

static LAST_ID: AtomicI64 = AtomicI64::new(0);

/// Unique, increasing row ids in ingestion order: the arrival millisecond
/// shifted by 12 bits, plus a sequence. They stay below 2^53, so the UI can
/// hold them as plain JavaScript numbers (until 2039).
pub fn next_id() -> i64 {
    let floor = crate::intake::logs::now_ms() << 12;
    let mut prev = LAST_ID.load(Ordering::Relaxed);
    loop {
        let id = floor.max(prev + 1);
        match LAST_ID.compare_exchange_weak(prev, id, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return id,
            Err(p) => prev = p,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_increase_and_stay_js_safe() {
        let a = next_id();
        let b = next_id();
        assert!(b > a);
        assert!(b < (1i64 << 53));
    }
}
