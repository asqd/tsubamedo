//! Collections from other tools, turned into a collection folder:
//!
//! - Insomnia: an export of version 4 (JSON) or 5 (YAML, Insomnia 10 and later);
//! - Postman: a collection v2.0 or v2.1, an environment, globals;
//! - OpenAPI 3 and Swagger 2, in JSON or YAML: a request per operation with example bodies;
//! - HAR from the browser's developer tools: the page's API calls, without pictures and scripts.
//!
//! [`parse`] finds the format by the content and gives an [`Imported`]; [`write`] puts it on disk,
//! as a new collection or into a folder of an open one. What has no place in TsubameDo is
//! skipped and named in [`Imported::warnings`], so nothing is lost silently.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use toml_edit::{DocumentMut, Item, Table};

use crate::collection::MANIFEST;
use crate::model::{Body, Pair, Request};
use crate::organize::{clean_name, free_path};
use crate::request_file::RequestFile;
use crate::template::Vars;
use crate::{Error, Result, prepare, scripts};

mod bundle;
mod har;
mod insomnia;
mod openapi;
mod postman;

/// Bigger files are refused: they are not collections.
pub const SIZE_LIMIT: u64 = 64 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Format {
    Insomnia,
    Postman,
    PostmanEnvironment,
    OpenApi,
    Swagger,
    Har,
}

