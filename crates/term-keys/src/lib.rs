//! Import PuTTY `.ppk` private keys (ADR 0038).
//!
//! Parse PPK v2 (SHA-1 KDF, HMAC-SHA-1) and v3 (Argon2id, HMAC-SHA-256),
//! verify the MAC, decrypt the private blob, and re-encode the key as an
//! OpenSSH private key encrypted under a passphrase the caller supplies. The
//! output is never written unencrypted: [`import_ppk`] refuses an empty new
//! passphrase.
//!
//! Everything here runs in-process and depends only on permissive crates
//! (ADR 0006); see [`docs/decisions/0038-ppk-import.md`].

use std::convert::TryInto;

use aes::cipher::{block_padding::NoPadding, BlockDecryptMut, KeyIvInit};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use hmac::{Hmac, Mac};
use sha1::{Digest, Sha1};
use sha2::Sha256;
use zeroize::Zeroizing;

type HmacSha1 = Hmac<Sha1>;
type HmacSha256 = Hmac<Sha256>;

/// The symmetric key, IV and MAC key derived from the passphrase.
type KeyMaterial = (Vec<u8>, Vec<u8>, Vec<u8>);

/// The fixed prefix of the PPK v2 MAC key (ADR 0038, PPK spec C.5.1).
const MAC_KEY_PREFIX: &[u8] = b"putty-private-key-file-mac-key";

/// An imported key: the encrypted OpenSSH private key and its public line.
pub struct Imported {
    /// An OpenSSH private-key PEM block, passphrase-encrypted.
    pub private_openssh: Zeroizing<String>,
    /// `ssh-ed25519 AAAA… comment`, ready for `authorized_keys`/`.pub`.
    pub public_openssh: String,
}

/// Parse `text` (the `.ppk` file), decrypt it with `old`, and re-encrypt the
/// key under `new`. `old` may be empty for an unencrypted PPK; `new` must not
/// be empty.
pub fn import_ppk(text: &str, old: &str, new: &str) -> Result<Imported, String> {
    if new.is_empty() {
        return Err("the new passphrase must not be empty".into());
    }
    let ppk = Ppk::parse(text)?;
    let plain = ppk.decrypt(old)?;
    ppk.verify_mac(old, &plain)?;
    let unencrypted = ppk.to_openssh_private(&plain)?;

    let key = ssh_key::PrivateKey::from_openssh(unencrypted.as_bytes())
        .map_err(|e| format!("the converted key is invalid: {e}"))?;
    let mut rng = ssh_key::rand_core::OsRng;
    let key = key
        .encrypt(&mut rng, new)
        .map_err(|e| format!("could not encrypt the key: {e}"))?;
    let pem = key
        .to_openssh(ssh_key::LineEnding::LF)
        .map_err(|e| e.to_string())?;

    let body = base64::engine::general_purpose::STANDARD.encode(&ppk.public);
    let public_openssh = if ppk.comment.is_empty() {
        format!("{} {body}\n", ppk.algorithm)
    } else {
        format!("{} {body} {}\n", ppk.algorithm, ppk.comment)
    };
    Ok(Imported {
        private_openssh: pem,
        public_openssh,
    })
}

/// A parsed PPK header plus the still-encoded private blob.
struct Ppk {
    version: u32,
    algorithm: String,
    encryption: String,
    comment: String,
    public: Vec<u8>,
    private: Zeroizing<Vec<u8>>,
    mac: Vec<u8>,
    argon: Option<Argon>,
}

struct Argon {
    memory: u32,
    passes: u32,
    parallelism: u32,
    salt: Vec<u8>,
}

