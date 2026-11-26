//! NextPane agent: one outbound WebSocket to the server, everything else is local.

mod apps;
mod firewall;
mod metrics;
mod pinned;
mod update;

use futures_util::{SinkExt, StreamExt};
use nextpane_proto::{AgentMsg, AppSpec, HostInfo, ServerMsg};
use std::{fs, io::Write, os::unix::fs::OpenOptionsExt, process::Command, time::{Duration, Instant}};
use tokio::{sync::mpsc, task::spawn_blocking, time::interval};
use tokio_tungstenite::{connect_async_tls_with_config, tungstenite::{client::IntoClientRequest, Message}, Connector, MaybeTlsStream};

/// Root-only agent state (host secret, firewall state, update downloads).
pub const STATE: &str = "/var/lib/nextpane-agent";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

type Error = Box<dyn std::error::Error>;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--version") => return println!("nextpane-agent {VERSION}"),
        // Called on uninstall: close every port NextPane opened.
        Some("--cleanup-firewall") => {
            for e in fs::read_dir(format!("{STATE}/firewall")).into_iter().flatten().flatten() {
                if let Some(app) = e.file_name().to_string_lossy().strip_suffix(".json") {
                    if let Err(err) = firewall::remove(app) {
                        eprintln!("{app}: {err}");
                    }
                }
            }
            return;
        }
        _ => {}
    }
    let server = std::env::var("NEXTPANE_SERVER").unwrap_or_default();
    if server.is_empty() {
        eprintln!("NEXTPANE_SERVER is not set (edit /etc/nextpane/agent.env)");
        std::process::exit(2);
    }
    let mut backoff = 2;
    loop {
        let started = Instant::now();
        if let Err(e) = session(&server).await {
            eprintln!("disconnected: {e}");
        }
        if started.elapsed() > Duration::from_secs(60) {
            backoff = 2;
        }
        tokio::time::sleep(Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(60);
    }
}

