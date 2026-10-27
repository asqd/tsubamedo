//! One request per TOML file.
//!
//! ```toml
//! name = "Получить пользователя"
//! method = "GET"
//! url = "{{base_url}}/users/{{id}}"
//!
//! [query]
//! expand = "orders"
//! debug = { value = "1", enabled = false }
//!
//! [headers]
//! Accept = "application/json"
//! X-Tag = ["a", "b"]            # the same header twice
//!
//! [auth]
//! type = "bearer"               # basic, digest: username, password; apikey: key, value, in
//! token = "{{token}}"
//!
//! # or OAuth 2.0: the token is got, kept and renewed by the app
//! # [auth]
//! # type = "oauth2"
//! # grant = "client_credentials"  # authorization_code (default), password
//! # token_url = "{{auth}}/oauth/token"
//! # client_id = "{{client_id}}"
//! # client_secret = "{{client_secret}}"
//! # scope = "orders:read"
//! # also: auth_url, redirect_uri, audience, username, password,
//! # pkce = false, client_auth = "body" (client_id and client_secret in the form)
//!
//! [body]
//! type = "json"                 # json, text, form, multipart, file
//! content = '''
//! {"name": "Котик"}
//! '''
//!
//! [options]
//! follow_redirects = false
//! timeout_ms = 5000
//! verify_tls = false
//! cookies = false               # neither send the environment's cookies nor keep new ones
//!
//! # What the response must be (see `checks`); a check can be switched off in place.
//! checks = [
//!   'status == 200',
//!   '$.items count > 0',
//!   { check = '$.debug not exists', enabled = false },
//! ]
//!
//! # Values from the response for other requests: they use {{token}}.
//! [capture]
//! token = '$.token'
//!
//! # JavaScript before and after the request (see `scripts`).
//! [scripts]
//! after = '''
//! test("есть заказы", () => expect(response.json().items).not.toHaveLength(0));
//! '''
//! ```
//!
//! (`checks` is written before the first table, as TOML requires of top-level keys.)
//!
//! [`RequestFile::update`] edits the parsed document in place and touches only the values that
//! differ, so comments, order and formatting survive and a save without changes rewrites
//! nothing: the git diff shows exactly what the user changed.

use std::fs;
use std::path::{Path, PathBuf};

use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, TableLike, Value};

use crate::model::{ApiKeyLocation, Auth, Body, Check, ClientAuth, Grant, OAuth2, Pair, Part, Request, RequestOptions, Scripts};
use crate::{Error, Result};

pub struct RequestFile {
    path: PathBuf,
    doc: DocumentMut,
    saved: String,
}

impl RequestFile {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let text = fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
        Self::parse(path, text)
    }

    pub fn parse(path: impl Into<PathBuf>, text: String) -> Result<Self> {
        let path = path.into();
        let doc = text
            .parse::<DocumentMut>()
            .map_err(|e| Error::Parse { path: path.clone(), message: e.to_string() })?;
        Ok(RequestFile { path, doc, saved: text })
    }

    /// A new file that is not written until [`RequestFile::save`].
    pub fn create(path: impl Into<PathBuf>, request: &Request) -> Self {
        let mut doc = DocumentMut::new();
        if let Some(name) = &request.name {
            doc.insert("name", Item::Value(string_value(name)));
        }
        doc.insert("method", Item::Value(string_value(&request.method)));
        doc.insert("url", Item::Value(string_value(&request.url)));
        let mut file = RequestFile { path: path.into(), doc, saved: String::new() };
        file.update(request);
        file
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn request(&self) -> Result<Request> {
        parse_request(self.doc.as_item())
            .map_err(|message| Error::Parse { path: self.path.clone(), message })
    }

    /// Brings the document to `request`, changing only what differs.
    pub fn update(&mut self, request: &Request) {
        let root = self.doc.as_item_mut();
        let t = root.as_table_like_mut().expect("document root is a table");
        sync_str(t, "name", request.name.as_deref());
        if !(request.method == "GET" && t.get("method").is_none()) {
            sync_str(t, "method", Some(&request.method));
        }
        sync_str(t, "url", Some(&request.url));
        sync_pairs(root, "query", &request.query);
        sync_pairs(root, "headers", &request.headers);
        sync_auth(root, &request.auth);
        sync_body(root, &request.body);
        sync_options(root, &request.options);
        sync_pairs(root, "capture", &request.captures);
        sync_checks(root, &request.checks);
        sync_scripts(root, &request.scripts);
    }

    pub fn text(&self) -> String {
        self.doc.to_string()
    }

    pub fn is_dirty(&self) -> bool {
        self.text() != self.saved
    }

    /// Writes the file if it changed, through a temporary file so a crash never leaves half
    /// of it. Returns whether anything was written.
    pub fn save(&mut self) -> Result<bool> {
        let text = self.text();
        if text == self.saved && self.path.exists() {
            return Ok(false);
        }
        let dir = self.path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        let tmp = tempfile::NamedTempFile::new_in(dir).map_err(|e| Error::io(dir, e))?;
        fs::write(tmp.path(), &text).map_err(|e| Error::io(tmp.path(), e))?;
        tmp.persist(&self.path).map_err(|e| Error::io(&self.path, e.error))?;
        self.saved = text;
        Ok(true)
    }
}

