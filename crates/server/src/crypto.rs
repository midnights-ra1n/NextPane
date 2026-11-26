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

fn put(b: &mut Vec<u8>, d: &[u8]) {
    b.extend((d.len() as u32).to_be_bytes());
    b.extend(d);
}

/// New ed25519 SSH key pair: (private key in OpenSSH format, unencrypted; public key line).
pub fn ssh_keypair(comment: &str) -> (String, String) {
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    let kp = ring::signature::Ed25519KeyPair::from_seed_unchecked(&seed).expect("ed25519 seed");
    let public = ring::signature::KeyPair::public_key(&kp).as_ref();
    let mut blob = vec![];
    put(&mut blob, b"ssh-ed25519");
    put(&mut blob, public);
    // openssh-key-v1: https://github.com/openssh/openssh-portable/blob/master/PROTOCOL.key
    let mut check = [0u8; 4];
    rand::thread_rng().fill_bytes(&mut check);
    let mut private = [check, check].concat();
    put(&mut private, b"ssh-ed25519");
    put(&mut private, public);
    put(&mut private, &[&seed[..], public].concat());
    put(&mut private, comment.as_bytes());
    for i in 1..=7u8 {
        if private.len() % 8 == 0 {
            break;
        }
        private.push(i);
    }
    let mut file = b"openssh-key-v1\0".to_vec();
    put(&mut file, b"none");
    put(&mut file, b"none");
    put(&mut file, b"");
    file.extend(1u32.to_be_bytes());
    put(&mut file, &blob);
    put(&mut file, &private);
    let b64 = B64.encode(file);
    let lines: Vec<&str> = b64.as_bytes().chunks(70).map(|c| std::str::from_utf8(c).unwrap()).collect();
    (
        format!("-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----\n", lines.join("\n")),
        format!("ssh-ed25519 {} {comment}", B64.encode(blob)),
    )
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
    fn ssh_keypair_is_valid_openssh() {
        let (private, public) = ssh_keypair("nextpane");
        assert!(private.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----\n") && public.starts_with("ssh-ed25519 "));
        assert!(nextpane_proto::valid_ssh_pubkey(&public));
        // When ssh-keygen is available, it must read the key back and derive the same public key.
        let path = std::env::temp_dir().join(format!("np-test-{}", token()));
        fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).unwrap().write_all(private.as_bytes()).unwrap();
        let out = std::process::Command::new("ssh-keygen").arg("-y").arg("-f").arg(&path).output();
        let _ = fs::remove_file(&path);
        if let Ok(o) = out {
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
            let derived = String::from_utf8_lossy(&o.stdout);
            assert_eq!(derived.split_whitespace().nth(1), public.split_whitespace().nth(1));
        }
    }

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
