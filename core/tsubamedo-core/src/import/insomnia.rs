//! Insomnia exports: version 4 (JSON, a flat list of resources with `parentId`) and version 5
//! (YAML of Insomnia 10 and later, a tree). Both become one tree and go through one walker.
//!
//! Insomnia writes variables as `{{ _.name }}`; a value from another request's response is a
//! tag, `{% response 'body', 'req_1', 'b64::JC50b2tlbg==::46b', 'never', 60 %}`. Such a tag
//! becomes `{{token}}` here and `[capture] token = '$.token'` in that request: TsubameDo
//! sends it first when the value is missing, as Insomnia does.

use std::collections::{BTreeMap, HashMap};

use base64::Engine;
use serde_json::Value;

use super::{Environment, Format, Imported, Placed, Var, array_at, body_of, join_scripts, map_builtins, map_texts, path_name, request_scripts, split_query, str_at, text};
use crate::model::{ApiKeyLocation, Auth, AwsV4, Body, ClientAuth, Grant, OAuth2, Pair, Part, Request};
use crate::{Error, Result};

enum Tree<'a> {
    Folder { v: &'a Value, kids: Vec<Tree<'a>> },
    Request(&'a Value),
    Skip(&'static str),
}

pub fn v4(doc: &Value) -> Result<Imported> {
    let resources = array_at(doc, "resources");
    let id = |v: &Value| str_at(v, "_id").to_string();
    let mut kids: HashMap<&str, Vec<&Value>> = HashMap::new();
    for r in resources {
        kids.entry(str_at(r, "parentId")).or_default().push(r);
    }
    for list in kids.values_mut() {
        list.sort_by(|a, b| sort_key(a, "metaSortKey").total_cmp(&sort_key(b, "metaSortKey")));
    }
    fn build<'a>(parent: &str, kids: &HashMap<&str, Vec<&'a Value>>) -> Vec<Tree<'a>> {
        let Some(list) = kids.get(parent) else { return Vec::new() };
        list.iter()
            .filter_map(|v| match str_at(v, "_type") {
                "request_group" => Some(Tree::Folder { v, kids: build(str_at(v, "_id"), kids) }),
                "request" => Some(Tree::Request(v)),
                "grpc_request" => Some(Tree::Skip(tr_static!("gRPC-запросы", "gRPC requests"))),
                "websocket_request" => Some(Tree::Skip(tr_static!("WebSocket-запросы", "WebSocket requests"))),
                "unit_test_suite" => Some(Tree::Skip(tr_static!("наборы тестов Insomnia", "Insomnia test suites"))),
                _ => None,
            })
            .collect()
    }

    let workspaces: Vec<&Value> = resources.iter().filter(|r| str_at(r, "_type") == "workspace").collect();
    let mut c = Converter::default();
    let mut environments: Vec<Environment> = Vec::new();
    let mut vars = Vec::new();
    let trees: Vec<(String, Vec<Tree>)> = if workspaces.is_empty() {
        // A partial export: its top is whatever has no parent among the resources.
        let ids: Vec<String> = resources.iter().map(id).collect();
        let tops: Vec<&str> = kids.keys().copied().filter(|p| !ids.iter().any(|i| i == p)).collect();
        vec![(String::new(), tops.iter().flat_map(|p| build(p, &kids)).collect())]
    } else {
        workspaces.iter().map(|w| (str_at(w, "name").to_string(), build(str_at(w, "_id"), &kids))).collect()
    };

    // The base environment hangs off the workspace, the others off the base.
    for w in &workspaces {
        for base in kids.get(str_at(w, "_id")).into_iter().flatten().filter(|r| str_at(r, "_type") == "environment") {
            vars.extend(c.env_vars(base));
            for sub in kids.get(str_at(base, "_id")).into_iter().flatten().filter(|r| str_at(r, "_type") == "environment") {
                environments.push(Environment { name: str_at(sub, "name").to_string(), vars: c.env_vars(sub) });
            }
        }
    }

    let several = trees.len() > 1;
    for (name, tree) in &trees {
        let folder = if several { vec![name.clone()] } else { Vec::new() };
        c.walk(tree, &folder, &Inherited::default());
    }
    let name = if several || trees.is_empty() { "Insomnia".to_string() } else { trees[0].0.clone() };
    Ok(c.finish(name, vars, environments))
}