impl Format {
    pub fn title(self) -> &'static str {
        match self {
            Format::Insomnia => "Insomnia",
            Format::Postman => "коллекция Postman",
            Format::PostmanEnvironment => "окружение Postman",
            Format::OpenApi => "OpenAPI",
            Format::Swagger => "Swagger 2",
            Format::Har => "HAR",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Imported {
    pub format: Format,
    pub name: String,
    pub requests: Vec<Placed>,
    /// For every environment: tsubamedo.toml, or tsubamedo.local.toml when secret.
    pub vars: Vec<Var>,
    pub environments: Vec<Environment>,
    pub warnings: Vec<String>,
    /// Other files, by path from where the requests go: JSON schemas of OpenAPI responses.
    pub files: Vec<(String, String)>,
}

/// A request and the folders it sits in, outermost first.
#[derive(Debug, Clone)]
pub struct Placed {
    pub folder: Vec<String>,
    pub request: Request,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Var {
    pub name: String,
    pub value: String,
    /// Goes to a `*.local.toml` file, which stays out of git.
    pub secret: bool,
}

impl Var {
    /// Secret when the source says so or the name looks like one: the collection file goes to
    /// git, and a token there is a leak.
    pub fn new(name: impl Into<String>, value: impl Into<String>, secret: bool) -> Self {
        let name = name.into();
        let secret = secret || looks_secret(&name);
        Var { name, value: value.into(), secret }
    }
}

fn looks_secret(name: &str) -> bool {
    let n = name.to_lowercase();
    ["token", "secret", "password", "passwd", "pwd", "apikey", "api_key", "api-key", "cookie", "session", "private", "credential"]
        .iter()
        .any(|w| n.contains(w))
}

#[derive(Debug, Clone)]
pub struct Environment {
    pub name: String,
    pub vars: Vec<Var>,
}

/// What [`write`] did.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Written {
    pub root: PathBuf,
    /// The folder the requests went to, `""` for a new collection.
    pub folder: String,
    pub requests: usize,
    pub environments: Vec<String>,
    /// Variables that were already in the collection with another value: theirs is kept.
    pub kept: Vec<String>,
}

pub fn read(path: &Path) -> Result<Imported> {
    let size = fs::metadata(path).map_err(|e| Error::io(path, e))?.len();
    if size > SIZE_LIMIT {
        return Err(Error::Invalid(format!("{}: файл больше {} МБ — это не коллекция", path.display(), SIZE_LIMIT >> 20)));
    }
    let text = fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    let mut imported = parse_from(&text, Some(path)).map_err(|e| match e {
        Error::Invalid(message) => Error::Parse { path: path.to_path_buf(), message },
        e => e,
    })?;
    if imported.name.trim().is_empty() {
        imported.name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Импорт".into());
    }
    Ok(imported)
}

/// Finds the format by the content: file extensions lie, `.json` holds all of them.
pub fn parse(text: &str) -> Result<Imported> {
    parse_from(text, None)
}

/// `file` is where the text came from: an OpenAPI description may refer to files next to it.
fn parse_from(text: &str, file: Option<&Path>) -> Result<Imported> {
    let text = text.trim_start_matches('\u{feff}');
    let mut doc: Value = if text.trim_start().starts_with(['{', '[']) {
        serde_json::from_str(text).map_err(|e| Error::Invalid(format!("не JSON: {e}")))?
    } else {
        serde_yaml_ng::from_str(text).map_err(|e| Error::Invalid(format!("ни JSON, ни YAML: {e}")))?
    };
    let s = |key: &str| doc.get(key).and_then(Value::as_str).unwrap_or_default();
    let mut imported = if s("_type") == "export" {
        insomnia::v4(&doc)?
    } else if s("type").contains("insomnia.rest/") {
        insomnia::v5(&doc)?
    } else if doc.get("info").is_some() && doc.get("item").is_some() {
        postman::collection(&doc)?
    } else if doc.get("_postman_variable_scope").is_some() || (doc.get("values").is_some_and(Value::is_array) && doc.get("name").is_some()) {
        postman::environment(&doc)
    } else if doc.get("openapi").is_some() || doc.get("swagger").is_some() {
        let outside = file.map(|f| bundle::bundle(&mut doc, f)).unwrap_or_default();
        let mut imported = if doc.get("openapi").is_some() { openapi::v3(&doc)? } else { openapi::v2(&doc)? };
        imported.warnings.splice(0..0, outside);
        imported
    } else if doc.pointer("/log/entries").is_some() {
        har::har(&doc)?
    } else {
        return Err(Error::Invalid(
            "не похоже ни на экспорт Insomnia, ни на коллекцию или окружение Postman, ни на OpenAPI, Swagger или HAR".into(),
        ));
    };
    for item in &mut imported.requests {
        tidy(&mut item.request);
    }
    settle_vars(&mut imported);
    dedupe_warnings(&mut imported.warnings);
    Ok(imported)
}

/// Into `dir`: a new collection there, or, when `dir` is a collection, a new folder in it named
/// after the import. Variables go where the collection keeps them; ones it already has are kept.
pub fn write(imported: &Imported, dir: &Path) -> Result<Written> {
    let existing = dir.join(MANIFEST).is_file();
    let (root, base) = if existing {
        let folder = free_path(dir, &clean_name(&imported.name).unwrap_or_else(|_| "Импорт".into()), "", None);
        (dir.to_path_buf(), folder)
    } else {
        fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        let mut manifest = DocumentMut::new();
        manifest.insert("name", toml_edit::value(imported.name.clone()));
        write_doc(&dir.join(MANIFEST), &manifest)?;
        let ignore = dir.join(".gitignore");
        if !ignore.exists() {
            fs::write(&ignore, "*.local.toml\n").map_err(|e| Error::io(&ignore, e))?;
        }
        (dir.to_path_buf(), dir.to_path_buf())
    };

    let mut kept = Vec::new();
    let (shared, secret): (Vec<&Var>, Vec<&Var>) = imported.vars.iter().partition(|v| !v.secret);
    add_vars(&root.join(MANIFEST), &shared, &mut kept, "коллекция")?;
    add_vars(&root.join("tsubamedo.local.toml"), &secret, &mut kept, "коллекция, секрет")?;
    let mut environments = Vec::new();
    let known = crate::collection::Collection::open(&root).map(|c| c.environments()).unwrap_or_default();
    for env in &imported.environments {
        let name = clean_name(&env.name).unwrap_or_else(|_| "env".into());
        // `Stage` and `stage` are one file on a Mac: the collection's spelling wins everywhere.
        let name = known.iter().find(|k| k.to_lowercase() == name.to_lowercase()).cloned().unwrap_or(name);
        let (shared, secret): (Vec<&Var>, Vec<&Var>) = env.vars.iter().partition(|v| !v.secret);
        let env_dir = root.join("env");
        // The environment is its file without `.local`, even when every value is secret.
        let file = env_dir.join(format!("{name}.toml"));
        if !file.exists() {
            write_doc(&file, &"[vars]\n".parse().expect("valid TOML"))?;
        }
        add_vars(&file, &shared, &mut kept, &name)?;
        add_vars(&env_dir.join(format!("{name}.local.toml")), &secret, &mut kept, &format!("{name}, секрет"))?;
        environments.push(name);
    }

    for (rel, text) in &imported.files {
        let path = base.join(rel);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        fs::write(&path, text).map_err(|e| Error::io(&path, e))?;
    }

    // Numbered names keep the order the requests had there, where it was not alphabetical.
    let mut by_folder: BTreeMap<&[String], Vec<&Request>> = BTreeMap::new();
    for item in &imported.requests {
        by_folder.entry(&item.folder).or_default().push(&item.request);
    }
    for (folder, requests) in by_folder {
        let mut dir = base.clone();
        for part in folder {
            dir.push(file_name(part));
        }
        fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        let names: Vec<String> = requests.iter().map(|r| file_name(r.name.as_deref().unwrap_or("Запрос"))).collect();
        let mut sorted = names.clone();
        sorted.sort_by_key(|n| n.to_lowercase());
        let numbered = imported.format != Format::OpenApi && imported.format != Format::Swagger && names != sorted;
        let width = requests.len().to_string().len().max(2);
        for (i, (request, name)) in requests.iter().zip(names).enumerate() {
            let base_name = if numbered { format!("{:0width$} {name}", i + 1) } else { name };
            let path = free_path(&dir, &base_name, ".toml", None);
            RequestFile::create(&path, request).save()?;
        }
    }

    let folder = base.strip_prefix(&root).unwrap_or(&base).to_string_lossy().into_owned();
    Ok(Written { root, folder, requests: imported.requests.len(), environments, kept })
}

/// A free folder in `parent` named after the import, for a new collection.
pub fn new_dir(imported: &Imported, parent: &Path) -> PathBuf {
    free_path(parent, &file_name(&imported.name), "", None)
}

/// A file name from a request or folder name: separators become spaces, not dashes, so
/// `/users/{id}` reads as `users {id}`.
pub(crate) fn file_name(name: &str) -> String {
    let spaced: String = name.chars().map(|c| if c == '/' || c == '\\' { ' ' } else { c }).collect();
    let spaced = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let short: String = spaced.chars().take(80).collect();
    clean_name(&short).unwrap_or_else(|_| "Запрос".into())
}

/// Adds the variables the file does not have yet; `kept` gets those it has with another value.
fn add_vars(path: &Path, vars: &[&Var], kept: &mut Vec<String>, place: &str) -> Result<()> {
    if vars.is_empty() {
        return Ok(());
    }
    let mut doc = match fs::read_to_string(path) {
        Ok(text) => text.parse::<DocumentMut>().map_err(|e| Error::Parse { path: path.to_path_buf(), message: e.to_string() })?,
        Err(_) => DocumentMut::new(),
    };
    if path.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".local.toml")) && doc.is_empty() {
        doc.decor_mut().set_prefix("# Секреты: этот файл не попадает в git.\n");
    }
    let table = doc.entry("vars").or_insert_with(|| Item::Table(Table::new()));
    let Some(table) = table.as_table_like_mut() else {
        return Err(Error::Parse { path: path.to_path_buf(), message: "vars должен быть таблицей".into() });
    };
    for var in vars {
        match table.get(&var.name).and_then(|i| i.as_str()) {
            Some(old) if old != var.value => kept.push(format!("{} ({place})", var.name)),
            Some(_) => {}
            None if table.contains_key(&var.name) => kept.push(format!("{} ({place})", var.name)),
            None => {
                table.insert(&var.name, toml_edit::value(var.value.clone()));
            }
        }
    }
    write_doc(path, &doc)
}