// ---- reading ----

pub(crate) fn parse_request(root: &Item) -> Result<Request, String> {
    let t = root.as_table_like().ok_or("файл запроса должен быть таблицей")?;
    Ok(Request {
        name: opt_str(t, "name")?,
        method: opt_str(t, "method")?.unwrap_or_else(|| "GET".into()),
        url: opt_str(t, "url")?.unwrap_or_default(),
        query: pairs(t.get("query"), "query")?,
        headers: pairs(t.get("headers"), "headers")?,
        auth: parse_auth(t.get("auth"))?,
        body: parse_body(t.get("body"))?,
        options: parse_options(t.get("options"))?,
        captures: pairs(t.get("capture"), "capture")?,
        checks: parse_checks(t.get("checks"))?,
        scripts: parse_scripts(t.get("scripts"))?,
    })
}

fn parse_scripts(item: Option<&Item>) -> Result<Scripts, String> {
    let Some(t) = item.and_then(Item::as_table_like) else {
        return if item.is_some() { Err("scripts должен быть таблицей".into()) } else { Ok(Scripts::default()) };
    };
    Ok(Scripts {
        before: opt_str(t, "before")?.unwrap_or_default(),
        after: opt_str(t, "after")?.unwrap_or_default(),
    })
}

fn parse_checks(item: Option<&Item>) -> Result<Vec<Check>, String> {
    let Some(item) = item else { return Ok(Vec::new()) };
    let array = item.as_array().ok_or("checks должен быть списком строк")?;
    array.iter().map(parse_check).collect()
}

fn parse_check(v: &Value) -> Result<Check, String> {
    match v {
        Value::String(s) => Ok(Check::new(s.value().clone())),
        Value::InlineTable(t) => Ok(Check {
            text: opt_str(t, "check")?.ok_or("checks: у { … } нет check")?,
            enabled: opt_bool(t, "enabled")?.unwrap_or(true),
        }),
        _ => Err("checks: нужна строка или { check = \"…\", enabled = false }".to_string()),
    }
}

/// One value of a pair table: text, or a file in multipart forms.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    value: String,
    is_file: bool,
    enabled: bool,
}

fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.value().clone()),
        Value::Integer(i) => Some(i.value().to_string()),
        Value::Float(f) => Some(f.value().to_string()),
        Value::Boolean(b) => Some(b.value().to_string()),
        Value::Datetime(d) => Some(d.value().to_string()),
        _ => None,
    }
}

fn opt_str(t: &dyn TableLike, key: &str) -> Result<Option<String>, String> {
    match t.get(key) {
        None => Ok(None),
        Some(item) => item
            .as_value()
            .and_then(scalar)
            .map(Some)
            .ok_or_else(|| format!("{key} должен быть строкой")),
    }
}

fn opt_bool(t: &dyn TableLike, key: &str) -> Result<Option<bool>, String> {
    match t.get(key) {
        None => Ok(None),
        Some(item) => item.as_bool().map(Some).ok_or_else(|| format!("{key}: нужно true или false")),
    }
}

fn entries_of(name: &str, item: &Item) -> Result<Vec<Entry>, String> {
    match item {
        Item::Value(Value::Array(array)) => array.iter().map(|v| entry_of(name, v)).collect(),
        Item::Value(v) => entry_of(name, v).map(|e| vec![e]),
        Item::Table(t) => entry_of_object(name, t).map(|e| vec![e]),
        _ => Err(format!("{name}: непонятное значение")),
    }
}

fn entry_of(name: &str, v: &Value) -> Result<Entry, String> {
    if let Some(value) = scalar(v) {
        return Ok(Entry { value, is_file: false, enabled: true });
    }
    match v {
        Value::InlineTable(t) => entry_of_object(name, t),
        _ => Err(format!("{name}: нужна строка или {{ value = \"…\" }}")),
    }
}

