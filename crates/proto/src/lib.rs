//! Wire protocol shared by the NextPane server and agent.
//! Every message is one JSON text frame on the agent WebSocket.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::IpAddr;

/// Agent -> server.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum AgentMsg {
    Hello(HostInfo),
    Metrics(Metrics),
    /// Answer to a `ServerMsg::Cmd` with the same id. `data` carries a command-specific value
    /// (deployed commit sha, generated public key...).
    Result { id: u64, ok: bool, output: String, #[serde(default)] data: Option<String> },
    /// Current head of an auto-deployed app's branch (sent on every poll).
    Head { app: String, sha: String },
}

/// Server -> agent.
#[allow(clippy::large_enum_variant)]
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ServerMsg {
    /// Sent once, the first time a host enrolls. The agent stores it and presents it on reconnect.
    Welcome { host_secret: String },
    /// Desired state, sent after every (re)connection.
    Sync { apps: Vec<AppSpec>, ssh_keys: Vec<String>, metrics_interval: u64, poll_interval: u64 },
    Cmd { id: u64, cmd: Command },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Command {
    Deploy { app: AppSpec, env: Vec<EnvVar>, sha: Option<String> },
    /// Rewrite unit, env file and firewall rules; restart the app if it was deployed.
    Configure { app: AppSpec, env: Vec<EnvVar> },
    Action { app: String, action: AppAction },
    Remove { app: String },
    ClearCache { app: String, paths: Vec<String> },
    Logs { app: String, lines: u32 },
    DeployKey { app: String },
    SshKeys { keys: Vec<String> },
    /// `files` maps release asset names to sha256; the agent picks the one for its arch/install method.
    SelfUpdate { base_url: String, version: String, files: BTreeMap<String, String> },
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AppAction { Start, Stop, Restart }

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct HostInfo {
    pub version: String,
    pub hostname: String,
    pub os: String,
    pub kernel: String,
    pub arch: String,
    pub cpus: u32,
    pub mem_total: u64,
    pub firewall: String,
    /// deb | rpm | binary
    pub install: String,
    /// Local address the agent uses to reach the panel (apps are served on <ip>:<port>).
    #[serde(default)]
    pub ip: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Metrics {
    pub ts: u64,
    /// Whole-host CPU usage, percent of total capacity.
    pub cpu: f32,
    pub load: [f32; 3],
    pub mem_used: u64,
    pub mem_total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    pub disk_used: u64,
    pub disk_total: u64,
    /// Bytes per second, all interfaces but loopback.
    pub net_rx: u64,
    pub net_tx: u64,
    pub uptime: u64,
    pub apps: Vec<AppMetrics>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct AppMetrics {
    pub name: String,
    pub running: bool,
    /// Percent of total host capacity, so it is directly comparable with `Metrics::cpu`.
    pub cpu: f32,
    pub mem: u64,
    pub disk: u64,
    /// discord.js version found in the deployed release (the app is a Discord bot).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discord_js: Option<String>,
    /// Framework detected from the deployed package.json (Next.js, Nuxt, Vue...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub framework: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct AppSpec {
    pub name: String,
    pub repo: String,
    pub branch: String,
    /// Empty = detected from the lockfile.
    #[serde(default)]
    pub install_cmd: String,
    /// Empty = `<pm> run build` when package.json has a build script.
    #[serde(default)]
    pub build_cmd: String,
    /// Empty = `npm start`.
    #[serde(default)]
    pub start_cmd: String,
    #[serde(default)]
    pub port: Option<u16>,
    /// systemd CPUQuota, 100 = one full core.
    #[serde(default)]
    pub cpu_percent: Option<u32>,
    #[serde(default)]
    pub memory_mb: Option<u64>,
    /// Paths relative to the release dir, wiped by "clear cache".
    #[serde(default)]
    pub cache_paths: Vec<String>,
    #[serde(default)]
    pub auto_deploy: bool,
    #[serde(default)]
    pub firewall: Vec<FwRule>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct FwRule {
    pub port: u16,
    /// tcp | udp
    pub proto: String,
    /// IP or CIDR; None = anywhere.
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct EnvVar {
    pub key: String,
    pub value: String,
}

fn no_ctrl(field: &str, v: &str) -> Result<(), String> {
    if v.chars().any(|c| c == '\n' || c == '\r' || c == '\0') {
        return Err(format!("{field}: line breaks are not allowed"));
    }
    Ok(())
}

pub fn valid_app_name(n: &str) -> bool {
    // <= 24 chars so firewall chain names ("NP-" + name) stay under iptables' 28-char limit.
    let b = n.as_bytes();
    !b.is_empty() && b.len() <= 24 && b[0] != b'-'
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

pub fn valid_env_key(k: &str) -> bool {
    let b = k.as_bytes();
    !b.is_empty() && !b[0].is_ascii_digit() && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_')
}

pub fn valid_branch(b: &str) -> bool {
    !b.is_empty() && !b.starts_with('-') && !b.contains("..") && b.len() <= 200
        && b.bytes().all(|c| c.is_ascii_alphanumeric() || b"._/-".contains(&c))
}

pub fn valid_cidr(s: &str) -> bool {
    let (ip, prefix) = s.split_once('/').map_or((s, None), |(a, p)| (a, Some(p)));
    let Ok(ip) = ip.parse::<IpAddr>() else { return false };
    match prefix {
        None => true,
        Some(p) => p.parse::<u8>().is_ok_and(|p| p <= if ip.is_ipv4() { 32 } else { 128 }),
    }
}

pub fn valid_rel_path(p: &str) -> bool {
    !p.is_empty() && !p.starts_with('/') && p.split('/').all(|c| !c.is_empty() && c != "." && c != "..")
        && no_ctrl("path", p).is_ok()
}

pub fn valid_ssh_pubkey(k: &str) -> bool {
    let k = k.trim();
    no_ctrl("key", k).is_ok()
        && ["ssh-", "ecdsa-", "sk-"].iter().any(|p| k.starts_with(p))
        && k.split_whitespace().count() >= 2
}

impl AppSpec {
    /// Checked by the server on input and again by the agent before touching the host.
    pub fn validate(&self) -> Result<(), String> {
        if !valid_app_name(&self.name) {
            return Err("name: 1-24 chars, lowercase letters, digits and '-'".into());
        }
        if self.repo.is_empty() || self.repo.starts_with('-') || self.repo.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err("repo: invalid git URL".into());
        }
        if !valid_branch(&self.branch) {
            return Err("branch: invalid".into());
        }
        no_ctrl("install_cmd", &self.install_cmd)?;
        no_ctrl("build_cmd", &self.build_cmd)?;
        no_ctrl("start_cmd", &self.start_cmd)?;
        if self.cpu_percent == Some(0) || self.memory_mb == Some(0) {
            return Err("limits must be > 0".into());
        }
        if let Some(p) = self.cache_paths.iter().find(|p| !valid_rel_path(p)) {
            return Err(format!("cache path '{p}' must be relative, without '..'"));
        }
        for r in &self.firewall {
            if r.port == 0 || !(r.proto == "tcp" || r.proto == "udp") {
                return Err("firewall: port 1-65535, proto tcp|udp".into());
            }
            if r.source.as_deref().is_some_and(|s| !valid_cidr(s)) {
                return Err("firewall: source must be an IP or CIDR".into());
            }
        }
        Ok(())
    }
}

pub fn validate_env(env: &[EnvVar]) -> Result<(), String> {
    for v in env {
        if !valid_env_key(&v.key) {
            return Err(format!("invalid variable name '{}'", v.key));
        }
        no_ctrl(&v.key, &v.value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> AppSpec {
        AppSpec { name: "web".into(), repo: "git@github.com:a/b.git".into(), branch: "main".into(), ..Default::default() }
    }

    #[test]
    fn validation() {
        assert!(spec().validate().is_ok());
        for bad in ["", "-x", "Web", "a_b", "abcdefghijklmnopqrstuvwxy"] {
            assert!(!valid_app_name(bad), "{bad}");
        }
        assert!(AppSpec { repo: "--upload-pack=x".into(), ..spec() }.validate().is_err());
        assert!(AppSpec { branch: "-f".into(), ..spec() }.validate().is_err());
        assert!(AppSpec { start_cmd: "node a\nUser=root".into(), ..spec() }.validate().is_err());
        assert!(AppSpec { cache_paths: vec!["../etc".into()], ..spec() }.validate().is_err());
        assert!(AppSpec { cache_paths: vec![".next/cache".into()], ..spec() }.validate().is_ok());
        assert!(valid_cidr("10.0.0.0/8") && valid_cidr("::1") && valid_cidr("2001:db8::/32"));
        assert!(!valid_cidr("10.0.0.0/33") && !valid_cidr("any") && !valid_cidr("1.2.3.4; rm"));
        assert!(validate_env(&[EnvVar { key: "1A".into(), value: "x".into() }]).is_err());
        assert!(validate_env(&[EnvVar { key: "A".into(), value: "x\ny".into() }]).is_err());
        assert!(valid_ssh_pubkey("ssh-ed25519 AAAA me@x") && !valid_ssh_pubkey("ssh-ed25519 A\nB"));
    }
}
