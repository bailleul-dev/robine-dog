//! Sign-in with Robine ID (OpenID Connect authorization code flow, PKCE S256
//! and nonce, RS256 ID token checked against the issuer's JWKS), and the
//! HMAC-signed session cookie that follows.

use crate::AppState;
use crate::config::Oidc;
use anyhow::{Context, Result, anyhow, bail};
use axum::extract::{Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use axum::{Extension, Json};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use tokio::sync::OnceCell;

const SESSION_COOKIE: &str = "robine_dog_session";
const TX_COOKIE: &str = "robine_dog_oidc";
const SESSION_SECONDS: i64 = 12 * 3600;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub sub: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub exp: i64,
}

#[derive(Serialize, Deserialize)]
struct Transaction {
    state: String,
    nonce: String,
    verifier: String,
    return_to: String,
}

#[derive(Deserialize, Clone)]
struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
}

#[derive(Deserialize)]
struct IdClaims {
    sub: String,
    nonce: Option<String>,
    name: Option<String>,
    email: Option<String>,
    preferred_username: Option<String>,
}

pub struct Auth {
    http: reqwest::Client,
    discovery: OnceCell<Discovery>,
}

impl Auth {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("http client"),
            discovery: OnceCell::new(),
        }
    }

    async fn discovery(&self, oidc: &Oidc) -> Result<Discovery> {
        self.discovery
            .get_or_try_init(|| async {
                let url = format!("{}/.well-known/openid-configuration", oidc.issuer);
                let d: Discovery = self
                    .http
                    .get(&url)
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                if d.issuer.trim_end_matches('/') != oidc.issuer {
                    bail!(
                        "discovery issuer {} does not match {}",
                        d.issuer,
                        oidc.issuer
                    );
                }
                Ok(d)
            })
            .await
            .cloned()
    }
}

fn now() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

fn random() -> String {
    let mut b = [0u8; 32];
    rand::rng().fill_bytes(&mut b);
    B64.encode(b)
}

fn sign<T: Serialize>(value: &T, secret: &[u8]) -> String {
    let body = B64.encode(serde_json::to_vec(value).expect("serializable"));
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("hmac key");
    mac.update(body.as_bytes());
    format!("{body}.{}", B64.encode(mac.finalize().into_bytes()))
}

fn verify<T: DeserializeOwned>(value: &str, secret: &[u8]) -> Option<T> {
    let (body, sig) = value.split_once('.')?;
    let sig = B64.decode(sig).ok()?;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).ok()?;
    mac.update(body.as_bytes());
    mac.verify_slice(&sig).ok()?;
    serde_json::from_slice(&B64.decode(body).ok()?).ok()
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
}

fn set_cookie(st: &AppState, name: &str, value: &str, max_age: i64) -> HeaderValue {
    let secure = if st.cfg.base_url.starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    HeaderValue::from_str(&format!(
        "{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}"
    ))
    .expect("cookie header")
}

pub fn identity(st: &AppState, headers: &HeaderMap) -> Option<Identity> {
    if st.cfg.auth_disabled {
        return Some(Identity {
            sub: "local".into(),
            name: Some("Local user".into()),
            email: None,
            exp: i64::MAX,
        });
    }
    let id: Identity = verify(&cookie(headers, SESSION_COOKIE)?, &st.cfg.session_secret)?;
    let allowed = st
        .cfg
        .oidc
        .as_ref()
        .is_some_and(|o| o.subjects.contains(&id.sub));
    (id.exp > now() && allowed).then_some(id)
}

/// Middleware for `/api/ui/*`: a valid session, or 401.
pub async fn require(State(st): State<AppState>, mut req: Request, next: Next) -> Response {
    match identity(&st, req.headers()) {
        Some(id) => {
            req.extensions_mut().insert(id);
            next.run(req).await
        }
        None => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "unauthenticated"})),
        )
            .into_response(),
    }
}

pub async fn me(Extension(id): Extension<Identity>) -> Json<Identity> {
    Json(id)
}

#[derive(Deserialize)]
pub struct LoginParams {
    return_to: Option<String>,
}

fn safe_return(path: Option<String>) -> String {
    match path {
        Some(p) if p.starts_with('/') && !p.starts_with("//") && !p.starts_with("/\\") => p,
        _ => "/logs".into(),
    }
}

fn callback_url(st: &AppState) -> String {
    format!("{}/auth/callback", st.cfg.base_url)
}

pub async fn login(State(st): State<AppState>, Query(p): Query<LoginParams>) -> Response {
    let Some(oidc) = st.cfg.oidc.clone() else {
        return Redirect::to(&safe_return(p.return_to)).into_response();
    };
    let provider = match st.auth.discovery(&oidc).await {
        Ok(d) => d,
        Err(e) => {
            tracing::error!(error = %e, "Robine ID discovery failed");
            return (StatusCode::BAD_GATEWAY, "Robine ID is unreachable").into_response();
        }
    };
    let tx = Transaction {
        state: random(),
        nonce: random(),
        verifier: random(),
        return_to: safe_return(p.return_to),
    };
    let challenge = B64.encode(Sha256::digest(tx.verifier.as_bytes()));
    let mut url = match url::Url::parse(&provider.authorization_endpoint) {
        Ok(u) => u,
        Err(_) => {
            return (StatusCode::BAD_GATEWAY, "invalid authorization endpoint").into_response();
        }
    };
    url.query_pairs_mut()
        .append_pair("client_id", &oidc.client_id)
        .append_pair("redirect_uri", &callback_url(&st))
        .append_pair("response_type", "code")
        .append_pair("scope", "openid profile email")
        .append_pair("state", &tx.state)
        .append_pair("nonce", &tx.nonce)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    let mut resp = Redirect::to(url.as_str()).into_response();
    resp.headers_mut().append(
        header::SET_COOKIE,
        set_cookie(&st, TX_COOKIE, &sign(&tx, &st.cfg.session_secret), 600),
    );
    resp
}