fn entry_of_object(name: &str, t: &dyn TableLike) -> Result<Entry, String> {
    let enabled = opt_bool(t, "enabled")?.unwrap_or(true);
    if let Some(path) = opt_str(t, "file")? {
        return Ok(Entry { value: path, is_file: true, enabled });
    }
    let value = opt_str(t, "value")?.ok_or_else(|| format!("{name}: нет value"))?;
    Ok(Entry { value, is_file: false, enabled })
}

fn entry_list(item: Option<&Item>, section: &str) -> Result<Vec<(String, Entry)>, String> {
    let Some(item) = item else { return Ok(Vec::new()) };
    let t = item.as_table_like().ok_or_else(|| format!("{section} должен быть таблицей"))?;
    let mut out = Vec::new();
    for (name, item) in t.iter() {
        for entry in entries_of(name, item).map_err(|e| format!("{section}.{e}"))? {
            out.push((name.to_string(), entry));
        }
    }
    Ok(out)
}

fn pairs(item: Option<&Item>, section: &str) -> Result<Vec<Pair>, String> {
    entry_list(item, section)?
        .into_iter()
        .map(|(name, e)| {
            if e.is_file {
                return Err(format!("{section}.{name}: файл можно отправить только в multipart"));
            }
            Ok(Pair { name, value: e.value, enabled: e.enabled })
        })
        .collect()
}

fn parse_auth(item: Option<&Item>) -> Result<Auth, String> {
    let Some(t) = item.and_then(Item::as_table_like) else {
        return if item.is_some() { Err("auth должен быть таблицей".into()) } else { Ok(Auth::None) };
    };
    let field = |key: &str| opt_str(t, key).map(Option::unwrap_or_default);
    Ok(match opt_str(t, "type")?.as_deref() {
        None | Some("none") => Auth::None,
        Some("basic") => Auth::Basic { username: field("username")?, password: field("password")? },
        Some("bearer") => Auth::Bearer { token: field("token")? },
        Some("digest") => Auth::Digest { username: field("username")?, password: field("password")? },
        Some("apikey") => Auth::ApiKey {
            key: field("key")?,
            value: field("value")?,
            location: match opt_str(t, "in")?.as_deref() {
                None | Some("header") => ApiKeyLocation::Header,
                Some("query") => ApiKeyLocation::Query,
                Some(other) => return Err(format!("auth.in: «{other}», нужно header или query")),
            },
        },
        Some("oauth2") => Auth::OAuth2(OAuth2 {
            grant: match opt_str(t, "grant")?.as_deref() {
                None | Some("authorization_code") => Grant::AuthorizationCode,
                Some("client_credentials") => Grant::ClientCredentials,
                Some("password") => Grant::Password,
                Some(other) => return Err(format!("auth.grant: «{other}», нужно authorization_code, client_credentials или password")),
            },
            auth_url: field("auth_url")?,
            token_url: field("token_url")?,
            client_id: field("client_id")?,
            client_secret: field("client_secret")?,
            scope: field("scope")?,
            audience: field("audience")?,
            redirect_uri: field("redirect_uri")?,
            pkce: opt_bool(t, "pkce")?.unwrap_or(true),
            client_auth: match opt_str(t, "client_auth")?.as_deref() {
                None | Some("basic") => ClientAuth::Basic,
                Some("body") => ClientAuth::Body,
                Some(other) => return Err(format!("auth.client_auth: «{other}», нужно basic или body")),
            },
            username: field("username")?,
            password: field("password")?,
        }),
        Some(other) => return Err(format!("auth.type: неизвестный тип «{other}»")),
    })
}

fn parse_body(item: Option<&Item>) -> Result<Body, String> {
    let Some(t) = item.and_then(Item::as_table_like) else {
        return if item.is_some() { Err("body должен быть таблицей".into()) } else { Ok(Body::None) };
    };
    let content = || opt_str(t, "content").map(Option::unwrap_or_default);
    Ok(match opt_str(t, "type")?.as_deref() {
        None => return Err("в body не указан type".into()),
        Some("none") => Body::None,
        Some("json") => Body::Json { content: content()? },
        Some("text") => Body::Text { content: content()?, content_type: opt_str(t, "content_type")? },
        Some("form") => Body::Form { fields: pairs(t.get("fields"), "body.fields")? },
        Some("multipart") => Body::Multipart {
            parts: entry_list(t.get("fields"), "body.fields")?
                .into_iter()
                .map(|(name, e)| Part { name, value: e.value, is_file: e.is_file, enabled: e.enabled })
                .collect(),
        },
        Some("file") => Body::File {
            path: opt_str(t, "path")?.ok_or("body: у type = \"file\" нет path")?,
            content_type: opt_str(t, "content_type")?,
        },
        Some(other) => return Err(format!("body.type: неизвестный тип «{other}»")),
    })
}