fn write_doc(path: &Path, doc: &DocumentMut) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    }
    fs::write(path, doc.to_string()).map_err(|e| Error::io(path, e))
}

/// Empty placeholders go for values a request produces, or that request would never be sent
/// first; variables nothing defines are named, so the user knows what to fill in.
fn settle_vars(imported: &mut Imported) {
    let mut produced = BTreeSet::new();
    let mut used = BTreeSet::new();
    for item in &imported.requests {
        let r = &item.request;
        produced.extend(r.captures.iter().map(|c| c.name.clone()));
        produced.extend(scripts::sets(&r.scripts.before));
        produced.extend(scripts::sets(&r.scripts.after));
        used.extend(prepare::variables(r, &Vars::new()));
        used.extend(prepare::scripts_variables(r));
    }
    let placeholder = |v: &Var| v.value.is_empty() && produced.contains(&v.name);
    imported.vars.retain(|v| !placeholder(v));
    for env in &mut imported.environments {
        env.vars.retain(|v| !placeholder(v));
    }
    let defined: BTreeSet<&String> = imported.vars.iter().chain(imported.environments.iter().flat_map(|e| &e.vars)).map(|v| &v.name).collect();
    let missing: Vec<&str> = used.iter().filter(|n| !defined.contains(n) && !produced.contains(*n)).map(String::as_str).collect();
    if !missing.is_empty() {
        imported.warnings.push(format!(
            "не заданы переменные: {} — впишите их в tsubamedo.toml или окружение, секреты — в *.local.toml",
            missing.join(", ")
        ));
    }
}

