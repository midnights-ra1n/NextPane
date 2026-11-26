//! NextPane server: REST API + embedded web UI + WebSocket hub for agents.

mod api;
mod auth;
mod crypto;
mod db;
mod discord;
mod hub;
mod notify;
mod tls;
mod update;

use axum::{
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use nextpane_proto::{HostInfo, Metrics, ServerMsg};
use rusqlite::Connection;
use std::{
    collections::{HashMap, VecDeque},
    net::SocketAddr,
    path::PathBuf,
    sync::{atomic::AtomicU64, Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{mpsc, oneshot};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct Config {
    pub listen: SocketAddr,
    pub data_dir: PathBuf,
    /// e.g. https://github.com/you/nextpane/releases ; empty disables updates.
    pub release_url: String,
    /// deb | rpm | docker | binary
    pub install: String,
    pub metrics_interval: u64,
    pub poll_interval: u64,
    /// HTTPS served by the panel itself (NEXTPANE_TLS).
    pub tls: bool,
}

pub struct Live {
    pub tx: mpsc::UnboundedSender<ServerMsg>,
    pub info: HostInfo,
    pub conn: u64,
}

pub struct CmdResult {
    pub ok: bool,
    pub output: String,
    pub data: Option<String>,
}

pub struct Ctx {
    pub cfg: Config,
    pub db: Mutex<Connection>,
    pub key: [u8; 32],
    pub live: Mutex<HashMap<i64, Live>>,
    // shortcut: metrics history is in memory only (last hour), persist it if longer history is needed.
    pub history: Mutex<HashMap<i64, VecDeque<Metrics>>>,
    pub pending: Mutex<HashMap<u64, oneshot::Sender<CmdResult>>>,
    pub next_id: AtomicU64,
    pub manifest: Mutex<Option<(Instant, update::Manifest)>>,
    /// One-time code required to create the first admin, printed in the logs.
    pub setup_code: Mutex<Option<String>>,
    /// Hosts we sent a "host_offline" notification for (so we announce when they come back).
    pub offline_notified: Mutex<std::collections::HashSet<i64>>,
    /// Set when the panel uses its self-signed certificate.
    pub tls_pin: Option<tls::Pin>,
}
pub type St = Arc<Ctx>;

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// JSON error response: `{"error": "..."}`.
#[derive(Debug)]
pub struct ApiErr(pub StatusCode, pub String);
impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}
impl From<rusqlite::Error> for ApiErr {
    fn from(e: rusqlite::Error) -> Self {
        match e {
            rusqlite::Error::QueryReturnedNoRows => ApiErr(StatusCode::NOT_FOUND, "not found".into()),
            e => {
                eprintln!("database error: {e}");
                ApiErr(StatusCode::INTERNAL_SERVER_ERROR, "database error".into())
            }
        }
    }
}
pub fn bad(msg: impl Into<String>) -> ApiErr {
    ApiErr(StatusCode::BAD_REQUEST, msg.into())
}

/// deb | rpm | binary (Docker sets NEXTPANE_INSTALL=docker explicitly).
fn detect_install() -> &'static str {
    let ok = |p: &str, a: &[&str]| std::process::Command::new(p).args(a).output().is_ok_and(|o| o.status.success());
    if ok("dpkg-query", &["-W", "nextpane-server"]) {
        "deb"
    } else if ok("rpm", &["-q", "nextpane-server"]) {
        "rpm"
    } else {
        "binary"
    }
}

fn env(k: &str, default: &str) -> String {
    std::env::var(k).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.into())
}