fn parse_options(item: Option<&Item>) -> Result<RequestOptions, String> {
    let Some(t) = item.and_then(Item::as_table_like) else { return Ok(RequestOptions::default()) };
    let timeout_ms = match t.get("timeout_ms") {
        None => None,
        Some(item) => Some(
            item.as_integer()
                .and_then(|i| u64::try_from(i).ok())
                .ok_or("options.timeout_ms: нужно число миллисекунд")?,
        ),
    };
    Ok(RequestOptions {
        follow_redirects: opt_bool(t, "follow_redirects")?,
        timeout_ms,
        verify_tls: opt_bool(t, "verify_tls")?,
        cookies: opt_bool(t, "cookies")?,
    })
}

// ---- writing ----

/// A string value in the most readable form: multi-line text (JSON bodies) as `'''` blocks.
fn string_value(s: &str) -> Value {
    let literal_ok = s.contains('\n')
        && !s.contains("'''")
        && !s.ends_with('\'')
        && !s.chars().any(|c| c.is_control() && c != '\n' && c != '\t');
    if literal_ok
        && let Ok(doc) = format!("v = '''\n{s}'''\n").parse::<DocumentMut>()
        && let Some(mut v) = doc.get("v").and_then(Item::as_value).cloned()
    {
        v.decor_mut().clear();
        return v;
    }
    Value::from(s)
}

/// Replaces a value but keeps the comment and spacing around it.
fn set_value(t: &mut dyn TableLike, key: &str, mut value: Value) {
    if let Some(Item::Value(old)) = t.get_mut(key) {
        *value.decor_mut() = old.decor().clone();
        *old = value;
    } else {
        value.decor_mut().clear();
        t.insert(key, Item::Value(value));
    }
}

fn sync_str(t: &mut dyn TableLike, key: &str, want: Option<&str>) {
    let have = t.get(key).and_then(Item::as_value).and_then(scalar);
    match want {
        None if t.get(key).is_some() => {
            t.remove(key);
        }
        Some(want) if have.as_deref() != Some(want) => set_value(t, key, string_value(want)),
        _ => {}
    }
}

fn sync_bool(t: &mut dyn TableLike, key: &str, want: Option<bool>) {
    match want {
        None if t.get(key).is_some() => {
            t.remove(key);
        }
        Some(want) if t.get(key).and_then(Item::as_bool) != Some(want) => set_value(t, key, want.into()),
        _ => {}
    }
}

fn sync_int(t: &mut dyn TableLike, key: &str, want: Option<u64>) {
    match want {
        None if t.get(key).is_some() => {
            t.remove(key);
        }
        Some(want) => {
            let want = i64::try_from(want).unwrap_or(i64::MAX);
            if t.get(key).and_then(Item::as_integer) != Some(want) {
                set_value(t, key, want.into());
            }
        }
        _ => {}
    }
}

/// The sub-table `key` of `parent`, created inline when the parent itself is inline.
fn child<'a>(parent: &'a mut Item, key: &str) -> &'a mut Item {
    let inline = parent.is_inline_table();
    let t = parent.as_table_like_mut().expect("parent is a table");
    if !t.get(key).is_some_and(Item::is_table_like) {
        let fresh = if inline { Item::Value(Value::InlineTable(InlineTable::new())) } else { Item::Table(Table::new()) };
        t.insert(key, fresh);
    }
    t.get_mut(key).expect("just inserted")
}

fn remove(parent: &mut Item, key: &str) {
    if let Some(t) = parent.as_table_like_mut() {
        t.remove(key);
    }
}

fn entry_value(e: &Entry) -> Value {
    if !e.is_file && e.enabled {
        return string_value(&e.value);
    }
    let mut t = InlineTable::new();
    t.insert(if e.is_file { "file" } else { "value" }, string_value(&e.value));
    if !e.enabled {
        t.insert("enabled", false.into());
    }
    t.fmt();
    Value::InlineTable(t)
}

