//! What was sent and what came back, kept in the app's data folder (never in the collection,
//! so it cannot end up in git): one JSON line per exchange, plus the body when it is small.
//!
//! The request is stored as written, with `{{variables}}`, so secrets from `*.local.toml` are
//! not copied out. Values of auth-like headers in the "sent" part are masked.
//!
//! Next to it live what the session must remember to keep working: captured values, cookies
//! and OAuth 2.0 tokens.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tsubamedo_core::Request;
use tsubamedo_core::cookies::Jar;
use tsubamedo_core::oauth::Tokens;

/// Bodies up to this size are kept, so an old answer opens again without sending.
pub const BODY_LIMIT: u64 = 1 << 20;
/// Entries kept per collection; older ones are dropped with their bodies.
pub const KEEP: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    /// Unix milliseconds.
    pub at: u64,
    /// The collection request it came from; `None` for a scratch request.
    pub request_id: Option<String>,
    pub env: Option<String>,
    pub request: Request,
    pub status: Option<u16>,
    pub error: Option<String>,
    pub total_ms: Option<f64>,
    pub size: Option<u64>,
    pub has_body: bool,
    /// The response as the window shows it (without its body and response id).
    pub meta: Option<Value>,
}

/// Captured values by environment (`""` without one): variable → value and where it came from.
pub type CapturedStore = BTreeMap<String, BTreeMap<String, CapturedValue>>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapturedValue {
    pub value: String,
    /// The request id, or «черновик».
    pub from: String,
}

pub struct History {
    dir: PathBuf,
}

