//! Self-update: download the release asset matching this host, check its sha256, install it.

use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt, path::Path, process::Command, time::Duration};

pub fn asset_name(version: &str, method: &str) -> String {
    let arch = std::env::consts::ARCH;
    match method {
        "deb" | "rpm" => format!("nextpane-agent-{version}-{arch}.{method}"),
        _ => format!("nextpane-agent-{version}-{arch}"),
    }
}

pub fn sha256_file(p: &Path) -> Result<String, String> {
    Ok(hex::encode(Sha256::digest(fs::read(p).map_err(|e| e.to_string())?)))
}

pub fn run(base_url: &str, version: &str, files: &BTreeMap<String, String>) -> Result<String, String> {
    if !version.bytes().all(|c| c.is_ascii_alphanumeric() || b".-+".contains(&c)) {
        return Err("invalid version".into());
    }
    let method = crate::install_method();
    let name = asset_name(version, method);
    let want = files.get(&name).ok_or_else(|| format!("release {version} has no asset {name}"))?;
    let dir = Path::new(crate::STATE).join("update");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(&name);
    let url = format!("{}/{name}", base_url.trim_end_matches('/'));
    let o = Command::new("curl").args(["-fsSL", "--proto", "=https,http", "--max-time", "600", "-o"]).arg(&file).arg(&url).output().map_err(|e| format!("curl: {e}"))?;
    if !o.status.success() {
        return Err(format!("download of {url} failed: {}", String::from_utf8_lossy(&o.stderr)));
    }
    let got = sha256_file(&file)?;
    if !got.eq_ignore_ascii_case(want) {
        let _ = fs::remove_file(&file);
        return Err(format!("checksum mismatch for {name}: expected {want}, got {got}"));
    }
    // The package's postinst restarts the agent, so the install runs in its own transient unit
    // instead of inside our cgroup (which the restart would kill).
    let install = |args: &[&str]| -> Result<(), String> {
        let o = Command::new("systemd-run").args(["--unit=nextpane-agent-update", "--collect"]).args(args).arg(&file).output().map_err(|e| e.to_string())?;
        if o.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&o.stderr).into()) }
    };
    match method {
        "deb" => install(&["dpkg", "-i"])?,
        "rpm" => install(&["rpm", "-U", "--replacepkgs"])?,
        _ => {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let tmp = exe.with_extension("new");
            fs::copy(&file, &tmp).map_err(|e| e.to_string())?;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
            fs::rename(&tmp, &exe).map_err(|e| e.to_string())?;
            let _ = fs::remove_file(&file);
            // Exit after the result is sent; systemd (Restart=always) starts the new binary.
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_secs(2));
                std::process::exit(0);
            });
        }
    }
    Ok(format!("updating to {version} ({method}), the agent will reconnect shortly\n"))
}
