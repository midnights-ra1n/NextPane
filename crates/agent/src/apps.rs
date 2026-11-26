//! App lifecycle: git fetch, install/build, systemd unit, env file, cache, SSH keys.
//!
//! Security rule: /var/lib/nextpane is owned by the unprivileged `nextpane` user, so every
//! filesystem write inside it runs *as that user*. Root never follows a path the user controls.

use crate::firewall;
use nextpane_proto::{validate_env, valid_app_name, valid_rel_path, valid_ssh_pubkey, AppAction, AppSpec, Command, EnvVar};
use std::{
    fs,
    io::Write,
    os::unix::{fs::OpenOptionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command as Cmd, Stdio},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

pub const HOME: &str = "/var/lib/nextpane";
const USER: &str = "nextpane";
const ENV_DIR: &str = "/etc/nextpane/apps";
const KEEP_RELEASES: usize = 3;
/// Shared deploy key, relative to HOME.
const GIT_KEY: &str = ".ssh/nextpane_git";
const MAX_LOG: usize = 100_000;

/// (output, data) on success, full log on failure.
pub type Out = Result<(String, Option<String>), String>;

static BUSY: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// One mutating operation per app at a time.
struct Busy(String);
impl Busy {
    fn take(app: &str) -> Result<Busy, String> {
        let mut b = BUSY.lock().unwrap();
        if b.iter().any(|a| a == app) {
            return Err(format!("{app} is busy with another operation"));
        }
        b.push(app.to_string());
        Ok(Busy(app.to_string()))
    }
}
impl Drop for Busy {
    fn drop(&mut self) {
        BUSY.lock().unwrap().retain(|a| a != &self.0);
    }
}

pub fn app_dir(name: &str) -> PathBuf {
    Path::new(HOME).join("apps").join(name)
}
fn unit_name(name: &str) -> String {
    format!("nextpane-app-{name}.service")
}
fn env_path(name: &str) -> String {
    format!("{ENV_DIR}/{name}.env")
}
fn path_var() -> String {
    std::env::var("NEXTPANE_PATH").unwrap_or_else(|_| "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into())
}

pub fn handle(cmd: Command) -> Out {
    let check = |n: &str| if valid_app_name(n) { Ok(()) } else { Err("invalid app name".to_string()) };
    match cmd {
        Command::Deploy { app, env, sha } => logged(|log| deploy(log, &app, &env, sha.as_deref()).map(Some)),
        Command::Configure { app, env } => logged(|log| configure(log, &app, &env).map(|_| None)),
        Command::Action { app, action } => {
            check(&app)?;
            let verb = match action { AppAction::Start => "start", AppAction::Stop => "stop", AppAction::Restart => "restart" };
            logged(|log| run(log, Cmd::new("systemctl").args([verb, &unit_name(&app)])).map(|_| None))
        }
        Command::Remove { app } => { check(&app)?; logged(|log| remove(log, &app).map(|_| None)) }
        Command::ClearCache { app, paths } => { check(&app)?; logged(|log| clear_cache(log, &app, &paths).map(|_| None)) }
        Command::Logs { app, lines } => {
            check(&app)?;
            let o = Cmd::new("journalctl").args(["-u", &unit_name(&app), "-n", &lines.min(5000).to_string(), "--no-pager", "-o", "short-iso"]).output().map_err(|e| e.to_string())?;
            Ok((String::from_utf8_lossy(&o.stdout).into(), None))
        }
        Command::DeployKey { app } => { check(&app)?; logged(|log| deploy_key(log, &app).map(Some)) }
        Command::SshKeys { keys } => logged(|log| authorized_keys(log, &keys).map(|_| None)),
        Command::SelfUpdate { base_url, version, files } => crate::update::run(&base_url, &version, &files).map(|o| (o, None)),
    }
}

fn logged(f: impl FnOnce(&mut String) -> Result<Option<String>, String>) -> Out {
    let mut log = String::new();
    let r = f(&mut log);
    if log.len() > MAX_LOG {
        let cut = log.len() - MAX_LOG;
        let cut = (cut..log.len()).find(|i| log.is_char_boundary(*i)).unwrap_or(cut);
        log = format!("[... truncated]\n{}", &log[cut..]);
    }
    match r {
        Ok(data) => Ok((log, data)),
        Err(e) => Err(format!("{log}\nerror: {e}")),
    }
}

fn run(log: &mut String, cmd: &mut Cmd) -> Result<String, String> {
    let prog = cmd.get_program().to_string_lossy().to_string();
    let out = cmd.stdin(Stdio::null()).output().map_err(|e| format!("{prog}: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    log.push_str(&stdout);
    log.push_str(&String::from_utf8_lossy(&out.stderr));
    if !out.status.success() {
        return Err(format!("{prog} failed ({})", out.status));
    }
    Ok(stdout)
}

fn user_ids() -> Result<(u32, u32), String> {
    fs::read_to_string("/etc/passwd")
        .map_err(|e| e.to_string())?
        .lines()
        .find_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            (f.len() > 3 && f[0] == USER).then(|| Some((f[2].parse().ok()?, f[3].parse().ok()?)))?
        })
        .ok_or_else(|| format!("user '{USER}' does not exist (reinstall the agent)"))
}

/// A process running as the app user (std drops supplementary groups when switching uid as root).
fn as_user(prog: &str) -> Result<Cmd, String> {
    let (uid, gid) = user_ids()?;
    let mut c = Cmd::new(prog);
    c.uid(uid).gid(gid).env_clear().env("HOME", HOME).env("PATH", path_var()).env("GIT_TERMINAL_PROMPT", "0").current_dir(HOME);
    Ok(c)
}

/// `sh -c script` as the app user; values go in as positional args ($1, $2...), never into the script.
fn user_sh(log: &mut String, script: &str, args: &[&str]) -> Result<String, String> {
    run(log, as_user("sh")?.arg("-c").arg(script).arg("sh").args(args))
}

/// The app's own key first: if the provider does not know it, ssh falls back to the shared key.
fn ssh_keys(app: &str) -> Vec<PathBuf> {
    [app_dir(app).join("deploy_key"), Path::new(HOME).join(GIT_KEY)].into_iter().filter(|k| k.exists()).collect()
}

fn git(app: &str) -> Result<Cmd, String> {
    let mut c = as_user("git")?;
    let keys: String = ssh_keys(app).iter().map(|k| format!("-i {} ", k.display())).collect();
    let only = if keys.is_empty() { "" } else { "-o IdentitiesOnly=yes " };
    c.env("GIT_SSH_COMMAND", format!("ssh {keys}{only}-o StrictHostKeyChecking=accept-new"));
    Ok(c)
}

pub fn remote_head(app: &AppSpec) -> Result<String, String> {
    app.validate()?;
    let mut log = String::new();
    let out = run(&mut log, git(&app.name)?.args(["ls-remote", "--", &app.repo, &format!("refs/heads/{}", app.branch)])).map_err(|e| format!("{e}: {log}"))?;
    out.split_whitespace().next().filter(|s| valid_sha(s)).map(String::from).ok_or_else(|| format!("branch {} not found", app.branch))
}

fn valid_sha(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.bytes().all(|c| c.is_ascii_hexdigit())
}

fn write_env(name: &str, env: &[EnvVar]) -> Result<(), String> {
    fs::create_dir_all(ENV_DIR).map_err(|e| e.to_string())?;
    let mut body = String::new();
    for v in env {
        body.push_str(&format!("{}=\"{}\"\n", v.key, v.value.replace('\\', "\\\\").replace('"', "\\\"")));
    }
    let tmp = format!("{}.tmp", env_path(name));
    let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).map_err(|e| e.to_string())?;
    f.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    fs::rename(&tmp, env_path(name)).map_err(|e| e.to_string())
}

fn package_manager(dir: &Path) -> &'static str {
    if dir.join("pnpm-lock.yaml").exists() {
        "pnpm"
    } else if dir.join("yarn.lock").exists() {
        "yarn"
    } else if dir.join("bun.lockb").exists() || dir.join("bun.lock").exists() {
        "bun"
    } else {
        "npm"
    }
}