/// The same things said once.
fn dedupe_warnings(warnings: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    warnings.retain(|w| seen.insert(w.clone()));
}

/// Headers the body already gives: a `Content-Type` that says what the body kind says goes
/// away; for a text body it becomes the body's own type.
fn tidy(request: &mut Request) {
    let content_type = request.headers.iter().position(|p| p.name.eq_ignore_ascii_case("content-type"));
    let Some(i) = content_type else { return };
    let value = request.headers[i].value.to_lowercase();
    let drop = match &mut request.body {
        Body::Json { .. } => value.starts_with("application/json"),
        Body::Form { .. } => value.starts_with("application/x-www-form-urlencoded"),
        // The boundary comes with the body; a fixed one would break it.
        Body::Multipart { .. } => value.starts_with("multipart/"),
        Body::Text { content_type, .. } if content_type.is_none() && request.headers[i].enabled => {
            *content_type = Some(request.headers[i].value.clone());
            true
        }
        Body::Text { content_type: Some(t), .. } => t.eq_ignore_ascii_case(&request.headers[i].value),
        _ => false,
    };
    if drop {
        request.headers.remove(i);
    }
}

// ---- helpers shared by the formats ----

/// A JSON value as the text a field holds: strings as they are, other values as JSON.
fn text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn str_at<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn array_at<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
}

/// The address without its query, and the query as pairs.
fn split_query(url: &str) -> (String, Vec<Pair>) {
    let url = url.split('#').next().unwrap_or_default();
    match url.split_once('?') {
        Some((base, query)) => {
            let pairs = query
                .split('&')
                .filter(|p| !p.is_empty())
                .map(|p| {
                    let (n, v) = p.split_once('=').unwrap_or((p, ""));
                    Pair::new(decode(n), decode(v))
                })
                .collect();
            (base.to_string(), pairs)
        }
        None => (url.to_string(), Vec::new()),
    }
}

