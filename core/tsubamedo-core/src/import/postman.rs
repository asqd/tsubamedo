//! Postman: collections v2.0 and v2.1, environments and globals.
//!
//! Postman's `{{variables}}` are ours already. A request without `auth` takes its folder's or
//! the collection's, and their scripts run before the request's own: both are copied into
//! every request, since a request file here stands alone.

use serde_json::Value;

use super::{Environment, Format, Imported, Placed, Var, array_at, body_of, join_scripts, map_builtins, map_texts, path_name, split_query, str_at, text};
use crate::model::{ApiKeyLocation, Auth, Body, ClientAuth, Grant, OAuth2, Pair, Part, Request};
use crate::Result;

#[derive(Clone, Default)]
struct Inherited {
    auth: Option<Value>,
    before: Vec<(String, String)>,
    after: Vec<(String, String)>,
}

#[derive(Default)]
struct Converter {
    out: Vec<Placed>,
    warnings: Vec<String>,
}

pub fn collection(doc: &Value) -> Result<Imported> {
    let info = &doc["info"];
    let mut c = Converter::default();
    let vars = array_at(doc, "variable")
        .iter()
        .filter(|v| !disabled(v))
        .map(|v| Var::new(key(v), text(&v["value"]), str_at(v, "type") == "secret"))
        .collect();
    let mut root = Inherited::default();
    c.inherit(doc, "из коллекции", &mut root);
    c.walk(array_at(doc, "item"), &[], &root);
    let mut warnings = c.warnings;
    for v in &mut c.out {
        map_texts(&mut v.request, |t| map_builtins(t, &mut warnings));
    }
    Ok(Imported { format: Format::Postman, name: str_at(info, "name").to_string(), requests: c.out, vars, environments: Vec::new(), warnings, files: Vec::new() })
}

/// An environment becomes one; globals become the collection's variables.
pub fn environment(doc: &Value) -> Imported {
    let vars: Vec<Var> = array_at(doc, "values")
        .iter()
        .filter(|v| v.get("enabled").and_then(Value::as_bool).unwrap_or(true))
        .map(|v| Var::new(key(v), text(&v["value"]), str_at(v, "type") == "secret"))
        .collect();
    let name = str_at(doc, "name").to_string();
    let globals = str_at(doc, "_postman_variable_scope") == "globals";
    Imported {
        format: Format::PostmanEnvironment,
        name: if globals { "Глобальные переменные".into() } else { name.clone() },
        requests: Vec::new(),
        vars: if globals { vars.clone() } else { Vec::new() },
        environments: if globals { Vec::new() } else { vec![Environment { name, vars }] },
        warnings: Vec::new(),
        files: Vec::new(),
    }
}

impl Converter {
    fn walk(&mut self, items: &[Value], folder: &[String], inherited: &Inherited) {
        for item in items {
            let name = str_at(item, "name").trim().to_string();
            if let Some(kids) = item.get("item").and_then(Value::as_array) {
                let mut next = inherited.clone();
                self.inherit(item, &format!("из папки «{name}»"), &mut next);
                let mut path = folder.to_vec();
                path.push(if name.is_empty() { "Папка".into() } else { name });
                self.walk(kids, &path, &next);
            } else if let Some(r) = item.get("request") {
                let mut request = self.request(r, inherited);
                if !name.is_empty() {
                    request.name = Some(name);
                }
                let mut befores: Vec<(String, &str)> = inherited.before.iter().map(|(f, s)| (f.clone(), s.as_str())).collect();
                let mut afters: Vec<(String, &str)> = inherited.after.iter().map(|(f, s)| (f.clone(), s.as_str())).collect();
                let (before, after) = events(item);
                befores.push(("запроса".into(), &before));
                afters.push(("запроса".into(), &after));
                request.scripts.before = join_scripts(&befores);
                request.scripts.after = join_scripts(&afters);
                self.out.push(Placed { folder: folder.to_vec(), request });
            }
        }
    }

    /// A folder's or the collection's auth and scripts, for the requests under it.
    fn inherit(&mut self, v: &Value, from: &str, into: &mut Inherited) {
        if let Some(auth) = v.get("auth").filter(|a| !a.is_null()) {
            into.auth = Some(auth.clone());
        }
        let (before, after) = events(v);
        into.before.push((from.into(), before));
        into.after.push((from.into(), after));
    }