/// FNV-1a: a stable folder name for a collection path, whatever the Rust version.
fn key(root: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in root.bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// Header values that are credentials whatever the API calls them.
pub fn mask_secrets(meta: &mut Value) {
    let Some(headers) = meta.pointer_mut("/sent/headers").and_then(Value::as_array_mut) else { return };
    for pair in headers {
        let Some(name) = pair.get(0).and_then(Value::as_str).map(str::to_ascii_lowercase) else { continue };
        let secret = ["authorization", "cookie", "token", "secret", "key", "password", "session"].iter().any(|w| name.contains(w));
        if secret && let Some(value) = pair.get_mut(1) {
            *value = Value::String("••••".into());
        }
    }
}

impl History {
    /// The history of a collection (by its folder), or of scratch requests without one.
    pub fn open(data_dir: &Path, collection: Option<&Path>) -> Self {
        let dir = match collection {
            Some(root) => {
                let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
                data_dir.join("history").join(key(&root.to_string_lossy()))
            }
            None => data_dir.join("history").join("scratch"),
        };
        if let Some(root) = collection {
            let _ = fs::create_dir_all(&dir);
            let _ = fs::write(dir.join("collection.txt"), root.to_string_lossy().as_bytes());
        }
        History { dir }
    }

    fn file(&self) -> PathBuf {
        self.dir.join("history.jsonl")
    }

    fn body_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.body"))
    }

    fn read_all(&self) -> Vec<Entry> {
        let Ok(file) = fs::File::open(self.file()) else { return Vec::new() };
        std::io::BufReader::new(file).lines().map_while(Result::ok).filter_map(|l| serde_json::from_str(&l).ok()).collect()
    }

    fn write_all(&self, entries: &[Entry]) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let tmp = self.dir.join("history.jsonl.tmp");
        let mut out = std::io::BufWriter::new(fs::File::create(&tmp)?);
        for e in entries {
            serde_json::to_writer(&mut out, e)?;
            out.write_all(b"\n")?;
        }
        out.flush()?;
        drop(out);
        fs::rename(tmp, self.file())
    }

    pub fn add(&self, entry: &Entry, body: Option<&[u8]>) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        if let Some(body) = body {
            fs::write(self.body_path(&entry.id), body)?;
        }
        let mut file = fs::OpenOptions::new().create(true).append(true).open(self.file())?;
        let mut line = serde_json::to_vec(entry)?;
        line.push(b'\n');
        file.write_all(&line)?;
        // Trimming rewrites the file, so it happens once per 50 extra entries, not every time.
        let entries = self.read_all();
        if entries.len() > KEEP + 50 {
            let (old, keep) = entries.split_at(entries.len() - KEEP);
            for e in old {
                let _ = fs::remove_file(self.body_path(&e.id));
            }
            self.write_all(keep)?;
        }
        Ok(())
    }

    /// Newest first; only those of one request when `request_id` is given.
    pub fn list(&self, request_id: Option<&str>, limit: usize) -> Vec<Entry> {
        let mut entries = self.read_all();
        entries.reverse();
        entries.into_iter().filter(|e| request_id.is_none() || e.request_id.as_deref() == request_id).take(limit).collect()
    }

    pub fn get(&self, id: &str) -> Option<(Entry, Option<Vec<u8>>)> {
        let entry = self.read_all().into_iter().find(|e| e.id == id)?;
        let body = entry.has_body.then(|| fs::read(self.body_path(id)).ok()).flatten();
        Some((entry, body))
    }

    /// After a request or a folder was renamed or moved: `old` → `new`, or `old/…` → `new/…`.
    pub fn rename(&self, old: &str, new: &str) -> std::io::Result<()> {
        let mut entries = self.read_all();
        let mut changed = false;
        for e in &mut entries {
            let Some(id) = &e.request_id else { continue };
            let renamed = if id == old {
                Some(new.to_string())
            } else {
                id.strip_prefix(&format!("{old}/")).map(|rest| format!("{new}/{rest}"))
            };
            if let Some(renamed) = renamed {
                e.request_id = Some(renamed);
                changed = true;
            }
        }
        if changed { self.write_all(&entries) } else { Ok(()) }
    }

    /// Values captured from responses, kept between runs so a token from yesterday's login
    /// still works.
    pub fn captured(&self) -> CapturedStore {
        fs::read(self.dir.join("captured.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save_captured(&self, store: &CapturedStore) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        fs::write(self.dir.join("captured.json"), serde_json::to_vec_pretty(store)?)
    }

    /// Cookie jars by environment (`""` without one).
    pub fn cookies(&self) -> BTreeMap<String, Jar> {
        fs::read(self.dir.join("cookies.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save_cookies(&self, jars: &BTreeMap<String, Jar>) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        fs::write(self.dir.join("cookies.json"), serde_json::to_vec_pretty(jars)?)
    }

    /// OAuth 2.0 tokens, so a sign-in in the browser lasts beyond one start of the app.
    pub fn tokens(&self) -> Tokens {
        fs::read(self.dir.join("tokens.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save_tokens(&self, tokens: &Tokens) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        fs::write(self.dir.join("tokens.json"), serde_json::to_vec_pretty(tokens)?)
    }

    /// History, kept bodies, captured values, cookies and tokens.
    pub fn clear(&self) -> std::io::Result<()> {
        match fs::remove_dir_all(&self.dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(id: &str, request_id: Option<&str>) -> Entry {
        Entry {
            id: id.into(),
            at: 0,
            request_id: request_id.map(str::to_string),
            env: None,
            request: Request::default(),
            status: Some(200),
            error: None,
            total_ms: Some(1.0),
            size: Some(2),
            has_body: false,
            meta: None,
        }
    }

    #[test]
    fn keeps_lists_renames_and_trims() {
        let data = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let h = History::open(data.path(), Some(root.path()));
        h.add(&entry("1", Some("orders/create")), None).unwrap();
        h.add(&Entry { has_body: true, ..entry("2", Some("orders/list")) }, Some(b"[1]")).unwrap();
        h.add(&entry("3", None), None).unwrap();
        assert_eq!(h.list(None, 10).iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["3", "2", "1"]);
        assert_eq!(h.list(Some("orders/list"), 10).len(), 1);
        assert_eq!(h.get("2").unwrap().1.as_deref(), Some(&b"[1]"[..]));

        h.rename("orders", "Заказы").unwrap();
        assert_eq!(h.list(Some("Заказы/create"), 10).len(), 1);
        h.rename("Заказы/create", "Заказы/new").unwrap();
        assert_eq!(h.list(Some("Заказы/new"), 10).len(), 1);

        // The same folder opened by another path shares the history.
        let again = History::open(data.path(), Some(&root.path().join(".")));
        assert_eq!(again.list(None, 10).len(), 3);

        for i in 0..KEEP + 60 {
            h.add(&Entry { has_body: true, ..entry(&format!("n{i}"), None) }, Some(b"x")).unwrap();
        }
        let left = h.list(None, usize::MAX);
        assert_eq!(left.len(), KEEP + 12);
        assert!(!h.dir.join("2.body").exists());
        h.clear().unwrap();
        assert!(h.list(None, 10).is_empty());
    }

    #[test]
    fn masks_credentials() {
        let mut meta = json!({ "sent": { "headers": [["Authorization", "Bearer x"], ["X-Api-Key", "k"], ["Accept", "*/*"]] } });
        mask_secrets(&mut meta);
        assert_eq!(meta["sent"]["headers"], json!([["Authorization", "••••"], ["X-Api-Key", "••••"], ["Accept", "*/*"]]));
    }
}