impl Ppk {
    fn parse(text: &str) -> Result<Self, String> {
        // PPK files use LF, but tolerate CR/CRLF (PPK spec C.2).
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let mut lines = text.lines();
        let first = lines.next().ok_or("the file is empty")?;
        let (version, algorithm) = if let Some(v) = first.strip_prefix("PuTTY-User-Key-File-2: ") {
            (2u32, v.trim().to_string())
        } else if let Some(v) = first.strip_prefix("PuTTY-User-Key-File-3: ") {
            (3u32, v.trim().to_string())
        } else {
            return Err("not a PuTTY .ppk file (version 1 is not supported)".into());
        };

        let (mut encryption, mut comment, mut public, mut private, mut mac) =
            (None, None, None, None, None);
        let (mut kdf, mut memory, mut passes, mut parallelism, mut salt) =
            (None, None, None, None, None);
        while let Some(raw) = lines.next() {
            let line = raw.trim_end();
            if line.is_empty() {
                continue;
            }
            let (key, value) = match line.split_once(": ") {
                Some((k, v)) => (k, v),
                None => (line, ""),
            };
            match key {
                "Encryption" => encryption = Some(value.to_string()),
                "Comment" => comment = Some(value.to_string()),
                "Public-Lines" => public = Some(read_blob(&mut lines, value, "Public-Lines")?),
                "Private-Lines" => private = Some(read_blob(&mut lines, value, "Private-Lines")?),
                "Private-MAC" => mac = Some(hex_decode(value)?),
                "Key-Derivation" => kdf = Some(value.to_string()),
                "Argon2-Memory" => memory = value.parse().ok(),
                "Argon2-Passes" => passes = value.parse().ok(),
                "Argon2-Parallelism" => parallelism = value.parse().ok(),
                "Argon2-Salt" => salt = Some(hex_decode(value)?),
                _ => {}
            }
        }

        let argon = match (kdf, salt) {
            (Some(kdf), Some(salt)) => {
                if !kdf.eq_ignore_ascii_case("argon2id") {
                    return Err(format!("unsupported key derivation: {kdf}"));
                }
                Some(Argon {
                    memory: memory.ok_or("missing Argon2-Memory")?,
                    passes: passes.ok_or("missing Argon2-Passes")?,
                    parallelism: parallelism.ok_or("missing Argon2-Parallelism")?,
                    salt,
                })
            }
            (None, _) => None,
            (Some(kdf), None) => return Err(format!("key derivation {kdf} without a salt")),
        };

        Ok(Ppk {
            version,
            algorithm,
            encryption: encryption.ok_or("missing Encryption header")?,
            comment: comment.ok_or("missing Comment header")?,
            public: public.ok_or("missing Public-Lines")?,
            private: Zeroizing::new(private.ok_or("missing Private-Lines")?),
            mac: mac.ok_or("missing Private-MAC")?,
            argon,
        })
    }

    /// Derive the key material, then decrypt the private blob. For `none` the
    /// blob is returned as-is (there is no padding to strip either way).
    fn decrypt(&self, passphrase: &str) -> Result<Zeroizing<Vec<u8>>, String> {
        match self.encryption.as_str() {
            "none" => Ok(self.private.clone()),
            "aes256-cbc" => {
                let (key, iv, _) = self.derive(passphrase)?;
                let mut data = self.private.to_vec();
                cbc::Decryptor::<aes::Aes256>::new_from_slices(&key, &iv)
                    .map_err(|e| e.to_string())?
                    .decrypt_padded_mut::<NoPadding>(&mut data)
                    .map_err(|_| "decryption failed".to_string())?;
                Ok(Zeroizing::new(data))
            }
            other => Err(format!("unsupported encryption: {other}")),
        }
    }