fn default_install(dir: &Path) -> Option<String> {
    if !dir.join("package.json").exists() {
        return None;
    }
    Some(match package_manager(dir) {
        "npm" if dir.join("package-lock.json").exists() => "npm ci".into(),
        "npm" => "npm install".into(),
        pm => format!("{pm} install --frozen-lockfile"),
    })
}

fn package_json(dir: &Path) -> Option<serde_json::Value> {
    serde_json::from_str(&fs::read_to_string(dir.join("package.json")).ok()?).ok()
}

fn default_build(dir: &Path) -> Option<String> {
    package_json(dir)?.pointer("/scripts/build").is_some().then(|| format!("{} run build", package_manager(dir)))
}

/// Framework from package.json dependencies: (display name, start command used when there is no
/// "start" script). Static sites are served with their own preview server, bound to every interface.
pub fn framework(pkg: &serde_json::Value) -> Option<(&'static str, Option<&'static str>)> {
    let has = |d: &str| pkg.pointer(&format!("/dependencies/{d}")).or_else(|| pkg.pointer(&format!("/devDependencies/{d}"))).is_some();
    const VITE: &str = "node_modules/.bin/vite preview --host 0.0.0.0 --port ${PORT:-3000} --strictPort";
    Some(if has("next") {
        ("Next.js", Some("node_modules/.bin/next start"))
    } else if has("nuxt") {
        ("Nuxt", Some("node .output/server/index.mjs"))
    } else if has("@sveltejs/kit") {
        ("SvelteKit", Some("node build"))
    } else if has("astro") {
        ("Astro", Some("node_modules/.bin/astro preview --host 0.0.0.0 --port ${PORT:-3000}"))
    } else if has("gatsby") {
        ("Gatsby", Some("node_modules/.bin/gatsby serve -H 0.0.0.0 -p ${PORT:-3000}"))
    } else if has("@remix-run/react") {
        ("Remix", None)
    } else if has("@react-router/dev") {
        ("React Router", None)
    } else if has("@nestjs/core") {
        ("NestJS", Some("node dist/main"))
    } else if has("@angular/core") {
        ("Angular", None)
    } else if has("vite") {
        (if has("vue") { "Vue" } else if has("react") { "React" } else if has("svelte") { "Svelte" } else if has("solid-js") { "Solid" } else { "Vite" }, Some(VITE))
    } else if has("vue") {
        ("Vue", None)
    } else if has("express") {
        ("Express", None)
    } else if has("fastify") {
        ("Fastify", None)
    } else if has("hono") {
        ("Hono", None)
    } else {
        return None;
    })
}

