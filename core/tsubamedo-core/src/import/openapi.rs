//! OpenAPI 3 and Swagger 2: a request per operation, in a folder per tag.
//!
//! The server address becomes `{{base_url}}`; with several servers each is an environment.
//! Path parameters take their example or become `{{name}}`; optional query parameters are
//! there but switched off. A body is the example from the description or one built from its
//! schema. Each request checks the success status the description promises and, when the
//! answer is JSON with a schema, `$ schema schemas/…json`: the schemas become JSON Schema files
//! (2020-12, `nullable` and the like translated), one per component, referring to each other.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use super::{Environment, Format, Imported, Placed, Var, array_at, file_name, str_at, text};
use crate::model::{ApiKeyLocation, Auth, Body, Check, ClientAuth, Grant, OAuth2, Pair, Part, Request};
use crate::organize::clean_name;
use crate::Result;

const METHODS: [&str; 8] = ["get", "post", "put", "patch", "delete", "head", "options", "trace"];
/// Nesting of a built example body; deeper schemas are cut.
const DEPTH: usize = 8;

struct Spec<'a> {
    doc: &'a Value,
    v2: bool,
    warnings: Vec<String>,
    /// Component name → its schema file in `schemas/`, for those the checks use.
    components: BTreeMap<String, String>,
    files: Vec<(String, String)>,
}

const SCHEMAS: &str = "schemas";
const DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

pub fn v3(doc: &Value) -> Result<Imported> {
    let mut spec = Spec { doc, v2: false, warnings: Vec::new(), components: BTreeMap::new(), files: Vec::new() };
    let servers: Vec<(String, String)> = array_at(doc, "servers")
        .iter()
        .map(|s| {
            let mut url = str_at(s, "url").to_string();
            if let Some(vars) = s.get("variables").and_then(Value::as_object) {
                for (name, v) in vars {
                    url = url.replace(&format!("{{{name}}}"), &text(&v["default"]));
                }
            }
            (url.trim_end_matches('/').to_string(), str_at(s, "description").to_string())
        })
        .collect();
    Ok(spec.build(servers, Format::OpenApi))
}

pub fn v2(doc: &Value) -> Result<Imported> {
    let mut spec = Spec { doc, v2: true, warnings: Vec::new(), components: BTreeMap::new(), files: Vec::new() };
    let host = str_at(doc, "host");
    let base_path = str_at(doc, "basePath").trim_end_matches('/');
    let schemes: Vec<&str> = array_at(doc, "schemes").iter().filter_map(Value::as_str).collect();
    let servers = if host.is_empty() {
        vec![(base_path.to_string(), String::new())]
    } else {
        let scheme = if schemes.is_empty() || schemes.contains(&"https") { "https" } else { schemes[0] };
        vec![(format!("{scheme}://{host}{base_path}"), String::new())]
    };
    Ok(spec.build(servers, Format::Swagger))
}