fn entries_value(entries: &[Entry]) -> Value {
    match entries {
        [one] => entry_value(one),
        many => {
            let mut array: Array = many.iter().map(entry_value).collect();
            array.fmt();
            Value::Array(array)
        }
    }
}

/// Entries grouped by name in order of first appearance: repeated headers become arrays.
fn group(list: impl IntoIterator<Item = (String, Entry)>) -> Vec<(String, Vec<Entry>)> {
    let mut groups: Vec<(String, Vec<Entry>)> = Vec::new();
    for (name, entry) in list {
        match groups.iter_mut().find(|(n, _)| *n == name) {
            Some((_, entries)) => entries.push(entry),
            None => groups.push((name, vec![entry])),
        }
    }
    groups
}

fn sync_entries(parent: &mut Item, key: &str, list: Vec<(String, Entry)>) {
    if list.is_empty() {
        remove(parent, key);
        return;
    }
    let groups = group(list);
    let t = child(parent, key).as_table_like_mut().expect("table");
    let stale: Vec<String> = t
        .iter()
        .map(|(k, _)| k.to_string())
        .filter(|k| !groups.iter().any(|(name, _)| name == k))
        .collect();
    for k in stale {
        t.remove(&k);
    }
    for (name, entries) in &groups {
        let unchanged = t.get(name).map(|item| entries_of(name, item)).is_some_and(|have| have.as_ref() == Ok(entries));
        if !unchanged {
            set_value(t, name, entries_value(entries));
        }
    }
}

fn pair_entries(pairs: &[Pair]) -> Vec<(String, Entry)> {
    pairs
        .iter()
        .map(|p| (p.name.clone(), Entry { value: p.value.clone(), is_file: false, enabled: p.enabled }))
        .collect()
}

fn sync_pairs(parent: &mut Item, key: &str, pairs: &[Pair]) {
    sync_entries(parent, key, pair_entries(pairs));
}

/// Writes the fields of a typed section; keys known to other types are removed, unknown
/// ones are left for whoever wrote them.
fn sync_section<'a>(parent: &'a mut Item, key: &str, known: &[&str], fields: &[(&str, Option<&str>)]) -> &'a mut Item {
    let item = child(parent, key);
    let t = item.as_table_like_mut().expect("table");
    for k in known {
        if !fields.iter().any(|(f, _)| f == k) && t.get(k).is_some() {
            t.remove(k);
        }
    }
    for (k, v) in fields {
        sync_str(t, k, *v);
    }
    item
}

fn sync_auth(root: &mut Item, auth: &Auth) {
    if parse_auth(root.get("auth")).as_ref() == Ok(auth) {
        return;
    }
    const KNOWN: &[&str] = &[
        "type", "username", "password", "token", "key", "value", "in", "grant", "auth_url", "token_url", "client_id", "client_secret",
        "scope", "audience", "redirect_uri", "pkce", "client_auth",
    ];
    match auth {
        Auth::None => remove(root, "auth"),
        Auth::Basic { username, password } => {
            sync_section(root, "auth", KNOWN, &[("type", Some("basic")), ("username", Some(username)), ("password", Some(password))]);
        }
        Auth::Bearer { token } => {
            sync_section(root, "auth", KNOWN, &[("type", Some("bearer")), ("token", Some(token))]);
        }
        Auth::Digest { username, password } => {
            sync_section(root, "auth", KNOWN, &[("type", Some("digest")), ("username", Some(username)), ("password", Some(password))]);
        }
        Auth::ApiKey { key, value, location } => {
            let location = match location {
                ApiKeyLocation::Header => None,
                ApiKeyLocation::Query => Some("query"),
            };
            sync_section(root, "auth", KNOWN, &[("type", Some("apikey")), ("key", Some(key)), ("value", Some(value)), ("in", location)]);
        }
        Auth::OAuth2(o) => {
            // Only what is set, so a client-credentials request is a few lines.
            let given = |s: &str| Some(s.to_string()).filter(|s| !s.is_empty());
            let code = o.grant == Grant::AuthorizationCode;
            let fields = [
                ("type", Some("oauth2".to_string())),
                ("grant", (!code).then(|| o.grant.as_str().to_string())),
                ("auth_url", given(&o.auth_url)),
                ("token_url", Some(o.token_url.clone())),
                ("client_id", Some(o.client_id.clone())),
                ("client_secret", given(&o.client_secret)),
                ("scope", given(&o.scope)),
                ("audience", given(&o.audience)),
                ("redirect_uri", given(&o.redirect_uri)),
                ("client_auth", (o.client_auth == ClientAuth::Body).then(|| "body".to_string())),
                ("username", given(&o.username)),
                ("password", given(&o.password)),
            ];
            let fields: Vec<(&str, Option<&str>)> = fields.iter().map(|(k, v)| (*k, v.as_deref())).collect();
            let section = sync_section(root, "auth", KNOWN, &fields);
            let t = section.as_table_like_mut().expect("table");
            sync_bool(t, "pkce", (code && !o.pkce).then_some(false));
        }
    }
}

