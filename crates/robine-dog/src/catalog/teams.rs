//! Teams, as Datadog's `/api/v2/team`: a name and a handle, members (users
//! by id, which robine-dog takes to be e-mail addresses), links. Services
//! belong to the team their definition names (`team: <handle>`).

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use crate::store::pg::new_id;
use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

pub const KIND: &str = "team";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Member {
    pub user: String,
    /// `admin`, or none.
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Link {
    pub id: String,
    pub label: String,
    pub url: String,
    #[serde(default)]
    pub position: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Team {
    pub name: String,
    pub handle: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub members: Vec<Member>,
    #[serde(default)]
    pub links: Vec<Link>,
}

fn validate(t: &Team) -> Result<(), String> {
    if t.name.trim().is_empty() {
        return Err("name is required".into());
    }
    if t.handle.is_empty()
        || t.handle.len() > 195
        || !t
            .handle
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-_.".contains(c))
    {
        return Err("handle is lowercase letters, digits, -, _ and .".into());
    }
    Ok(())
}

fn out(id: &str, t: &Team, created: i64, updated: i64) -> Value {
    json!({
        "id": id,
        "type": "team",
        "attributes": {
            "name": t.name, "handle": t.handle, "description": t.description,
            "summary": t.description.as_deref().map(|d| d.chars().take(120).collect::<String>()),
            "avatar": t.avatar, "banner": null,
            "user_count": t.members.len(), "link_count": t.links.len(),
            "created_at": crate::logconfig::iso(created),
            "modified_at": crate::logconfig::iso(updated),
        },
        "relationships": {
            "team_links": {"data": t.links.iter().map(|l| json!({"id": l.id, "type": "team_links"})).collect::<Vec<_>>()},
            "user_team_permissions": {"data": null},
        }
    })
}

pub async fn all(st: &AppState) -> anyhow::Result<Vec<(String, Team, i64, i64)>> {
    let mut v: Vec<(String, Team, i64, i64)> = st
        .store
        .pg
        .docs(KIND)
        .await?
        .into_iter()
        .filter_map(|d| {
            Some((
                d.id,
                serde_json::from_value(d.body).ok()?,
                d.created_at,
                d.updated_at,
            ))
        })
        .collect();
    v.sort_by_key(|a| a.1.name.to_lowercase());
    Ok(v)
}

async fn load(st: &AppState, id: &str) -> Result<Team, ApiError> {
    let d = st.store.pg.doc(KIND, id).await?.ok_or(ApiError::NotFound)?;
    serde_json::from_value(d.body).map_err(|e| ApiError::Internal(e.into()))
}

async fn save(st: &AppState, id: &str, t: &Team) -> Result<(i64, i64), ApiError> {
    let d = st
        .store
        .pg
        .put_doc(
            KIND,
            id,
            &serde_json::to_value(t).map_err(anyhow::Error::from)?,
        )
        .await?;
    Ok((d.created_at, d.updated_at))
}

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(rename = "filter[keyword]")]
    keyword: Option<String>,
}

/// `GET /api/v2/team`.
pub async fn list(State(st): State<AppState>, Query(q): Query<ListQuery>) -> ApiResult<Value> {
    let k = q.keyword.unwrap_or_default().to_lowercase();
    let data: Vec<Value> = all(&st)
        .await?
        .iter()
        .filter(|(_, t, ..)| {
            k.is_empty() || t.name.to_lowercase().contains(&k) || t.handle.contains(&k)
        })
        .map(|(id, t, c, u)| out(id, t, *c, *u))
        .collect();
    let n = data.len();
    Ok(Json(
        json!({"data": data, "meta": {"pagination": {"total": n}}}),
    ))
}

/// The UI's view: teams with their members, links and services.
pub async fn ui_list(State(st): State<AppState>) -> ApiResult<Value> {
    let defs = super::definitions::summaries(&st).await?;
    let mut services: HashMap<String, Vec<String>> = HashMap::new();
    for d in defs {
        if let Some(t) = d.team {
            services.entry(t).or_default().push(d.service);
        }
    }
    let data: Vec<Value> = all(&st)
        .await?
        .into_iter()
        .map(|(id, t, _, updated)| {
            json!({
                "id": id, "name": t.name, "handle": t.handle, "description": t.description,
                "members": t.members, "links": t.links,
                "services": services.get(&t.handle).cloned().unwrap_or_default(),
                "updated_at": updated,
            })
        })
        .collect();
    Ok(Json(json!(data)))
}

fn attributes(v: &Value) -> Value {
    v.pointer("/data/attributes")
        .cloned()
        .unwrap_or_else(|| v.clone())
}

/// `POST /api/v2/team`.
pub async fn create(State(st): State<AppState>, Json(v): Json<Value>) -> ApiResult<Value> {
    let a = attributes(&v);
    let t = Team {
        name: a["name"].as_str().unwrap_or("").trim().to_string(),
        handle: a["handle"].as_str().unwrap_or("").to_string(),
        description: a["description"].as_str().map(String::from),
        avatar: a["avatar"].as_str().map(String::from),
        ..Default::default()
    };
    validate(&t).map_err(ApiError::BadQuery)?;
    if all(&st)
        .await?
        .iter()
        .any(|(_, o, ..)| o.handle == t.handle)
    {
        return Err(ApiError::BadQuery(format!(
            "the handle {} is taken",
            t.handle
        )));
    }
    let id = new_id();
    let (c, u) = save(&st, &id, &t).await?;
    Ok(Json(json!({"data": out(&id, &t, c, u)})))
}

