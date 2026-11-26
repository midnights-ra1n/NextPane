//! Optional built-in HTTPS, so the panel and agents work without a reverse proxy.

use base64::Engine;
use rcgen::PublicKeyData;
use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use sha2::{Digest, Sha256};
use std::{fs, io::Write, os::unix::fs::OpenOptionsExt, path::Path, sync::Arc};

/// Pins of a self-signed certificate: what agents and `curl` must trust.
pub struct Pin {
    /// sha256 of the certificate (DER), hex. Used by the agent (`NEXTPANE_FINGERPRINT`).
    pub cert: String,
    /// base64 sha256 of the public key (SPKI). Used by `curl --pinnedpubkey sha256//…`.
    pub spki: String,
}

/// Builds the TLS config. `cert`/`key` are PEM files; without them a self-signed certificate
/// is created once in `data_dir` and its pins are returned.
pub fn load(data_dir: &Path, cert: &str, key: &str) -> Result<(Arc<rustls::ServerConfig>, Option<Pin>), String> {
    let (cert_pem, key_pem, pin) = if !cert.is_empty() {
        let rd = |p: &str| fs::read_to_string(p).map_err(|e| format!("{p}: {e}"));
        (rd(cert)?, rd(key)?, None)
    } else {
        let (cp, kp) = (data_dir.join("tls.crt"), data_dir.join("tls.key"));
        if !cp.exists() || !kp.exists() {
            let c = rcgen::generate_simple_self_signed(vec!["nextpane".to_string()]).map_err(|e| e.to_string())?;
            let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&kp).map_err(|e| e.to_string())?;
            f.write_all(c.signing_key.serialize_pem().as_bytes()).map_err(|e| e.to_string())?;
            fs::write(&cp, c.cert.pem()).map_err(|e| e.to_string())?;
        }
        let (c, k) = (fs::read_to_string(&cp).map_err(|e| e.to_string())?, fs::read_to_string(&kp).map_err(|e| e.to_string())?);
        let spki = rcgen::KeyPair::from_pem(&k).map_err(|e| e.to_string())?.subject_public_key_info();
        let der = CertificateDer::from_pem_slice(c.as_bytes()).map_err(|e| e.to_string())?;
        let pin = Pin { cert: hex::encode(Sha256::digest(&der)), spki: base64::engine::general_purpose::STANDARD.encode(Sha256::digest(spki)) };
        (c, k, Some(pin))
    };
    let certs = CertificateDer::pem_slice_iter(cert_pem.as_bytes()).collect::<Result<Vec<_>, _>>().map_err(|e| format!("certificate: {e}"))?;
    let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).map_err(|e| format!("private key: {e}"))?;
    let cfg = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| e.to_string())?;
    Ok((Arc::new(cfg), pin))
}

#[cfg(test)]
mod tests {
    #[test]
    fn self_signed_is_created_once_and_pinned() {
        let dir = std::env::temp_dir().join(format!("nextpane-tls-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (_, a) = super::load(&dir, "", "").unwrap();
        let (_, b) = super::load(&dir, "", "").unwrap();
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_eq!((a.cert.len(), a.cert.as_str(), a.spki.as_str()), (64, b.cert.as_str(), b.spki.as_str()));
        std::fs::remove_dir_all(dir).ok();
    }
}
