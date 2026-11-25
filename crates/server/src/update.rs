//! Release manifest + server self-update.
//!
//! A release publishes `latest.json` (`{"version": "x.y.z", "files": {"<asset>": "<sha256>"}}`) next to its
//! assets. deb/rpm installs hand the verified package to a root-owned systemd path unit
//! (`nextpane-server-update.path`); the server itself never needs root.

use crate::{auth::User, bad, db::audit, ApiErr, St, VERSION};
use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io::Read, os::unix::fs::PermissionsExt, path::PathBuf, time::{Duration, Instant}};

#[derive(Deserialize, Serialize, Clone)]
pub struct Manifest {
    pub version: String,
    pub files: BTreeMap<String, String>,
}

fn parse_version(v: &str) -> Vec<u64> {
    v.trim_start_matches('v').split(['.', '-']).map(|p| p.parse().unwrap_or(0)).collect()
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    parse_version(latest) > parse_version(current)
}

pub fn cached_version(st: &St) -> Option<String> {
    st.manifest.lock().unwrap().as_ref().map(|(_, m)| m.version.clone())
}

pub async fn manifest(st: &St, force: bool) -> Result<Manifest, ApiErr> {
    if st.cfg.release_url.is_empty() {
        return Err(bad("updates are disabled: set NEXTPANE_RELEASE_URL on the server"));
    }
    if let Some((at, m)) = st.manifest.lock().unwrap().as_ref() {
        if !force && at.elapsed() < Duration::from_secs(600) {
            return Ok(m.clone());
        }
    }
    let url = format!("{}/latest/download/latest.json", st.cfg.release_url);
    let m: Manifest = tokio::task::spawn_blocking(move || ureq::get(&url).timeout(Duration::from_secs(20)).call().map_err(|e| e.to_string())?.into_json::<Manifest>().map_err(|e| e.to_string()))
        .await
        .map_err(|e| bad(e.to_string()))?
        .map_err(|e| ApiErr(StatusCode::BAD_GATEWAY, format!("cannot read release manifest: {e}")))?;
    if m.version.is_empty() || !m.version.bytes().all(|c| c.is_ascii_alphanumeric() || b".-+".contains(&c)) {
        return Err(ApiErr(StatusCode::BAD_GATEWAY, "invalid version in release manifest".into()));
    }
    *st.manifest.lock().unwrap() = Some((Instant::now(), m.clone()));
    Ok(m)
}

pub async fn system(State(st): State<St>, _: User) -> Json<Value> {
    let m = manifest(&st, false).await;
    let latest = m.as_ref().ok().map(|m| m.version.clone());
    Json(json!({
        "version": VERSION,
        "install": st.cfg.install,
        "updates_enabled": !st.cfg.release_url.is_empty(),
        "latest": latest,
        "update_available": latest.as_deref().is_some_and(|l| is_newer(l, VERSION)),
        "error": m.err().map(|e| e.1),
    }))
}

fn download(url: &str, dest: &PathBuf, want: &str) -> Result<(), String> {
    let resp = ureq::get(url).timeout(Duration::from_secs(600)).call().map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    resp.into_reader().take(512 << 20).read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let got = hex::encode(Sha256::digest(&buf));
    if !got.eq_ignore_ascii_case(want) {
        return Err(format!("checksum mismatch: expected {want}, got {got}"));
    }
    fs::write(dest, &buf).map_err(|e| e.to_string())
}

pub async fn update_server(State(st): State<St>, _: User) -> Result<Json<Value>, ApiErr> {
    if st.cfg.install == "docker" {
        return Err(bad("running in Docker: pull the new image and recreate the container (docker compose pull && docker compose up -d)"));
    }
    let m = manifest(&st, true).await?;
    if !is_newer(&m.version, VERSION) {
        return Err(bad(format!("already up to date ({VERSION})")));
    }
    let arch = std::env::consts::ARCH;
    let v = &m.version;
    let upd = st.cfg.data_dir.join("update");
    fs::create_dir_all(&upd).map_err(|e| bad(e.to_string()))?;
    let (name, dest) = match st.cfg.install.as_str() {
        "deb" => (format!("nextpane-server-{v}-{arch}.deb"), upd.join("package.deb")),
        "rpm" => (format!("nextpane-server-{v}-{arch}.rpm"), upd.join("package.rpm")),
        _ => (format!("nextpane-server-{v}-{arch}"), std::env::current_exe().map_err(|e| bad(e.to_string()))?.with_extension("new")),
    };
    let want = m.files.get(&name).cloned().ok_or_else(|| bad(format!("release {v} has no asset {name}")))?;
    let url = format!("{}/download/v{v}/{name}", st.cfg.release_url);
    let d = dest.clone();
    tokio::task::spawn_blocking(move || download(&url, &d, &want)).await.map_err(|e| bad(e.to_string()))?.map_err(|e| ApiErr(StatusCode::BAD_GATEWAY, e))?;
    audit(&st.db.lock().unwrap(), "server.update", &format!("{VERSION} -> {v}"));

    if st.cfg.install == "deb" || st.cfg.install == "rpm" {
        // Picked up by nextpane-server-update.path, which installs the package as root and restarts us.
        fs::write(upd.join("pending"), v).map_err(|e| bad(e.to_string()))?;
    } else {
        let exe = std::env::current_exe().map_err(|e| bad(e.to_string()))?;
        fs::set_permissions(&dest, fs::Permissions::from_mode(0o755)).map_err(|e| bad(e.to_string()))?;
        fs::rename(&dest, &exe).map_err(|e| bad(format!("cannot replace {}: {e}", exe.display())))?;
        tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            std::process::exit(0); // the service manager restarts the new binary
        });
    }
    Ok(Json(json!({ "ok": true, "output": format!("updating to {v}, the panel will restart in a few seconds") })))
}

#[cfg(test)]
mod tests {
    #[test]
    fn versions() {
        assert!(super::is_newer("0.10.0", "0.9.3"));
        assert!(super::is_newer("v1.0.0", "0.9.9"));
        assert!(!super::is_newer("0.1.0", "0.1.0"));
    }
}