#[derive(Deserialize)]
pub struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

async fn finish(st: &AppState, oidc: &Oidc, tx: &Transaction, code: &str) -> Result<Identity> {
    let provider = st.auth.discovery(oidc).await?;
    let callback = callback_url(st);
    let tokens: serde_json::Value = st
        .auth
        .http
        .post(&provider.token_endpoint)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", callback.as_str()),
            ("code_verifier", tx.verifier.as_str()),
            ("client_id", oidc.client_id.as_str()),
            ("client_secret", oidc.client_secret.as_str()),
        ])
        .send()
        .await?
        .error_for_status()
        .context("token exchange")?
        .json()
        .await?;
    let id_token = tokens["id_token"]
        .as_str()
        .ok_or_else(|| anyhow!("no id_token in token response"))?;

    let header = jsonwebtoken::decode_header(id_token)?;
    if header.alg != jsonwebtoken::Algorithm::RS256 {
        bail!("unexpected ID token algorithm {:?}", header.alg);
    }
    let kid = header.kid.ok_or_else(|| anyhow!("ID token has no kid"))?;
    let jwks: serde_json::Value = st
        .auth
        .http
        .get(&provider.jwks_uri)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let jwk = jwks["keys"]
        .as_array()
        .and_then(|keys| {
            keys.iter()
                .find(|k| k["kid"].as_str() == Some(kid.as_str()))
        })
        .ok_or_else(|| anyhow!("signing key not found in JWKS"))?;
    let key = jsonwebtoken::DecodingKey::from_rsa_components(
        jwk["n"].as_str().unwrap_or_default(),
        jwk["e"].as_str().unwrap_or_default(),
    )?;
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.set_issuer(&[&oidc.issuer]);
    validation.set_audience(&[&oidc.client_id]);
    validation.leeway = 30;
    let claims = jsonwebtoken::decode::<IdClaims>(id_token, &key, &validation)?.claims;
    if claims.nonce.as_deref() != Some(tx.nonce.as_str()) {
        bail!("ID token nonce mismatch");
    }
    Ok(Identity {
        sub: claims.sub,
        name: claims.name.or(claims.preferred_username),
        email: claims.email,
        exp: now() + SESSION_SECONDS,
    })
}

pub async fn callback(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(p): Query<CallbackParams>,
) -> Response {
    let clear_tx = set_cookie(&st, TX_COOKIE, "", 0);
    let fail = |msg: &str| {
        let mut r = Redirect::to(&format!(
            "/login?error={}",
            url::form_urlencoded::byte_serialize(msg.as_bytes()).collect::<String>()
        ))
        .into_response();
        r.headers_mut().append(header::SET_COOKIE, clear_tx.clone());
        r
    };
    let Some(oidc) = st.cfg.oidc.clone() else {
        return fail("Robine ID is not configured");
    };
    if let Some(e) = p.error {
        return fail(&format!("Robine ID refused the sign-in: {e}"));
    }
    let tx: Option<Transaction> =
        cookie(&headers, TX_COOKIE).and_then(|c| verify(&c, &st.cfg.session_secret));
    let (Some(tx), Some(code), Some(state)) = (tx, p.code, p.state) else {
        return fail("The sign-in expired, please try again");
    };
    if tx.state != state {
        return fail("The sign-in expired, please try again");
    }
    let id = match finish(&st, &oidc, &tx, &code).await {
        Ok(id) => id,
        Err(e) => {
            tracing::warn!(error = %e, "sign-in failed");
            return fail("Robine ID sign-in failed");
        }
    };
    if !oidc.subjects.contains(&id.sub) {
        tracing::warn!("sign-in refused: subject is not in ROBINE_DOG_OIDC_SUBJECTS");
        return fail("This Robine ID account is not allowed here");
    }
    let mut r = Redirect::to(&tx.return_to).into_response();
    r.headers_mut().append(header::SET_COOKIE, clear_tx);
    r.headers_mut().append(
        header::SET_COOKIE,
        set_cookie(
            &st,
            SESSION_COOKIE,
            &sign(&id, &st.cfg.session_secret),
            SESSION_SECONDS,
        ),
    );
    r
}

pub async fn logout(State(st): State<AppState>) -> Response {
    let mut r = Redirect::to("/login?signed_out=1").into_response();
    r.headers_mut()
        .append(header::SET_COOKIE, set_cookie(&st, SESSION_COOKIE, "", 0));
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_values_round_trip_and_reject_tampering() {
        let secret = b"0123456789abcdef0123456789abcdef";
        let id = Identity {
            sub: "s".into(),
            name: None,
            email: None,
            exp: 1,
        };
        let token = sign(&id, secret);
        assert_eq!(verify::<Identity>(&token, secret).unwrap().sub, "s");
        assert!(verify::<Identity>(&token, b"another-secret-another-secret-xx").is_none());
        let (body, sig) = token.split_once('.').unwrap();
        let forged = B64.encode(br#"{"sub":"admin","name":null,"email":null,"exp":9999999999}"#);
        assert_ne!(forged, body);
        assert!(verify::<Identity>(&format!("{forged}.{sig}"), secret).is_none());
    }

    #[test]
    fn return_paths_stay_local() {
        assert_eq!(
            safe_return(Some("/apm/traces?x=1".into())),
            "/apm/traces?x=1"
        );
        assert_eq!(safe_return(Some("//evil.example".into())), "/logs");
        assert_eq!(safe_return(Some("https://evil.example".into())), "/logs");
        assert_eq!(safe_return(None), "/logs");
    }
}