/// `<pm> start` when package.json has a start script, else the framework's own server.
fn default_start(dir: &Path) -> String {
    let pm_start = format!("{} start", package_manager(dir));
    let Some(pkg) = package_json(dir) else { return pm_start };
    if pkg.pointer("/scripts/start").is_some() {
        return pm_start;
    }
    framework(&pkg).and_then(|f| f.1).map_or(pm_start, String::from)
}

/// Quote a command for `ExecStart=/bin/sh -c "..."` (systemd unescapes \\ and \", expands % and $).
fn systemd_quote(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%").replace('$', "$$")
}

fn limits(spec: &AppSpec) -> Vec<String> {
    let mut l = vec![];
    if let Some(m) = spec.memory_mb {
        l.push(format!("MemoryMax={m}M"));
    }
    if let Some(c) = spec.cpu_percent {
        l.push(format!("CPUQuota={c}%"));
    }
    l
}

pub fn unit_file(spec: &AppSpec, default_start: &str) -> String {
    let start = if spec.start_cmd.is_empty() { default_start } else { &spec.start_cmd };
    let port = spec.port.map(|p| format!("Environment=PORT={p}\n")).unwrap_or_default();
    let limits: String = limits(spec).iter().map(|l| format!("{l}\n")).collect();
    format!(
        "# Managed by NextPane, do not edit.\n[Unit]\nDescription=NextPane app {name}\nAfter=network.target\n\n\
[Service]\nUser={USER}\nGroup={USER}\nWorkingDirectory={dir}/current\nEnvironment=NODE_ENV=production\nEnvironment=HOME={HOME}\n\
Environment=PATH={path}\n{port}EnvironmentFile=-{env}\nExecStart=/bin/sh -c \"{start}\"\nRestart=on-failure\nRestartSec=3\n\
{limits}NoNewPrivileges=yes\nPrivateTmp=yes\nProtectSystem=full\nProtectHome=yes\n\n[Install]\nWantedBy=multi-user.target\n",
        name = spec.name,
        dir = app_dir(&spec.name).display(),
        path = path_var(),
        env = env_path(&spec.name),
        start = systemd_quote(start),
    )
}

