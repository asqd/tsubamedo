//! OpenAPI 3 and Swagger 2: a request per operation, in a folder per tag.
//!
//! The server address becomes `{{base_url}}`; with several servers each is an environment.
//! Path parameters take their example or become `{{name}}`; optional query parameters are
//! there but switched off. A body is the example from the description or one built from its
//! schema. Each request checks the success status the description promises.

use serde_json::{Map, Value, json};

use super::{Environment, Format, Imported, Placed, Var, array_at, str_at, text};
use crate::model::{ApiKeyLocation, Auth, Body, Check, Pair, Part, Request};
use crate::organize::clean_name;
use crate::Result;

const METHODS: [&str; 8] = ["get", "post", "put", "patch", "delete", "head", "options", "trace"];
/// Nesting of a built example body; deeper schemas are cut.
const DEPTH: usize = 8;

struct Spec<'a> {
    doc: &'a Value,
    v2: bool,
    warnings: Vec<String>,
}

pub fn v3(doc: &Value) -> Result<Imported> {
    let mut spec = Spec { doc, v2: false, warnings: Vec::new() };
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
    let mut spec = Spec { doc, v2: true, warnings: Vec::new() };
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
        Imported { format, name: title, requests, vars, environments, warnings: std::mem::take(&mut self.warnings) }
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
        let checks = op
            .get("responses")
            .and_then(Value::as_object)
            .and_then(|r| r.keys().find(|k| k.starts_with('2') && k.len() == 3 && k.parse::<u16>().is_ok()).cloned())
            .map(|code| vec![Check::new(format!("status == {code}"))])
            .unwrap_or_default();
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
        let Some(name) = security
            .and_then(Value::as_array)
            .and_then(|list| list.first())
            .and_then(Value::as_object)
            .and_then(|req| req.keys().next())
        else {
            return Auth::None;
        };
        let schemes = if self.v2 { self.doc.get("securityDefinitions") } else { self.doc.pointer("/components/securitySchemes") };
        let Some(scheme) = schemes.and_then(|s| s.get(name)).map(|s| self.resolve(s)) else { return Auth::None };
        let var = |n: &str| format!("{{{{{n}}}}}");
        match (str_at(&scheme, "type"), str_at(&scheme, "scheme").to_lowercase().as_str()) {
            ("http", "bearer") | ("oauth2", _) | ("openIdConnect", _) => Auth::Bearer { token: var("token") },
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

    /// The object a local `$ref` points to; other values as they are.
    fn resolve(&mut self, v: &Value) -> Value {
        let mut current = v.clone();
        for _ in 0..16 {
            let Some(r) = current.get("$ref").and_then(Value::as_str) else { return current };
            let Some(pointer) = r.strip_prefix('#') else {
                self.warnings.push(format!("ссылки на другие файлы не поддерживаются: {r}"));
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
