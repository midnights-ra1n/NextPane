//! Single-admin auth: argon2 password, opaque session cookie (HttpOnly, SameSite=Strict).

use crate::{bad, crypto, db::audit, now, ApiErr, St};
use argon2::{password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString}, Argon2};
use axum::{
    extract::{FromRequestParts, State},
    http::{header, request::Parts, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use rusqlite::params;
use serde::Deserialize;
use serde_json::{json, Value};

const COOKIE: &str = "np_session";
const SESSION_TTL: i64 = 7 * 24 * 3600;

pub struct User(pub i64);

fn unauthorized() -> ApiErr {
    ApiErr(StatusCode::UNAUTHORIZED, "not signed in".into())
}

fn cookie<'a>(h: &'a HeaderMap, name: &str) -> Option<&'a str> {
    h.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|c| c.trim().strip_prefix(name)?.strip_prefix('='))
}

impl FromRequestParts<St> for User {
    type Rejection = ApiErr;
    async fn from_request_parts(parts: &mut Parts, st: &St) -> Result<Self, ApiErr> {
        let t = cookie(&parts.headers, COOKIE).ok_or_else(unauthorized)?;
        let db = st.db.lock().unwrap();
        db.query_row("SELECT user_id FROM sessions WHERE token_hash=?1 AND expires>?2", params![crypto::sha(t), now()], |r| r.get(0))
            .map(User)
            .map_err(|_| unauthorized())
    }
}

pub fn hash_password(pw: &str) -> String {
    Argon2::default().hash_password(pw.as_bytes(), &SaltString::generate(&mut OsRng)).expect("argon2").to_string()
}

pub fn verify_password(st: &St, user: i64, pw: &str) -> bool {
    let hash: Option<String> = st.db.lock().unwrap().query_row("SELECT password FROM users WHERE id=?1", [user], |r| r.get(0)).ok();
    hash.as_deref().and_then(|h| PasswordHash::new(h).ok()).is_some_and(|h| Argon2::default().verify_password(pw.as_bytes(), &h).is_ok())
}

fn session_response(st: &St, headers: &HeaderMap, user: i64) -> Response {
    let t = crypto::token();
    st.db.lock().unwrap().execute("INSERT INTO sessions (token_hash, user_id, expires) VALUES (?1, ?2, ?3)", params![crypto::sha(&t), user, now() + SESSION_TTL]).ok();
    // Secure when the panel is reached through HTTPS (TLS is terminated by the reverse proxy).
    let https = headers.get("x-forwarded-proto").is_some_and(|v| v == "https");
    let c = format!("{COOKIE}={t}; HttpOnly; SameSite=Strict; Path=/; Max-Age={SESSION_TTL}{}", if https { "; Secure" } else { "" });
    ([(header::SET_COOKIE, c)], Json(json!({ "ok": true }))).into_response()
}

#[derive(Deserialize)]
pub struct Creds {
    username: String,
    password: String,
    #[serde(default)]
    code: String,
}

pub async fn setup(State(st): State<St>, headers: HeaderMap, Json(c): Json<Creds>) -> Result<Response, ApiErr> {
    let ok = {
        let mut code = st.setup_code.lock().unwrap();
        let ok = code.as_deref().is_some_and(|k| crypto::sha(k) == crypto::sha(c.code.trim()));
        if ok {
            *code = None;
        }
        ok
    };
    if !ok {
        return Err(ApiErr(StatusCode::FORBIDDEN, "invalid setup code (see the server logs)".into()));
    }
    if c.username.trim().is_empty() || c.password.len() < 10 {
        *st.setup_code.lock().unwrap() = Some(c.code.trim().into());
        return Err(bad("username required, password must be at least 10 characters"));
    }
    let id = {
        let db = st.db.lock().unwrap();
        db.execute("INSERT INTO users (username, password) VALUES (?1, ?2)", params![c.username.trim(), hash_password(&c.password)])?;
        let id = db.last_insert_rowid();
        audit(&db, "setup", c.username.trim());
        id
    };
    Ok(session_response(&st, &headers, id))
}

pub async fn login(State(st): State<St>, headers: HeaderMap, Json(c): Json<Creds>) -> Result<Response, ApiErr> {
    let id: Option<i64> = st.db.lock().unwrap().query_row("SELECT id FROM users WHERE username=?1", [c.username.trim()], |r| r.get(0)).ok();
    let st2 = st.clone();
    let ok = tokio::task::spawn_blocking(move || id.is_some_and(|id| verify_password(&st2, id, &c.password))).await.unwrap_or(false);
    match id.filter(|_| ok) {
        Some(id) => Ok(session_response(&st, &headers, id)),
        None => {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await; // slows down guessing
            Err(ApiErr(StatusCode::UNAUTHORIZED, "invalid username or password".into()))
        }
    }
}

pub async fn logout(State(st): State<St>, headers: HeaderMap) -> Response {
    if let Some(t) = cookie(&headers, COOKIE) {
        st.db.lock().unwrap().execute("DELETE FROM sessions WHERE token_hash=?1", [crypto::sha(t)]).ok();
    }
    ([(header::SET_COOKIE, format!("{COOKIE}=; Path=/; Max-Age=0"))], Json(json!({ "ok": true }))).into_response()
}

pub async fn me(State(st): State<St>, headers: HeaderMap) -> Json<Value> {
    let setup = st.setup_code.lock().unwrap().is_some();
    let mut parts = axum::http::Request::new(()).into_parts().0;
    parts.headers = headers;
    let user = User::from_request_parts(&mut parts, &st).await.ok();
    let name: Option<String> = user.and_then(|u| st.db.lock().unwrap().query_row("SELECT username FROM users WHERE id=?1", [u.0], |r| r.get(0)).ok());
    Json(json!({ "user": name, "needs_setup": setup }))
}

#[derive(Deserialize)]
pub struct PasswordChange {
    current: String,
    new: String,
}

pub async fn password(State(st): State<St>, User(uid): User, Json(p): Json<PasswordChange>) -> Result<Json<Value>, ApiErr> {
    if p.new.len() < 10 {
        return Err(bad("password must be at least 10 characters"));
    }
    if !verify_password(&st, uid, &p.current) {
        return Err(ApiErr(StatusCode::FORBIDDEN, "current password is wrong".into()));
    }
    let db = st.db.lock().unwrap();
    db.execute("UPDATE users SET password=?1 WHERE id=?2", params![hash_password(&p.new), uid])?;
    db.execute("DELETE FROM sessions WHERE user_id=?1", [uid])?;
    audit(&db, "password", "changed, all sessions revoked");
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    #[test]
    fn cookie_parse() {
        let mut h = axum::http::HeaderMap::new();
        h.insert("cookie", "a=1; np_session=abc; b=2".parse().unwrap());
        assert_eq!(super::cookie(&h, "np_session"), Some("abc"));
        assert_eq!(super::cookie(&h, "np"), None);
    }
}