    /// The cipher key, IV and MAC key for this file (PPK spec C.4 and C.5.1).
    fn derive(&self, passphrase: &str) -> Result<KeyMaterial, String> {
        if self.version == 2 {
            // The MAC key is SHA1(prefix‖pass) even for an unencrypted file
            // (with an empty passphrase); the cipher key is as in C.5.1.
            let mac_key = {
                let mut h = Sha1::new();
                h.update(MAC_KEY_PREFIX);
                h.update(passphrase.as_bytes());
                h.finalize().to_vec()
            };
            if self.encryption == "none" {
                return Ok((Vec::new(), Vec::new(), mac_key));
            }
            // Key = first 32 bytes of SHA1(0‖pass) ‖ SHA1(1‖pass); IV is zero.
            let h0 = sha1_seq(0, passphrase.as_bytes());
            let h1 = sha1_seq(1, passphrase.as_bytes());
            let mut stream = Vec::with_capacity(40);
            stream.extend_from_slice(&h0);
            stream.extend_from_slice(&h1);
            return Ok((stream[..32].to_vec(), vec![0u8; 16], mac_key));
        }
        if self.encryption == "none" {
            // v3 without encryption: zero-length material and a zero MAC key.
            return Ok((Vec::new(), Vec::new(), vec![0u8; 32]));
        }
        // v3: Argon2id over the passphrase and the stored salt, 80 bytes out
        // (32 key, 16 IV, 32 MAC key).
        let argon = self.argon.as_ref().ok_or("v3 key without Argon2 headers")?;
        let params = Params::new(argon.memory, argon.passes, argon.parallelism, Some(80))
            .map_err(|e| e.to_string())?;
        let a2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
        let mut out = Zeroizing::new(vec![0u8; 80]);
        a2.hash_password_into(passphrase.as_bytes(), &argon.salt, &mut out)
            .map_err(|e| format!("Argon2 failed: {e}"))?;
        Ok((out[..32].to_vec(), out[32..48].to_vec(), out[48..].to_vec()))
    }

    /// Verify `Private-MAC` over the header fields, the public blob and the
    /// *plaintext* private blob (including random padding).
    fn verify_mac(&self, passphrase: &str, plain: &[u8]) -> Result<(), String> {
        let bad = || "the MAC did not verify (wrong passphrase or corrupt file)".to_string();
        let (_, _, mac_key) = self.derive(passphrase)?;
        let mut preimage = Vec::new();
        put_string(&mut preimage, self.algorithm.as_bytes());
        put_string(&mut preimage, self.encryption.as_bytes());
        put_string(&mut preimage, self.comment.as_bytes());
        put_string(&mut preimage, &self.public);
        put_string(&mut preimage, plain);
        if self.version == 2 {
            let mut mac = HmacSha1::new_from_slice(&mac_key).map_err(|_| bad())?;
            mac.update(&preimage);
            mac.verify_slice(&self.mac).map_err(|_| bad())
        } else {
            let mut mac = HmacSha256::new_from_slice(&mac_key).map_err(|_| bad())?;
            mac.update(&preimage);
            mac.verify_slice(&self.mac).map_err(|_| bad())
        }
    }

