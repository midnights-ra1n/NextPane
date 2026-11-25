//! Per-app "allow" rules on ufw, firewalld, nftables or iptables.
//! Commands are spawned with argv (no shell); every rule field was validated by `AppSpec::validate`.

use nextpane_proto::FwRule;
use std::{fs, path::PathBuf, process::Command};

pub fn backend() -> &'static str {
    match std::env::var("NEXTPANE_FIREWALL").as_deref() {
        Ok("ufw") => "ufw",
        Ok("firewalld") => "firewalld",
        Ok("nftables") => "nftables",
        Ok("iptables") => "iptables",
        Ok("none") => "none",
        _ => detect(),
    }
}

fn detect() -> &'static str {
    if out("ufw", &["status"]).is_some_and(|o| o.contains("Status: active")) {
        "ufw"
    } else if out("firewall-cmd", &["--state"]).is_some_and(|o| o.trim() == "running") {
        "firewalld"
    } else if out("nft", &["list", "tables"]).is_some() {
        "nftables"
    } else if out("iptables", &["-S", "INPUT"]).is_some() {
        "iptables"
    } else {
        "none"
    }
}

fn out(prog: &str, args: &[&str]) -> Option<String> {
    Command::new(prog).args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).into())
}

fn run(prog: &str, args: &[&str]) -> Result<(), String> {
    let o = Command::new(prog).args(args).output().map_err(|e| format!("{prog}: {e}"))?;
    if o.status.success() {
        Ok(())
    } else {
        Err(format!("{prog} {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim()))
    }
}

fn run_owned(prog: &str, args: Vec<String>) -> Result<(), String> {
    run(prog, &args.iter().map(String::as_str).collect::<Vec<_>>())
}

fn state_file(app: &str) -> PathBuf {
    PathBuf::from(crate::STATE).join("firewall").join(format!("{app}.json"))
}

fn is_v6(src: &str) -> bool {
    src.contains(':')
}

fn ufw_args(r: &FwRule) -> Vec<String> {
    let src = r.source.clone().unwrap_or_else(|| "any".into());
    ["allow", "proto", &r.proto, "from", &src, "to", "any", "port", &r.port.to_string()].map(String::from).to_vec()
}

fn rich_rule(r: &FwRule) -> String {
    match &r.source {
        Some(s) => format!(
            "rule family=\"{}\" source address=\"{s}\" port port=\"{}\" protocol=\"{}\" accept",
            if is_v6(s) { "ipv6" } else { "ipv4" },
            r.port,
            r.proto
        ),
        None => format!("rule port port=\"{}\" protocol=\"{}\" accept", r.port, r.proto),
    }
}

/// Make the host firewall match `rules` for this app. Returns a log line.
pub fn apply(app: &str, rules: &[FwRule]) -> Result<String, String> {
    let backend = backend();
    let state = state_file(app);
    let old: Vec<FwRule> = fs::read_to_string(&state).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    match backend {
        // Persistent backends: diff against what we applied last time.
        "ufw" | "firewalld" => {
            if old == rules {
                return Ok(format!("firewall ({backend}): unchanged\n"));
            }
            for r in &old {
                let _ = match backend {
                    "ufw" => run_owned("ufw", [vec!["delete".into()], ufw_args(r)].concat()),
                    _ => run("firewall-cmd", &["--permanent", &format!("--remove-rich-rule={}", rich_rule(r))]),
                };
            }
            for r in rules {
                match backend {
                    "ufw" => run_owned("ufw", [ufw_args(r), vec!["comment".into(), format!("nextpane:{app}")]].concat())?,
                    _ => run("firewall-cmd", &["--permanent", &format!("--add-rich-rule={}", rich_rule(r))])?,
                }
            }
            if backend == "firewalld" {
                run("firewall-cmd", &["--reload"])?;
            }
        }
        // Runtime-only backends: one chain per app, rebuilt from scratch (also re-run at every agent start).
        "nftables" => nft_apply(app, rules)?,
        "iptables" => ipt_apply(app, rules)?,
        _ if rules.is_empty() => return Ok(String::new()),
        _ => return Err("no supported firewall found (ufw, firewalld, nftables, iptables)".into()),
    }
    if let Some(dir) = state.parent() {
        let _ = fs::create_dir_all(dir);
    }
    fs::write(&state, serde_json::to_string(rules).unwrap()).map_err(|e| e.to_string())?;
    Ok(format!("firewall ({backend}): {} rule(s) applied\n", rules.len()))
}

pub fn remove(app: &str) -> Result<String, String> {
    let r = apply(app, &[])?;
    let _ = fs::remove_file(state_file(app));
    Ok(r)
}

fn ipt_apply(app: &str, rules: &[FwRule]) -> Result<(), String> {
    let chain = format!("NP-{app}");
    for (bin, v6) in [("iptables", false), ("ip6tables", true)] {
        if out(bin, &["-S", "INPUT"]).is_none() {
            continue;
        }
        if rules.is_empty() {
            let _ = run(bin, &["-D", "INPUT", "-j", &chain]);
            let _ = run(bin, &["-F", &chain]);
            let _ = run(bin, &["-X", &chain]);
            continue;
        }
        let _ = run(bin, &["-N", &chain]); // fails if it already exists
        run(bin, &["-F", &chain])?;
        for r in rules.iter().filter(|r| r.source.as_deref().is_none_or(|s| is_v6(s) == v6)) {
            let port = r.port.to_string();
            let mut a = vec!["-A", &chain, "-p", &r.proto];
            if let Some(s) = &r.source {
                a.extend(["-s", s]);
            }
            a.extend(["--dport", &port, "-j", "ACCEPT"]);
            run(bin, &a)?;
        }
        if run(bin, &["-C", "INPUT", "-j", &chain]).is_err() {
            run(bin, &["-I", "INPUT", "1", "-j", &chain])?;
        }
    }
    Ok(())
}

/// Rules live in `inet filter` (the distro default table) so they are evaluated by the same
/// chain that holds the host's drop policy.
fn nft_apply(app: &str, rules: &[FwRule]) -> Result<(), String> {
    let chain = format!("np-{app}");
    if out("nft", &["list", "chain", "inet", "filter", "input"]).is_none() {
        if rules.is_empty() {
            return Ok(());
        }
        run("nft", &["add", "table", "inet", "filter"])?;
        run("nft", &["add", "chain", "inet", "filter", "input", "{ type filter hook input priority 0 ; policy accept ; }"])?;
    }
    let listing = out("nft", &["-a", "list", "chain", "inet", "filter", "input"]).unwrap_or_default();
    let jump = format!("jump {chain} ");
    let handle = listing.lines().find(|l| l.contains(&jump)).and_then(|l| l.rsplit("handle ").next()).map(|h| h.trim().to_string());
    if rules.is_empty() {
        if let Some(h) = handle {
            run("nft", &["delete", "rule", "inet", "filter", "input", "handle", &h])?;
        }
        let _ = run("nft", &["delete", "chain", "inet", "filter", &chain]);
        return Ok(());
    }
    run("nft", &["add", "chain", "inet", "filter", &chain])?;
    run("nft", &["flush", "chain", "inet", "filter", &chain])?;
    for r in rules {
        let port = r.port.to_string();
        let mut a = vec!["add", "rule", "inet", "filter", &chain];
        if let Some(s) = &r.source {
            a.extend([if is_v6(s) { "ip6" } else { "ip" }, "saddr", s]);
        }
        a.extend([r.proto.as_str(), "dport", &port, "accept"]);
        run("nft", &a)?;
    }
    if handle.is_none() {
        run("nft", &["insert", "rule", "inet", "filter", "input", "jump", &chain])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_rendering() {
        let r = FwRule { port: 3000, proto: "tcp".into(), source: Some("10.0.0.0/8".into()) };
        assert_eq!(rich_rule(&r), r#"rule family="ipv4" source address="10.0.0.0/8" port port="3000" protocol="tcp" accept"#);
        assert_eq!(ufw_args(&FwRule { source: None, ..r.clone() }).join(" "), "allow proto tcp from any to any port 3000");
        assert!(rich_rule(&FwRule { source: Some("::1".into()), ..r }).contains("ipv6"));
    }
}
