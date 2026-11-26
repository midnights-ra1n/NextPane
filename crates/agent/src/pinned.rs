//! TLS for a panel with a self-signed certificate: trust exactly the certificate whose sha256
//! is NEXTPANE_FINGERPRINT (shown by the panel), instead of a certificate authority.

use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{ring, verify_tls12_signature, verify_tls13_signature, CryptoProvider},
    pki_types::{CertificateDer, ServerName, UnixTime},
    DigitallySignedStruct, Error, SignatureScheme,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[derive(Debug)]
struct Pinned {
    sha256: String,
    provider: CryptoProvider,
}

pub fn config(sha256: &str) -> Arc<rustls::ClientConfig> {
    let provider = ring::default_provider();
    let verifier = Arc::new(Pinned { sha256: sha256.into(), provider: provider.clone() });
    let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(provider))
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions")
        .dangerous() // the pin replaces CA validation; the handshake signature is still checked below
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Arc::new(cfg)
}

fn matches(pin: &str, cert: &[u8]) -> bool {
    hex::encode(Sha256::digest(cert)) == pin
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(&self, cert: &CertificateDer, _: &[CertificateDer], _: &ServerName, _: &[u8], _: UnixTime) -> Result<ServerCertVerified, Error> {
        if matches(&self.sha256, cert) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(Error::General(format!("server certificate {} does not match NEXTPANE_FINGERPRINT", hex::encode(Sha256::digest(cert)))))
        }
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(m, c, d, &self.provider.signature_verification_algorithms)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(m, c, d, &self.provider.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn pin_matches_only_the_same_certificate() {
        let pin = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(b"cert"));
        assert!(super::matches(&pin, b"cert"));
        assert!(!super::matches(&pin, b"other"));
    }
}
