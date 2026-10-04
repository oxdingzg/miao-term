//! Optional end-to-end-encrypted sync of hosts and snippets (B4.5, ADR 0033).
//!
//! The transport is a folder the user already syncs. Each device writes only
//! its own encrypted file, `<sync-dir>/mtty-sync/<device>.mtsync`, holding
//! every entry it knows with a timestamp (deletions are tombstones); a sync
//! merges all devices' files entry by entry, newest first. The key lives in
//! `~/.config/mtty/sync.key` and never enters the folder.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::hosts::{Host, HostBook};
use crate::snippets::{Snippet, SnippetBook};

const MAGIC: &[u8] = b"MTSYNC1\n";
const NONCE_LEN: usize = 24;
const PAIRING_PREFIX: &str = "mtty-sync:";

/// The shared 256-bit sync key.
#[derive(Clone, PartialEq, Eq)]
pub struct Key([u8; 32]);

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key(***)")
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn b64url(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

fn unb64url(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut acc = 0u32;
    let mut bits = 0;
    for c in text.bytes() {
        let v = B64.iter().position(|b| *b == c)? as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

impl Key {
    pub fn generate() -> Self {
        let key = XChaCha20Poly1305::generate_key(&mut OsRng);
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&key);
        Self(bytes)
    }

    /// `mtty-sync:<base64url>`: what a second device pastes to join.
    pub fn pairing_code(&self) -> String {
        format!("{PAIRING_PREFIX}{}", b64url(&self.0))
    }

    pub fn from_pairing_code(code: &str) -> Result<Self, String> {
        let body = code
            .trim()
            .strip_prefix(PAIRING_PREFIX)
            .unwrap_or(code.trim());
        let bytes = unb64url(body)
            .filter(|b| b.len() == 32)
            .ok_or("not a pairing code")?;
        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        Ok(Self(key))
    }

    pub fn path() -> Option<PathBuf> {
        Some(crate::config_dir()?.join("sync.key"))
    }

    pub fn load_from(path: &Path) -> Result<Option<Self>, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::from_pairing_code(&text).map(Some),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Write the key readable by the owner only.
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        let mut file = options.open(path).map_err(|e| e.to_string())?;
        file.write_all(format!("{}\n", self.pairing_code()).as_bytes())
            .map_err(|e| e.to_string())
    }

    pub fn seal(&self, plaintext: &[u8]) -> Vec<u8> {
        let cipher = XChaCha20Poly1305::new((&self.0).into());
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let sealed = cipher
            .encrypt(&nonce, plaintext)
            .expect("encryption of an in-memory buffer cannot fail");
        [MAGIC, nonce.as_slice(), &sealed].concat()
    }

    pub fn open(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        let body = data.strip_prefix(MAGIC).ok_or("not an mtty sync file")?;
        if body.len() < NONCE_LEN {
            return Err("truncated sync file".into());
        }
        let (nonce, sealed) = body.split_at(NONCE_LEN);
        XChaCha20Poly1305::new((&self.0).into())
            .decrypt(XNonce::from_slice(nonce), sealed)
            .map_err(|_| "made with a different sync key, or damaged".to_string())
    }
}

/// One host or snippet, or the record that it was deleted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub kind: String,
    pub key: String,
    /// Milliseconds since the Unix epoch.
    pub updated: u64,
    pub device: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
}

impl Entry {
    fn id(&self) -> (String, String) {
        (self.kind.clone(), self.key.clone())
    }

    /// Newest wins; the device id breaks a tie the same way everywhere.
    fn beats(&self, other: &Entry) -> bool {
        (self.updated, &self.device) > (other.updated, &other.device)
    }
}

/// What a device knows: written (encrypted) to the folder, and kept locally
/// as the baseline for spotting this device's own changes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeviceState {
    pub device: String,
    pub entries: Vec<Entry>,
}