pub fn v5(doc: &Value) -> Result<Imported> {
    let kind = str_at(doc, "type");
    fn build(list: &[Value]) -> Vec<Tree<'_>> {
        let mut sorted: Vec<&Value> = list.iter().collect();
        if sorted.iter().all(|v| v.pointer("/meta/sortKey").is_some()) {
            sorted.sort_by(|a, b| sort_key(&a["meta"], "sortKey").total_cmp(&sort_key(&b["meta"], "sortKey")));
        }
        sorted
            .into_iter()
            .map(|v| {
                let id = v.pointer("/meta/id").and_then(Value::as_str).unwrap_or_default();
                if let Some(kids) = v.get("children").and_then(Value::as_array) {
                    Tree::Folder { v, kids: build(kids) }
                } else if id.starts_with("greq_") || v.get("protoMethodName").is_some() {
                    Tree::Skip(tr_static!("gRPC-запросы", "gRPC requests"))
                } else if id.starts_with("ws-req_") {
                    Tree::Skip(tr_static!("WebSocket-запросы", "WebSocket requests"))
                } else {
                    Tree::Request(v)
                }
            })
            .collect()
    }
    if !kind.starts_with("collection.") && !kind.starts_with("environment.") {
        return Err(Error::Invalid(tr!(
            "Insomnia {kind}: импортируются коллекции и окружения (для документа дизайна есть импорт OpenAPI)",
            "Insomnia {kind}: collections and environments are imported (for a design document, import OpenAPI)"
        )));
    }
    let mut c = Converter::default();
    let tree = build(array_at(doc, "collection"));
    let mut vars = Vec::new();
    let mut environments = Vec::new();
    if let Some(env) = doc.get("environments") {
        vars = c.env_vars(env);
        for sub in array_at(env, "subEnvironments") {
            environments.push(Environment { name: str_at(sub, "name").to_string(), vars: c.env_vars(sub) });
        }
    }
    c.walk(&tree, &[], &Inherited::default());
    Ok(c.finish(str_at(doc, "name").to_string(), vars, environments))
}

