//! Outgoing notifications (Slack, Discord, Matrix or generic JSON webhook) for deploys and host status.

use crate::{auth::User, bad, crypto, db::audit, now, ApiErr, St};
use axum::{extract::State, http::StatusCode, Json};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

pub const EVENTS: [&str; 4] = ["deploy_failed", "deploy_success", "host_offline", "host_online"];

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Notifications {
    #[serde(default)]
    pub url: String,
    /// slack | discord | matrix | generic
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub events: Vec<String>,
    /// Matrix only: room ID (`!abc:example.org`) and the bot account's access token. `url` is the homeserver.
    #[serde(default)]
    pub room: String,
    #[serde(default)]
    pub token: String,
}

/// Stored sealed: webhook URLs embed their own credentials.
pub fn load(st: &St) -> Notifications {
    let v: Option<String> = st.db.lock().unwrap().query_row("SELECT value FROM settings WHERE key='notifications'", [], |r| r.get(0)).optional().ok().flatten();
    v.and_then(|v| crypto::open(&st.key, &v)).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn post(n: &Notifications, event: &str, msg: &str) -> Result<(), String> {
    if n.format == "matrix" {
        let room: String = n.room.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
        let url = format!("{}/_matrix/client/v3/rooms/{room}/send/m.room.message/np{}", n.url.trim_end_matches('/'), crypto::token());
        return ureq::put(&url)
            .set("Authorization", &format!("Bearer {}", n.token))
            .timeout(Duration::from_secs(10))
            .send_json(json!({ "msgtype": "m.text", "body": msg }))
            .map(|_| ())
            .map_err(|e| e.to_string());
    }
    let body = match n.format.as_str() {
        "slack" => json!({ "text": msg }),
        "discord" => json!({ "content": msg }),
        _ => json!({ "event": event, "message": msg, "ts": now() }),
    };
    ureq::post(&n.url).timeout(Duration::from_secs(10)).send_json(body).map(|_| ()).map_err(|e| e.to_string())
}

/// Fire and forget; a failing webhook never blocks the caller.
pub fn send(st: &St, event: &'static str, msg: String) {
    let n = load(st);
    if n.url.is_empty() || !n.events.iter().any(|e| e == event) {
        return;
    }
    tokio::task::spawn_blocking(move || {
        if let Err(e) = post(&n, event, &msg) {
            eprintln!("notification {event} failed: {e}");
        }
    });
}

/// The Matrix access token is write-only: the UI gets `token_set` and sends an empty token to keep it.
pub async fn get_settings(State(st): State<St>, _: User) -> Json<Value> {
    let mut n = load(&st);
    let token_set = !n.token.is_empty();
    n.token.clear();
    Json(json!({ "notifications": n, "token_set": token_set, "events": EVENTS }))
}

fn keep_token(st: &St, mut n: Notifications) -> Notifications {
    if n.token.is_empty() {
        n.token = load(st).token;
    }
    n
}

#[derive(Deserialize)]
pub struct SettingsPut {
    notifications: Notifications,
}

fn validate(n: &Notifications) -> Result<(), ApiErr> {
    if !n.url.is_empty() && !(n.url.starts_with("https://") || n.url.starts_with("http://")) {
        return Err(bad("webhook URL must start with https:// or http://"));
    }
    if !["slack", "discord", "matrix", "generic"].contains(&n.format.as_str()) {
        return Err(bad("format must be slack, discord, matrix or generic"));
    }
    if n.format == "matrix" && !n.url.is_empty() {
        if !(n.room.starts_with('!') && n.room.contains(':') && n.room.len() <= 255) {
            return Err(bad("Matrix room must be a room ID like !abcdef:example.org (Room settings → Advanced)"));
        }
        if n.token.is_empty() {
            return Err(bad("Matrix needs an access token"));
        }
    }
    if let Some(e) = n.events.iter().find(|e| !EVENTS.contains(&e.as_str())) {
        return Err(bad(format!("unknown event {e}")));
    }
    Ok(())
}

pub async fn put_settings(State(st): State<St>, _: User, Json(p): Json<SettingsPut>) -> Result<Json<Value>, ApiErr> {
    let n = keep_token(&st, p.notifications);
    validate(&n)?;
    let sealed = crypto::seal(&st.key, &serde_json::to_string(&n).unwrap());
    let db = st.db.lock().unwrap();
    db.execute("INSERT INTO settings (key, value) VALUES ('notifications', ?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![sealed])?;
    audit(&db, "settings.notifications", &format!("{} events, format {}", n.events.len(), n.format));
    Ok(Json(json!({ "ok": true })))
}

/// Sends a test message with the submitted (possibly unsaved) settings.
pub async fn test(State(st): State<St>, _: User, Json(p): Json<SettingsPut>) -> Result<Json<Value>, ApiErr> {
    let n = keep_token(&st, p.notifications);
    validate(&n)?;
    if n.url.is_empty() {
        return Err(bad("set a webhook URL first"));
    }
    tokio::task::spawn_blocking(move || post(&n, "test", "NextPane: test notification, everything works."))
        .await
        .map_err(|e| bad(e.to_string()))?
        .map_err(|e| ApiErr(StatusCode::BAD_GATEWAY, format!("webhook failed: {e}")))?;
    Ok(Json(json!({ "ok": true })))
}

pub fn host_name(st: &St, id: i64) -> String {
    st.db.lock().unwrap().query_row("SELECT name FROM hosts WHERE id=?1", [id], |r| r.get(0)).unwrap_or_else(|_| format!("host {id}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn n(url: &str, format: &str, events: &[&str]) -> Notifications {
        Notifications { url: url.into(), format: format.into(), events: events.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }
    #[test]
    fn validates_settings() {
        assert!(validate(&n("https://hooks.slack.com/x", "slack", &["deploy_failed"])).is_ok());
        assert!(validate(&n("", "generic", &[])).is_ok());
        assert!(validate(&n("file:///etc/passwd", "slack", &[])).is_err());
        assert!(validate(&n("https://x", "teams", &[])).is_err());
        assert!(validate(&n("https://x", "discord", &["rm -rf"])).is_err());
        let m = |room: &str, token: &str| Notifications { room: room.into(), token: token.into(), ..n("https://matrix.org", "matrix", &[]) };
        assert!(validate(&m("!abc:matrix.org", "syt_x")).is_ok());
        assert!(validate(&m("#alias:matrix.org", "syt_x")).is_err());
        assert!(validate(&m("!abc:matrix.org", "")).is_err());
    }
}