pub async fn get(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let d = st
        .store
        .pg
        .doc(KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let t: Team = serde_json::from_value(d.body).map_err(anyhow::Error::from)?;
    Ok(Json(
        json!({"data": out(&id, &t, d.created_at, d.updated_at)}),
    ))
}

/// `PATCH /api/v2/team/{team_id}`.
pub async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    let mut t = load(&st, &id).await?;
    let a = attributes(&v);
    if let Some(n) = a["name"].as_str() {
        t.name = n.trim().to_string();
    }
    if let Some(h) = a["handle"].as_str()
        && h != t.handle
    {
        if all(&st)
            .await?
            .iter()
            .any(|(i, o, ..)| o.handle == h && *i != id)
        {
            return Err(ApiError::BadQuery(format!("the handle {h} is taken")));
        }
        t.handle = h.to_string();
    }
    if a.get("description").is_some() {
        t.description = a["description"].as_str().map(String::from);
    }
    if a.get("avatar").is_some() {
        t.avatar = a["avatar"].as_str().map(String::from);
    }
    validate(&t).map_err(ApiError::BadQuery)?;
    let (c, u) = save(&st, &id, &t).await?;
    Ok(Json(json!({"data": out(&id, &t, c, u)})))
}

pub async fn delete(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    if !st.store.pg.delete_doc(KIND, &id).await? {
        return Err(ApiError::NotFound);
    }
    Ok(Json(json!({})))
}

fn membership(team: &str, m: &Member) -> Value {
    json!({
        "id": format!("TeamMembership-{team}-{}", m.user),
        "type": "team_memberships",
        "attributes": {"role": m.role, "provisioned_by": "robine-dog"},
        "relationships": {"user": {"data": {"id": m.user, "type": "users"}}},
    })
}

/// `GET /api/v2/team/{team_id}/memberships`.
pub async fn members(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let t = load(&st, &id).await?;
    let data: Vec<Value> = t.members.iter().map(|m| membership(&id, m)).collect();
    Ok(Json(json!({"data": data})))
}

/// `POST /api/v2/team/{team_id}/memberships`.
pub async fn add_member(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    let mut t = load(&st, &id).await?;
    let user = v
        .pointer("/data/relationships/user/data/id")
        .or_else(|| v.get("user"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .ok_or_else(|| ApiError::BadQuery("the user is missing".into()))?
        .to_string();
    let role = v
        .pointer("/data/attributes/role")
        .or_else(|| v.get("role"))
        .and_then(Value::as_str)
        .map(String::from);
    if role.as_deref().is_some_and(|r| r != "admin") {
        return Err(ApiError::BadQuery("role is admin, or none".into()));
    }
    let m = Member {
        user: user.clone(),
        role,
        created_at: crate::intake::logs::now_ms(),
    };
    t.members.retain(|o| o.user != user);
    t.members.push(m.clone());
    save(&st, &id, &t).await?;
    Ok(Json(json!({"data": membership(&id, &m)})))
}

/// `DELETE /api/v2/team/{team_id}/memberships/{user_id}`.
pub async fn remove_member(
    State(st): State<AppState>,
    Path((id, user)): Path<(String, String)>,
) -> ApiResult<Value> {
    let mut t = load(&st, &id).await?;
    let n = t.members.len();
    t.members.retain(|m| m.user != user);
    if t.members.len() == n {
        return Err(ApiError::NotFound);
    }
    save(&st, &id, &t).await?;
    Ok(Json(json!({})))
}

fn link_out(team: &str, l: &Link) -> Value {
    json!({
        "id": l.id,
        "type": "team_links",
        "attributes": {"label": l.label, "url": l.url, "position": l.position, "team_id": team},
    })
}

/// `GET /api/v2/team/{team_id}/links`.
pub async fn links(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let t = load(&st, &id).await?;
    Ok(Json(
        json!({"data": t.links.iter().map(|l| link_out(&id, l)).collect::<Vec<_>>()}),
    ))
}

/// `POST /api/v2/team/{team_id}/links`.
pub async fn add_link(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    let mut t = load(&st, &id).await?;
    let a = attributes(&v);
    let label = a["label"].as_str().unwrap_or("").trim().to_string();
    let url = a["url"].as_str().unwrap_or("").trim().to_string();
    if label.is_empty() || !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(ApiError::BadQuery(
            "a link has a label and an http(s) url".into(),
        ));
    }
    let l = Link {
        id: new_id(),
        label,
        url,
        position: a["position"].as_i64().unwrap_or(t.links.len() as i64),
    };
    t.links.push(l.clone());
    t.links.sort_by_key(|l| l.position);
    save(&st, &id, &t).await?;
    Ok(Json(json!({"data": link_out(&id, &l)})))
}

/// `DELETE /api/v2/team/{team_id}/links/{link_id}`.
pub async fn remove_link(
    State(st): State<AppState>,
    Path((id, link)): Path<(String, String)>,
) -> ApiResult<Value> {
    let mut t = load(&st, &id).await?;
    let n = t.links.len();
    t.links.retain(|l| l.id != link);
    if t.links.len() == n {
        return Err(ApiError::NotFound);
    }
    save(&st, &id, &t).await?;
    Ok(Json(json!({})))
}