fn install_unit(log: &mut String, spec: &AppSpec, dir: &Path) -> Result<(), String> {
    fs::write(format!("/etc/systemd/system/{}", unit_name(&spec.name)), unit_file(spec, &default_start(dir))).map_err(|e| e.to_string())?;
    run(log, Cmd::new("systemctl").arg("daemon-reload"))?;
    Ok(())
}

/// Install/build steps run in a transient systemd unit: as the app user, with the app's env and limits,
/// and accounted outside the agent's own cgroup.
fn build_step(log: &mut String, spec: &AppSpec, rel: &Path, cmd: &str) -> Result<(), String> {
    log.push_str(&format!("\n$ {cmd}\n"));
    for attempt in 1.. {
        let mut c = Cmd::new("systemd-run");
        c.args(["--wait", "--pipe", "--collect", "--quiet"])
            .arg(format!("--unit=nextpane-build-{}", spec.name))
            .arg(format!("--uid={USER}"))
            .arg(format!("--gid={USER}"))
            .args(["-p", &format!("WorkingDirectory={}", rel.display())])
            .args(["-p", &format!("EnvironmentFile=-{}", env_path(&spec.name))])
            .args(["-E", &format!("HOME={HOME}"), "-E", &format!("PATH={}", path_var())]);
        for l in limits(spec) {
            c.args(["-p", &l]);
        }
        let mut out = String::new();
        let r = run(&mut out, c.args(["/bin/sh", "-c", cmd])).map(|_| ());
        log.push_str(&out);
        // The unit never started (e.g. systemd was reloading during a package update): safe to retry.
        if r.is_err() && out.contains("Failed to start transient service unit") && attempt < 4 {
            log.push_str("systemd was busy, retrying in 5s\n");
            std::thread::sleep(std::time::Duration::from_secs(5));
            continue;
        }
        return r;
    }
    unreachable!()
}

