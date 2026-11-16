use super::{ApiError, ApiResult, Scope, parse_query, without_key};
use crate::AppState;
use crate::query::sql::{self, Dataset, Target};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use rusqlite::types::Value as SqlValue;
use serde::{Deserialize, Serialize};

fn dataset(name: &str) -> Result<Dataset, ApiError> {
    match name {
        "logs" => Ok(Dataset::Logs),
        "spans" => Ok(Dataset::Spans),
        other => Err(ApiError::BadQuery(format!("unknown dataset {other}"))),
    }
}

#[derive(Deserialize)]
pub struct ValuesReq {
    dataset: String,
    #[serde(flatten)]
    scope: Scope,
    facets: Vec<String>,
    limit: Option<i64>,
}

#[derive(Serialize)]
pub struct FacetValue {
    value: String,
    count: i64,
}

#[derive(Serialize)]
pub struct FacetValues {
    path: String,
    values: Vec<FacetValue>,
    /// Numeric facets (measures) also report their range.
    min: Option<f64>,
    max: Option<f64>,
}

#[derive(Serialize)]
pub struct ValuesResp {
    facets: Vec<FacetValues>,
}

fn time_clause(ds: Dataset) -> &'static str {
    match ds {
        Dataset::Logs => "ts >= ? AND ts <= ?",
        Dataset::Spans => "start >= ? AND start <= ?",
    }
}

fn time_params(ds: Dataset, from: i64, to: i64) -> Vec<SqlValue> {
    match ds {
        Dataset::Logs => vec![from.into(), to.into()],
        Dataset::Spans => vec![(from * 1_000_000).into(), (to * 1_000_000 + 999_999).into()],
    }
}

fn table(ds: Dataset) -> &'static str {
    match ds {
        Dataset::Logs => "logs",
        Dataset::Spans => "spans",
    }
}

fn facet_values(
    c: &rusqlite::Connection,
    ds: Dataset,
    scope: &Scope,
    path: &str,
    limit: i64,
) -> anyhow::Result<FacetValues> {
    let expr = without_key(
        crate::query::parser::parse(&scope.query).map_err(|e| anyhow::anyhow!(e.0))?,
        path,
    );
    let f = sql::compile(&expr, ds);
    let mut params = time_params(ds, scope.from, scope.to);
    params.extend(f.params);
    let base = format!("FROM {} WHERE {} AND {}", table(ds), time_clause(ds), f.sql);
    let target = sql::resolve(path, ds);
    let mut out = FacetValues {
        path: path.to_string(),
        values: Vec::new(),
        min: None,
        max: None,
    };
    let (sql, extra_first, extra_last): (String, Vec<SqlValue>, Vec<SqlValue>) = match &target {
        Target::Tag(key) => {
            let prefix = format!("{key}:");
            (
                format!(
                    "SELECT substr(j.value, ?) AS v, COUNT(*) AS n FROM (SELECT tags {base}) AS l, json_each(l.tags) AS j \
                     WHERE j.value LIKE ? ESCAPE '\\' GROUP BY v ORDER BY n DESC LIMIT ?"
                ),
                vec![(prefix.chars().count() as i64 + 1).into()],
                vec![
                    (sql::like_pattern(&prefix, false) + "%").into(),
                    limit.into(),
                ],
            )
        }
        Target::Duration => {
            let (min, max): (Option<f64>, Option<f64>) = c.query_row(
                &format!("SELECT MIN(duration), MAX(duration) {base}"),
                rusqlite::params_from_iter(params.iter()),
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            out.min = min;
            out.max = max;
            return Ok(out);
        }
        t => {
            let mut expr_params = Vec::new();
            let e = sql::target_expr(t, &mut expr_params);
            (
                format!(
                    "SELECT CAST({e} AS TEXT) AS v, COUNT(*) AS n {base} AND {e} IS NOT NULL GROUP BY v ORDER BY n DESC LIMIT ?"
                ),
                expr_params.clone(),
                {
                    let mut v = expr_params;
                    v.push(limit.into());
                    v
                },
            )
        }
    };
    let mut all = extra_first;
    all.extend(params);
    all.extend(extra_last);
    out.values = c
        .prepare(&sql)?
        .query_map(rusqlite::params_from_iter(all), |r| {
            Ok(FacetValue {
                value: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                count: r.get(1)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(out)
}

pub async fn values(
    State(st): State<AppState>,
    Json(req): Json<ValuesReq>,
) -> ApiResult<ValuesResp> {
    let ds = dataset(&req.dataset)?;
    parse_query(&req.scope.query)?;
    let limit = req.limit.unwrap_or(10).clamp(1, 100);
    let facets = st
        .db
        .read(move |c| {
            req.facets
                .iter()
                .map(|p| facet_values(c, ds, &req.scope, p, limit))
                .collect::<anyhow::Result<Vec<_>>>()
        })
        .await?;
    Ok(Json(ValuesResp { facets }))
}

#[derive(Serialize, Deserialize)]
pub struct FacetDef {
    #[serde(default)]
    id: i64,
    #[serde(default)]
    dataset: String,
    path: String,
    name: String,
    #[serde(default = "default_group")]
    group: String,
}

fn default_group() -> String {
    "Attributes".into()
}

#[derive(Deserialize)]
pub struct ListParams {
    dataset: String,
}

pub async fn list(
    State(st): State<AppState>,
    Query(p): Query<ListParams>,
) -> ApiResult<Vec<FacetDef>> {
    dataset(&p.dataset)?;
    let defs = st
        .db
        .read(move |c| {
            Ok(c.prepare("SELECT id, dataset, path, name, grp FROM facets WHERE dataset = ? ORDER BY grp, name")?
                .query_map([&p.dataset], |r| {
                    Ok(FacetDef { id: r.get(0)?, dataset: r.get(1)?, path: r.get(2)?, name: r.get(3)?, group: r.get(4)? })
                })?
                .collect::<Result<Vec<_>, _>>()?)
        })
        .await?;
    Ok(Json(defs))
}

pub async fn create(
    State(st): State<AppState>,
    Json(mut def): Json<FacetDef>,
) -> ApiResult<FacetDef> {
    dataset(&def.dataset)?;
    if def.path.trim().is_empty() {
        return Err(ApiError::BadQuery("facet path is required".into()));
    }
    let d = st
        .db
        .write(move |c| {
            c.execute(
                "INSERT INTO facets (dataset, path, name, grp) VALUES (?, ?, ?, ?)
                 ON CONFLICT(dataset, path) DO UPDATE SET name = excluded.name, grp = excluded.grp",
                [&def.dataset, &def.path, &def.name, &def.group],
            )?;
            def.id = c.query_row(
                "SELECT id FROM facets WHERE dataset = ? AND path = ?",
                [&def.dataset, &def.path],
                |r| r.get(0),
            )?;
            Ok(def)
        })
        .await?;
    Ok(Json(d))
}

pub async fn delete(
    State(st): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    st.db
        .write(move |c| Ok(c.execute("DELETE FROM facets WHERE id = ?", [id])?))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