async fn session(server: &str) -> Result<(), Error> {
    let mut req = server.into_client_request()?;
    let h = req.headers_mut();
    h.insert("x-machine-id", fs::read_to_string("/etc/machine-id")?.trim().parse()?);
    if let Ok(t) = std::env::var("NEXTPANE_TOKEN").map(|t| t.trim().to_string()) {
        if !t.is_empty() {
            h.insert("authorization", format!("Bearer {t}").parse()?);
        }
    }
    if let Ok(s) = fs::read_to_string(format!("{STATE}/host.secret")) {
        h.insert("x-host-secret", s.trim().parse()?);
    }
    let fp = std::env::var("NEXTPANE_FINGERPRINT").unwrap_or_default().replace(':', "").to_ascii_lowercase();
    let connector = (!fp.trim().is_empty()).then(|| Connector::Rustls(pinned::config(fp.trim())));
    let (ws, _) = connect_async_tls_with_config(req, None, false, connector).await?;
    eprintln!("connected to {server}");
    let (tx, mut rx) = mpsc::unbounded_channel::<AgentMsg>();
    let mut info = spawn_blocking(host_info).await?;
    let tcp = match ws.get_ref() {
        MaybeTlsStream::Plain(t) => Some(t),
        MaybeTlsStream::Rustls(t) => Some(t.get_ref().0),
        _ => None,
    };
    info.ip = tcp.and_then(|t| t.local_addr().ok()).map(|a| a.ip().to_string()).unwrap_or_default();
    let (mut sink, mut stream) = ws.split();
    sink.send(Message::text(serde_json::to_string(&AgentMsg::Hello(info))?)).await?;

    let mut apps: Vec<AppSpec> = vec![];
    let mut collector = metrics::Collector::default();
    let mut disk_at = Instant::now() - Duration::from_secs(3600);
    let mut metrics_tick = interval(Duration::from_secs(10));
    let mut poll_tick = interval(Duration::from_secs(60));
    let mut ping = interval(Duration::from_secs(30));
    let mut last_rx = Instant::now();

    loop {
        tokio::select! {
            msg = stream.next() => {
                let Some(msg) = msg else { return Ok(()) };
                last_rx = Instant::now();
                let text = match msg? {
                    Message::Text(t) => t,
                    Message::Close(_) => return Ok(()),
                    _ => continue,
                };
                match serde_json::from_str::<ServerMsg>(&text) {
                    Ok(ServerMsg::Welcome { host_secret }) => save_secret(&host_secret)?,
                    Ok(ServerMsg::Sync { apps: a, ssh_keys, metrics_interval, poll_interval, git_key }) => {
                        apps = a;
                        metrics_tick = interval(Duration::from_secs(metrics_interval.max(2)));
                        poll_tick = interval(Duration::from_secs(poll_interval.max(15)));
                        let a = apps.clone();
                        spawn_blocking(move || apps::sync(&a, &ssh_keys, git_key.as_deref()));
                    }
                    Ok(ServerMsg::Cmd { id, cmd }) => {
                        let tx = tx.clone();
                        spawn_blocking(move || {
                            let msg = match apps::handle(cmd) {
                                Ok((output, data)) => AgentMsg::Result { id, ok: true, output, data },
                                Err(output) => AgentMsg::Result { id, ok: false, output, data: None },
                            };
                            let _ = tx.send(msg);
                        });
                    }
                    Err(e) => eprintln!("ignoring message: {e}"),
                }
            }
            Some(m) = rx.recv() => sink.send(Message::text(serde_json::to_string(&m)?)).await?,
            _ = metrics_tick.tick() => {
                let names: Vec<String> = apps.iter().map(|a| a.name.clone()).collect();
                if disk_at.elapsed() > Duration::from_secs(300) {
                    disk_at = Instant::now();
                    let (n, d) = (names.clone(), collector.disk.clone());
                    spawn_blocking(move || metrics::refresh_disk(&n, &d));
                }
                let m = collector.sample(&names);
                sink.send(Message::text(serde_json::to_string(&AgentMsg::Metrics(m))?)).await?;
            }
            _ = poll_tick.tick() => {
                for app in apps.iter().filter(|a| a.auto_deploy).cloned() {
                    let tx = tx.clone();
                    spawn_blocking(move || match apps::remote_head(&app) {
                        Ok(sha) => { let _ = tx.send(AgentMsg::Head { app: app.name, sha }); }
                        Err(e) => eprintln!("{}: git ls-remote failed: {e}", app.name),
                    });
                }
            }
            _ = ping.tick() => {
                if last_rx.elapsed() > Duration::from_secs(90) {
                    return Err("server stopped answering".into());
                }
                sink.send(Message::Ping(Default::default())).await?;
            }
        }
    }
}

fn save_secret(secret: &str) -> std::io::Result<()> {
    fs::create_dir_all(STATE)?;
    let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(format!("{STATE}/host.secret"))?;
    f.write_all(secret.as_bytes())
}

fn read(p: &str) -> String {
    fs::read_to_string(p).unwrap_or_default().trim().to_string()
}

fn host_info() -> HostInfo {
    let os = read("/etc/os-release").lines().find_map(|l| l.strip_prefix("PRETTY_NAME=")).unwrap_or("Linux").trim_matches('"').to_string();
    let mem_kb: u64 = read("/proc/meminfo").lines().next().and_then(|l| l.split_whitespace().nth(1)?.parse().ok()).unwrap_or(0);
    HostInfo {
        version: VERSION.into(),
        hostname: read("/proc/sys/kernel/hostname"),
        os,
        kernel: read("/proc/sys/kernel/osrelease"),
        arch: std::env::consts::ARCH.into(),
        cpus: std::thread::available_parallelism().map_or(1, |n| n.get() as u32),
        mem_total: mem_kb * 1024,
        firewall: firewall::backend().into(),
        install: install_method().into(),
        ip: String::new(), // filled from the panel connection
    }
}

/// How this agent was installed, which decides how it updates itself.
pub fn install_method() -> &'static str {
    let ok = |p: &str, a: &[&str]| Command::new(p).args(a).output().is_ok_and(|o| o.status.success());
    if ok("dpkg-query", &["-W", "nextpane-agent"]) {
        "deb"
    } else if ok("rpm", &["-q", "nextpane-agent"]) {
        "rpm"
    } else {
        "binary"
    }
}