/// The local files as entries' bodies, keyed by (kind, name).
pub fn snapshot(hosts: &[Host], snippets: &[Snippet]) -> BTreeMap<(String, String), Value> {
    let mut out = BTreeMap::new();
    for h in hosts {
        if let Ok(v) = serde_json::to_value(h) {
            out.insert(("host".to_string(), h.name.clone()), v);
        }
    }
    for s in snippets {
        if let Ok(v) = serde_json::to_value(s) {
            out.insert(("snippet".to_string(), s.name.clone()), v);
        }
    }
    out
}

/// This device's changes since `last`, merged with every other device's
/// entries: one entry per (kind, name), newest wins.
pub fn merge(
    current: &BTreeMap<(String, String), Value>,
    last: &[Entry],
    others: &[Vec<Entry>],
    device: &str,
    now: u64,
) -> Vec<Entry> {
    let last: BTreeMap<_, _> = last.iter().map(|e| (e.id(), e)).collect();
    let mut mine: Vec<Entry> = Vec::new();
    for ((kind, key), body) in current {
        let id = (kind.clone(), key.clone());
        match last.get(&id) {
            Some(e) if !e.deleted && e.body.as_ref() == Some(body) => mine.push((*e).clone()),
            _ => mine.push(Entry {
                kind: kind.clone(),
                key: key.clone(),
                updated: now,
                device: device.to_string(),
                deleted: false,
                body: Some(body.clone()),
            }),
        }
    }
    for (id, e) in &last {
        if current.contains_key(id) {
            continue;
        }
        if e.deleted {
            mine.push((*e).clone());
        } else {
            mine.push(Entry {
                updated: now,
                device: device.to_string(),
                deleted: true,
                body: None,
                ..(*e).clone()
            });
        }
    }
    let mut merged: BTreeMap<(String, String), Entry> = BTreeMap::new();
    for e in mine.into_iter().chain(others.iter().flatten().cloned()) {
        match merged.get(&e.id()) {
            Some(have) if !e.beats(have) => {}
            _ => {
                merged.insert(e.id(), e);
            }
        }
    }
    merged.into_values().collect()
}

/// Rebuild a list from merged entries, keeping the local order for entries
/// that stay and appending new ones by name.
fn apply<T: Clone + serde::de::DeserializeOwned>(
    local: &[T],
    name: impl Fn(&T) -> &str,
    merged: &[Entry],
    kind: &str,
) -> Vec<T> {
    let live: BTreeMap<&str, T> = merged
        .iter()
        .filter(|e| e.kind == kind && !e.deleted)
        .filter_map(|e| {
            Some((
                e.key.as_str(),
                serde_json::from_value(e.body.clone()?).ok()?,
            ))
        })
        .collect();
    let mut out: Vec<T> = local
        .iter()
        .filter_map(|item| live.get(name(item)).cloned())
        .collect();
    for (key, item) in &live {
        if !local.iter().any(|l| name(l) == *key) {
            out.push(item.clone());
        }
    }
    out
}

/// Where a sync reads and writes; the defaults are the real config files.
#[derive(Debug, Clone)]
pub struct Paths {
    pub hosts: PathBuf,
    pub snippets: PathBuf,
    /// This device's id and last synced entries.
    pub state: PathBuf,
}

impl Paths {
    pub fn default_paths() -> Option<Self> {
        Some(Self {
            hosts: HostBook::path()?,
            snippets: SnippetBook::path()?,
            state: crate::data_dir()?.join("sync-state.json"),
        })
    }
}

/// What a sync did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub hosts_changed: bool,
    pub snippets_changed: bool,
    /// Other devices whose files were merged.
    pub devices: usize,
    /// Files that could not be read (another key, damaged), by name.
    pub problems: Vec<String>,
}

fn read_toml<T: Default + serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).map_err(|e| {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            format!(
                "{name}: {}",
                e.to_string().lines().next().unwrap_or_default()
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e.to_string()),
    }
}