fn sync_body(root: &mut Item, body: &Body) {
    if parse_body(root.get("body")).as_ref() == Ok(body) {
        return;
    }
    const KNOWN: &[&str] = &["type", "content", "content_type", "fields", "path"];
    // Form fields are synced on their own, so an edited field keeps the table's comments.
    const WITH_FIELDS: &[&str] = &["type", "content", "content_type", "path"];
    let kind = Some(body.kind());
    match body {
        Body::None => remove(root, "body"),
        Body::Json { content } => {
            sync_section(root, "body", KNOWN, &[("type", kind), ("content", Some(content))]);
        }
        Body::Text { content, content_type } => {
            sync_section(root, "body", KNOWN, &[("type", kind), ("content", Some(content)), ("content_type", content_type.as_deref())]);
        }
        Body::Form { fields } => {
            let section = sync_section(root, "body", WITH_FIELDS, &[("type", kind)]);
            sync_pairs(section, "fields", fields);
        }
        Body::Multipart { parts } => {
            let section = sync_section(root, "body", WITH_FIELDS, &[("type", kind)]);
            let list = parts
                .iter()
                .map(|p| (p.name.clone(), Entry { value: p.value.clone(), is_file: p.is_file, enabled: p.enabled }))
                .collect();
            sync_entries(section, "fields", list);
        }
        Body::File { path, content_type } => {
            sync_section(root, "body", KNOWN, &[("type", kind), ("path", Some(path)), ("content_type", content_type.as_deref())]);
        }
    }
}

/// A check as written: in single quotes when it has double ones, so `$.name == "Кот"` stays
/// readable.
fn check_string(s: &str) -> Value {
    if s.contains('"')
        && !s.contains('\'')
        && !s.chars().any(char::is_control)
        && let Ok(doc) = format!("v = '{s}'").parse::<DocumentMut>()
        && let Some(mut v) = doc.get("v").and_then(Item::as_value).cloned()
    {
        v.decor_mut().clear();
        return v;
    }
    Value::from(s)
}

/// One check per line, so adding or editing one shows as one line in the diff.
fn sync_checks(root: &mut Item, checks: &[Check]) {
    if parse_checks(root.get("checks")).as_deref() == Ok(checks) {
        return;
    }
    if checks.is_empty() {
        remove(root, "checks");
        return;
    }
    // Lines that stay are kept as they were written, quotes and all.
    let old: Vec<(Check, Value)> = root
        .get("checks")
        .and_then(Item::as_array)
        .map(|a| a.iter().filter_map(|v| Some((parse_check(v).ok()?, v.clone()))).collect())
        .unwrap_or_default();
    let mut array = Array::new();
    for c in checks {
        if let Some((_, v)) = old.iter().find(|(o, _)| o == c) {
            let mut value = v.clone();
            value.decor_mut().set_prefix("\n  ");
            value.decor_mut().set_suffix("");
            array.push_formatted(value);
            continue;
        }
        let mut value = if c.enabled {
            check_string(&c.text)
        } else {
            let mut t = InlineTable::new();
            t.insert("check", check_string(&c.text));
            t.insert("enabled", false.into());
            t.fmt();
            Value::InlineTable(t)
        };
        value.decor_mut().set_prefix("\n  ");
        array.push_formatted(value);
    }
    array.set_trailing_comma(true);
    array.set_trailing("\n");
    let t = root.as_table_like_mut().expect("document root is a table");
    set_value(t, "checks", Value::Array(array));
}

fn sync_scripts(root: &mut Item, scripts: &Scripts) {
    if parse_scripts(root.get("scripts")).as_ref() == Ok(scripts) {
        return;
    }
    if scripts.is_empty() {
        remove(root, "scripts");
        return;
    }
    let text = |s: &str| (!s.trim().is_empty()).then(|| if s.ends_with('\n') { s.to_string() } else { format!("{s}\n") });
    let (before, after) = (text(&scripts.before), text(&scripts.after));
    sync_section(root, "scripts", &["before", "after"], &[("before", before.as_deref()), ("after", after.as_deref())]);
}