    fn request(&mut self, r: &Value, inherited: &Inherited) -> Request {
        // v2.0 allows a request that is only its address.
        if let Some(url) = r.as_str() {
            let (url, query) = split_query(url);
            return Request { name: Some(path_name(&url)), url, query, ..Request::default() };
        }
        let (url, query) = self.url(r.get("url").unwrap_or(&Value::Null));
        let headers: Vec<Pair> = match r.get("header") {
            Some(Value::String(s)) => s
                .lines()
                .filter_map(|l| l.split_once(':'))
                .map(|(n, v)| Pair::new(n.trim(), v.trim()))
                .collect(),
            _ => array_at(r, "header").iter().map(|h| Pair { name: key(h).into(), value: text(&h["value"]), enabled: !disabled(h) }).collect(),
        };
        let auth = match r.get("auth").filter(|a| !a.is_null()).or(inherited.auth.as_ref()) {
            Some(a) => self.auth(a),
            None => Auth::None,
        };
        let body = self.body(r.get("body").unwrap_or(&Value::Null), &headers);
        let mut request = Request {
            name: Some(path_name(&url)),
            method: str_at(r, "method").to_uppercase(),
            url,
            query,
            headers,
            auth,
            body,
            ..Request::default()
        };
        if request.method.is_empty() {
            request.method = "GET".into();
        }
        request
    }

    /// The address with path variables (`:id`) filled in, and the query as pairs.
    fn url(&mut self, u: &Value) -> (String, Vec<Pair>) {
        if let Some(s) = u.as_str() {
            return split_query(s);
        }
        let raw = match u.get("raw").and_then(Value::as_str) {
            Some(raw) => raw.to_string(),
            None => {
                let part = |key: &str, sep: &str| match u.get(key) {
                    Some(Value::Array(a)) => a.iter().map(text).collect::<Vec<_>>().join(sep),
                    Some(other) => text(other),
                    None => String::new(),
                };
                let protocol = str_at(u, "protocol");
                let port = str_at(u, "port");
                format!(
                    "{}{}{}/{}",
                    if protocol.is_empty() { String::new() } else { format!("{protocol}://") },
                    part("host", "."),
                    if port.is_empty() { String::new() } else { format!(":{port}") },
                    part("path", "/")
                )
            }
        };
        let (mut base, from_raw) = split_query(&raw);
        let query = match u.get("query").and_then(Value::as_array) {
            Some(q) => q
                .iter()
                .filter(|p| !key(p).is_empty() || !text(&p["value"]).is_empty())
                .map(|p| Pair { name: key(p).into(), value: text(&p["value"]), enabled: !disabled(p) })
                .collect(),
            None => from_raw,
        };
        for v in array_at(u, "variable") {
            let name = key(v);
            let value = text(&v["value"]);
            let value = if value.is_empty() { format!("{{{{{name}}}}}") } else { value };
            base = replace_segment(&base, &format!(":{name}"), &value);
        }
        (base, query)
    }

    fn auth(&mut self, a: &Value) -> Auth {
        let kind = str_at(a, "type");
        // v2.1 keeps fields as a list of {key, value}, v2.0 as an object.
        let field = |name: &str| -> String {
            match a.get(kind) {
                Some(Value::Array(list)) => list.iter().find(|p| key(p) == name).map(|p| text(&p["value"])).unwrap_or_default(),
                Some(Value::Object(map)) => map.get(name).map(text).unwrap_or_default(),
                _ => String::new(),
            }
        };
        match kind {
            "noauth" | "" => Auth::None,
            "bearer" => Auth::Bearer { token: field("token") },
            "basic" => Auth::Basic { username: field("username"), password: field("password") },
            "digest" => Auth::Digest { username: field("username"), password: field("password") },
            "apikey" => Auth::ApiKey {
                key: field("key"),
                value: field("value"),
                location: if field("in") == "query" { ApiKeyLocation::Query } else { ApiKeyLocation::Header },
            },
            "oauth2" => {
                let grant = match field("grant_type").as_str() {
                    "client_credentials" => Grant::ClientCredentials,
                    "password_credentials" => Grant::Password,
                    "implicit" => {
                        self.warnings.push("OAuth 2.0 implicit из Postman перенесён как authorization code с PKCE: implicit устарел и не поддерживается".into());
                        Grant::AuthorizationCode
                    }
                    _ => Grant::AuthorizationCode,
                };
                Auth::OAuth2(OAuth2 {
                    grant,
                    auth_url: field("authUrl"),
                    token_url: field("accessTokenUrl"),
                    client_id: field("clientId"),
                    client_secret: field("clientSecret"),
                    scope: field("scope"),
                    // Postman's own page (oauth.pstmn.io) cannot bring the code here: a port on
                    // this machine is picked instead.
                    redirect_uri: Some(field("redirect_uri")).filter(|u| u.starts_with("http://127.0.0.1") || u.starts_with("http://localhost")).unwrap_or_default(),
                    // Postman has PKCE as a grant of its own; elsewhere it costs nothing to keep.
                    pkce: true,
                    client_auth: if field("client_authentication") == "body" { ClientAuth::Body } else { ClientAuth::Basic },
                    username: field("username"),
                    password: field("password"),
                    ..OAuth2::default()
                })
            }
            other => {
                self.warnings.push(format!("авторизация {other} из Postman пока не поддерживается — у таких запросов её нет"));
                Auth::None
            }
        }
    }