/// Write a file through a temporary name, keeping the old one as
/// `*.before-sync`.
fn replace(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    if path.exists() {
        let backup = path.with_extension("toml.before-sync");
        std::fs::copy(path, backup).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("toml.sync-tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

fn random_device_id() -> String {
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    nonce[..8].iter().map(|b| format!("{b:02x}")).collect()
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// One sync against `dir`. Blocks on file IO: run it off the UI thread.
pub fn run(dir: &Path, key: &Key, paths: &Paths, now: u64) -> Result<Outcome, String> {
    let book: HostBook = read_toml(&paths.hosts)?;
    let snips: SnippetBook = read_toml(&paths.snippets)?;
    let mut state: DeviceState = match std::fs::read(&paths.state) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("sync state: {e}"))?,
        Err(_) => DeviceState::default(),
    };
    if state.device.is_empty() {
        state.device = random_device_id();
    }
    let folder = dir.join("mtty-sync");
    std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let own = format!("{}.mtsync", state.device);
    let mut outcome = Outcome::default();
    let mut others = Vec::new();
    let mut names: Vec<PathBuf> = std::fs::read_dir(&folder)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "mtsync"))
        .collect();
    names.sort();
    for path in names {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if name == own {
            continue;
        }
        let read = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|data| key.open(&data))
            .and_then(|plain| {
                serde_json::from_slice::<DeviceState>(&plain).map_err(|e| e.to_string())
            });
        match read {
            Ok(other) => {
                outcome.devices += 1;
                others.push(other.entries);
            }
            Err(e) => outcome.problems.push(format!("{name}: {e}")),
        }
    }
    let current = snapshot(&book.hosts, &snips.snippets);
    let merged = merge(&current, &state.entries, &others, &state.device, now);
    let hosts = apply(&book.hosts, |h| h.name.as_str(), &merged, "host");
    let snippets = apply(&snips.snippets, |s| s.name.as_str(), &merged, "snippet");
    if snapshot(&hosts, &[]) != snapshot(&book.hosts, &[]) {
        let text = toml::to_string_pretty(&HostBook { hosts }).map_err(|e| e.to_string())?;
        replace(&paths.hosts, &text)?;
        outcome.hosts_changed = true;
    }
    if snapshot(&[], &snippets) != snapshot(&[], &snips.snippets) {
        let text = toml::to_string_pretty(&SnippetBook { snippets }).map_err(|e| e.to_string())?;
        replace(&paths.snippets, &text)?;
        outcome.snippets_changed = true;
    }
    state.entries = merged;
    let plain = serde_json::to_vec(&state).map_err(|e| e.to_string())?;
    let tmp = folder.join(format!("{own}.tmp"));
    std::fs::write(&tmp, key.seal(&plain)).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, folder.join(&own)).map_err(|e| e.to_string())?;
    if let Some(dir) = paths.state.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&paths.state, plain).map_err(|e| e.to_string())?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(name: &str, address: &str) -> Host {
        Host {
            name: name.into(),
            address: Some(address.into()),
            ..Default::default()
        }
    }

    #[test]
    fn keys_round_trip_and_seal() {
        let key = Key::generate();
        let code = key.pairing_code();
        assert!(
            code.starts_with("mtty-sync:") && code.len() == 10 + 43,
            "{code}"
        );
        assert_eq!(Key::from_pairing_code(&code).unwrap(), key);
        assert!(Key::from_pairing_code("mtty-sync:short").is_err());
        let sealed = key.seal(b"hosts");
        assert!(sealed.starts_with(MAGIC));
        assert!(!sealed.windows(5).any(|w| w == b"hosts"), "ciphertext only");
        assert_eq!(key.open(&sealed).unwrap(), b"hosts");
        assert!(Key::generate().open(&sealed).is_err(), "another key fails");
        let mut tampered = sealed.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(key.open(&tampered).is_err());
        assert_ne!(key.seal(b"x"), key.seal(b"x"), "fresh nonce each time");
        assert!(!format!("{key:?}").contains(&code[10..]));
    }

    #[test]
    fn merges_by_entry_with_tombstones() {
        let a = snapshot(&[host("web", "203.0.113.1")], &[]);
        // Device A's first sync records web.
        let s1 = merge(&a, &[], &[], "a", 100);
        assert_eq!(s1.len(), 1);
        // Unchanged: the timestamp stays.
        assert_eq!(merge(&a, &s1, &[], "a", 200), s1);
        // B added db at 150 and edited web at 300: both arrive.
        let b = vec![
            Entry {
                kind: "host".into(),
                key: "db".into(),
                updated: 150,
                device: "b".into(),
                deleted: false,
                body: serde_json::to_value(host("db", "203.0.113.2")).ok(),
            },
            Entry {
                kind: "host".into(),
                key: "web".into(),
                updated: 300,
                device: "b".into(),
                deleted: false,
                body: serde_json::to_value(host("web", "203.0.113.9")).ok(),
            },
        ];
        let m = merge(&a, &s1, std::slice::from_ref(&b), "a", 200);
        let hosts = apply(
            &[host("web", "203.0.113.1")],
            |h| h.name.as_str(),
            &m,
            "host",
        );
        assert_eq!(hosts.len(), 2);
        assert_eq!(
            hosts[0].address.as_deref(),
            Some("203.0.113.9"),
            "newer edit wins"
        );
        assert_eq!(hosts[1].name, "db");
        // A deletes db later: the tombstone beats B's older entry.
        let now = snapshot(&hosts[..1], &[]);
        let m2 = merge(&now, &m, &[b], "a", 400);
        let db = m2.iter().find(|e| e.key == "db").unwrap();
        assert!(db.deleted && db.updated == 400);
        assert_eq!(apply(&hosts, |h| h.name.as_str(), &m2, "host").len(), 1);
    }

    #[test]
    fn two_devices_converge_through_a_folder() {
        let root = std::env::temp_dir().join(format!("mtty-sync-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let folder = root.join("cloud");
        let key = Key::generate();
        let dev = |n: &str| Paths {
            hosts: root.join(n).join("hosts.toml"),
            snippets: root.join(n).join("snippets.toml"),
            state: root.join(n).join("sync-state.json"),
        };
        let (a, b) = (dev("a"), dev("b"));
        let write_hosts = |p: &Paths, hosts: Vec<Host>| {
            std::fs::create_dir_all(p.hosts.parent().unwrap()).unwrap();
            std::fs::write(&p.hosts, toml::to_string(&HostBook { hosts }).unwrap()).unwrap();
        };
        let names = |p: &Paths| -> Vec<String> {
            let book: HostBook = read_toml(&p.hosts).unwrap();
            book.hosts.into_iter().map(|h| h.name).collect()
        };
        write_hosts(&a, vec![host("web", "203.0.113.1")]);
        write_hosts(&b, vec![host("db", "203.0.113.2")]);
        std::fs::write(
            &b.snippets,
            "[[snippet]]\nname = \"disk\"\ncommand = \"df -h\"\n",
        )
        .unwrap();
        run(&folder, &key, &a, 100).unwrap();
        let out = run(&folder, &key, &b, 200).unwrap();
        assert_eq!(out.devices, 1);
        assert!(out.hosts_changed);
        assert_eq!(names(&b), ["db", "web"]);
        assert!(b.hosts.with_extension("toml.before-sync").exists());
        let out = run(&folder, &key, &a, 300).unwrap();
        assert!(out.hosts_changed && out.snippets_changed);
        assert_eq!(names(&a), ["web", "db"]);
        // Nothing readable in the folder.
        for f in std::fs::read_dir(folder.join("mtty-sync"))
            .unwrap()
            .flatten()
        {
            let data = std::fs::read(f.path()).unwrap();
            assert!(!data.windows(3).any(|w| w == b"web"));
        }
        // A deletes web; B follows.
        write_hosts(&a, vec![host("db", "203.0.113.2")]);
        run(&folder, &key, &a, 400).unwrap();
        run(&folder, &key, &b, 500).unwrap();
        assert_eq!(names(&b), ["db"]);
        // Steady state: nothing changes.
        let out = run(&folder, &key, &b, 600).unwrap();
        assert!(!out.hosts_changed && !out.snippets_changed);
        // A device with another key is reported, not merged.
        let c = dev("c");
        let out = run(&folder, &Key::generate(), &c, 700).unwrap();
        assert_eq!((out.devices, out.problems.len()), (0, 2));
        assert!(names(&c).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
