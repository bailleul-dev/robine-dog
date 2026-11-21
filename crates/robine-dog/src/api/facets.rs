use super::{ApiError, ApiResult, Scope, parse_query, time_clause, without_key};
use crate::AppState;
use crate::query::sql::{self, Dataset, Target};
use crate::store::greptime::{Greptime, f64_at, i64_at, lit, str_at};
use crate::store::pg::FacetDef;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
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

async fn facet_values(
    gt: &Greptime,
    ds: Dataset,
    scope: &Scope,
    path: &str,
    limit: i64,
) -> anyhow::Result<FacetValues> {
    let expr = without_key(
        parse_query(&scope.query).map_err(|e| match e {
            ApiError::BadQuery(m) => anyhow::anyhow!(m),
            e => anyhow::anyhow!("{e:?}"),
        })?,
        path,
    );
    let base = format!(
        "FROM {} WHERE {} AND {}",
        super::table_of(ds, &scope.query),
        time_clause(ds, scope.from, scope.to),
        sql::compile(&expr, ds)
    );
    let mut out = FacetValues {
        path: path.to_string(),
        values: Vec::new(),
        min: None,
        max: None,
    };
    let sql = match sql::resolve(path, ds) {
        Target::Tag(key) => {
            let prefix = format!("{key}:");
            format!(
                "SELECT substr(v, {}) AS val, count(*) AS n
                 FROM (SELECT unnest(string_to_array(tags, ',')) AS v {base})
                 WHERE starts_with(v, {}) GROUP BY val ORDER BY n DESC LIMIT {limit}",
                prefix.chars().count() + 1,
                lit(&prefix)
            )
        }
        Target::Duration => {
            let rows = gt
                .query(&format!("SELECT min(duration), max(duration) {base}"))
                .await?;
            if let Some(r) = rows.first() {
                out.min = f64_at(r, 0);
                out.max = f64_at(r, 1);
            }
            return Ok(out);
        }
        t => {
            let e = sql::target_expr(&t);
            format!(
                "SELECT {e} AS val, count(*) AS n {base} AND {e} IS NOT NULL
                 GROUP BY val ORDER BY n DESC LIMIT {limit}"
            )
        }
    };
    out.values = gt
        .query(&sql)
        .await?
        .iter()
        .map(|r| FacetValue {
            value: str_at(r, 0).unwrap_or_default(),
            count: i64_at(r, 1),
        })
        .collect();
    Ok(out)
}

pub async fn values(
    State(st): State<AppState>,
    Json(req): Json<ValuesReq>,
) -> ApiResult<ValuesResp> {
    let ds = dataset(&req.dataset)?;
    parse_query(&req.scope.query)?;
    let limit = req.limit.unwrap_or(10).clamp(1, 100);
    let gt = &st.store.gt;
    let facets = futures::future::try_join_all(
        req.facets
            .iter()
            .map(|p| facet_values(gt, ds, &req.scope, p, limit)),
    )
    .await?;
    Ok(Json(ValuesResp { facets }))
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
    Ok(Json(st.store.pg.facets(&p.dataset).await?))
}

pub async fn create(State(st): State<AppState>, Json(def): Json<FacetDef>) -> ApiResult<FacetDef> {
    dataset(&def.dataset)?;
    if def.path.trim().is_empty() {
        return Err(ApiError::BadQuery("facet path is required".into()));
    }
    Ok(Json(st.store.pg.upsert_facet(def).await?))
}

pub async fn delete(
    State(st): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    st.store.pg.delete_facet(id).await?;
    Ok(StatusCode::NO_CONTENT)
}