#[tokio::main]
async fn main() {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("nextpane-server {VERSION}");
        return;
    }
    let cfg = Config {
        listen: env("NEXTPANE_LISTEN", "0.0.0.0:3000").parse().expect("NEXTPANE_LISTEN must be host:port"),
        data_dir: env("NEXTPANE_DATA", "/var/lib/nextpane-server").into(),
        release_url: env("NEXTPANE_RELEASE_URL", "").trim_end_matches('/').into(),
        install: env("NEXTPANE_INSTALL", detect_install()),
        metrics_interval: env("NEXTPANE_METRICS_INTERVAL", "10").parse().unwrap_or(10),
        poll_interval: env("NEXTPANE_POLL_INTERVAL", "60").parse().unwrap_or(60),
        tls: false,
    };
    std::fs::create_dir_all(&cfg.data_dir).expect("cannot create data dir");
    let conn = db::open(&cfg.data_dir.join("nextpane.db")).expect("cannot open database");
    let key = crypto::load_key(&cfg.data_dir.join("secret.key")).expect("cannot load secret.key");

    let needs_setup: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0)).unwrap();
    let setup_code = (needs_setup == 0).then(|| {
        let c = crypto::token()[..12].to_string();
        println!("\n  No admin account yet. Open the panel and use this setup code: {c}\n");
        c
    });

    let (cert, key_file) = (env("NEXTPANE_TLS_CERT", ""), env("NEXTPANE_TLS_KEY", ""));
    let (tls_cfg, tls_pin) = match env("NEXTPANE_TLS", if cert.is_empty() { "off" } else { "on" }).as_str() {
        "off" => (None, None),
        "on" | "self-signed" => {
            if cert.is_empty() != key_file.is_empty() {
                panic!("set both NEXTPANE_TLS_CERT and NEXTPANE_TLS_KEY");
            }
            let (c, p) = tls::load(&cfg.data_dir, &cert, &key_file).unwrap_or_else(|e| panic!("TLS: {e}"));
            if let Some(p) = &p {
                println!("  Self-signed certificate. Agents: NEXTPANE_FINGERPRINT={}  curl: --pinnedpubkey sha256//{}", p.cert, p.spki);
            }
            (Some(c), p)
        }
        v => panic!("NEXTPANE_TLS must be off, on or self-signed, not {v:?}"),
    };
    let cfg = Config { tls: tls_cfg.is_some(), ..cfg };

    let listen = cfg.listen;
    let st: St = Arc::new(Ctx {
        cfg,
        db: Mutex::new(conn),
        key,
        live: Default::default(),
        history: Default::default(),
        pending: Default::default(),
        next_id: AtomicU64::new(1),
        manifest: Default::default(),
        setup_code: Mutex::new(setup_code),
        offline_notified: Default::default(),
        tls_pin,
    });

    let app = Router::new()
        .route("/api/setup", post(auth::setup))
        .route("/api/login", post(auth::login))
        .route("/api/logout", post(auth::logout))
        .route("/api/me", get(auth::me))
        .route("/api/password", post(auth::password))
        .route("/api/hosts", get(api::hosts))
        .route("/api/hosts/{id}", get(api::host).patch(api::patch_host).delete(api::delete_host))
        .route("/api/hosts/{id}/update", post(api::update_agent))
        .route("/api/ssh-keys", get(api::saved_keys).post(api::save_key))
        .route("/api/ssh-keys/{id}", delete(api::forget_key))
        .route("/api/hosts/{id}/ssh-keys", post(api::add_ssh_key))
        .route("/api/hosts/{id}/ssh-keys/{kid}", delete(api::delete_ssh_key))
        .route("/api/hosts/{id}/apps", post(api::create_app))
        .route("/api/apps", get(api::all_apps))
        .route("/api/deployments", get(api::deployments))
        .route("/api/settings", get(notify::get_settings).put(notify::put_settings))
        .route("/api/settings/test", post(notify::test))
        .route("/api/apps/{id}", get(api::app).put(api::update_app).delete(api::delete_app))
        .route("/api/apps/{id}/deploy", post(api::deploy))
        .route("/api/apps/{id}/action", post(api::action))
        .route("/api/apps/{id}/cache", post(api::clear_cache))
        .route("/api/apps/{id}/logs", get(api::logs))
        .route("/api/apps/{id}/deploy-key", post(api::deploy_key))
        .route("/api/apps/{id}/env", get(api::get_env).put(api::put_env))
        .route("/api/apps/{id}/env/export", post(api::export_env))
        .route("/api/apps/{id}/discord", get(discord::info))
        .route("/api/apps/{id}/discord/commands/{cmd}", delete(discord::delete_command))
        .route("/api/apps/{id}/discord/guilds/{guild}", delete(discord::leave_guild))
        .route("/api/deployments/{id}", get(api::deployment))
        .route("/api/enroll-keys", get(api::enroll_keys).post(api::create_enroll_key))
        .route("/api/enroll-keys/{id}", delete(api::delete_enroll_key))
        .route("/api/audit", get(api::audit_log))
        .route("/api/system", get(update::system))
        .route("/api/system/update", post(update::update_server))
        .route("/hooks/{app}/{secret}", post(api::webhook))
        .route("/agent/ws", get(hub::ws))
        .route("/install.sh", get(install_sh))
        .fallback(static_file)
        .with_state(st);

    let scheme = if tls_cfg.is_some() { "https" } else { "http" };
    println!("nextpane-server {VERSION} listening on {scheme}://{listen}");
    match tls_cfg {
        // shortcut: the certificate is read at start only; restart the panel after renewing it.
        Some(c) => axum_server::bind_rustls(listen, axum_server::tls_rustls::RustlsConfig::from_config(c)).serve(app.into_make_service()).await.unwrap(),
        None => axum::serve(tokio::net::TcpListener::bind(listen).await.expect("cannot bind"), app).await.unwrap(),
    }
}

async fn install_sh(axum::extract::State(st): axum::extract::State<St>) -> Response {
    if st.cfg.release_url.is_empty() {
        return (StatusCode::SERVICE_UNAVAILABLE, "NEXTPANE_RELEASE_URL is not configured on the server\n").into_response();
    }
    let body = include_str!("../../../packaging/install-agent.sh").replace("__RELEASE_URL__", &st.cfg.release_url);
    ([(header::CONTENT_TYPE, "text/x-shellscript")], body).into_response()
}

async fn static_file(uri: axum::http::Uri) -> Response {
    let (body, mime): (&[u8], &str) = match uri.path() {
        "/app.js" => (include_bytes!("../../../web/app.js"), "text/javascript"),
        "/style.css" => (include_bytes!("../../../web/style.css"), "text/css"),
        "/logo.svg" => (include_bytes!("../../../web/logo.svg"), "image/svg+xml"),
        "/fonts/Geist-Variable.woff2" => (include_bytes!("../../../web/fonts/Geist-Variable.woff2"), "font/woff2"),
        "/fonts/GeistMono-Variable.woff2" => (include_bytes!("../../../web/fonts/GeistMono-Variable.woff2"), "font/woff2"),
        p if p.starts_with("/api/") => return ApiErr(StatusCode::NOT_FOUND, "not found".into()).into_response(),
        _ => (include_bytes!("../../../web/index.html"), "text/html; charset=utf-8"),
    };
    let mut h = HeaderMap::new();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_str(mime).unwrap());
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'self'; img-src 'self' data: https://cdn.discordapp.com; frame-ancestors 'none'"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    (h, body).into_response()
}