fn deploy(log: &mut String, spec: &AppSpec, env: &[EnvVar], want: Option<&str>) -> Result<String, String> {
    spec.validate()?;
    validate_env(env)?;
    let _busy = Busy::take(&spec.name)?;
    let dir = app_dir(&spec.name);
    let ds = dir.to_string_lossy().to_string();
    write_env(&spec.name, env)?;
    user_sh(log, "mkdir -p \"$1/releases\" && { [ -d \"$1/repo.git\" ] || git init -q --bare \"$1/repo.git\"; }", &[&ds])?;

    let keys: Vec<String> = ssh_keys(&spec.name).iter().map(|k| k.display().to_string()).collect();
    log.push_str(&format!("ssh keys: {}\n", if keys.is_empty() { "none (no shared deploy key received from the panel)".into() } else { keys.join(", ") }));
    log.push_str(&format!("$ git fetch {} {}\n", spec.repo, spec.branch));
    let refspec = format!("+refs/heads/{0}:refs/heads/{0}", spec.branch);
    run(log, git(&spec.name)?.arg(format!("--git-dir={ds}/repo.git")).args(["fetch", "--force", "--depth=50", "--", &spec.repo, &refspec]))?;
    let sha = match want {
        Some(s) if valid_sha(s) => s.to_string(),
        Some(s) => return Err(format!("invalid commit '{s}'")),
        None => run(&mut String::new(), git(&spec.name)?.arg(format!("--git-dir={ds}/repo.git")).args(["rev-parse", &format!("refs/heads/{}", spec.branch)]))?.trim().to_string(),
    };
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let rel = dir.join("releases").join(format!("{ts}-{}", &sha[..12]));
    let rs = rel.to_string_lossy().to_string();
    log.push_str(&format!("$ checkout {sha}\n"));
    user_sh(log, "mkdir -p \"$2\" && cd \"$2\" && git --git-dir=\"$1/repo.git\" --work-tree=. checkout -f \"$3\" -- .", &[&ds, &rs, &sha])?;

    if let Some((name, _)) = package_json(&rel).as_ref().and_then(framework) {
        log.push_str(&format!("detected {name}\n"));
    }
    let install = Some(spec.install_cmd.clone()).filter(|c| !c.is_empty()).or_else(|| default_install(&rel));
    if let Some(cmd) = install {
        build_step(log, spec, &rel, &cmd)?;
    }
    let build = Some(spec.build_cmd.clone()).filter(|c| !c.is_empty()).or_else(|| default_build(&rel));
    if let Some(cmd) = build {
        build_step(log, spec, &rel, &cmd)?;
    }

    install_unit(log, spec, &rel)?;
    user_sh(log, "ln -sfn \"$2\" \"$1/.current.tmp\" && mv -Tf \"$1/.current.tmp\" \"$1/current\"", &[&ds, &rs])?;
    run(log, Cmd::new("systemctl").args(["enable", &unit_name(&spec.name)]))?;
    run(log, Cmd::new("systemctl").args(["restart", &unit_name(&spec.name)]))?;
    log.push_str(&format!("\nstarted {} at {}\n", spec.name, &sha[..12]));

    // Keep the newest releases (names start with a timestamp, so they sort chronologically).
    let mut releases: Vec<String> = fs::read_dir(dir.join("releases")).map_err(|e| e.to_string())?.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    releases.sort();
    for old in releases.iter().rev().skip(KEEP_RELEASES) {
        let _ = user_sh(log, "rm -rf -- \"$1/releases/$2\"", &[&ds, old]);
    }
    log.push_str(&firewall::apply(&spec.name, &spec.firewall).unwrap_or_else(|e| format!("firewall: {e}\n")));
    Ok(sha)
}

fn configure(log: &mut String, spec: &AppSpec, env: &[EnvVar]) -> Result<(), String> {
    spec.validate()?;
    validate_env(env)?;
    let _busy = Busy::take(&spec.name)?;
    write_env(&spec.name, env)?;
    let current = app_dir(&spec.name).join("current");
    if current.exists() {
        install_unit(log, spec, &current)?;
        run(log, Cmd::new("systemctl").args(["try-restart", &unit_name(&spec.name)]))?;
        log.push_str("configuration applied\n");
    } else {
        log.push_str("saved, will apply on first deploy\n");
    }
    log.push_str(&firewall::apply(&spec.name, &spec.firewall)?);
    Ok(())
}

fn remove(log: &mut String, name: &str) -> Result<(), String> {
    let _busy = Busy::take(name)?;
    let _ = run(log, Cmd::new("systemctl").args(["disable", "--now", &unit_name(name)]));
    let _ = fs::remove_file(format!("/etc/systemd/system/{}", unit_name(name)));
    let _ = run(log, Cmd::new("systemctl").arg("daemon-reload"));
    log.push_str(&firewall::remove(name)?);
    let _ = fs::remove_file(env_path(name));
    user_sh(log, "rm -rf -- \"$1\"", &[&app_dir(name).to_string_lossy()])?;
    log.push_str(&format!("{name} removed\n"));
    Ok(())
}

fn clear_cache(log: &mut String, name: &str, paths: &[String]) -> Result<(), String> {
    let _busy = Busy::take(name)?;
    let current = app_dir(name).join("current");
    for p in paths {
        if !valid_rel_path(p) {
            return Err(format!("invalid cache path '{p}'"));
        }
        user_sh(log, "rm -rf -- \"$1/$2\"", &[&current.to_string_lossy(), p])?;
        log.push_str(&format!("cleared {p}\n"));
    }
    run(log, Cmd::new("systemctl").args(["try-restart", &unit_name(name)]))?;
    Ok(())
}

