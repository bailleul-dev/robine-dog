//! JSON API used by the web UI, under `/api/ui`, behind a session.

pub mod apm;
pub mod facets;
pub mod logs;
pub mod spans;

use crate::query::parser::{self, Expr};
use crate::query::sql::{self, Dataset, Filter};
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

pub enum ApiError {
    BadQuery(String),
    NotFound,
    Internal(anyhow::Error),
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError::Internal(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            ApiError::BadQuery(m) => {
                (StatusCode::BAD_REQUEST, Json(json!({"error": m}))).into_response()
            }
            ApiError::NotFound => {
                (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response()
            }
            ApiError::Internal(e) => {
                tracing::error!(error = %e, "api error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": "internal error"})),
                )
                    .into_response()
            }
        }
    }
}

pub type ApiResult<T> = Result<Json<T>, ApiError>;

/// Time range in epoch milliseconds, and a search query.
#[derive(Debug, Clone, Deserialize)]
pub struct Scope {
    #[serde(default)]
    pub query: String,
    pub from: i64,
    pub to: i64,
}

pub fn parse_query(q: &str) -> Result<Expr, ApiError> {
    parser::parse(q).map_err(|e| ApiError::BadQuery(e.0))
}

pub fn compile(q: &str, ds: Dataset) -> Result<Filter, ApiError> {
    Ok(sql::compile(&parse_query(q)?, ds))
}

/// Picks a round bucket size giving about `target` buckets over the range.
pub fn interval_ms(from: i64, to: i64, target: i64) -> i64 {
    const STEPS: &[i64] = &[
        1_000,
        2_000,
        5_000,
        10_000,
        15_000,
        30_000,
        60_000,
        120_000,
        300_000,
        600_000,
        900_000,
        1_800_000,
        3_600_000,
        7_200_000,
        10_800_000,
        21_600_000,
        43_200_000,
        86_400_000,
        172_800_000,
        604_800_000,
    ];
    let raw = ((to - from).max(1)) / target.max(1);
    *STEPS
        .iter()
        .find(|s| **s >= raw)
        .unwrap_or(STEPS.last().unwrap())
}

/// Removes the top-level clauses on `key`, so a facet's counts ignore its own selection.
pub fn without_key(e: Expr, key: &str) -> Expr {
    fn mentions_only(e: &Expr, key: &str) -> bool {
        match e {
            Expr::Field { key: k, .. } => k == key,
            Expr::Not(inner) => mentions_only(inner, key),
            Expr::Or(items) | Expr::And(items) => items.iter().all(|i| mentions_only(i, key)),
            _ => false,
        }
    }
    if mentions_only(&e, key) {
        return Expr::All;
    }
    match e {
        Expr::And(items) => {
            let kept: Vec<Expr> = items
                .into_iter()
                .filter(|i| !mentions_only(i, key))
                .collect();
            match kept.len() {
                0 => Expr::All,
                1 => kept.into_iter().next().unwrap(),
                _ => Expr::And(kept),
            }
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervals_are_round() {
        assert_eq!(interval_ms(0, 15 * 60_000, 60), 15_000);
        assert_eq!(interval_ms(0, 86_400_000, 60), 1_800_000);
    }

    #[test]
    fn facet_counts_ignore_their_own_clause() {
        let e = parser::parse("service:(a OR b) -service:c env:prod").unwrap();
        assert_eq!(
            without_key(e, "service"),
            parser::parse("env:prod").unwrap()
        );
        assert_eq!(
            without_key(parser::parse("service:a").unwrap(), "service"),
            Expr::All
        );
        let e = parser::parse("service:a OR env:prod").unwrap();
        assert_eq!(without_key(e.clone(), "service"), e);
    }
}
