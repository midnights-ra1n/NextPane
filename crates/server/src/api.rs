//! REST API used by the web UI. Every handler except `webhook` requires a session (`User`).

use crate::{auth::{verify_password, User}, bad, crypto, db::audit, hub, notify, now, ApiErr, St};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use nextpane_proto::{validate_env, valid_ssh_pubkey, AppAction, AppSpec, Command, EnvVar, HostInfo};
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

type R = Result<Json<Value>, ApiErr>;

fn ok() -> R {
    Ok(Json(json!({ "ok": true })))
}

pub(crate) fn host_of_app(st: &St, app_id: i64) -> Result<(i64, AppSpec), ApiErr> {
    let (h, s): (i64, String) = st.db.lock().unwrap().query_row("SELECT host_id, spec FROM apps WHERE id=?1", [app_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok((h, serde_json::from_str(&s).map_err(|e| bad(e.to_string()))?))
}

pub(crate) fn load_env(st: &St, app_id: i64) -> Result<Vec<EnvVar>, ApiErr> {
    let db = st.db.lock().unwrap();
    let mut q = db.prepare("SELECT key, value FROM env WHERE app_id=?1 ORDER BY key")?;
    let rows = q.query_map([app_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    rows.map(|r| {
        let (key, v) = r?;
        let value = crypto::open(&st.key, &v).ok_or_else(|| ApiErr(StatusCode::INTERNAL_SERVER_ERROR, "cannot decrypt env (secret.key changed?)".into()))?;
        Ok(EnvVar { key, value })
    })
    .collect()
}

fn cmd_json(r: crate::CmdResult) -> R {
    if r.ok {
        Ok(Json(json!({ "ok": true, "output": r.output, "data": r.data })))
    } else {
        Err(ApiErr(StatusCode::BAD_GATEWAY, r.output))
    }
}

// ---------- hosts ----------

fn host_summary(st: &St, id: i64, name: String, info: String, last_seen: i64) -> Value {
    let live = st.live.lock().unwrap();
    let info: HostInfo = live.get(&id).map(|l| l.info.clone()).unwrap_or_else(|| serde_json::from_str(&info).unwrap_or_default());
    let metrics = st.history.lock().unwrap().get(&id).and_then(|q| q.back().cloned());
    json!({ "id": id, "name": name, "online": live.contains_key(&id), "info": info, "last_seen": last_seen, "metrics": metrics })
}

pub async fn hosts(State(st): State<St>, _: User) -> R {
    let rows: Vec<(i64, String, String, i64, i64)> = {
        let db = st.db.lock().unwrap();
        let mut q = db.prepare("SELECT h.id, h.name, h.info, h.last_seen, (SELECT COUNT(*) FROM apps a WHERE a.host_id=h.id) FROM hosts h ORDER BY h.name")?;
        let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<Result<_, _>>()?;
        rows
    };
    let latest = crate::update::cached_version(&st);
    if !st.cfg.release_url.is_empty() {
        let st = st.clone(); // refresh the release cache in the background (10 min TTL) for the "update" badges
        tokio::spawn(async move { crate::update::manifest(&st, false).await.ok() });
    }
    let list: Vec<Value> = rows
        .into_iter()
        .map(|(id, name, info, seen, apps)| {
            let mut v = host_summary(&st, id, name, info, seen);
            v["apps"] = apps.into();
            v
        })
        .collect();
    Ok(Json(json!({ "hosts": list, "latest_agent": latest })))
}

pub async fn host(State(st): State<St>, _: User, Path(id): Path<i64>) -> R {
    let (name, info, seen): (String, String, i64) = st.db.lock().unwrap().query_row("SELECT name, info, last_seen FROM hosts WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    let mut v = host_summary(&st, id, name, info, seen);
    v["history"] = json!(st.history.lock().unwrap().get(&id).map(|q| q.iter().cloned().collect::<Vec<_>>()).unwrap_or_default());
    let db = st.db.lock().unwrap();
    let apps: Vec<Value> = db
        .prepare(
            "SELECT a.id, a.spec, a.last_sha, d.status, d.started FROM apps a
             LEFT JOIN deployments d ON d.id = (SELECT MAX(id) FROM deployments WHERE app_id=a.id)
             WHERE a.host_id=?1 ORDER BY a.name",
        )?
        .query_map([id], |r| {
            Ok(json!({ "id": r.get::<_, i64>(0)?, "spec": serde_json::from_str::<Value>(&r.get::<_, String>(1)?).unwrap_or_default(),
                       "last_sha": r.get::<_, Option<String>>(2)?, "last_status": r.get::<_, Option<String>>(3)?, "last_deploy": r.get::<_, Option<i64>>(4)? }))
        })?
        .collect::<Result<_, _>>()?;
    let keys: Vec<Value> = db
        .prepare("SELECT id, name, pubkey FROM ssh_keys WHERE host_id=?1 ORDER BY id")?
        .query_map([id], |r| Ok(json!({ "id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)?, "pubkey": r.get::<_, String>(2)? })))?
        .collect::<Result<_, _>>()?;
    v["apps"] = apps.into();
    v["ssh_keys"] = keys.into();
    v["latest_agent"] = json!(crate::update::cached_version(&st));
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct HostPatch {
    name: Option<String>,
    #[serde(default)]
    reset_identity: bool,
}

pub async fn patch_host(State(st): State<St>, _: User, Path(id): Path<i64>, Json(p): Json<HostPatch>) -> R {
    let db = st.db.lock().unwrap();
    if let Some(n) = p.name.map(|n| n.trim().to_string()) {
        if n.is_empty() || n.len() > 64 {
            return Err(bad("name must be 1-64 characters"));
        }
        db.execute("UPDATE hosts SET name=?1 WHERE id=?2", params![n, id])?;
    }
    if p.reset_identity {
        db.execute("UPDATE hosts SET secret_hash='' WHERE id=?1", [id])?;
        audit(&db, "host.reset_identity", &id.to_string());
    }
    ok()
}

pub async fn delete_host(State(st): State<St>, _: User, Path(id): Path<i64>) -> R {
    let db = st.db.lock().unwrap();
    db.execute("DELETE FROM hosts WHERE id=?1", [id])?;
    audit(&db, "host.delete", &id.to_string());
    drop(db);
    st.live.lock().unwrap().remove(&id); // dropping the sender closes the agent's session
    st.history.lock().unwrap().remove(&id);
    ok()
}

pub async fn update_agent(State(st): State<St>, _: User, Path(id): Path<i64>) -> R {
    let m = crate::update::manifest(&st, true).await?;
    let base_url = format!("{}/download/v{}", st.cfg.release_url, m.version);
    audit(&st.db.lock().unwrap(), "host.update", &format!("host {id} -> {}", m.version));
    let r = hub::command(&st, id, Command::SelfUpdate { base_url, version: m.version, files: m.files }, Duration::from_secs(900)).await?;
    cmd_json(r)
}

#[derive(Deserialize)]
pub struct NewKey {
    name: String,
    pubkey: String,
}

pub async fn add_ssh_key(State(st): State<St>, _: User, Path(id): Path<i64>, Json(k): Json<NewKey>) -> R {
    if !valid_ssh_pubkey(&k.pubkey) {
        return Err(bad("not an OpenSSH public key"));
    }
    {
        let db = st.db.lock().unwrap();
        db.execute("INSERT INTO ssh_keys (host_id, name, pubkey) VALUES (?1, ?2, ?3)", params![id, k.name.trim(), k.pubkey.trim()])?;
        audit(&db, "ssh_key.add", &format!("host {id}: {}", k.name.trim()));
    }
    hub::resync(&st, id);
    ok()
}

pub async fn delete_ssh_key(State(st): State<St>, _: User, Path((id, kid)): Path<(i64, i64)>) -> R {
    {
        let db = st.db.lock().unwrap();
        db.execute("DELETE FROM ssh_keys WHERE id=?1 AND host_id=?2", [kid, id])?;
        audit(&db, "ssh_key.delete", &format!("host {id}: key {kid}"));
    }
    hub::resync(&st, id);
    ok()
}

/// Shared deploy key: (public key line, private key), if generated.
pub(crate) fn load_git_key(db: &rusqlite::Connection, key: &[u8; 32]) -> Option<(String, String)> {
    let v: String = db.query_row("SELECT value FROM settings WHERE key='git_key'", [], |r| r.get(0)).ok()?;
    let v: Value = serde_json::from_str(&v).ok()?;
    Some((v["public"].as_str()?.to_string(), crypto::open(key, v["private"].as_str()?)?))
}

/// One SSH key for every app on every host: generated here on first use, sent to the agents.
fn new_git_key(st: &St, action: &str) -> R {
    let (private, public) = crypto::ssh_keypair("nextpane-deploy");
    {
        let db = st.db.lock().unwrap();
        let v = json!({ "public": public, "private": crypto::seal(&st.key, &private) }).to_string();
        db.execute("INSERT INTO settings (key, value) VALUES ('git_key', ?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [v])?;
        audit(&db, action, &public);
    }
    hub::resync_all(st);
    Ok(Json(json!({ "public": public })))
}

pub async fn get_git_key(State(st): State<St>, _: User) -> R {
    let existing = load_git_key(&st.db.lock().unwrap(), &st.key);
    match existing {
        Some((public, _)) => Ok(Json(json!({ "public": public }))),
        None => new_git_key(&st, "git_key.create"),
    }
}

pub async fn rotate_git_key(State(st): State<St>, _: User) -> R {
    new_git_key(&st, "git_key.rotate")
}

/// Key library: public keys saved once and added to any host in one click.
pub async fn saved_keys(State(st): State<St>, _: User) -> R {
    let db = st.db.lock().unwrap();
    let keys: Vec<Value> = db
        .prepare("SELECT id, name, pubkey FROM saved_keys ORDER BY name")?
        .query_map([], |r| Ok(json!({ "id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)?, "pubkey": r.get::<_, String>(2)? })))?
        .collect::<Result<_, _>>()?;
    Ok(Json(json!({ "keys": keys })))
}

pub async fn save_key(State(st): State<St>, _: User, Json(k): Json<NewKey>) -> R {
    if !valid_ssh_pubkey(&k.pubkey) {
        return Err(bad("not an OpenSSH public key"));
    }
    let db = st.db.lock().unwrap();
    db.execute("INSERT INTO saved_keys (name, pubkey) VALUES (?1, ?2) ON CONFLICT (pubkey) DO UPDATE SET name=excluded.name", params![k.name.trim(), k.pubkey.trim()])?;
    audit(&db, "ssh_key.save", k.name.trim());
    ok()
}

/// Removes the key from the library only; hosts that have it keep it.
pub async fn forget_key(State(st): State<St>, _: User, Path(id): Path<i64>) -> R {
    let db = st.db.lock().unwrap();
    db.execute("DELETE FROM saved_keys WHERE id=?1", [id])?;
    audit(&db, "ssh_key.forget", &id.to_string());
    ok()
}

// ---------- apps ----------

pub async fn create_app(State(st): State<St>, _: User, Path(host): Path<i64>, Json(spec): Json<AppSpec>) -> R {
    spec.validate().map_err(bad)?;
    let id = {
        let db = st.db.lock().unwrap();
        db.query_row("SELECT 1 FROM hosts WHERE id=?1", [host], |_| Ok(()))?;
        db.execute(
            "INSERT INTO apps (host_id, name, spec, webhook_secret, created) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![host, spec.name, serde_json::to_string(&spec).unwrap(), crypto::token(), now()],
        )
        .map_err(|_| bad(format!("an app named '{}' already exists on this host", spec.name)))?;
        let id = db.last_insert_rowid();
        audit(&db, "app.create", &spec.name);
        id
    };
    hub::resync(&st, host);
    Ok(Json(json!({ "id": id })))
}

pub async fn app(State(st): State<St>, _: User, Path(id): Path<i64>) -> R {
    let db = st.db.lock().unwrap();
    let mut v = db.query_row(
        "SELECT a.host_id, h.name, a.spec, a.last_sha, a.deploy_key, a.webhook_secret FROM apps a JOIN hosts h ON h.id=a.host_id WHERE a.id=?1",
        [id],
        |r| {
            Ok(json!({ "id": id, "host_id": r.get::<_, i64>(0)?, "host_name": r.get::<_, String>(1)?,
                       "spec": serde_json::from_str::<Value>(&r.get::<_, String>(2)?).unwrap_or_default(),
                       "last_sha": r.get::<_, Option<String>>(3)?, "deploy_key": r.get::<_, Option<String>>(4)?,
                       "webhook_path": format!("/hooks/{id}/{}", r.get::<_, String>(5)?) }))
        },
    )?;
    let deps: Vec<Value> = db
        .prepare("SELECT id, sha, trigger, status, started, finished FROM deployments WHERE app_id=?1 ORDER BY id DESC LIMIT 30")?
        .query_map([id], |r| {
            Ok(json!({ "id": r.get::<_, i64>(0)?, "sha": r.get::<_, Option<String>>(1)?, "trigger": r.get::<_, String>(2)?,
                       "status": r.get::<_, String>(3)?, "started": r.get::<_, i64>(4)?, "finished": r.get::<_, Option<i64>>(5)? }))
        })?
        .collect::<Result<_, _>>()?;
    v["deployments"] = deps.into();
    let host = v["host_id"].as_i64().unwrap_or(0);
    drop(db);
    let live = st.live.lock().unwrap();
    v["online"] = live.contains_key(&host).into();
    v["host_ip"] = live.get(&host).map(|l| l.info.ip.clone()).unwrap_or_default().into();
    drop(live);
    let has_token = crate::discord::has_token_var(&st, id); // before taking the history lock
    let name = v["spec"]["name"].as_str().unwrap_or_default().to_string();
    let hist = st.history.lock().unwrap();
    let q = hist.get(&host);
    v["metrics"] = json!(q.and_then(|q| q.back()).and_then(|m| m.apps.iter().find(|a| a.name == name).cloned()));
    v["discord"] = json!(has_token || v["metrics"]["discord_js"].is_string());
    v["history"] = json!(q.map(|q| q.iter().filter_map(|m| m.apps.iter().find(|a| a.name == name).map(|a| json!({ "ts": m.ts, "cpu": a.cpu, "mem": a.mem }))).collect::<Vec<_>>()).unwrap_or_default());
    Ok(Json(v))
}

pub async fn update_app(State(st): State<St>, _: User, Path(id): Path<i64>, Json(mut spec): Json<AppSpec>) -> R {
    let (host, old) = host_of_app(&st, id)?;
    spec.name = old.name; // the name is the on-host identity, it never changes
    spec.validate().map_err(bad)?;
    {
        let db = st.db.lock().unwrap();
        db.execute("UPDATE apps SET spec=?1 WHERE id=?2", params![serde_json::to_string(&spec).unwrap(), id])?;
        audit(&db, "app.update", &spec.name);
    }
    hub::resync(&st, host);
    configure(&st, id).await
}

/// Push unit, env and firewall to the host (restarts the app if it runs).
async fn configure(st: &St, id: i64) -> R {
    let (host, spec) = host_of_app(st, id)?;
    let env = load_env(st, id)?;
    match hub::command(st, host, Command::Configure { app: spec, env }, Duration::from_secs(120)).await {
        Ok(r) => cmd_json(r),
        Err(e) if e.0 == StatusCode::SERVICE_UNAVAILABLE => Ok(Json(json!({ "ok": true, "output": "saved; host offline, changes apply at next deploy" }))),
        Err(e) => Err(e),
    }
}

#[derive(Deserialize)]
pub struct Force {
    #[serde(default)]
    force: bool,
}

pub async fn delete_app(State(st): State<St>, _: User, Path(id): Path<i64>, Query(f): Query<Force>) -> R {
    let (host, spec) = host_of_app(&st, id)?;
    match hub::command(&st, host, Command::Remove { app: spec.name.clone() }, Duration::from_secs(120)).await {
        Ok(r) if !r.ok && !f.force => return Err(ApiErr(StatusCode::BAD_GATEWAY, r.output)),
        Err(e) if !f.force => return Err(e),
        _ => {}
    }
    {
        let db = st.db.lock().unwrap();
        db.execute("DELETE FROM apps WHERE id=?1", [id])?;
        audit(&db, "app.delete", &spec.name);
    }
    hub::resync(&st, host);
    ok()
}

/// Record a deployment and run it in the background. Errors if the host is offline or a deploy is running.
pub fn start_deploy(st: &St, app_id: i64, sha: Option<String>, trigger: &str) -> Result<i64, ApiErr> {
    let (host, spec) = host_of_app(st, app_id)?;
    if !st.live.lock().unwrap().contains_key(&host) {
        return Err(ApiErr(StatusCode::SERVICE_UNAVAILABLE, "host is offline".into()));
    }
    let env = load_env(st, app_id)?;
    let dep = {
        let db = st.db.lock().unwrap();
        if db.query_row("SELECT 1 FROM deployments WHERE app_id=?1 AND status='running'", [app_id], |_| Ok(())).optional()?.is_some() {
            return Err(ApiErr(StatusCode::CONFLICT, "a deployment is already running".into()));
        }
        db.execute("INSERT INTO deployments (app_id, sha, trigger, status, started) VALUES (?1, ?2, ?3, 'running', ?4)", params![app_id, sha, trigger, now()])?;
        let dep = db.last_insert_rowid();
        if let Some(s) = &sha {
            db.execute("UPDATE apps SET last_sha=?1 WHERE id=?2", params![s, app_id])?; // never retry the same commit automatically
        }
        audit(&db, "app.deploy", &format!("{} ({trigger})", spec.name));
        dep
    };
    let st = st.clone();
    let (name, trigger) = (spec.name.clone(), trigger.to_string());
    tokio::spawn(async move {
        let r = hub::command(&st, host, Command::Deploy { app: spec, env, sha }, Duration::from_secs(3600)).await;
        let (status, mut log, sha) = match r {
            Ok(r) => (if r.ok { "success" } else { "failed" }, r.output, r.data),
            Err(e) => ("failed", e.1, None),
        };
        if status == "failed" {
            log.push_str(git_hint(&log));
        }
        {
            let db = st.db.lock().unwrap();
            db.execute("UPDATE deployments SET status=?1, log=?2, finished=?3, sha=COALESCE(?4, sha) WHERE id=?5", params![status, log, now(), sha, dep]).ok();
            if let Some(s) = &sha {
                db.execute("UPDATE apps SET last_sha=?1 WHERE id=?2", params![s, app_id]).ok();
            }
        }
        let at = sha.as_deref().map(|s| format!(" at {}", &s[..s.len().min(7)])).unwrap_or_default();
        let msg = format!("NextPane: deployment of {name} on {} {}{at} ({trigger})", notify::host_name(&st, host), if status == "success" { "succeeded" } else { "FAILED" });
        notify::send(&st, if status == "success" { "deploy_success" } else { "deploy_failed" }, msg);
    });
    Ok(dep)
}

/// Explain the usual git access errors (host SSH keys are often mistaken for git credentials).
fn git_hint(log: &str) -> &'static str {
    if log.contains("Permission denied (publickey)") {
        "\nhint: the host cannot read this repository. Add the NextPane deploy key (Settings > Deploy key) once to your Git \
account (GitHub: Settings > SSH and GPG keys; GitLab: Preferences > SSH Keys; Gitea/Forgejo: Settings > SSH / GPG Keys). \
Never paste a private key into NextPane. Keys in the host's \"SSH keys\" tab only allow logging into the host, git does not use them.\n"
    } else if log.contains("could not read Username") {
        "\nhint: private repository over HTTPS. Use the SSH clone URL (git@…) and add the NextPane deploy key (Settings > Deploy key) to your Git account.\n"
    } else {
        ""
    }
}

/// Auto-deploy: the agent reports the branch head; deploy when it moved.
pub fn on_head(st: &St, host: i64, app: &str, sha: &str) {
    let row: Option<(i64, String, Option<String>)> =
        st.db.lock().unwrap().query_row("SELECT id, spec, last_sha FROM apps WHERE host_id=?1 AND name=?2", params![host, app], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).ok();
    let Some((id, spec, last)) = row else { return };
    let auto = serde_json::from_str::<AppSpec>(&spec).is_ok_and(|s| s.auto_deploy);
    if auto && last.as_deref() != Some(sha) {
        let _ = start_deploy(st, id, Some(sha.to_string()), "auto (new commit)");
    }
}

#[derive(Deserialize, Default)]
pub struct DeployReq {
    sha: Option<String>,
}

pub async fn deploy(State(st): State<St>, _: User, Path(id): Path<i64>, body: Option<Json<DeployReq>>) -> R {
    let sha = body.and_then(|b| b.0.sha).filter(|s| !s.is_empty());
    let dep = start_deploy(&st, id, sha, "manual")?;
    Ok(Json(json!({ "deployment": dep })))
}

pub async fn webhook(State(st): State<St>, Path((id, secret)): Path<(i64, String)>, body: Bytes) -> Result<(StatusCode, Json<Value>), ApiErr> {
    let stored: Option<String> = st.db.lock().unwrap().query_row("SELECT webhook_secret FROM apps WHERE id=?1", [id], |r| r.get(0)).ok();
    if stored.map(|s| crypto::sha(&s)) != Some(crypto::sha(&secret)) {
        return Err(ApiErr(StatusCode::NOT_FOUND, "not found".into()));
    }
    let (_, spec) = host_of_app(&st, id)?;
    // GitHub, GitLab, Gitea and Forgejo push payloads all carry "ref": "refs/heads/<branch>".
    if let Some(r) = serde_json::from_slice::<Value>(&body).ok().and_then(|v| v["ref"].as_str().map(String::from)) {
        if r != format!("refs/heads/{}", spec.branch) {
            return Ok((StatusCode::OK, Json(json!({ "skipped": format!("push to {r}, app tracks {}", spec.branch) }))));
        }
    }
    let dep = start_deploy(&st, id, None, "webhook")?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "deployment": dep }))))
}

#[derive(Deserialize)]
pub struct ActionReq {
    action: AppAction,
}

pub async fn action(State(st): State<St>, _: User, Path(id): Path<i64>, Json(a): Json<ActionReq>) -> R {
    let (host, spec) = host_of_app(&st, id)?;
    audit(&st.db.lock().unwrap(), "app.action", &format!("{} {:?}", spec.name, a.action));
    cmd_json(hub::command(&st, host, Command::Action { app: spec.name, action: a.action }, Duration::from_secs(60)).await?)
}

pub async fn clear_cache(State(st): State<St>, _: User, Path(id): Path<i64>) -> R {
    let (host, spec) = host_of_app(&st, id)?;
    if spec.cache_paths.is_empty() {
        return Err(bad("no cache paths configured for this app (Settings > Cache paths)"));
    }
    audit(&st.db.lock().unwrap(), "app.cache_clear", &spec.name);
    cmd_json(hub::command(&st, host, Command::ClearCache { app: spec.name, paths: spec.cache_paths }, Duration::from_secs(120)).await?)
}

#[derive(Deserialize)]
pub struct LogsReq {
    lines: Option<u32>,
}

pub async fn logs(State(st): State<St>, _: User, Path(id): Path<i64>, Query(q): Query<LogsReq>) -> R {
    let (host, spec) = host_of_app(&st, id)?;
    cmd_json(hub::command(&st, host, Command::Logs { app: spec.name, lines: q.lines.unwrap_or(300) }, Duration::from_secs(20)).await?)
}

pub async fn deploy_key(State(st): State<St>, _: User, Path(id): Path<i64>) -> R {
    let (host, spec) = host_of_app(&st, id)?;
    let r = hub::command(&st, host, Command::DeployKey { app: spec.name }, Duration::from_secs(30)).await?;
    if let (true, Some(k)) = (r.ok, &r.data) {
        st.db.lock().unwrap().execute("UPDATE apps SET deploy_key=?1 WHERE id=?2", params![k, id])?;
    }
    cmd_json(r)
}

pub async fn deployment(State(st): State<St>, _: User, Path(id): Path<i64>) -> R {
    let v = st.db.lock().unwrap().query_row("SELECT app_id, sha, trigger, status, log, started, finished FROM deployments WHERE id=?1", [id], |r| {
        Ok(json!({ "id": id, "app_id": r.get::<_, i64>(0)?, "sha": r.get::<_, Option<String>>(1)?, "trigger": r.get::<_, String>(2)?, "status": r.get::<_, String>(3)?,
                   "log": r.get::<_, String>(4)?, "started": r.get::<_, i64>(5)?, "finished": r.get::<_, Option<i64>>(6)? }))
    })?;
    Ok(Json(v))
}

/// Every app of every host, with live status, for the Apps page and the command palette.
pub async fn all_apps(State(st): State<St>, _: User) -> R {
    type Row = (i64, i64, String, String, Option<String>, Option<String>, Option<i64>);
    let rows: Vec<Row> = {
        let db = st.db.lock().unwrap();
        let mut q = db.prepare(
            "SELECT a.id, a.host_id, h.name, a.spec, a.last_sha, d.status, d.started FROM apps a JOIN hosts h ON h.id=a.host_id
             LEFT JOIN deployments d ON d.id = (SELECT MAX(id) FROM deployments WHERE app_id=a.id) ORDER BY a.name, h.name",
        )?;
        let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))?.collect::<Result<_, _>>()?;
        rows
    };
    let live = st.live.lock().unwrap();
    let hist = st.history.lock().unwrap();
    let apps: Vec<Value> = rows
        .into_iter()
        .map(|(id, host_id, host_name, spec, sha, status, started)| {
            let spec: Value = serde_json::from_str(&spec).unwrap_or_default();
            let online = live.contains_key(&host_id);
            let metrics = hist.get(&host_id).and_then(|q| q.back()).filter(|_| online).and_then(|m| m.apps.iter().find(|a| Some(a.name.as_str()) == spec["name"].as_str()).cloned());
            json!({ "id": id, "host_id": host_id, "host_name": host_name, "online": online, "spec": spec, "last_sha": sha, "last_status": status, "last_deploy": started, "metrics": metrics })
        })
        .collect();
    Ok(Json(json!({ "apps": apps })))
}

#[derive(Deserialize)]
pub struct DeploymentsQuery {
    limit: Option<u32>,
    status: Option<String>,
}

/// Recent deployments across all apps.
pub async fn deployments(State(st): State<St>, _: User, Query(q): Query<DeploymentsQuery>) -> R {
    let db = st.db.lock().unwrap();
    let rows: Vec<Value> = db
        .prepare(
            "SELECT d.id, d.app_id, a.name, h.id, h.name, d.sha, d.trigger, d.status, d.started, d.finished
             FROM deployments d JOIN apps a ON a.id=d.app_id JOIN hosts h ON h.id=a.host_id
             WHERE (?1 IS NULL OR d.status=?1) ORDER BY d.id DESC LIMIT ?2",
        )?
        .query_map(params![q.status.filter(|s| !s.is_empty()), q.limit.unwrap_or(50).min(500)], |r| {
            Ok(json!({ "id": r.get::<_, i64>(0)?, "app_id": r.get::<_, i64>(1)?, "app": r.get::<_, String>(2)?, "host_id": r.get::<_, i64>(3)?,
                       "host": r.get::<_, String>(4)?, "sha": r.get::<_, Option<String>>(5)?, "trigger": r.get::<_, String>(6)?,
                       "status": r.get::<_, String>(7)?, "started": r.get::<_, i64>(8)?, "finished": r.get::<_, Option<i64>>(9)? }))
        })?
        .collect::<Result<_, _>>()?;
    Ok(Json(json!({ "deployments": rows })))
}

// ---------- environment ----------

pub async fn get_env(State(st): State<St>, _: User, Path(id): Path<i64>) -> R {
    Ok(Json(json!({ "vars": load_env(&st, id)? })))
}

#[derive(Deserialize)]
pub struct EnvPut {
    vars: Vec<EnvVar>,
    #[serde(default)]
    apply: bool,
}

pub async fn put_env(State(st): State<St>, _: User, Path(id): Path<i64>, Json(p): Json<EnvPut>) -> R {
    validate_env(&p.vars).map_err(bad)?;
    let (_, spec) = host_of_app(&st, id)?;
    {
        let mut db = st.db.lock().unwrap();
        let tx = db.transaction()?;
        tx.execute("DELETE FROM env WHERE app_id=?1", [id])?;
        for v in &p.vars {
            tx.execute("INSERT INTO env (app_id, key, value) VALUES (?1, ?2, ?3)", params![id, v.key, crypto::seal(&st.key, &v.value)])
                .map_err(|_| bad(format!("duplicate variable {}", v.key)))?;
        }
        tx.commit()?;
        let keys: Vec<&str> = p.vars.iter().map(|v| v.key.as_str()).collect();
        audit(&db, "env.update", &format!("{}: {}", spec.name, keys.join(", ")));
    }
    if p.apply { configure(&st, id).await } else { ok() }
}

#[derive(Deserialize)]
pub struct ExportReq {
    password: String,
    #[serde(default)]
    passphrase: String,
}

/// Export requires the admin password again. With a passphrase the file is encrypted (OpenSSL format).
pub async fn export_env(State(st): State<St>, User(uid): User, Path(id): Path<i64>, Json(r): Json<ExportReq>) -> R {
    let st2 = st.clone();
    let pw = r.password.clone();
    if !tokio::task::spawn_blocking(move || verify_password(&st2, uid, &pw)).await.unwrap_or(false) {
        tokio::time::sleep(Duration::from_secs(1)).await;
        return Err(ApiErr(StatusCode::FORBIDDEN, "wrong password".into()));
    }
    let (_, spec) = host_of_app(&st, id)?;
    let body: String = load_env(&st, id)?.iter().map(|v| format!("{}=\"{}\"\n", v.key, v.value.replace('\\', "\\\\").replace('"', "\\\""))).collect();
    let encrypted = !r.passphrase.is_empty();
    if encrypted && r.passphrase.len() < 12 {
        return Err(bad("passphrase must be at least 12 characters"));
    }
    audit(&st.db.lock().unwrap(), "env.export", &format!("{} ({})", spec.name, if encrypted { "encrypted" } else { "PLAINTEXT" }));
    Ok(Json(if encrypted {
        let pass = r.passphrase;
        let enc = tokio::task::spawn_blocking(move || crypto::openssl_enc(&pass, body.as_bytes())).await.map_err(|e| bad(e.to_string()))?;
        json!({ "filename": format!("{}.env.enc", spec.name), "content": enc, "iter": crypto::EXPORT_ITER })
    } else {
        json!({ "filename": format!("{}.env", spec.name), "content": body })
    }))
}

// ---------- enrollment keys & audit ----------

pub async fn enroll_keys(State(st): State<St>, _: User) -> R {
    let db = st.db.lock().unwrap();
    let keys: Vec<Value> = db
        .prepare("SELECT id, name, created FROM enroll_keys ORDER BY id DESC")?
        .query_map([], |r| Ok(json!({ "id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)?, "created": r.get::<_, i64>(2)? })))?
        .collect::<Result<_, _>>()?;
    Ok(Json(json!({ "keys": keys })))
}

#[derive(Deserialize)]
pub struct NewEnroll {
    name: String,
}

pub async fn create_enroll_key(State(st): State<St>, _: User, Json(n): Json<NewEnroll>) -> R {
    let key = format!("npk_{}", crypto::token());
    let db = st.db.lock().unwrap();
    let name = if n.name.trim().is_empty() { "default" } else { n.name.trim() };
    db.execute("INSERT INTO enroll_keys (name, key_hash, created) VALUES (?1, ?2, ?3)", params![name, crypto::sha(&key), now()])?;
    let id = db.last_insert_rowid();
    audit(&db, "enroll_key.create", name);
    Ok(Json(json!({ "id": id, "key": key })))
}

pub async fn delete_enroll_key(State(st): State<St>, _: User, Path(id): Path<i64>) -> R {
    let db = st.db.lock().unwrap();
    db.execute("DELETE FROM enroll_keys WHERE id=?1", [id])?;
    audit(&db, "enroll_key.delete", &id.to_string());
    ok()
}

pub async fn audit_log(State(st): State<St>, _: User) -> R {
    let db = st.db.lock().unwrap();
    let rows: Vec<Value> = db
        .prepare("SELECT ts, action, detail FROM audit ORDER BY id DESC LIMIT 200")?
        .query_map([], |r| Ok(json!({ "ts": r.get::<_, i64>(0)?, "action": r.get::<_, String>(1)?, "detail": r.get::<_, String>(2)? })))?
        .collect::<Result<_, _>>()?;
    Ok(Json(json!({ "entries": rows })))
}