    fn body(&mut self, b: &Value, headers: &[Pair]) -> Body {
        if b.get("disabled").and_then(Value::as_bool).unwrap_or(false) {
            return Body::None;
        }
        let header_type = headers.iter().find(|h| h.enabled && h.name.eq_ignore_ascii_case("content-type")).map(|h| h.value.as_str());
        match str_at(b, "mode") {
            "raw" => {
                let language = b.pointer("/options/raw/language").and_then(Value::as_str).unwrap_or_default();
                let mime = match (language, header_type) {
                    (_, Some(t)) => t.to_string(),
                    ("json", None) => "application/json".into(),
                    ("xml", None) => "application/xml".into(),
                    ("html", None) => "text/html".into(),
                    ("javascript", None) => "application/javascript".into(),
                    _ => "text/plain".into(),
                };
                body_of(&mime, str_at(b, "raw").into())
            }
            "urlencoded" => Body::Form {
                fields: array_at(b, "urlencoded").iter().map(|p| Pair { name: key(p).into(), value: text(&p["value"]), enabled: !disabled(p) }).collect(),
            },
            "formdata" => Body::Multipart {
                parts: array_at(b, "formdata")
                    .iter()
                    .map(|p| {
                        let is_file = str_at(p, "type") == "file";
                        let value = if is_file {
                            match &p["src"] {
                                Value::Array(a) => a.first().map(text).unwrap_or_default(),
                                other => text(other),
                            }
                        } else {
                            text(&p["value"])
                        };
                        Part { name: key(p).into(), value, is_file, enabled: !disabled(p) }
                    })
                    .collect(),
            },
            "file" => match b.pointer("/file/src").map(text).filter(|s| !s.is_empty()) {
                Some(path) => Body::File { path, content_type: header_type.map(str::to_string) },
                None => Body::None,
            },
            "graphql" => {
                let g = &b["graphql"];
                let variables = match &g["variables"] {
                    Value::String(s) if !s.trim().is_empty() => serde_json::from_str(s).unwrap_or(Value::String(s.clone())),
                    Value::String(_) | Value::Null => Value::Object(Default::default()),
                    other => other.clone(),
                };
                let body = serde_json::json!({ "query": str_at(g, "query"), "variables": variables });
                Body::Json { content: serde_json::to_string_pretty(&body).unwrap_or_default() }
            }
            _ => Body::None,
        }
    }
}

/// `key` in v2.1, sometimes `name` in older exports.
fn key(v: &Value) -> &str {
    v.get("key").or_else(|| v.get("name")).and_then(Value::as_str).unwrap_or_default()
}

fn disabled(v: &Value) -> bool {
    v.get("disabled").and_then(Value::as_bool).unwrap_or(false)
}

/// The pre-request and test scripts of an item, collection or folder.
fn events(v: &Value) -> (String, String) {
    let mut before = String::new();
    let mut after = String::new();
    for e in array_at(v, "event") {
        if e.get("disabled").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        let code = match e.pointer("/script/exec") {
            Some(Value::Array(lines)) => lines.iter().map(text).collect::<Vec<_>>().join("\n"),
            Some(other) => text(other),
            None => String::new(),
        };
        match str_at(e, "listen") {
            "prerequest" => before.push_str(&code),
            "test" => after.push_str(&code),
            _ => {}
        }
    }
    (before, after)
}

/// `:id` as a whole path segment, so `:id` does not hit `:identifier` or a port.
fn replace_segment(url: &str, segment: &str, value: &str) -> String {
    url.split('/').map(|s| if s == segment { value } else { s }).collect::<Vec<_>>().join("/")
}
