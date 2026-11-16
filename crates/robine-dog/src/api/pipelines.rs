//! Log pipelines: the UI's Pipelines page, and the same objects on Datadog's
//! public API paths (`/api/v1/logs/config/pipelines`, `…/pipeline-order`).

use super::{ApiError, ApiResult};
use crate::AppState;
use crate::pipelines::grok::Grok;
use crate::pipelines::{self, GrokRules, PipelineDef};
use crate::store::pg::{Pg, new_id};
use axum::Json;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub const KIND: &str = "log_pipeline";

fn from_doc(d: crate::store::pg::Doc) -> Option<PipelineDef> {
    let mut p: PipelineDef = serde_json::from_value(d.body).ok()?;
    p.id = Some(d.id);
    Some(p)
}

pub async fn stored(pg: &Pg) -> anyhow::Result<Vec<PipelineDef>> {
    Ok(pg.docs(KIND).await?.into_iter().filter_map(from_doc).collect())
}

/// Installs the integration pipelines missing, and refreshes the read-only
/// ones already there (keeping whether they are enabled).
pub async fn ensure_integrations(pg: &Pg) -> anyhow::Result<()> {
    let existing = stored(pg).await?;
    for mut p in pipelines::integration_pipelines() {
        match existing.iter().find(|e| e.is_read_only && e.name == p.name) {
            Some(e) => {
                p.is_enabled = e.is_enabled;
                if e.processors != p.processors || e.filter != p.filter {
                    let id = e.id.clone().unwrap_or_default();
                    pg.put_doc(KIND, &id, &serde_json::to_value(&p)?).await?;
                }
            }
            None => {
                pg.put_doc(KIND, &new_id(), &serde_json::to_value(&p)?)
                    .await?;
            }
        }
    }
    Ok(())
}

pub async fn reload(st: &AppState) -> anyhow::Result<()> {
    st.pipelines.load(&stored(&st.store.pg).await?);
    Ok(())
}

pub async fn list(State(st): State<AppState>) -> ApiResult<Vec<PipelineDef>> {
    Ok(Json(stored(&st.store.pg).await?))
}

pub async fn get(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<PipelineDef> {
    st.store
        .pg
        .doc(KIND, &id)
        .await?
        .and_then(from_doc)
        .map(Json)
        .ok_or(ApiError::NotFound)
}

async fn save(st: &AppState, id: &str, mut p: PipelineDef) -> Result<PipelineDef, ApiError> {
    pipelines::validate(&p).map_err(ApiError::BadQuery)?;
    p.id = None;
    let doc = st
        .store
        .pg
        .put_doc(KIND, id, &serde_json::to_value(&p).map_err(anyhow::Error::from)?)
        .await?;
    reload(st).await?;
    from_doc(doc).ok_or_else(|| ApiError::Internal(anyhow::anyhow!("unreadable pipeline")))
}

pub async fn create(State(st): State<AppState>, Json(mut p): Json<PipelineDef>) -> ApiResult<PipelineDef> {
    p.is_read_only = false;
    Ok(Json(save(&st, &new_id(), p).await?))
}

pub async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(mut p): Json<PipelineDef>,
) -> ApiResult<PipelineDef> {
    let current = st
        .store
        .pg
        .doc(KIND, &id)
        .await?
        .and_then(from_doc)
        .ok_or(ApiError::NotFound)?;
    if current.is_read_only {
        // An integration pipeline can only be switched on or off.
        let enabled = p.is_enabled;
        p = current;
        p.is_enabled = enabled;
    }
    Ok(Json(save(&st, &id, p).await?))
}

pub async fn delete(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let current = st
        .store
        .pg
        .doc(KIND, &id)
        .await?
        .and_then(from_doc)
        .ok_or(ApiError::NotFound)?;
    if current.is_read_only {
        return Err(ApiError::BadQuery(
            "integration pipelines cannot be deleted, only disabled".into(),
        ));
    }
    st.store.pg.delete_doc(KIND, &id).await?;
    reload(&st).await?;
    Ok(Json(json!({})))
}

/// An editable copy of a pipeline (how integration pipelines get changed).
pub async fn clone(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<PipelineDef> {
    let mut p = st
        .store
        .pg
        .doc(KIND, &id)
        .await?
        .and_then(from_doc)
        .ok_or(ApiError::NotFound)?;
    p.is_read_only = false;
    p.name = format!("{} (clone)", p.name);
    Ok(Json(save(&st, &new_id(), p).await?))
}

#[derive(Serialize, Deserialize)]
pub struct Order {
    pipeline_ids: Vec<String>,
}

pub async fn order(State(st): State<AppState>) -> ApiResult<Order> {
    Ok(Json(Order {
        pipeline_ids: stored(&st.store.pg)
            .await?
            .into_iter()
            .filter_map(|p| p.id)
            .collect(),
    }))
}

pub async fn set_order(State(st): State<AppState>, Json(o): Json<Order>) -> ApiResult<Order> {
    st.store.pg.reorder_docs(KIND, &o.pipeline_ids).await?;
    reload(&st).await?;
    order(State(st)).await
}

#[derive(Deserialize)]
pub struct GrokTest {
    grok: GrokRules,
    #[serde(default)]
    samples: Vec<String>,
}

#[derive(Serialize)]
pub struct GrokSample {
    sample: String,
    rule: Option<String>,
    attributes: Map<String, Value>,
}

#[derive(Serialize)]
pub struct GrokResult {
    error: Option<String>,
    samples: Vec<GrokSample>,
}

/// The grok editor's live preview: what each sample parses into.
pub async fn grok_test(Json(t): Json<GrokTest>) -> Json<GrokResult> {
    match Grok::new(&t.grok.match_rules, &t.grok.support_rules) {
        Err(e) => Json(GrokResult {
            error: Some(e),
            samples: vec![],
        }),
        Ok(g) => Json(GrokResult {
            error: None,
            samples: t
                .samples
                .into_iter()
                .filter(|s| !s.is_empty())
                .map(|s| {
                    let out = g.parse(&s);
                    GrokSample {
                        rule: out.as_ref().and_then(|o| o.rule.clone()),
                        attributes: out.map(|o| o.attributes).unwrap_or_default(),
                        sample: s,
                    }
                })
                .collect(),
        }),
    }
}