fn sort_key(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

/// What a folder passes down to its requests.
#[derive(Default, Clone)]
struct Inherited {
    auth: Option<Value>,
    headers: Vec<Pair>,
    before: Vec<(String, String)>,
    after: Vec<(String, String)>,
}

#[derive(Default)]
struct Converter {
    warnings: Vec<String>,
    out: Vec<(String, Placed)>,
    /// Request id → values other requests take from its response.
    captures: BTreeMap<String, Vec<Pair>>,
    /// (request id, query) → variable name, and the names taken.
    names: BTreeMap<(String, String), String>,
    skipped: BTreeMap<&'static str, usize>,
}

impl Converter {
    fn walk(&mut self, tree: &[Tree], folder: &[String], inherited: &Inherited) {
        for node in tree {
            match node {
                Tree::Skip(what) => *self.skipped.entry(what).or_default() += 1,
                Tree::Folder { v, kids } => {
                    let mut path = folder.to_vec();
                    path.push(str_at(v, "name").to_string());
                    let mut next = inherited.clone();
                    if let Some(auth) = v.get("authentication").filter(|a| !is_inherit(a)) {
                        next.auth = Some(auth.clone());
                    }
                    next.headers.extend(self.pairs(array_at(v, "headers")));
                    let (before, after) = scripts(v);
                    let from = tr!("из папки «{}»", "from folder \"{}\"", str_at(v, "name"));
                    next.before.push((from.clone(), before.to_string()));
                    next.after.push((from, after.to_string()));
                    self.walk(kids, &path, &next);
                }
                Tree::Request(v) => {
                    let request = self.request(v, inherited);
                    let id = v.get("_id").or_else(|| v.pointer("/meta/id")).and_then(Value::as_str).unwrap_or_default();
                    self.out.push((id.to_string(), Placed { folder: folder.to_vec(), request }));
                }
            }
        }
    }

    fn request(&mut self, v: &Value, inherited: &Inherited) -> Request {
        let (url, mut query) = split_query(str_at(v, "url"));
        query.extend(self.pairs(array_at(v, "parameters")));
        let mut headers: Vec<Pair> = inherited
            .headers
            .iter()
            .filter(|h| !array_at(v, "headers").iter().any(|o| str_at(o, "name").eq_ignore_ascii_case(&h.name)))
            .cloned()
            .collect();
        headers.extend(self.pairs(array_at(v, "headers")));
        let own_auth = v.get("authentication").filter(|a| !is_inherit(a));
        let auth = match own_auth.or(inherited.auth.as_ref()) {
            Some(a) => self.auth(a, &mut headers),
            None => Auth::None,
        };
        let name = str_at(v, "name").trim().to_string();
        let mut request = Request {
            name: Some(if name.is_empty() { path_name(&url) } else { name }),
            method: str_at(v, "method").to_uppercase(),
            url,
            query,
            headers,
            auth,
            body: self.body(v.get("body").unwrap_or(&Value::Null)),
            ..Request::default()
        };
        if request.method.is_empty() {
            request.method = "GET".into();
        }
        let follow = v.get("settingFollowRedirects").or_else(|| v.pointer("/settings/followRedirects")).and_then(Value::as_str);
        request.options.follow_redirects = match follow {
            Some("on") => Some(true),
            Some("off") => Some(false),
            _ => None,
        };
        let (before, after) = scripts(v);
        let mut befores: Vec<(String, &str)> = inherited.before.iter().map(|(f, s)| (f.clone(), s.as_str())).collect();
        befores.push((request_scripts().into(), before));
        let mut afters: Vec<(String, &str)> = inherited.after.iter().map(|(f, s)| (f.clone(), s.as_str())).collect();
        afters.push((request_scripts().into(), after));
        request.scripts.before = join_scripts(&befores);
        request.scripts.after = join_scripts(&afters);

        let mut warnings = Vec::new();
        map_texts(&mut request, |t| {
            let t = self.template(t, &mut warnings);
            map_builtins(&t, &mut warnings)
        });
        self.warnings.extend(warnings);
        request
    }

    fn pairs(&mut self, list: &[Value]) -> Vec<Pair> {
        list.iter()
            .filter(|p| !str_at(p, "name").is_empty() || !text(&p["value"]).is_empty())
            .map(|p| Pair { name: str_at(p, "name").to_string(), value: text(&p["value"]), enabled: !disabled(p) })
            .collect()
    }

    fn auth(&mut self, a: &Value, headers: &mut Vec<Pair>) -> Auth {
        if disabled(a) {
            return Auth::None;
        }
        match str_at(a, "type") {
            "" | "none" => Auth::None,
            "bearer" => {
                let token = str_at(a, "token").to_string();
                match str_at(a, "prefix").trim() {
                    "" | "Bearer" => Auth::Bearer { token },
                    prefix => {
                        headers.push(Pair::new("Authorization", format!("{prefix} {token}")));
                        Auth::None
                    }
                }
            }
            "basic" => Auth::Basic { username: str_at(a, "username").into(), password: str_at(a, "password").into() },
            "digest" => Auth::Digest { username: str_at(a, "username").into(), password: str_at(a, "password").into() },
            "iam" => Auth::AwsV4(AwsV4 {
                access_key: str_at(a, "accessKeyId").into(),
                secret_key: str_at(a, "secretAccessKey").into(),
                session_token: str_at(a, "sessionToken").into(),
                region: str_at(a, "region").into(),
                service: str_at(a, "service").into(),
            }),
            "apikey" => Auth::ApiKey {
                key: str_at(a, "key").into(),
                value: str_at(a, "value").into(),
                location: if str_at(a, "addTo") == "queryParams" { ApiKeyLocation::Query } else { ApiKeyLocation::Header },
            },
            "oauth2" => {
                let grant = match str_at(a, "grantType") {
                    "client_credentials" => Grant::ClientCredentials,
                    "password" => Grant::Password,
                    "implicit" => {
                        self.warnings.push(tr!(
                            "OAuth 2.0 implicit из Insomnia перенесён как authorization code с PKCE: implicit устарел и не поддерживается",
                            "OAuth 2.0 implicit from Insomnia became authorization code with PKCE: implicit is obsolete and not supported"
                        ));
                        Grant::AuthorizationCode
                    }
                    _ => Grant::AuthorizationCode,
                };
                let redirect = str_at(a, "redirectUrl");
                let loopback = redirect.starts_with("http://127.0.0.1") || redirect.starts_with("http://localhost");
                Auth::OAuth2(OAuth2 {
                    grant,
                    auth_url: str_at(a, "authorizationUrl").into(),
                    token_url: str_at(a, "accessTokenUrl").into(),
                    client_id: str_at(a, "clientId").into(),
                    client_secret: str_at(a, "clientSecret").into(),
                    scope: str_at(a, "scope").into(),
                    audience: str_at(a, "audience").into(),
                    redirect_uri: if loopback { redirect.into() } else { String::new() },
                    pkce: a.get("usePkce").and_then(Value::as_bool).unwrap_or(grant == Grant::AuthorizationCode),
                    client_auth: if a.get("credentialsInBody").and_then(Value::as_bool).unwrap_or(false) { ClientAuth::Body } else { ClientAuth::Basic },
                    username: str_at(a, "username").into(),
                    password: str_at(a, "password").into(),
                })
            }
            other => {
                self.warnings.push(tr!(
                    "авторизация {other} из Insomnia пока не поддерживается — у таких запросов её нет",
                    "{other} authorization from Insomnia is not supported yet — such requests have none"
                ));
                Auth::None
            }
        }
    }

    fn body(&mut self, b: &Value) -> Body {
        let mime = str_at(b, "mimeType");
        let params = array_at(b, "params");
        match mime {
            "application/x-www-form-urlencoded" => Body::Form { fields: self.pairs(params) },
            "multipart/form-data" => Body::Multipart {
                parts: params
                    .iter()
                    .map(|p| {
                        let is_file = str_at(p, "type") == "file";
                        let value = if is_file { str_at(p, "fileName").to_string() } else { text(&p["value"]) };
                        Part { name: str_at(p, "name").into(), value, is_file, enabled: !disabled(p) }
                    })
                    .collect(),
            },
            "application/octet-stream" if !str_at(b, "fileName").is_empty() => {
                Body::File { path: str_at(b, "fileName").into(), content_type: None }
            }
            // GraphQL goes over HTTP as JSON with `query` and `variables`, which is how it is kept.
            "application/graphql" => body_of("application/json", str_at(b, "text").into()),
            _ => body_of(mime, str_at(b, "text").into()),
        }
    }

    /// The variables of an environment; nested objects become `parent.child`.
    fn env_vars(&mut self, env: &Value) -> Vec<Var> {
        fn flatten(prefix: &str, v: &Value, out: &mut Vec<(String, String)>) {
            match v {
                Value::Object(map) => {
                    for (k, v) in map {
                        let name = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                        flatten(&name, v, out);
                    }
                }
                other => out.push((prefix.to_string(), text(other))),
            }
        }
        let mut list = Vec::new();
        flatten("", env.get("data").unwrap_or(&Value::Null), &mut list);
        let private = env.get("isPrivate").and_then(Value::as_bool).unwrap_or(false);
        let mut warnings = Vec::new();
        let vars = list
            .into_iter()
            .filter(|(k, _)| !k.is_empty())
            .map(|(k, v)| {
                let value = map_builtins(&self.template(&v, &mut warnings), &mut warnings);
                Var::new(k, value, private)
            })
            .collect();
        self.warnings.extend(warnings);
        vars
    }

    /// `{{ _.name }}` → `{{name}}`, tags → ours where there is one.
    fn template(&mut self, input: &str, warnings: &mut Vec<String>) -> String {
        let mut out = String::with_capacity(input.len());
        let mut rest = input;
        loop {
            let var = rest.find("{{");
            let tag = rest.find("{%");
            let (open, is_tag) = match (var, tag) {
                (Some(v), Some(t)) => (v.min(t), t < v),
                (Some(v), None) => (v, false),
                (None, Some(t)) => (t, true),
                (None, None) => break,
            };
            out.push_str(&rest[..open]);
            let close = if is_tag { "%}" } else { "}}" };
            let Some(end) = rest[open + 2..].find(close) else {
                out.push_str(&rest[open..]);
                return out;
            };
            let inner = rest[open + 2..open + 2 + end].trim();
            let whole = &rest[open..open + 2 + end + 2];
            if is_tag {
                out.push_str(&self.tag(inner, whole, warnings));
            } else {
                let name = inner.strip_prefix("_.").unwrap_or(inner);
                let name = name
                    .strip_prefix("_['")
                    .or_else(|| name.strip_prefix("_[\""))
                    .and_then(|n| n.strip_suffix("']").or_else(|| n.strip_suffix("\"]")))
                    .unwrap_or(name);
                if name.contains('|') || name.contains('(') {
                    warnings.push(tr!("выражение {whole} из Insomnia оставлено как есть", "the Insomnia expression {whole} was left as is"));
                    out.push_str(whole);
                } else {
                    out.push_str(&format!("{{{{{}}}}}", name.trim()));
                }
            }
            rest = &rest[open + 2 + end + 2..];
        }
        out.push_str(rest);
        out
    }

    fn tag(&mut self, inner: &str, whole: &str, warnings: &mut Vec<String>) -> String {
        let (name, args) = inner.split_once(char::is_whitespace).unwrap_or((inner, ""));
        let args = tag_args(args);
        let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or_default();
        match name {
            "uuid" => "{{$uuid}}".into(),
            // The faker plugin of Insomnia takes Postman's names.
            "faker" if crate::fake::value(arg(0)).is_some() => format!("{{{{${}}}}}", arg(0)),
            "now" | "timestamp" => match arg(0) {
                "millis" | "" if name == "now" => "{{$timestampMs}}".into(),
                "unix" => "{{$timestamp}}".into(),
                "iso-8601" => "{{$isoTimestamp}}".into(),
                _ if name == "timestamp" => "{{$timestampMs}}".into(),
                other => {
                    warnings.push(tr!("тег {{% now '{other}' %}} заменён на время в миллисекундах", "the tag {{% now '{other}' %}} became the time in milliseconds"));
                    "{{$timestampMs}}".into()
                }
            },
            "response" => {
                let path = decode_b64(arg(2));
                let query = match arg(0) {
                    "body" if path.starts_with('$') => path.clone(),
                    "body" if path.is_empty() => "body".into(),
                    "header" => format!("header {path}"),
                    "raw" => "body".into(),
                    other => {
                        warnings.push(tr!(
                            "тег response «{other}» с путём {path} не перенесён: поддерживаются JSONPath, заголовок и тело",
                            "the response tag \"{other}\" with path {path} was not carried over: JSONPath, a header and the body are supported"
                        ));
                        return whole.to_string();
                    }
                };
                let var = self.capture(arg(1), &query, &path);
                format!("{{{{{var}}}}}")
            }
            other => {
                warnings.push(tr!("тег Insomnia {{% {other} %}} оставлен как есть: у нас его нет", "the Insomnia tag {{% {other} %}} was left as is: there is no such tag here"));
                whole.to_string()
            }
        }
    }

    /// The variable a request's response gives for `query`, with a name from the query.
    fn capture(&mut self, request: &str, query: &str, path: &str) -> String {
        let key = (request.to_string(), query.to_string());
        if let Some(name) = self.names.get(&key) {
            return name.clone();
        }
        let base = if let Some(header) = query.strip_prefix("header ") {
            header.to_lowercase().replace('-', "_")
        } else {
            let word = regex::Regex::new(r"[A-Za-z_][A-Za-z0-9_]*").ok().and_then(|re| re.find_iter(path).last().map(|m| m.as_str().to_string()));
            word.unwrap_or_else(|| "value".into())
        };
        let mut name = base.clone();
        let mut n = 2;
        while self.names.values().any(|v| *v == name) {
            name = format!("{base}_{n}");
            n += 1;
        }
        self.names.insert(key, name.clone());
        self.captures.entry(request.to_string()).or_default().push(Pair::new(&name, query));
        name
    }

    fn finish(mut self, name: String, vars: Vec<Var>, environments: Vec<Environment>) -> Imported {
        for (what, n) in &self.skipped {
            self.warnings.push(tr!("{what} ({n}) пропущены: в TsubameDo их пока нет", "{what} ({n}) skipped: TsubameDo does not have them yet"));
        }
        let ids: Vec<&String> = self.out.iter().map(|(id, _)| id).collect();
        for (id, captures) in &self.captures {
            if !ids.contains(&id) {
                let names: Vec<&str> = captures.iter().map(|p| p.name.as_str()).collect();
                self.warnings.push(tr!(
                    "значения {} берутся из запроса {id}, которого нет в файле",
                    "values {} come from request {id}, which is not in the file",
                    names.join(", ")
                ));
            }
        }
        let requests = self
            .out
            .into_iter()
            .map(|(id, mut placed)| {
                if let Some(c) = self.captures.get(&id) {
                    placed.request.captures = c.clone();
                }
                placed
            })
            .collect();
        Imported { format: Format::Insomnia, name, requests, vars, environments, warnings: self.warnings, files: Vec::new() }
    }
}

fn scripts(v: &Value) -> (&str, &str) {
    let before = v.get("preRequestScript").or_else(|| v.pointer("/scripts/preRequest")).and_then(Value::as_str);
    let after = v.get("afterResponseScript").or_else(|| v.pointer("/scripts/afterResponse")).and_then(Value::as_str);
    (before.unwrap_or_default(), after.unwrap_or_default())
}

fn disabled(v: &Value) -> bool {
    v.get("disabled").and_then(Value::as_bool).unwrap_or(false)
}

/// An empty authentication (`{}`) or an explicit inherit takes the folder's.
fn is_inherit(a: &Value) -> bool {
    a.as_object().is_none_or(|o| o.is_empty()) || str_at(a, "type") == "inherit"
}

/// `'body', 'req_1', 'b64::…::46b', 'never', 60` → the values without quotes.
fn tag_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match (quote, c) {
            (None, '\'' | '"') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, ',') => out.push(std::mem::take(&mut current).trim().to_string()),
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out
}

/// Insomnia keeps a tag's path as `b64::<base64>::46b`.
fn decode_b64(s: &str) -> String {
    s.strip_prefix("b64::")
        .and_then(|rest| rest.strip_suffix("::46b"))
        .and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_else(|| s.to_string())
}
