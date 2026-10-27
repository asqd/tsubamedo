//! A collection is a folder:
//!
//! ```text
//! my-api/
//!   tsubamedo.toml          name, [vars] shared by every environment
//!   tsubamedo.local.toml    [vars] that never go to git (tokens, passwords)
//!   env/stage.toml          [vars] of the environment
//!   env/stage.local.toml    its secrets
//!   orders/01-create.toml   requests; folders group them, names sort them
//! ```
//!
//! `tsubamedo.toml` may also list client certificates by host (`[[certificates]]`, see
//! [`crate::http::client_cert`]).
//!
//! Variables, from weakest to strongest: collection, collection local, environment,
//! environment local, then whatever the caller passes (`--var` in the CLI).

use std::fs;
use std::path::{Path, PathBuf};

use toml_edit::{ArrayOfTables, DocumentMut, Item, Table};

use crate::http::CertSpec;
use crate::request_file::RequestFile;
use crate::template::Vars;
use crate::{Error, Request, Result};

pub const MANIFEST: &str = "tsubamedo.toml";
const LOCAL_MANIFEST: &str = "tsubamedo.local.toml";
const ENV_DIR: &str = "env";

pub struct Collection {
    pub root: PathBuf,
    pub name: String,
    pub requests: Vec<Entry>,
    /// Every folder, empty ones too, as paths from the root: `orders`, `orders/archive`.
    pub folders: Vec<String>,
    /// Files that could not be read; the rest of the collection still works.
    pub broken: Vec<Error>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct VarSource {
    pub name: String,
    pub value: String,
    /// «коллекция», «окружение stage», «stage, секрет»…
    pub source: String,
    /// From a `*.local.toml` file: shown masked.
    pub secret: bool,
}

pub struct Entry {
    /// Path from the collection root without `.toml`: `orders/01-create`.
    pub id: String,
    pub path: PathBuf,
    pub request: Request,
}

impl Entry {
    pub fn display_name(&self) -> &str {
        self.request.name.as_deref().unwrap_or(&self.id)
    }

    pub fn dir(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new("."))
    }
}

