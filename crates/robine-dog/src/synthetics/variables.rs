//! Variables of the tests: global ones (`/api/v1/synthetics/variables`),
//! local ones generated from patterns (`{{ numeric(4) }}`, `{{ uuid }}`,
//! `{{ date(1d, YYYY-MM-DD) }}`), and those extracted by earlier steps. They
//! are used as `{{ NAME }}` in requests and browser steps.

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use axum::Json;
use axum::extract::{Path, State};
use rand::Rng;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::LazyLock;

pub const KIND: &str = "synthetics_variable";

static TEMPLATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{\s*([^{}]+?)\s*\}\}").unwrap());
static NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Z][A-Z0-9_]*$").unwrap());

pub type Vars = HashMap<String, String>;

/// `{{ NAME }}` → its value; unknown names stay.
pub fn render(s: &str, vars: &Vars) -> String {
    if !s.contains("{{") {
        return s.to_string();
    }
    TEMPLATE
        .replace_all(s, |c: &regex::Captures| {
            vars.get(c[1].trim())
                .cloned()
                .unwrap_or_else(|| c[0].to_string())
        })
        .into_owned()
}

/// Every string of a JSON value, rendered.
pub fn render_value(v: &Value, vars: &Vars) -> Value {
    match v {
        Value::String(s) => Value::String(render(s, vars)),
        Value::Array(a) => Value::Array(a.iter().map(|x| render_value(x, vars)).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, x)| (render(k, vars), render_value(x, vars)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// The value of a pattern: its `{{ … }}` functions evaluated.
pub fn generate(pattern: &str, now_ms: i64) -> String {
    TEMPLATE
        .replace_all(pattern, |c: &regex::Captures| {
            call(c[1].trim(), now_ms).unwrap_or_else(|| c[0].to_string())
        })
        .into_owned()
}

fn random_from(chars: &[u8], n: usize) -> String {
    let mut rng = rand::rng();
    (0..n.min(256))
        .map(|_| chars[rng.random_range(0..chars.len())] as char)
        .collect()
}

/// `1d`, `-2h`, `+30m`, `0` → milliseconds.
fn offset_ms(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() || s == "0" {
        return Some(0);
    }
    let unit_at = s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len());
    let n: i64 = s[..unit_at].trim_start_matches('+').parse().ok()?;
    let unit = match &s[unit_at..] {
        "" | "ms" => 1,
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return None,
    };
    Some(n * unit)
}

fn call(expr: &str, now_ms: i64) -> Option<String> {
    let (name, args) = match expr.find('(') {
        Some(i) => (
            expr[..i].trim(),
            expr[i + 1..]
                .trim_end_matches(')')
                .split(',')
                .map(str::trim)
                .collect::<Vec<_>>(),
        ),
        None => (expr, vec![]),
    };
    let n = || {
        args.first()
            .and_then(|a| a.parse::<usize>().ok())
            .unwrap_or(8)
    };
    Some(match name {
        "numeric" => random_from(b"0123456789", n()),
        "alphabetic" => random_from(b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ", n()),
        "alphanumeric" => random_from(
            b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
            n(),
        ),
        "uuid" => {
            let b: [u8; 16] = rand::random();
            let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
            format!(
                "{}-{}-4{}-{:x}{}-{}",
                &h[0..8],
                &h[8..12],
                &h[13..16],
                8 + (b[8] & 3),
                &h[17..20],
                &h[20..32]
            )
        }
        "timestamp" => {
            let off = args.first().and_then(|a| offset_ms(a)).unwrap_or(0);
            let t = now_ms + off;
            match args.get(1).copied().unwrap_or("s") {
                "ms" => t.to_string(),
                _ => (t / 1000).to_string(),
            }
        }
        "date" => {
            let off = args.first().and_then(|a| offset_ms(a)).unwrap_or(0);
            let fmt = args.get(1).copied().unwrap_or("YYYY-MM-DD");
            let dt = chrono::DateTime::from_timestamp_millis(now_ms + off)?;
            [
                ("YYYY", "%Y"),
                ("YY", "%y"),
                ("MM", "%m"),
                ("DD", "%d"),
                ("HH", "%H"),
                ("mm", "%M"),
                ("ss", "%S"),
            ]
            .iter()
            .fold(fmt.to_string(), |acc, (token, spec)| {
                acc.replace(token, &dt.format(spec).to_string())
            })
        }
        _ => return None,
    })
}

// --- global variables ------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VarValue {
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub secure: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalVariable {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub value: VarValue,
}

/// As the API shows it: secure values never leave the server.
fn shown(v: &GlobalVariable) -> Value {
    let mut out = serde_json::to_value(v).unwrap_or_default();
    if v.value.secure {
        out["value"] = json!({"secure": true});
    }
    out
}

pub async fn all(st: &AppState) -> anyhow::Result<Vec<GlobalVariable>> {
    Ok(st
        .store
        .pg
        .docs(KIND)
        .await?
        .into_iter()
        .filter_map(|d| serde_json::from_value(d.body).ok())
        .collect())
}

fn check(v: &GlobalVariable) -> Result<(), ApiError> {
    if !NAME.is_match(&v.name) {
        return Err(ApiError::BadQuery(
            "a variable's name is in capitals, digits and underscores (API_TOKEN)".into(),
        ));
    }
    Ok(())
}

async fn unique(st: &AppState, v: &GlobalVariable) -> Result<(), ApiError> {
    if all(st)
        .await?
        .iter()
        .any(|o| o.name == v.name && o.id != v.id)
    {
        return Err(ApiError::BadQuery(format!(
            "there is already a variable {}",
            v.name
        )));
    }
    Ok(())
}

pub async fn list(State(st): State<AppState>) -> ApiResult<Value> {
    let mut vars = all(&st).await?;
    vars.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(
        json!({"variables": vars.iter().map(shown).collect::<Vec<_>>()}),
    ))
}

pub async fn get(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let d = st
        .store
        .pg
        .doc(KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let v: GlobalVariable = serde_json::from_value(d.body).map_err(anyhow::Error::from)?;
    Ok(Json(shown(&v)))
}

pub async fn create(
    State(st): State<AppState>,
    Json(mut v): Json<GlobalVariable>,
) -> ApiResult<Value> {
    check(&v)?;
    v.id = crate::store::pg::new_id().to_lowercase();
    unique(&st, &v).await?;
    st.store
        .pg
        .put_doc(
            KIND,
            &v.id,
            &serde_json::to_value(&v).map_err(anyhow::Error::from)?,
        )
        .await?;
    Ok(Json(shown(&v)))
}

pub async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(mut v): Json<GlobalVariable>,
) -> ApiResult<Value> {
    let d = st
        .store
        .pg
        .doc(KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let old: GlobalVariable = serde_json::from_value(d.body).map_err(anyhow::Error::from)?;
    check(&v)?;
    v.id = id.clone();
    // A secure value left empty keeps the stored one: the UI never sees it.
    if v.value.value.is_empty() && old.value.secure {
        v.value.value = old.value.value;
    }
    unique(&st, &v).await?;
    st.store
        .pg
        .put_doc(
            KIND,
            &id,
            &serde_json::to_value(&v).map_err(anyhow::Error::from)?,
        )
        .await?;
    Ok(Json(shown(&v)))
}

pub async fn delete(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    if !st.store.pg.delete_doc(KIND, &id).await? {
        return Err(ApiError::NotFound);
    }
    Ok(Json(json!({})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendering() {
        let vars = Vars::from([
            ("HOST".to_string(), "shop.example".to_string()),
            ("TOKEN".to_string(), "abc".to_string()),
        ]);
        assert_eq!(
            render("https://{{ HOST }}/x?t={{TOKEN}}&u={{ OTHER }}", &vars),
            "https://shop.example/x?t=abc&u={{ OTHER }}"
        );
        let v = render_value(
            &json!({"h": {"Authorization": "Bearer {{ TOKEN }}"}, "n": 1}),
            &vars,
        );
        assert_eq!(v["h"]["Authorization"], "Bearer abc");
    }

    #[test]
    fn patterns() {
        let now = 1_767_225_600_000; // 2026-01-01T00:00:00Z
        let n = generate("{{ numeric(6) }}", now);
        assert!(n.len() == 6 && n.chars().all(|c| c.is_ascii_digit()));
        assert_eq!(generate("{{ alphabetic(4) }}", now).len(), 4);
        assert_eq!(generate("id-{{ uuid }}", now).len(), 39);
        assert_eq!(generate("{{ timestamp(0, s) }}", now), "1767225600");
        assert_eq!(generate("{{ timestamp(1h, ms) }}", now), "1767229200000");
        assert_eq!(generate("{{ date(1d, DD/MM/YYYY) }}", now), "02/01/2026");
        assert_eq!(generate("{{ date(-1d, YYYY-MM-DD) }}", now), "2025-12-31");
        assert_eq!(generate("{{ nope(1) }}", now), "{{ nope(1) }}");
    }
}
