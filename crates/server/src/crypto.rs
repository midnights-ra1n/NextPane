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

fn take<'a>(r: &mut &'a [u8]) -> Option<&'a [u8]> {
    let len = u32::from_be_bytes(r.get(..4)?.try_into().ok()?) as usize;
    let d = r.get(4..4 + len)?;
    *r = &r[4 + len..];
    Some(d)
}

/// Checks a pasted OpenSSH private key and returns it re-wrapped (canonical PEM) with its public key line.
pub fn import_ssh_key(pasted: &str) -> Result<(String, String), &'static str> {
    const BEGIN: &str = "-----BEGIN OPENSSH PRIVATE KEY-----";
    const END: &str = "-----END OPENSSH PRIVATE KEY-----";
    if pasted.len() > 16 * 1024 {
        return Err("key too large");
    }
    let body = pasted.trim().strip_prefix(BEGIN).and_then(|s| s.strip_suffix(END))
        .ok_or("expected an OpenSSH private key (-----BEGIN OPENSSH PRIVATE KEY----- … -----END OPENSSH PRIVATE KEY-----)")?;
    let raw = B64.decode(body.split_whitespace().collect::<String>()).map_err(|_| "invalid key encoding")?;
    let mut r = raw.strip_prefix(b"openssh-key-v1\0".as_slice()).ok_or("not an OpenSSH private key")?;
    let cipher = take(&mut r).ok_or("truncated key")?;
    if cipher != b"none" {
        return Err("this key is protected by a passphrase: NextPane needs it without one (ssh-keygen -p -N '' -f <key>)");
    }
    let (_kdf, _opts) = (take(&mut r).ok_or("truncated key")?, take(&mut r).ok_or("truncated key")?);
    if r.get(..4) != Some(&1u32.to_be_bytes()[..]) {
        return Err("expected exactly one key");
    }
    r = &r[4..];
    let blob = take(&mut r).ok_or("truncated key")?;
    let kind = std::str::from_utf8(take(&mut &blob[..]).ok_or("truncated key")?).map_err(|_| "invalid key type")?;
    if !(kind == "ssh-ed25519" || kind == "ssh-rsa" || kind.starts_with("ecdsa-sha2-")) {
        return Err("unsupported key type (use ed25519, RSA or ECDSA)");
    }
    take(&mut r).ok_or("truncated key")?; // the private section must be there
    let b64 = B64.encode(&raw);
    let lines: Vec<&str> = b64.as_bytes().chunks(70).map(|c| std::str::from_utf8(c).unwrap()).collect();
    Ok((format!("{BEGIN}\n{}\n{END}\n", lines.join("\n")), format!("{kind} {} imported", B64.encode(blob))))
}

/// `SHA256:…` fingerprint of a public key line, as shown by ssh-keygen -l and Git providers.
pub fn ssh_fingerprint(public: &str) -> String {
    let blob = public.split_whitespace().nth(1).and_then(|b| B64.decode(b).ok()).unwrap_or_default();
    format!("SHA256:{}", base64::engine::general_purpose::STANDARD_NO_PAD.encode(Sha256::digest(blob)))
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
    fn import_ssh_key_roundtrip() {
        let (private, public) = ssh_keypair("x");
        // Pasted from a password manager: CRLF and one long base64 line.
        let body: String = private.lines().filter(|l| !l.starts_with("-----")).collect();
        let pasted = format!("-----BEGIN OPENSSH PRIVATE KEY-----\r\n{body}\r\n-----END OPENSSH PRIVATE KEY-----\r\n");
        let (canon, pubkey) = import_ssh_key(&pasted).unwrap();
        assert_eq!(canon, private);
        assert_eq!(pubkey.split_whitespace().nth(1), public.split_whitespace().nth(1));
        assert!(import_ssh_key("ssh-ed25519 AAAA me").is_err());
        assert!(import_ssh_key("-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----").is_err());
        let enc = B64.encode([b"openssh-key-v1\0".as_slice(), &[0, 0, 0, 10], b"aes256-ctr"].concat());
        let e = import_ssh_key(&format!("-----BEGIN OPENSSH PRIVATE KEY-----\n{enc}\n-----END OPENSSH PRIVATE KEY-----")).unwrap_err();
        assert!(e.contains("passphrase"));
    }

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
            let pubpath = std::env::temp_dir().join(format!("np-test-{}.pub", token()));
            fs::write(&pubpath, &public).unwrap();
            let l = std::process::Command::new("ssh-keygen").arg("-l").arg("-f").arg(&pubpath).output().unwrap();
            let _ = fs::remove_file(&pubpath);
            assert_eq!(String::from_utf8_lossy(&l.stdout).split_whitespace().nth(1), Some(ssh_fingerprint(&public).as_str()));
            for kind in ["ed25519", "rsa", "ecdsa"] {
                let p = std::env::temp_dir().join(format!("np-test-{}", token()));
                let ok = std::process::Command::new("ssh-keygen").args(["-q", "-t", kind, "-N", "", "-C", "me@laptop", "-f"]).arg(&p).status().unwrap().success();
                let (private, pubfile) = (fs::read_to_string(&p).unwrap_or_default(), fs::read_to_string(p.with_extension("pub")).unwrap_or_default());
                let _ = (fs::remove_file(&p), fs::remove_file(p.with_extension("pub")));
                assert!(ok);
                let (_, imported) = import_ssh_key(&private).unwrap();
                assert_eq!(imported.split_whitespace().take(2).collect::<Vec<_>>(), pubfile.split_whitespace().take(2).collect::<Vec<_>>(), "{kind}");
            }
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