    /// Assemble an *unencrypted* `openssh-key-v1` PEM from the PPK blobs.
    fn to_openssh_private(&self, plain: &[u8]) -> Result<Zeroizing<String>, String> {
        let mut public = Cursor::new(&self.public);
        let mut private = Cursor::new(plain);
        let section: Vec<u8> = match self.algorithm.as_str() {
            "ssh-ed25519" => {
                public.field()?; // algorithm name
                let pub32 = public.field()?;
                let seed = private.field()?;
                let mut secret = Vec::with_capacity(seed.len() + pub32.len());
                secret.extend_from_slice(seed);
                secret.extend_from_slice(pub32);
                let mut s = Vec::new();
                put_string(&mut s, b"ssh-ed25519");
                put_string(&mut s, pub32);
                put_string(&mut s, &secret);
                s
            }
            "ssh-rsa" => {
                public.field()?; // "ssh-rsa"
                let e = public.raw()?;
                let n = public.raw()?;
                let d = private.raw()?;
                let p = private.raw()?;
                let q = private.raw()?;
                let iqmp = private.raw()?;
                let mut s = Vec::new();
                put_string(&mut s, b"ssh-rsa");
                // OpenSSH order: n, e, d, iqmp, p, q.
                for field in [n, e, d, iqmp, p, q] {
                    s.extend_from_slice(field);
                }
                s
            }
            algo if algo.starts_with("ecdsa-sha2-") => {
                public.field()?; // algorithm name
                let curve = public.field()?;
                let point = public.field()?;
                let scalar = private.raw()?;
                let mut s = Vec::new();
                put_string(&mut s, algo.as_bytes());
                put_string(&mut s, curve);
                put_string(&mut s, point);
                s.extend_from_slice(scalar);
                s
            }
            other => return Err(format!("{other} keys are not supported (DSA is obsolete)")),
        };

        // checksum + comment + padding, then the outer container.
        let mut keys = Zeroizing::new(Vec::new());
        let mut rng = ssh_key::rand_core::OsRng;
        use ssh_key::rand_core::RngCore;
        let check = rng.next_u32();
        keys.extend_from_slice(&check.to_le_bytes());
        keys.extend_from_slice(&check.to_le_bytes());
        keys.extend_from_slice(&section);
        put_string(&mut keys, self.comment.as_bytes());
        let mut pad = 1u8;
        let mut first = true;
        while first || keys.len() % 8 != 0 {
            keys.push(pad);
            pad = pad.wrapping_add(1);
            first = false;
        }

        let mut blob = Vec::new();
        blob.extend_from_slice(b"openssh-key-v1\0");
        put_string(&mut blob, b"none");
        put_string(&mut blob, b"none");
        put_string(&mut blob, b"");
        blob.extend_from_slice(&1u32.to_be_bytes());
        put_string(&mut blob, &self.public);
        put_string(&mut blob, &keys);

        let body = base64::engine::general_purpose::STANDARD.encode(&blob);
        let mut pem = format!("-----BEGIN {} PRIVATE KEY-----\n", "OPENSSH");
        for chunk in body.as_bytes().chunks(70) {
            pem.push_str(std::str::from_utf8(chunk).unwrap());
            pem.push('\n');
        }
        pem.push_str(&format!("-----END {} PRIVATE KEY-----\n", "OPENSSH"));
        Ok(Zeroizing::new(pem))
    }
}

/// A little reader over one PPK blob's SSH fields.
struct Cursor<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Cursor { data, at: 0 }
    }

    /// The next field's bytes and advance past its length prefix.
    fn field(&mut self) -> Result<&'a [u8], String> {
        if self.at + 4 > self.data.len() {
            return Err("the key blob is truncated".into());
        }
        let n = u32::from_be_bytes(self.data[self.at..self.at + 4].try_into().unwrap()) as usize;
        let start = self.at + 4;
        if start + n > self.data.len() {
            return Err("the key blob is truncated".into());
        }
        self.at = start + n;
        Ok(&self.data[start..start + n])
    }

    /// The next field including its length prefix (already SSH wire form).
    fn raw(&mut self) -> Result<&'a [u8], String> {
        let start = self.at;
        self.field()?;
        Ok(&self.data[start..self.at])
    }
}

fn sha1_seq(seq: u32, passphrase: &[u8]) -> [u8; 20] {
    let mut h = Sha1::new();
    h.update(seq.to_be_bytes());
    h.update(passphrase);
    h.finalize().into()
}

fn put_string(out: &mut Vec<u8>, data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
}

fn read_blob<'a>(
    lines: &mut impl Iterator<Item = &'a str>,
    count: &str,
    what: &str,
) -> Result<Vec<u8>, String> {
    let n: usize = count
        .trim()
        .parse()
        .map_err(|_| format!("{what}: not a number"))?;
    let mut text = String::new();
    for _ in 0..n {
        text.push_str(lines.next().ok_or("the file is truncated")?.trim());
    }
    base64::engine::general_purpose::STANDARD
        .decode(text.as_bytes())
        .map_err(|e| format!("{what}: {e}"))
}

fn hex_decode(text: &str) -> Result<Vec<u8>, String> {
    let text = text.trim();
    if text.len() % 2 != 0 {
        return Err("odd-length hex".into());
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    for i in (0..text.len()).step_by(2) {
        out.push(u8::from_str_radix(&text[i..i + 2], 16).map_err(|_| "invalid hex".to_string())?);
    }
    Ok(out)
}
