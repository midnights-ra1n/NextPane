//! Secrets at rest (AES-256-GCM with a server key) and OpenSSL-compatible encrypted exports.

use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use cbc::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::{fs, io::Write, os::unix::fs::OpenOptionsExt, path::Path};

/// PBKDF2 iterations for exports; must match the `-iter` in the documented openssl command.
pub const EXPORT_ITER: u32 = 600_000;

pub fn load_key(path: &Path) -> std::io::Result<[u8; 32]> {
    if let Ok(k) = fs::read(path) {
        if let Ok(k) = k.try_into() {
            return Ok(k);
        }
        return Err(std::io::Error::other("secret.key must be 32 bytes"));
    }
    let mut k = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut k);
    fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?.write_all(&k)?;
    Ok(k)
}

pub fn token() -> String {
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

pub fn sha(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

pub fn seal(key: &[u8; 32], plain: &str) -> String {
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    let ct = Aes256Gcm::new(key.into()).encrypt(Nonce::from_slice(&nonce), plain.as_bytes()).expect("aes-gcm encrypt");
    B64.encode([&nonce[..], &ct].concat())
}

pub fn open(key: &[u8; 32], sealed: &str) -> Option<String> {
    let raw = B64.decode(sealed).ok()?;
    if raw.len() < 12 {
        return None;
    }
    let pt = Aes256Gcm::new(key.into()).decrypt(Nonce::from_slice(&raw[..12]), &raw[12..]).ok()?;
    String::from_utf8(pt).ok()
}

/// Same output as `openssl enc -aes-256-cbc -pbkdf2 -iter 600000 -md sha256 -a -A`, so exports can be
/// decrypted anywhere without NextPane.
pub fn openssl_enc(pass: &str, data: &[u8]) -> String {
    let mut salt = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut salt);
    let mut kiv = [0u8; 48];
    pbkdf2::pbkdf2_hmac::<Sha256>(pass.as_bytes(), &salt, EXPORT_ITER, &mut kiv);
    let ct = cbc::Encryptor::<aes::Aes256>::new(kiv[..32].into(), kiv[32..].into()).encrypt_padded_vec_mut::<Pkcs7>(data);
    B64.encode([b"Salted__".as_slice(), &salt, &ct].concat())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cbc::cipher::BlockDecryptMut;

    #[test]
    fn seal_roundtrip() {
        let k = [7u8; 32];
        let s = seal(&k, "hunter2");
        assert_eq!(open(&k, &s).as_deref(), Some("hunter2"));
        assert_eq!(open(&[8u8; 32], &s), None);
    }

    #[test]
    fn openssl_format_roundtrip() {
        let out = B64.decode(openssl_enc("pw", b"A=1\n")).unwrap();
        assert_eq!(&out[..8], b"Salted__");
        let mut kiv = [0u8; 48];
        pbkdf2::pbkdf2_hmac::<Sha256>(b"pw", &out[8..16], EXPORT_ITER, &mut kiv);
        let pt = cbc::Decryptor::<aes::Aes256>::new(kiv[..32].into(), kiv[32..].into()).decrypt_padded_vec_mut::<Pkcs7>(&out[16..]).unwrap();
        assert_eq!(pt, b"A=1\n");
    }
}