fn sync_options(root: &mut Item, options: &RequestOptions) {
    if *options == RequestOptions::default() {
        remove(root, "options");
        return;
    }
    let t = child(root, "options").as_table_like_mut().expect("table");
    sync_bool(t, "follow_redirects", options.follow_redirects);
    sync_int(t, "timeout_ms", options.timeout_ms);
    sync_bool(t, "verify_tls", options.verify_tls);
    sync_bool(t, "cookies", options.cookies);
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"# Создание заказа: нужен токен из login
name = "Создать заказ"
method = "POST"
url    = "{{base_url}}/orders"   # без слэша в конце

[query]
dry_run = { value = "1", enabled = false }
page = 2

[headers]
Accept = "application/json"
X-Tag = ["a", "b"]

[auth]
type = "bearer"
token = "{{token}}"

[body]
type = "json"
content = '''
{
  "tour_id": 42
}
'''
"#;

    fn sample() -> RequestFile {
        RequestFile::parse("orders/create.toml", SAMPLE.to_string()).unwrap()
    }

    #[test]
    fn reads_every_section() {
        let r = sample().request().unwrap();
        assert_eq!(r.method, "POST");
        assert_eq!(r.query, vec![Pair { name: "dry_run".into(), value: "1".into(), enabled: false }, Pair::new("page", "2")]);
        assert_eq!(r.headers, vec![Pair::new("Accept", "application/json"), Pair::new("X-Tag", "a"), Pair::new("X-Tag", "b")]);
        assert_eq!(r.auth, Auth::Bearer { token: "{{token}}".into() });
        assert_eq!(r.body, Body::Json { content: "{\n  \"tour_id\": 42\n}\n".into() });
    }

    #[test]
    fn saving_the_same_request_changes_nothing() {
        let mut f = sample();
        let r = f.request().unwrap();
        f.update(&r);
        assert_eq!(f.text(), SAMPLE);
        assert!(!f.is_dirty());
    }

    #[test]
    fn an_edit_touches_only_its_line() {
        let mut f = sample();
        let mut r = f.request().unwrap();
        r.url = "{{base_url}}/v2/orders".into();
        r.headers.push(Pair::new("X-Request-Id", "{{$uuid}}"));
        r.query[0].enabled = true;
        f.update(&r);
        let want = SAMPLE
            .replace(r#"url    = "{{base_url}}/orders"   #"#, r#"url    = "{{base_url}}/v2/orders"   #"#)
            .replace(r#"dry_run = { value = "1", enabled = false }"#, r#"dry_run = "1""#)
            .replace("X-Tag = [\"a\", \"b\"]\n", "X-Tag = [\"a\", \"b\"]\nX-Request-Id = \"{{$uuid}}\"\n");
        assert_eq!(f.text(), want);
        assert_eq!(f.request().unwrap(), r);
    }

    #[test]
    fn switching_body_and_auth_types() {
        let mut f = sample();
        let mut r = f.request().unwrap();
        r.auth = Auth::Basic { username: "kot".into(), password: "{{password}}".into() };
        r.body = Body::Multipart {
            parts: vec![
                Part { name: "title".into(), value: "Котик".into(), is_file: false, enabled: true },
                Part { name: "photo".into(), value: "kot.png".into(), is_file: true, enabled: true },
            ],
        };
        r.options.timeout_ms = Some(5000);
        f.update(&r);
        let text = f.text();
        assert!(text.contains("[auth]\ntype = \"basic\"\nusername = \"kot\"\npassword = \"{{password}}\"\n"), "{text}");
        assert!(text.contains("[body]\ntype = \"multipart\"\n"), "{text}");
        assert!(text.contains("photo = { file = \"kot.png\" }"), "{text}");
        assert!(text.contains("[options]\ntimeout_ms = 5000\n"), "{text}");
        assert!(!text.contains("token"), "{text}");
        assert_eq!(f.request().unwrap(), r);

        r.body = Body::None;
        r.auth = Auth::None;
        r.options = RequestOptions::default();
        f.update(&r);
        assert!(!f.text().contains("[body") && !f.text().contains("[auth]") && !f.text().contains("[options]"), "{}", f.text());
    }

    #[test]
    fn oauth2_is_written_short_and_read_back() {
        let mut f = sample();
        let mut r = f.request().unwrap();
        r.auth = Auth::OAuth2(OAuth2 {
            grant: Grant::ClientCredentials,
            token_url: "{{auth}}/oauth/token".into(),
            client_id: "{{client_id}}".into(),
            client_secret: "{{client_secret}}".into(),
            scope: "orders:read".into(),
            ..OAuth2::default()
        });
        r.options.cookies = Some(false);
        f.update(&r);
        let text = f.text();
        assert!(
            text.contains(
                "[auth]\ntype = \"oauth2\"\ngrant = \"client_credentials\"\ntoken_url = \"{{auth}}/oauth/token\"\nclient_id = \"{{client_id}}\"\nclient_secret = \"{{client_secret}}\"\nscope = \"orders:read\"\n"
            ),
            "{text}"
        );
        assert!(text.contains("cookies = false"), "{text}");
        assert_eq!(f.request().unwrap(), r);

        let Auth::OAuth2(o) = &mut r.auth else { unreachable!() };
        o.grant = Grant::AuthorizationCode;
        o.auth_url = "{{auth}}/authorize".into();
        o.pkce = false;
        o.client_auth = ClientAuth::Body;
        f.update(&r);
        let text = f.text();
        assert!(!text.contains("grant =") && text.contains("pkce = false") && text.contains("client_auth = \"body\""), "{text}");
        assert_eq!(f.request().unwrap(), r);
    }

    #[test]
    fn checks_and_captures_round_trip() {
        let mut f = sample();
        let mut r = f.request().unwrap();
        r.checks = vec![Check::new("status == 201"), Check::new("$.name == \"Кот\""), Check { text: "time < 500".into(), enabled: false }];
        r.captures = vec![Pair::new("order_id", "$.id")];
        f.update(&r);
        let text = f.text();
        assert!(
            text.contains("checks = [\n  \"status == 201\",\n  '$.name == \"Кот\"',\n  { check = \"time < 500\", enabled = false },\n]\n"),
            "{text}"
        );
        assert!(text.find("checks = [").unwrap() < text.find("[query]").unwrap(), "{text}");
        assert!(text.ends_with("[capture]\norder_id = \"$.id\"\n"), "{text}");
        assert_eq!(f.request().unwrap(), r);
        let saved = f.text();
        f.update(&r);
        assert_eq!(f.text(), saved);

        r.checks[0].text = "status == 200".into();
        f.update(&r);
        assert_eq!(f.text(), saved.replace("status == 201", "status == 200"));
        // A line written by hand in single quotes keeps them when another line changes.
        let hand = f.text().replace("\"status == 200\"", "'status == 200'");
        let mut f = RequestFile::parse("orders/create.toml", hand.clone()).unwrap();
        r.checks.push(Check::new("size < 100"));
        f.update(&r);
        assert_eq!(f.text(), hand.replace("  { check = \"time < 500\", enabled = false },\n", "  { check = \"time < 500\", enabled = false },\n  \"size < 100\",\n"));
        r.checks.pop();
        r.checks.clear();
        r.captures.clear();
        f.update(&r);
        assert_eq!(f.text(), SAMPLE);
    }

    #[test]
    fn scripts_are_multiline_blocks() {
        let mut f = sample();
        let mut r = f.request().unwrap();
        r.scripts.after = "const o = response.json();\nvars.set(\"id\", o.id);".into();
        f.update(&r);
        let text = f.text();
        assert!(text.ends_with("[scripts]\nafter = '''\nconst o = response.json();\nvars.set(\"id\", o.id);\n'''\n"), "{text}");
        r.scripts.after.push('\n');
        assert_eq!(f.request().unwrap(), r);
        f.update(&r);
        assert_eq!(f.text(), text);
        r.scripts.after.clear();
        f.update(&r);
        assert_eq!(f.text(), SAMPLE);
    }

    #[test]
    fn new_file_is_readable() {
        let r = Request {
            name: Some("Логин".into()),
            method: "POST".into(),
            url: "{{base_url}}/login".into(),
            headers: vec![Pair::new("Accept", "application/json")],
            body: Body::Json { content: "{\"login\": \"{{login}}\"}\n".into() },
            ..Request::default()
        };
        let f = RequestFile::create("login.toml", &r);
        assert_eq!(
            f.text(),
            "name = \"Логин\"\nmethod = \"POST\"\nurl = \"{{base_url}}/login\"\n\n[headers]\nAccept = \"application/json\"\n\n[body]\ntype = \"json\"\ncontent = '''\n{\"login\": \"{{login}}\"}\n'''\n"
        );
        assert_eq!(f.request().unwrap(), r);
    }
}
