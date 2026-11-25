//! Discord bot management for apps built on discord.js: identity, servers, slash commands, diagnostics.
//! The bot token is read from the app's encrypted environment and never leaves the panel.

use crate::{api, auth::User, bad, db::audit, hub, ApiErr, St, VERSION};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use nextpane_proto::Command;
use serde_json::{json, Value};
use std::time::Duration;

type R = Result<Json<Value>, ApiErr>;

/// Env vars searched, in order, for the bot token.
pub const TOKEN_VARS: [&str; 4] = ["DISCORD_TOKEN", "DISCORD_BOT_TOKEN", "BOT_TOKEN", "TOKEN"];
const API: &str = "https://discord.com/api/v10";

fn token(st: &St, app: i64) -> Result<Option<(String, String)>, ApiErr> {
    let vars = api::load_env(st, app)?;
    Ok(TOKEN_VARS.iter().find_map(|k| vars.iter().find(|v| v.key == *k && !v.value.trim().is_empty()).map(|v| (v.key.clone(), v.value.trim().to_string()))))
}

pub fn has_token_var(st: &St, app: i64) -> bool {
    let db = st.db.lock().unwrap();
    TOKEN_VARS.iter().any(|k| db.query_row("SELECT 1 FROM env WHERE app_id=?1 AND key=?2", rusqlite::params![app, k], |_| Ok(())).is_ok())
}

/// Err((http status or 0, message)).
fn call(method: &str, token: &str, path: &str) -> Result<Value, (u16, String)> {
    let r = ureq::request(method, &format!("{API}{path}"))
        .set("Authorization", &format!("Bot {token}"))
        .set("User-Agent", &format!("DiscordBot (NextPane, {VERSION})"))
        .timeout(Duration::from_secs(10))
        .call();
    match r {
        Ok(r) if r.status() == 204 => Ok(Value::Null),
        Ok(r) => r.into_json().map_err(|e| (0, e.to_string())),
        Err(ureq::Error::Status(code, r)) => Err((code, r.into_json::<Value>().ok().and_then(|v| v["message"].as_str().map(String::from)).unwrap_or_else(|| format!("HTTP {code}")))),
        Err(e) => Err((0, e.to_string())),
    }
}

async fn discord(method: &'static str, token: &str, path: String) -> Result<Value, (u16, String)> {
    let token = token.to_string();
    tokio::task::spawn_blocking(move || call(method, &token, &path)).await.unwrap_or_else(|e| Err((0, e.to_string())))
}

fn derr((_, msg): (u16, String)) -> ApiErr {
    ApiErr(StatusCode::BAD_GATEWAY, format!("Discord: {msg}"))
}

fn snowflake(s: &str) -> Result<(), ApiErr> {
    if s.is_empty() || s.len() > 20 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad("invalid Discord id"));
    }
    Ok(())
}

/// Known discord.js failure signatures in the app journal → (level, advice).
const PATTERNS: [(&str, &str, &str); 9] = [
    ("TokenInvalid", "err", "Discord rejected the token. Reset it in the Developer Portal (Bot → Reset Token) and update the env var."),
    ("An invalid token was provided", "err", "Discord rejected the token. Reset it in the Developer Portal (Bot → Reset Token) and update the env var."),
    ("Used disallowed intents", "err", "The bot requests privileged intents that are not enabled: Developer Portal → Bot → Privileged Gateway Intents."),
    ("DisallowedIntents", "err", "The bot requests privileged intents that are not enabled: Developer Portal → Bot → Privileged Gateway Intents."),
    ("Cannot find module", "err", "A module is missing: check the install/build steps and the start command."),
    ("ENOTFOUND discord.com", "warn", "The host cannot resolve discord.com (DNS or network problem)."),
    ("DiscordAPIError[50013]", "warn", "Missing Permissions: the bot lacks a permission in a server or channel."),
    ("DiscordAPIError[50001]", "warn", "Missing Access: the bot cannot see a channel it tries to use."),
    ("DiscordAPIError[10062]", "warn", "Unknown interaction: the bot answered after 3 s. Use deferReply() for slow commands."),
];

/// Advice for each problem seen in the logs (deduplicated, with the last matching line) and the last "ready" line.
fn diagnose(log: &str) -> (Vec<Value>, Option<String>) {
    let mut out: Vec<Value> = vec![];
    for (pat, level, advice) in PATTERNS {
        if let Some(line) = log.lines().rev().find(|l| l.contains(pat)) {
            if !out.iter().any(|d| d["advice"] == advice) {
                out.push(json!({ "level": level, "advice": advice, "line": line.trim() }));
            }
        }
    }
    let ready = log.lines().rev().find(|l| l.contains("Logged in as") || l.contains("Ready!")).map(|l| l.trim().to_string());
    (out, ready)
}