fn deploy_key(log: &mut String, name: &str) -> Result<String, String> {
    let ds = app_dir(name).to_string_lossy().to_string();
    user_sh(log, "mkdir -p \"$1\" && { [ -f \"$1/deploy_key\" ] || ssh-keygen -q -t ed25519 -N '' -C \"nextpane@$2\" -f \"$1/deploy_key\"; } && cat \"$1/deploy_key.pub\"", &[&ds, name]).map(|k| k.trim().to_string())
}

/// Atomically write `body` to `~/<rel>` (mode 600) as the app user; the content goes through stdin.
fn user_write(log: &mut String, rel: &str, body: &str) -> Result<(), String> {
    let mut c = as_user("sh")?;
    c.args(["-c", "umask 077 && mkdir -p ~/.ssh && cat > ~/\"$1.tmp\" && mv -f ~/\"$1.tmp\" ~/\"$1\"", "sh", rel]);
    let mut child = c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    child.stdin.take().unwrap().write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    log.push_str(&String::from_utf8_lossy(&out.stderr));
    if !out.status.success() {
        return Err(format!("could not write ~/{rel}"));
    }
    Ok(())
}

fn authorized_keys(log: &mut String, keys: &[String]) -> Result<(), String> {
    if let Some(k) = keys.iter().find(|k| !valid_ssh_pubkey(k)) {
        return Err(format!("invalid public key: {k}"));
    }
    let body: String = keys.iter().map(|k| format!("{}\n", k.trim())).collect();
    user_write(log, ".ssh/authorized_keys", &body)?;
    log.push_str(&format!("{} authorized key(s) installed\n", keys.len()));
    Ok(())
}

fn git_key(key: Option<&str>) -> Result<(), String> {
    match key {
        Some(k) if k.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----") => user_write(&mut String::new(), GIT_KEY, k),
        Some(_) => Err("invalid shared deploy key".into()),
        None => user_sh(&mut String::new(), "rm -f ~/\"$1\"", &[GIT_KEY]).map(|_| ()),
    }
}

/// Desired state after (re)connect: firewall rules are re-applied (iptables/nft rules do not survive
/// reboots), SSH keys and the shared deploy key rewritten.
pub fn sync(apps: &[AppSpec], ssh_keys: &[String], shared_key: Option<&str>) {
    for a in apps.iter().filter(|a| a.validate().is_ok()) {
        if let Err(e) = firewall::apply(&a.name, &a.firewall) {
            eprintln!("{}: firewall: {e}", a.name);
        }
    }
    if let Err(e) = authorized_keys(&mut String::new(), ssh_keys) {
        eprintln!("ssh keys: {e}");
    }
    if let Err(e) = git_key(shared_key) {
        eprintln!("deploy key: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_quoting_and_limits() {
        let spec = AppSpec { name: "web".into(), start_cmd: r#"node server.js --msg "hi" $PORT 50%"#.into(), port: Some(3000), memory_mb: Some(512), cpu_percent: Some(50), ..Default::default() };
        let u = unit_file(&spec, "npm start");
        assert!(u.contains(r#"ExecStart=/bin/sh -c "node server.js --msg \"hi\" $$PORT 50%%""#), "{u}");
        assert!(u.contains("MemoryMax=512M\nCPUQuota=50%\n"));
        assert!(u.contains("Environment=PORT=3000\n"));
        assert!(unit_file(&AppSpec { name: "x".into(), ..Default::default() }, "pnpm start").contains("ExecStart=/bin/sh -c \"pnpm start\""));
        let vite = framework(&serde_json::json!({ "dependencies": { "vue": "3" }, "devDependencies": { "vite": "5" } })).unwrap();
        assert_eq!(vite.0, "Vue");
        assert!(unit_file(&AppSpec { name: "x".into(), ..Default::default() }, vite.1.unwrap()).contains("--port $${PORT:-3000}"));
        assert_eq!(framework(&serde_json::json!({ "dependencies": { "next": "15", "react": "19" } })).unwrap().0, "Next.js");
        assert!(framework(&serde_json::json!({ "dependencies": { "lodash": "4" } })).is_none());
        assert!(valid_sha(&"a".repeat(40)) && !valid_sha("abc"));
    }
}