/// Percent-decoded, `{{variables}}` untouched; `+` is a space as in forms.
fn decode(s: &str) -> String {
    url::form_urlencoded::parse(format!("x={}", s.replace('&', "%26")).as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_else(|| s.to_string())
}

/// A body from a media type and its text.
fn body_of(mime: &str, text: String) -> Body {
    let m = mime.to_lowercase();
    if text.is_empty() {
        Body::None
    } else if m.contains("json") {
        Body::Json { content: text }
    } else {
        Body::Text { content: text, content_type: (!mime.is_empty()).then(|| mime.to_string()) }
    }
}

/// `GET /users/{id}` for a request without a name.
fn path_name(url: &str) -> String {
    let path = url::Url::parse(url).map(|u| u.path().to_string()).unwrap_or_else(|_| {
        let no_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
        let after_vars = no_scheme.rsplit_once("}}").map_or(no_scheme, |(_, rest)| rest);
        after_vars.split('?').next().unwrap_or_default().to_string()
    });
    let path = path.trim_end_matches('/');
    if path.is_empty() { "/".into() } else { path.to_string() }
}

/// Every text of a request through `f`: templates of other tools become ours.
fn map_texts(request: &mut Request, mut f: impl FnMut(&str) -> String) {
    use crate::model::Auth;
    request.url = f(&request.url);
    for p in request.query.iter_mut().chain(request.headers.iter_mut()) {
        p.name = f(&p.name);
        p.value = f(&p.value);
    }
    match &mut request.auth {
        Auth::None => {}
        Auth::Basic { username, password } | Auth::Digest { username, password } => {
            *username = f(username);
            *password = f(password);
        }
        Auth::Bearer { token } => *token = f(token),
        Auth::ApiKey { key, value, .. } => {
            *key = f(key);
            *value = f(value);
        }
        Auth::OAuth2(o) => {
            for field in crate::prepare::oauth_fields(o) {
                *field = f(field);
            }
        }
        Auth::AwsV4(a) => {
            for field in crate::prepare::aws_fields(a) {
                *field = f(field);
            }
        }
    }
    match &mut request.body {
        Body::Json { content } | Body::Text { content, .. } => *content = f(content),
        Body::Form { fields } => {
            for p in fields {
                p.name = f(&p.name);
                p.value = f(&p.value);
            }
        }
        Body::Multipart { parts } => {
            for p in parts {
                p.name = f(&p.name);
                p.value = f(&p.value);
            }
        }
        Body::File { path, .. } => *path = f(path),
        Body::None => {}
    }
}

/// Built-in variables of Postman and Insomnia that have ours; the rest stay and are named.
/// Postman's made-up values (`$randomEmail`…) are ours as they are.
fn builtin(name: &str) -> Option<String> {
    Some(match name {
        "$guid" | "$randomUUID" | "$uuid" => "{{$uuid}}".into(),
        "$timestamp" | "$isoTimestamp" | "$randomInt" => format!("{{{{{name}}}}}"),
        _ if crate::fake::value(name.strip_prefix('$')?).is_some() => format!("{{{{{name}}}}}"),
        _ => return None,
    })
}

/// `{{$guid}}` → `{{$uuid}}` and so on; unknown built-ins are reported.
fn map_builtins(text: &str, warnings: &mut Vec<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            out.push_str(&rest[open..]);
            return out;
        };
        let name = after[..close].trim();
        match builtin(name) {
            Some(ours) => out.push_str(&ours),
            None => {
                if name.starts_with('$') && !name.starts_with("$env.") {
                    warnings.push(format!("встроенной переменной {{{{{name}}}}} у нас нет — она осталась как есть"));
                }
                out.push_str(&rest[open..open + 2 + close + 2]);
            }
        }
        rest = &after[close + 2..];
    }
    out.push_str(rest);
    out
}

/// Scripts of a folder and of the request, in the order the other tool runs them.
fn join_scripts(parts: &[(String, &str)]) -> String {
    let parts: Vec<&(String, &str)> = parts.iter().filter(|(_, s)| !s.trim().is_empty()).collect();
    match parts.as_slice() {
        [] => String::new(),
        [(from, only)] if from == "запроса" => format!("{}\n", only.trim_end()),
        many => many.iter().map(|(from, s)| format!("// {from}\n{}\n", s.trim_end())).collect::<Vec<_>>().join("\n"),
    }
}

#[cfg(test)]
mod tests;