impl Collection {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let manifest = root.join(MANIFEST);
        let doc = read_doc(&manifest)?;
        let name = doc
            .get("name")
            .and_then(Item::as_str)
            .map(str::to_string)
            .or_else(|| root.canonicalize().ok()?.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "Коллекция".into());
        let mut collection = Collection { root: root.clone(), name, requests: Vec::new(), folders: Vec::new(), broken: Vec::new() };
        collection.walk(&root, true);
        Ok(collection)
    }

    /// The nearest folder with `tsubamedo.toml`, from `start` up.
    pub fn find_root(start: &Path) -> Option<PathBuf> {
        let start = start.canonicalize().ok()?;
        let mut dir = if start.is_file() { start.parent()? } else { &start };
        loop {
            if dir.join(MANIFEST).is_file() {
                return Some(dir.to_path_buf());
            }
            dir = dir.parent()?;
        }
    }

    pub fn environments(&self) -> Vec<String> {
        let Ok(dir) = fs::read_dir(self.root.join(ENV_DIR)) else { return Vec::new() };
        let mut names: Vec<String> = dir
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter_map(|n| n.strip_suffix(".toml").map(str::to_string))
            .filter(|n| !n.ends_with(".local"))
            .collect();
        names.sort();
        names
    }

    pub fn vars(&self, env: Option<&str>, overrides: &Vars) -> Result<Vars> {
        let mut vars: Vars = self.var_sources(env)?.into_iter().map(|v| (v.name, v.value)).collect();
        vars.extend(overrides.iter().map(|(k, v)| (k.clone(), v.clone())));
        Ok(vars)
    }

    /// Every variable with the file it finally comes from, for hints in the editor.
    pub fn var_sources(&self, env: Option<&str>) -> Result<Vec<VarSource>> {
        let mut layers = vec![
            (self.root.join(MANIFEST), true, "коллекция".to_string(), false),
            (self.root.join(LOCAL_MANIFEST), false, "коллекция, секрет".to_string(), true),
        ];
        if let Some(env) = env {
            let dir = self.root.join(ENV_DIR);
            let file = dir.join(format!("{env}.toml"));
            if !file.is_file() {
                let known = self.environments();
                let hint = if known.is_empty() { "в папке env нет окружений".into() } else { format!("есть: {}", known.join(", ")) };
                return Err(Error::Invalid(format!("нет окружения «{env}» ({hint})")));
            }
            layers.push((file, true, format!("окружение {env}"), false));
            layers.push((dir.join(format!("{env}.local.toml")), false, format!("{env}, секрет"), true));
        }
        let mut out: Vec<VarSource> = Vec::new();
        for (path, required, source, secret) in layers {
            if !required && !path.is_file() {
                continue;
            }
            let doc = read_doc(&path)?;
            let Some(table) = doc.get("vars") else { continue };
            let table = table
                .as_table_like()
                .ok_or_else(|| Error::Parse { path: path.clone(), message: "vars должен быть таблицей".into() })?;
            for (key, item) in table.iter() {
                let value = item.as_value().and_then(|v| match v {
                    toml_edit::Value::String(s) => Some(s.value().clone()),
                    toml_edit::Value::Integer(i) => Some(i.value().to_string()),
                    toml_edit::Value::Float(f) => Some(f.value().to_string()),
                    toml_edit::Value::Boolean(b) => Some(b.value().to_string()),
                    _ => None,
                });
                let value = value.ok_or_else(|| Error::Parse { path: path.clone(), message: format!("vars.{key}: нужна строка или число") })?;
                out.retain(|v| v.name != key);
                out.push(VarSource { name: key.to_string(), value, source: source.clone(), secret });
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Client certificates as written, `{{variables}}` and all.
    pub fn certificates(&self) -> Result<Vec<CertSpec>> {
        let path = self.root.join(MANIFEST);
        let doc = read_doc(&path)?;
        let Some(item) = doc.get("certificates") else { return Ok(Vec::new()) };
        let bad = |message: String| Error::Parse { path: path.clone(), message };
        let tables: Vec<&dyn toml_edit::TableLike> = match item {
            Item::ArrayOfTables(a) => a.iter().map(|t| t as &dyn toml_edit::TableLike).collect(),
            Item::Value(toml_edit::Value::Array(a)) => a
                .iter()
                .map(|v| v.as_inline_table().map(|t| t as &dyn toml_edit::TableLike))
                .collect::<Option<_>>()
                .ok_or_else(|| bad("certificates: нужны таблицы [[certificates]]".into()))?,
            _ => return Err(bad("certificates: нужны таблицы [[certificates]]".into())),
        };
        tables
            .into_iter()
            .map(|t| {
                let field = |key: &str| match t.get(key) {
                    None => Ok(String::new()),
                    Some(v) => v.as_str().map(str::to_string).ok_or_else(|| bad(format!("certificates.{key}: нужна строка"))),
                };
                let spec = CertSpec { host: field("host")?, cert: field("cert")?, key: field("key")?, passphrase: field("passphrase")? };
                if spec.host.trim().is_empty() {
                    return Err(bad("certificates: у сертификата нет host".into()));
                }
                Ok(spec)
            })
            .collect()
    }

    /// Writes `[[certificates]]`, leaving the rest of `tsubamedo.toml` as it was.
    pub fn save_certificates(&self, list: &[CertSpec]) -> Result<()> {
        let path = self.root.join(MANIFEST);
        let mut doc = read_doc(&path)?;
        doc.remove("certificates");
        if !list.is_empty() {
            let mut tables = ArrayOfTables::new();
            for c in list {
                let mut t = Table::new();
                for (key, value) in [("host", &c.host), ("cert", &c.cert), ("key", &c.key), ("passphrase", &c.passphrase)] {
                    if !value.trim().is_empty() || key == "host" || key == "cert" {
                        t.insert(key, toml_edit::value(value.trim()));
                    }
                }
                tables.push(t);
            }
            doc.insert("certificates", Item::ArrayOfTables(tables));
        }
        fs::write(&path, doc.to_string()).map_err(|e| Error::io(&path, e))
    }

    pub fn find(&self, id: &str) -> Option<&Entry> {
        self.requests.iter().find(|e| e.id == id)
    }

    fn walk(&mut self, dir: &Path, top: bool) {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => return self.broken.push(Error::io(dir, e)),
        };
        let mut paths: Vec<PathBuf> = entries.filter_map(|e| Some(e.ok()?.path())).collect();
        paths.sort();
        for path in paths {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if name.starts_with('.') || name == "node_modules" || name == "target" {
                continue;
            }
            if path.is_dir() {
                if !(top && name == ENV_DIR) {
                    let rel = path.strip_prefix(&self.root).unwrap_or(&path);
                    let (folders, requests) = (self.folders.len(), self.requests.len());
                    self.folders.push(rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"));
                    self.walk(&path, false);
                    // A folder of other files (JSON schemas, data for bodies) is not one of requests;
                    // an empty one is, just made.
                    if self.requests.len() == requests && has_files(&path) {
                        self.folders.truncate(folders);
                    }
                }
                continue;
            }
            let Some(stem) = name.strip_suffix(".toml") else { continue };
            if stem.ends_with(".local") || (top && name == MANIFEST) {
                continue;
            }
            let rel = path.strip_prefix(&self.root).unwrap_or(&path).with_extension("");
            let id = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
            match RequestFile::open(&path).and_then(|f| f.request()) {
                Ok(request) => self.requests.push(Entry { id, path, request }),
                Err(e) => self.broken.push(e),
            }
        }
    }
}

/// Whether there is any file in the folder or below, hidden ones aside.
fn has_files(dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(dir) else { return false };
    entries.filter_map(|e| e.ok()).any(|e| {
        let hidden = e.file_name().to_string_lossy().starts_with('.');
        let path = e.path();
        !hidden && (path.is_file() || (path.is_dir() && has_files(&path)))
    })
}

fn read_doc(path: &Path) -> Result<DocumentMut> {
    let text = fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    text.parse().map_err(|e: toml_edit::TomlError| Error::Parse { path: path.to_path_buf(), message: e.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn reads_requests_and_layers_vars() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, MANIFEST, "name = \"Level API\"\n[vars]\nbase = \"https://{{host}}\"\nhost = \"dev.test\"\n");
        write(root, "tsubamedo.local.toml", "[vars]\ntoken = \"local\"\n");
        write(root, "env/stage.toml", "[vars]\nhost = \"stage.test\"\n");
        write(root, "env/stage.local.toml", "[vars]\ntoken = \"stage-secret\"\n");
        write(root, "orders/02-get.toml", "url = \"{{base}}/orders/1\"\n");
        write(root, "orders/01-create.toml", "name = \"Создать\"\nmethod = \"POST\"\nurl = \"{{base}}/orders\"\n");
        write(root, "orders/notes.md", "не запрос");
        write(root, "schemas/order.json", "{}");
        fs::create_dir_all(root.join("drafts")).unwrap();
        write(root, "broken.toml", "url = \n");

        let c = Collection::open(root).unwrap();
        assert_eq!(c.name, "Level API");
        assert_eq!(c.requests.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["orders/01-create", "orders/02-get"]);
        assert_eq!(c.requests[0].display_name(), "Создать");
        assert_eq!(c.broken.len(), 1);
        assert_eq!(c.folders, ["drafts", "orders"], "папка схем — не папка запросов, пустая — да");
        assert_eq!(c.environments(), ["stage"]);

        let dev = c.vars(None, &Vars::new()).unwrap();
        assert_eq!((dev["host"].as_str(), dev["token"].as_str()), ("dev.test", "local"));
        let mut overrides = Vars::new();
        overrides.insert("token".into(), "cli".into());
        let stage = c.vars(Some("stage"), &overrides).unwrap();
        assert_eq!((stage["host"].as_str(), stage["token"].as_str()), ("stage.test", "cli"));
        assert!(c.vars(Some("prod"), &Vars::new()).unwrap_err().to_string().contains("есть: stage"));

        let sources = c.var_sources(Some("stage")).unwrap();
        let token = sources.iter().find(|v| v.name == "token").unwrap();
        assert_eq!((token.source.as_str(), token.secret), ("stage, секрет", true));
        assert_eq!(sources.iter().find(|v| v.name == "host").unwrap().source, "окружение stage");

        let nested = root.join("orders");
        assert_eq!(Collection::find_root(&nested).unwrap(), root.canonicalize().unwrap());
    }
}