impl Spec<'_> {
    fn build(&mut self, servers: Vec<(String, String)>, format: Format) -> Imported {
        let mut vars = Vec::new();
        let mut environments = Vec::new();
        match servers.first() {
            None => {
                vars.push(Var::new("base_url", "http://localhost", false));
                self.warnings.push("в описании нет адреса сервера — в base_url записан http://localhost".into());
            }
            Some((url, _)) => {
                if !url.contains("://") {
                    self.warnings.push(format!("адрес сервера «{url}» без хоста — допишите его в base_url"));
                }
                vars.push(Var::new("base_url", url.clone(), false));
            }
        }
        if servers.len() > 1 {
            for (i, (url, description)) in servers.iter().enumerate() {
                let host = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string));
                let name = [description.as_str(), host.as_deref().unwrap_or_default()]
                    .into_iter()
                    .find_map(|n| clean_name(n).ok())
                    .unwrap_or_else(|| format!("server {}", i + 1));
                let name = if environments.iter().any(|e: &Environment| e.name == name) { format!("{name} {}", i + 1) } else { name };
                environments.push(Environment { name, vars: vec![Var::new("base_url", url.clone(), false)] });
            }
        }

        let mut requests = Vec::new();
        let paths = self.doc.get("paths").and_then(Value::as_object).cloned().unwrap_or_default();
        for (path, item) in &paths {
            let item = self.resolve(item);
            let shared = array_at(&item, "parameters").to_vec();
            for method in METHODS {
                let Some(op) = item.get(method) else { continue };
                let request = self.operation(path, method, op, &shared);
                let folder = array_at(op, "tags").first().and_then(Value::as_str).map(|t| vec![t.to_string()]).unwrap_or_default();
                requests.push(Placed { folder, request });
            }
        }
        let title = self.doc.pointer("/info/title").and_then(Value::as_str).unwrap_or_default().to_string();
        self.component_files();
        Imported { format, name: title, requests, vars, environments, warnings: std::mem::take(&mut self.warnings), files: std::mem::take(&mut self.files) }
    }

    fn operation(&mut self, path: &str, method: &str, op: &Value, shared: &[Value]) -> Request {
        // Operation parameters override the path's by name and place.
        let mut params: Vec<Value> = shared.iter().map(|p| self.resolve(p)).collect();
        for p in array_at(op, "parameters") {
            let p = self.resolve(p);
            params.retain(|q| !(str_at(q, "name") == str_at(&p, "name") && str_at(q, "in") == str_at(&p, "in")));
            params.push(p);
        }

        let mut url = format!("{{{{base_url}}}}{path}");
        let mut query = Vec::new();
        let mut headers = Vec::new();
        let mut form = Vec::new();
        let mut body = Body::None;
        for p in &params {
            let name = str_at(p, "name");
            let required = p.get("required").and_then(Value::as_bool).unwrap_or(false);
            let example = self.param_example(p);
            match str_at(p, "in") {
                "path" => {
                    let value = example.unwrap_or_else(|| format!("{{{{{name}}}}}"));
                    url = url.replace(&format!("{{{name}}}"), &value);
                }
                "query" => query.push(Pair { name: name.into(), value: example.unwrap_or_default(), enabled: required }),
                "header" if !["accept", "content-type", "authorization"].contains(&name.to_lowercase().as_str()) => {
                    headers.push(Pair { name: name.into(), value: example.unwrap_or_default(), enabled: required })
                }
                // Swagger 2 keeps the body and form fields among parameters.
                "body" => {
                    let schema = p.get("schema").cloned().unwrap_or(Value::Null);
                    let value = self.sample(&schema, 0, &mut Vec::new());
                    body = Body::Json { content: serde_json::to_string_pretty(&value).unwrap_or_default() };
                }
                "formData" => form.push((name.to_string(), example.unwrap_or_default(), str_at(p, "type") == "file", required)),
                _ => {}
            }
        }
        if !form.is_empty() {
            let consumes = array_at(op, "consumes").iter().chain(array_at(self.doc, "consumes")).filter_map(Value::as_str).next().unwrap_or_default();
            body = if consumes == "multipart/form-data" || form.iter().any(|f| f.2) {
                Body::Multipart { parts: form.into_iter().map(|(name, value, is_file, required)| Part { name, value, is_file, enabled: required || !is_file }).collect() }
            } else {
                Body::Form { fields: form.into_iter().map(|(name, value, _, _)| Pair::new(name, value)).collect() }
            };
        }
        if let Some(rb) = op.get("requestBody") {
            let rb = self.resolve(rb);
            body = self.request_body(&rb);
        }

        let auth = self.auth(op, &mut headers);
        let name = [str_at(op, "summary"), str_at(op, "operationId")]
            .into_iter()
            .map(|s| s.trim())
            .find(|s| !s.is_empty())
            .map(|s| s.lines().next().unwrap_or_default().to_string())
            .unwrap_or_else(|| path.to_string());
        let success = op
            .get("responses")
            .and_then(Value::as_object)
            .and_then(|r| r.iter().find(|(k, _)| k.starts_with('2') && k.len() == 3 && k.parse::<u16>().is_ok()).map(|(k, v)| (k.clone(), v.clone())));
        let mut checks = Vec::new();
        if let Some((code, response)) = success {
            checks.push(Check::new(format!("status == {code}")));
            if let Some(file) = self.response_schema(&response, &format!("{} {path}", method.to_uppercase())) {
                checks.push(Check::new(format!("$ schema {SCHEMAS}/{file}")));
            }
        }
        Request { name: Some(name), method: method.to_uppercase(), url, query, headers, auth, body, checks, ..Request::default() }
    }

    fn request_body(&mut self, rb: &Value) -> Body {
        let Some(content) = rb.get("content").and_then(Value::as_object) else { return Body::None };
        let pick = |want: &dyn Fn(&str) -> bool| content.iter().find(|(m, _)| want(&m.to_lowercase()));
        let chosen = pick(&|m| m == "application/json" || m.ends_with("+json"))
            .or_else(|| pick(&|m| m.contains("json")))
            .or_else(|| pick(&|m| m == "application/x-www-form-urlencoded"))
            .or_else(|| pick(&|m| m == "multipart/form-data"))
            .or_else(|| content.iter().next());
        let Some((mime, media)) = chosen else { return Body::None };
        let schema = media.get("schema").cloned().unwrap_or(Value::Null);
        let example = media
            .get("example")
            .cloned()
            .or_else(|| media.get("examples").and_then(Value::as_object).and_then(|e| e.values().next()).map(|e| self.resolve(e)).and_then(|e| e.get("value").cloned()))
            .unwrap_or_else(|| self.sample(&schema, 0, &mut Vec::new()));
        let fields = |example: &Value| -> Vec<(String, String)> {
            example.as_object().map(|o| o.iter().map(|(k, v)| (k.clone(), text(v))).collect()).unwrap_or_default()
        };
        match mime.to_lowercase().as_str() {
            "application/x-www-form-urlencoded" => Body::Form { fields: fields(&example).into_iter().map(|(n, v)| Pair::new(n, v)).collect() },
            "multipart/form-data" => {
                let props = self.resolve(&schema).get("properties").cloned().unwrap_or(Value::Null);
                Body::Multipart {
                    parts: fields(&example)
                        .into_iter()
                        .map(|(name, value)| {
                            let prop = self.resolve(&props[&name]);
                            let is_file = matches!(str_at(&prop, "format"), "binary" | "base64");
                            Part { value: if is_file { String::new() } else { value }, name, is_file, enabled: true }
                        })
                        .collect(),
                }
            }
            m if m.contains("json") => Body::Json { content: serde_json::to_string_pretty(&example).unwrap_or_default() },
            m if m.starts_with("application/octet-stream") || m.starts_with("image/") => Body::File { path: String::new(), content_type: Some(mime.clone()) },
            _ => Body::Text { content: example.as_str().map(str::to_string).unwrap_or_else(|| text(&example)), content_type: Some(mime.clone()) },
        }
    }

    /// The operation's security, or the document's: the first scheme of the first requirement.
    fn auth(&mut self, op: &Value, headers: &mut Vec<Pair>) -> Auth {
        let security = op.get("security").or_else(|| self.doc.get("security"));
        let Some((name, scopes)) = security
            .and_then(Value::as_array)
            .and_then(|list| list.first())
            .and_then(Value::as_object)
            .and_then(|req| req.iter().next())
        else {
            return Auth::None;
        };
        let scope = scopes.as_array().map(|s| s.iter().map(text).collect::<Vec<_>>().join(" ")).unwrap_or_default();
        let schemes = if self.v2 { self.doc.get("securityDefinitions") } else { self.doc.pointer("/components/securitySchemes") };
        let Some(scheme) = schemes.and_then(|s| s.get(name)).map(|s| self.resolve(s)) else { return Auth::None };
        let var = |n: &str| format!("{{{{{n}}}}}");
        match (str_at(&scheme, "type"), str_at(&scheme, "scheme").to_lowercase().as_str()) {
            ("oauth2", _) => self.oauth2(&scheme, scope).unwrap_or_else(|| Auth::Bearer { token: var("token") }),
            ("http", "bearer") | ("openIdConnect", _) => Auth::Bearer { token: var("token") },
            ("http", "basic") | ("basic", _) => Auth::Basic { username: var("username"), password: var("password") },
            ("apiKey", _) => {
                let key = str_at(&scheme, "name").to_string();
                match str_at(&scheme, "in") {
                    "query" => Auth::ApiKey { key, value: var("api_key"), location: ApiKeyLocation::Query },
                    "cookie" => {
                        headers.push(Pair::new("Cookie", format!("{key}={}", var("api_key"))));
                        Auth::None
                    }
                    _ => Auth::ApiKey { key, value: var("api_key"), location: ApiKeyLocation::Header },
                }
            }
            (kind, scheme) => {
                self.warnings.push(format!("схема авторизации {kind} {scheme} не перенесена"));
                Auth::None
            }
        }
    }

    /// The flow a client can go through on its own: client credentials, then the sign-in,
    /// then the password. Only implicit — `None`, a bearer token to paste.
    fn oauth2(&mut self, scheme: &Value, scope: String) -> Option<Auth> {
        let var = |n: &str| format!("{{{{{n}}}}}");
        // OpenAPI 3 lists flows; Swagger 2 has one flow with its own names.
        let flows: Vec<(Grant, Value)> = if self.v2 {
            let grant = match str_at(scheme, "flow") {
                "application" => Grant::ClientCredentials,
                "accessCode" => Grant::AuthorizationCode,
                "password" => Grant::Password,
                _ => return None,
            };
            vec![(grant, scheme.clone())]
        } else {
            [("clientCredentials", Grant::ClientCredentials), ("authorizationCode", Grant::AuthorizationCode), ("password", Grant::Password)]
                .into_iter()
                .filter_map(|(name, grant)| Some((grant, scheme.pointer(&format!("/flows/{name}"))?.clone())))
                .collect()
        };
        let (grant, flow) = flows.into_iter().next()?;
        Some(Auth::OAuth2(OAuth2 {
            grant,
            auth_url: str_at(&flow, "authorizationUrl").into(),
            token_url: str_at(&flow, "tokenUrl").into(),
            client_id: var("client_id"),
            client_secret: if grant == Grant::AuthorizationCode { String::new() } else { var("client_secret") },
            scope,
            username: if grant == Grant::Password { var("username") } else { String::new() },
            password: if grant == Grant::Password { var("password") } else { String::new() },
            client_auth: ClientAuth::Basic,
            ..OAuth2::default()
        }))
    }

    fn param_example(&mut self, p: &Value) -> Option<String> {
        let schema = p.get("schema").map(|s| self.resolve(s)).unwrap_or_else(|| p.clone());
        let found = p
            .get("example")
            .or_else(|| p.get("examples").and_then(Value::as_object).and_then(|e| e.values().next()).and_then(|e| e.get("value")))
            .or_else(|| schema.get("example"))
            .or_else(|| schema.get("default"))
            .or_else(|| schema.get("enum").and_then(Value::as_array).and_then(|e| e.first()))
            .cloned()?;
        Some(text(&found))
    }

    /// A value matching the schema: its examples where given, typical values otherwise.
    fn sample(&mut self, schema: &Value, depth: usize, refs: &mut Vec<String>) -> Value {
        if let Some(r) = schema.get("$ref").and_then(Value::as_str) {
            if refs.iter().any(|x| x == r) || depth > DEPTH {
                return Value::Object(Map::new());
            }
            refs.push(r.to_string());
            let resolved = self.resolve(schema);
            let value = self.sample(&resolved, depth + 1, refs);
            refs.pop();
            return value;
        }
        for key in ["example", "default", "const"] {
            if let Some(v) = schema.get(key) {
                return v.clone();
            }
        }
        if let Some(examples) = schema.get("examples").and_then(Value::as_array).and_then(|e| e.first()) {
            return examples.clone();
        }
        if let Some(first) = schema.get("enum").and_then(Value::as_array).and_then(|e| e.first()) {
            return first.clone();
        }
        if let Some(all) = schema.get("allOf").and_then(Value::as_array) {
            let mut merged = Map::new();
            for s in all {
                if let Value::Object(o) = self.sample(s, depth + 1, refs) {
                    merged.extend(o);
                }
            }
            return Value::Object(merged);
        }
        for key in ["oneOf", "anyOf"] {
            if let Some(first) = schema.get(key).and_then(Value::as_array).and_then(|e| e.first()) {
                return self.sample(first, depth + 1, refs);
            }
        }
        let kind = match &schema["type"] {
            Value::Array(types) => types.iter().filter_map(Value::as_str).find(|t| *t != "null").unwrap_or("null").to_string(),
            Value::String(t) => t.clone(),
            _ if schema.get("properties").is_some() => "object".into(),
            _ if schema.get("items").is_some() => "array".into(),
            _ => String::new(),
        };
        match kind.as_str() {
            "object" => {
                let mut out = Map::new();
                if depth < DEPTH
                    && let Some(props) = schema.get("properties").and_then(Value::as_object)
                {
                    for (name, prop) in props {
                        if prop.get("readOnly").and_then(Value::as_bool) == Some(true) {
                            continue;
                        }
                        out.insert(name.clone(), self.sample(prop, depth + 1, refs));
                    }
                }
                Value::Object(out)
            }
            "array" if depth < DEPTH => json!([self.sample(schema.get("items").unwrap_or(&Value::Null), depth + 1, refs)]),
            "array" => json!([]),
            "integer" => json!(0),
            "number" => json!(0.0),
            "boolean" => json!(true),
            "string" => json!(match str_at(schema, "format") {
                "date-time" => "2025-01-01T12:00:00Z",
                "date" => "2025-01-01",
                "time" => "12:00:00",
                "email" => "user@example.com",
                "uuid" => "3fa85f64-5717-4562-b3fc-2c963f66afa6",
                "uri" | "url" => "https://example.com",
                "ipv4" => "192.0.2.1",
                "password" => "",
                _ => "string",
            }),
            _ => Value::Null,
        }
    }

    /// The schema file a JSON answer is checked with: the component's own when the answer is
    /// one, otherwise a file of its own named after the operation.
    fn response_schema(&mut self, response: &Value, operation: &str) -> Option<String> {
        let response = self.resolve(response);
        let schema = if self.v2 {
            response.get("schema")?.clone()
        } else {
            let content = response.get("content")?.as_object()?;
            let media = content.iter().find(|(m, _)| m.to_lowercase().contains("json")).map(|(_, v)| v)?;
            media.get("schema")?.clone()
        };
        if let Some(name) = self.component_name(&schema) {
            return Some(self.component_file(&name));
        }
        let mut converted = self.convert(&schema);
        if let Value::Object(map) = &mut converted {
            map.insert("$schema".into(), json!(DIALECT));
        }
        let file = format!("{}.json", file_name(&format!("{operation} ответ")));
        self.files.push((format!("{SCHEMAS}/{file}"), serde_json::to_string_pretty(&converted).unwrap_or_default() + "\n"));
        Some(file)
    }

    /// `X` of a `$ref` to `#/components/schemas/X` (`#/definitions/X` in Swagger 2).
    fn component_name(&self, schema: &Value) -> Option<String> {
        let r = schema.get("$ref")?.as_str()?;
        let prefix = if self.v2 { "#/definitions/" } else { "#/components/schemas/" };
        Some(r.strip_prefix(prefix)?.replace("~1", "/").replace("~0", "~"))
    }

    fn component_file(&mut self, name: &str) -> String {
        if let Some(file) = self.components.get(name) {
            return file.clone();
        }
        let mut file = format!("{}.json", file_name(name));
        let mut n = 2;
        while self.components.values().any(|f| f.eq_ignore_ascii_case(&file)) {
            file = format!("{} {n}.json", file_name(name));
            n += 1;
        }
        self.components.insert(name.to_string(), file.clone());
        file
    }

    /// Files for the components the checks use, and those they use in turn.
    fn component_files(&mut self) {
        let mut written = std::collections::BTreeSet::new();
        while let Some((name, file)) = self.components.iter().find(|(n, _)| !written.contains(*n)).map(|(n, f)| (n.clone(), f.clone())) {
            written.insert(name.clone());
            let base = if self.v2 { "/definitions" } else { "/components/schemas" };
            let pointer = format!("{base}/{}", name.replace('~', "~0").replace('/', "~1"));
            let Some(schema) = self.doc.pointer(&pointer).cloned() else { continue };
            let mut converted = self.convert(&schema);
            if let Value::Object(map) = &mut converted {
                map.insert("$schema".into(), json!(DIALECT));
                map.entry("title").or_insert(json!(name));
            }
            self.files.push((format!("{SCHEMAS}/{file}"), serde_json::to_string_pretty(&converted).unwrap_or_default() + "\n"));
        }
    }

    /// An OpenAPI schema as JSON Schema 2020-12: components become files next to each other,
    /// `nullable` becomes a `null` type, the old boolean `exclusiveMinimum` a number.
    fn convert(&mut self, schema: &Value) -> Value {
        let Value::Object(map) = schema else { return schema.clone() };
        if map.contains_key("$ref") {
            if let Some(name) = self.component_name(schema) {
                return json!({ "$ref": self.component_file(&name) });
            }
            let resolved = self.resolve(schema);
            return if resolved.get("$ref").is_some() { json!({}) } else { self.convert(&resolved) };
        }
        let mut out = Map::new();
        for (key, value) in map {
            let value = match key.as_str() {
                // Values, not schemas.
                "example" | "examples" | "default" | "enum" | "const" | "discriminator" | "xml" | "externalDocs" => value.clone(),
                "properties" | "patternProperties" | "$defs" | "definitions" | "dependentSchemas" => match value {
                    Value::Object(props) => Value::Object(props.iter().map(|(k, v)| (k.clone(), self.convert(v))).collect()),
                    other => other.clone(),
                },
                "allOf" | "anyOf" | "oneOf" | "prefixItems" => match value {
                    Value::Array(list) => Value::Array(list.iter().map(|v| self.convert(v)).collect()),
                    other => other.clone(),
                },
                _ if value.is_object() => self.convert(value),
                _ => value.clone(),
            };
            out.insert(key.clone(), value);
        }
        // Swagger 2 has a `file` type JSON Schema does not know.
        if out.get("type").and_then(Value::as_str) == Some("file") {
            out.remove("type");
        }
        let nullable = [out.remove("nullable"), out.remove("x-nullable")].into_iter().flatten().any(|v| v == json!(true));
        if nullable {
            match out.get_mut("type") {
                Some(Value::String(t)) => {
                    let t = t.clone();
                    out.insert("type".into(), json!([t, "null"]));
                }
                Some(Value::Array(types)) if !types.contains(&json!("null")) => types.push(json!("null")),
                _ => {}
            }
            if let Some(Value::Array(values)) = out.get_mut("enum")
                && !values.contains(&Value::Null)
            {
                values.push(Value::Null);
            }
        }
        for (exclusive, bound) in [("exclusiveMinimum", "minimum"), ("exclusiveMaximum", "maximum")] {
            if let Some(Value::Bool(on)) = out.get(exclusive).cloned() {
                out.remove(exclusive);
                if on && let Some(limit) = out.remove(bound) {
                    out.insert(exclusive.into(), limit);
                }
            }
        }
        Value::Object(out)
    }

    /// The object a local `$ref` points to; other values as they are.
    fn resolve(&mut self, v: &Value) -> Value {
        let mut current = v.clone();
        for _ in 0..16 {
            let Some(r) = current.get("$ref").and_then(Value::as_str) else { return current };
            let Some(pointer) = r.strip_prefix('#') else {
                self.warnings.push(if r.starts_with("http://") || r.starts_with("https://") {
                    format!("ссылки по сети не загружаются: {r}")
                } else {
                    format!("ссылка на другой файл {r} не прочитана: импортируйте само описание файлом, а не текстом")
                });
                return Value::Null;
            };
            match self.doc.pointer(pointer) {
                Some(target) => current = target.clone(),
                None => {
                    self.warnings.push(format!("ссылка {r} никуда не ведёт"));
                    return Value::Null;
                }
            }
        }
        current
    }
}