pub async fn info(State(st): State<St>, _: User, Path(id): Path<i64>) -> R {
    let (host, spec) = api::host_of_app(&st, id)?;
    let online = st.live.lock().unwrap().contains_key(&host);
    let m = st.history.lock().unwrap().get(&host).and_then(|q| q.back()).and_then(|m| m.apps.iter().find(|a| a.name == spec.name).cloned());
    let mut v = json!({ "token_vars": TOKEN_VARS, "online": online, "discord_js": m.as_ref().and_then(|m| m.discord_js.clone()), "running": online && m.as_ref().is_some_and(|m| m.running) });
    if online {
        if let Ok(r) = hub::command(&st, host, Command::Logs { app: spec.name.clone(), lines: 300 }, Duration::from_secs(10)).await {
            let (diag, ready) = diagnose(&r.output);
            v["diagnostics"] = diag.into();
            v["ready_line"] = ready.into();
        }
    }
    let Some((var, token)) = token(&st, id)? else { return Ok(Json(v)) };
    v["token_var"] = var.into();
    let app = match discord("GET", &token, "/applications/@me".into()).await {
        Ok(app) => app,
        Err((401, _)) => {
            v["token_valid"] = false.into();
            return Ok(Json(v));
        }
        Err((_, e)) => {
            v["error"] = e.into();
            return Ok(Json(v));
        }
    };
    v["token_valid"] = true.into();
    let app_id = app["id"].as_str().unwrap_or_default().to_string();
    snowflake(&app_id)?;
    let flags = app["flags"].as_u64().unwrap_or(0);
    // Each privileged intent has a "verified bot" and a "limited" flag bit.
    v["intents"] = json!({ "presence": flags & (3 << 12) != 0, "members": flags & (3 << 14) != 0, "message_content": flags & (3 << 18) != 0 });
    v["application"] = json!({ "id": app_id, "name": app["name"], "description": app["description"], "public": app["bot_public"],
        "guild_count": app["approximate_guild_count"], "bot": { "id": app["bot"]["id"], "username": app["bot"]["username"], "avatar": app["bot"]["avatar"] } });
    let (guilds, cmds) = tokio::join!(discord("GET", &token, "/users/@me/guilds?with_counts=true".into()), discord("GET", &token, format!("/applications/{app_id}/commands")));
    let list = |r: Result<Value, (u16, String)>, f: &dyn Fn(&Value) -> Value| r.ok().and_then(|v| v.as_array().map(|a| a.iter().map(f).collect::<Vec<_>>())).unwrap_or_default();
    v["guilds"] = list(guilds, &|g| json!({ "id": g["id"], "name": g["name"], "icon": g["icon"], "owner": g["owner"], "members": g["approximate_member_count"] })).into();
    v["commands"] = list(cmds, &|c| json!({ "id": c["id"], "name": c["name"], "description": c["description"], "type": c["type"] })).into();
    Ok(Json(v))
}

fn need_token(st: &St, id: i64) -> Result<String, ApiErr> {
    token(st, id)?.map(|t| t.1).ok_or_else(|| bad(format!("no bot token: set {} in the environment", TOKEN_VARS[0])))
}

pub async fn delete_command(State(st): State<St>, _: User, Path((id, cmd)): Path<(i64, String)>) -> R {
    snowflake(&cmd)?;
    let (_, spec) = api::host_of_app(&st, id)?;
    let token = need_token(&st, id)?;
    let app = discord("GET", &token, "/applications/@me".into()).await.map_err(derr)?;
    let app_id = app["id"].as_str().unwrap_or_default();
    snowflake(app_id)?;
    discord("DELETE", &token, format!("/applications/{app_id}/commands/{cmd}")).await.map_err(derr)?;
    audit(&st.db.lock().unwrap(), "discord.command.delete", &format!("{}: command {cmd}", spec.name));
    Ok(Json(json!({ "ok": true })))
}

pub async fn leave_guild(State(st): State<St>, _: User, Path((id, guild)): Path<(i64, String)>) -> R {
    snowflake(&guild)?;
    let (_, spec) = api::host_of_app(&st, id)?;
    let token = need_token(&st, id)?;
    discord("DELETE", &token, format!("/users/@me/guilds/{guild}")).await.map_err(derr)?;
    audit(&st.db.lock().unwrap(), "discord.guild.leave", &format!("{}: left server {guild}", spec.name));
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnoses_logs() {
        let log = "Nov 25 web node[1]: Ready! Logged in as Bot#0001\nNov 25 web node[1]: Error [TokenInvalid]: An invalid token was provided.\nDiscordAPIError[50013]: Missing Permissions";
        let (d, ready) = diagnose(log);
        assert_eq!(d.len(), 2, "{d:?}"); // TokenInvalid + its message are one problem
        assert_eq!(d[0]["level"], "err");
        assert!(ready.unwrap().contains("Logged in as"));
        assert!(diagnose("all good").0.is_empty());
    }

    #[test]
    fn snowflakes() {
        assert!(snowflake("123456789012345678").is_ok());
        assert!(snowflake("1/../../x").is_err());
        assert!(snowflake("").is_err());
    }
}
