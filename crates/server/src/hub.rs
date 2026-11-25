//! Agent WebSocket: enrollment, metrics intake, command round-trips.

use crate::{api, crypto, db::audit, notify, now, ApiErr, CmdResult, Live, St};
use axum::{
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use nextpane_proto::{AgentMsg, AppSpec, Command, ServerMsg};
use rusqlite::{params, OptionalExtension};
use std::{sync::atomic::Ordering, time::Duration};
use tokio::{sync::{mpsc, oneshot}, time::timeout};

const HISTORY: usize = 360;

fn forbidden(m: &str) -> ApiErr {
    ApiErr(StatusCode::UNAUTHORIZED, m.into())
}

pub async fn ws(State(st): State<St>, headers: HeaderMap, upgrade: WebSocketUpgrade) -> Result<Response, ApiErr> {
    let h = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).map(str::trim).filter(|v| !v.is_empty());
    let machine = h("x-machine-id").filter(|m| m.len() <= 64 && m.bytes().all(|c| c.is_ascii_alphanumeric())).ok_or_else(|| forbidden("missing machine id"))?.to_string();
    let enroll_ok = |db: &rusqlite::Connection| {
        h("authorization").and_then(|a| a.strip_prefix("Bearer ")).is_some_and(|k| {
            db.query_row("SELECT 1 FROM enroll_keys WHERE key_hash=?1", [crypto::sha(k.trim())], |_| Ok(())).is_ok()
        })
    };

    let (host_id, welcome) = {
        let db = st.db.lock().unwrap();
        let row: Option<(i64, String)> = db.query_row("SELECT id, secret_hash FROM hosts WHERE machine_id=?1", [&machine], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        match row {
            // Known host: its own secret is required. An empty secret means "re-enrollment allowed" (reset from the UI).
            Some((id, hash)) if !hash.is_empty() => {
                if h("x-host-secret").map(crypto::sha) != Some(hash) {
                    return Err(forbidden("invalid host secret"));
                }
                (id, None)
            }
            row => {
                if !enroll_ok(&db) {
                    return Err(forbidden("invalid enrollment token"));
                }
                let secret = crypto::token();
                let id = match row {
                    Some((id, _)) => {
                        db.execute("UPDATE hosts SET secret_hash=?1 WHERE id=?2", params![crypto::sha(&secret), id])?;
                        id
                    }
                    None => {
                        db.execute(
                            "INSERT INTO hosts (machine_id, secret_hash, name, created) VALUES (?1, ?2, ?3, ?4)",
                            params![machine, crypto::sha(&secret), format!("host-{}", &machine[..machine.len().min(8)]), now()],
                        )?;
                        db.last_insert_rowid()
                    }
                };
                audit(&db, "host.enroll", &format!("host {id} ({machine})"));
                (id, Some(secret))
            }
        }
    };
    Ok(upgrade.on_upgrade(move |sock| session(st, host_id, welcome, sock)))
}

pub fn sync_msg(st: &St, host_id: i64) -> Result<ServerMsg, ApiErr> {
    let db = st.db.lock().unwrap();
    let apps = db.prepare("SELECT spec FROM apps WHERE host_id=?1")?.query_map([host_id], |r| r.get::<_, String>(0))?.filter_map(|s| serde_json::from_str::<AppSpec>(&s.ok()?).ok()).collect();
    let ssh_keys = db.prepare("SELECT pubkey FROM ssh_keys WHERE host_id=?1")?.query_map([host_id], |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok(ServerMsg::Sync { apps, ssh_keys, metrics_interval: st.cfg.metrics_interval, poll_interval: st.cfg.poll_interval })
}

/// Push the desired state again (after apps or keys changed). No-op when the host is offline.
pub fn resync(st: &St, host_id: i64) {
    if let Ok(m) = sync_msg(st, host_id) {
        if let Some(l) = st.live.lock().unwrap().get(&host_id) {
            let _ = l.tx.send(m);
        }
    }
}

async fn session(st: St, host_id: i64, welcome: Option<String>, sock: WebSocket) {
    let (mut sink, mut stream) = sock.split();
    let Ok(Some(Ok(Message::Text(first)))) = timeout(Duration::from_secs(15), stream.next()).await else { return };
    let Ok(AgentMsg::Hello(info)) = serde_json::from_str::<AgentMsg>(&first) else { return };

    let (tx, mut rx) = mpsc::unbounded_channel::<ServerMsg>();
    if let Some(s) = welcome {
        let _ = tx.send(ServerMsg::Welcome { host_secret: s });
    }
    if let Ok(m) = sync_msg(&st, host_id) {
        let _ = tx.send(m);
    }
    let conn = st.next_id.fetch_add(1, Ordering::Relaxed);
    {
        let db = st.db.lock().unwrap();
        db.execute("UPDATE hosts SET info=?1, last_seen=?2 WHERE id=?3", params![serde_json::to_string(&info).unwrap(), now(), host_id]).ok();
        // The first time we hear from a host, name it after its hostname.
        db.execute("UPDATE hosts SET name=?1 WHERE id=?2 AND name LIKE 'host-%'", params![info.hostname, host_id]).ok();
    }
    st.live.lock().unwrap().insert(host_id, Live { tx, info, conn });
    if st.offline_notified.lock().unwrap().remove(&host_id) {
        notify::send(&st, "host_online", format!("NextPane: {} is back online", notify::host_name(&st, host_id)));
    }

    let writer = tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            if sink.send(Message::Text(serde_json::to_string(&m).unwrap().into())).await.is_err() {
                break;
            }
        }
    });

    // Agents ping every 30 s; silence for 90 s means the link is dead.
    while let Ok(Some(Ok(msg))) = timeout(Duration::from_secs(90), stream.next()).await {
        // Host deleted in the panel, or replaced by a newer connection: drop this one.
        if !st.live.lock().unwrap().get(&host_id).is_some_and(|l| l.conn == conn) {
            break;
        }
        let Message::Text(t) = msg else {
            if matches!(msg, Message::Close(_)) { break }
            continue;
        };
        match serde_json::from_str::<AgentMsg>(&t) {
            Ok(AgentMsg::Metrics(m)) => {
                let mut h = st.history.lock().unwrap();
                let q = h.entry(host_id).or_default();
                q.push_back(m);
                while q.len() > HISTORY {
                    q.pop_front();
                }
            }
            Ok(AgentMsg::Result { id, ok, output, data }) => {
                if let Some(p) = st.pending.lock().unwrap().remove(&id) {
                    let _ = p.send(CmdResult { ok, output, data });
                }
            }
            Ok(AgentMsg::Head { app, sha }) => api::on_head(&st, host_id, &app, &sha),
            Ok(AgentMsg::Hello(info)) => {
                if let Some(l) = st.live.lock().unwrap().get_mut(&host_id) {
                    l.info = info;
                }
            }
            Err(e) => eprintln!("host {host_id}: bad message: {e}"),
        }
    }
    writer.abort();
    let ours = {
        let mut live = st.live.lock().unwrap();
        let ours = live.get(&host_id).is_some_and(|l| l.conn == conn);
        if ours {
            live.remove(&host_id);
        }
        ours
    };
    st.db.lock().unwrap().execute("UPDATE hosts SET last_seen=?1 WHERE id=?2", params![now(), host_id]).ok();
    // Grace period: agent restarts and updates reconnect within seconds and should not page anyone.
    if ours {
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let gone = !st.live.lock().unwrap().contains_key(&host_id);
            if gone && st.db.lock().unwrap().query_row("SELECT 1 FROM hosts WHERE id=?1", [host_id], |_| Ok(())).is_ok() && st.offline_notified.lock().unwrap().insert(host_id) {
                notify::send(&st, "host_offline", format!("NextPane: {} is OFFLINE (no agent connection for 1 minute)", notify::host_name(&st, host_id)));
            }
        });
    }
}

/// Send a command and wait for the agent's answer.
pub async fn command(st: &St, host_id: i64, cmd: Command, wait: Duration) -> Result<CmdResult, ApiErr> {
    let tx = st.live.lock().unwrap().get(&host_id).map(|l| l.tx.clone()).ok_or_else(|| ApiErr(StatusCode::SERVICE_UNAVAILABLE, "host is offline".into()))?;
    let id = st.next_id.fetch_add(1, Ordering::Relaxed);
    let (otx, orx) = oneshot::channel();
    st.pending.lock().unwrap().insert(id, otx);
    if tx.send(ServerMsg::Cmd { id, cmd }).is_err() {
        st.pending.lock().unwrap().remove(&id);
        return Err(ApiErr(StatusCode::SERVICE_UNAVAILABLE, "host is offline".into()));
    }
    match timeout(wait, orx).await {
        Ok(Ok(r)) => Ok(r),
        _ => {
            st.pending.lock().unwrap().remove(&id);
            Err(ApiErr(StatusCode::GATEWAY_TIMEOUT, "the agent did not answer in time".into()))
        }
    }
}
