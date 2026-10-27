//! HAR, as browsers save it from the developer tools: the page's API calls become requests,
//! what the browser loads for the page itself (pictures, styles, scripts, fonts) is left out,
//! and repeats of the same call are kept once. A folder per host when there are several.

use std::collections::HashSet;

use serde_json::Value;

use super::{Format, Imported, Placed, array_at, body_of, path_name, split_query, str_at, text};
use crate::model::{Body, Pair, Part, Request};
use crate::Result;

/// Resource types of Chrome's HAR that are the page's API.
const API: [&str; 2] = ["xhr", "fetch"];
const ASSETS: [&str; 16] =
    [".js", ".mjs", ".css", ".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp", ".avif", ".ico", ".woff", ".woff2", ".ttf", ".map", ".mp4"];

pub fn har(doc: &Value) -> Result<Imported> {
    let entries = doc.pointer("/log/entries").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let typed = entries.iter().any(|e| e.get("_resourceType").is_some());
    let mut warnings = Vec::new();
    let mut seen = HashSet::new();
    let mut skipped = 0;
    let mut kept: Vec<(String, Request)> = Vec::new();
    for e in entries {
        let r = &e["request"];
        let method = str_at(r, "method").to_uppercase();
        let full_url = str_at(r, "url");
        let parsed = url::Url::parse(full_url).ok();
        let Some(parsed) = parsed.filter(|u| u.scheme() == "http" || u.scheme() == "https") else {
            skipped += 1;
            continue;
        };
        if method == "OPTIONS" || !is_api(e, &parsed, &method, typed) {
            skipped += 1;
            continue;
        }
        let post = r.get("postData").unwrap_or(&Value::Null);
        if !seen.insert((method.clone(), full_url.to_string(), str_at(post, "text").to_string())) {
            continue;
        }

        let (url, _) = split_query(full_url);
        // The HAR's list is decoded already; the address's own query is the fallback.
        let query = match r.get("queryString").and_then(Value::as_array) {
            Some(q) if !q.is_empty() => q.iter().map(|p| Pair::new(str_at(p, "name"), text(&p["value"]))).collect(),
            _ => split_query(full_url).1,
        };
        let headers: Vec<Pair> = array_at(r, "headers")
            .iter()
            .map(|h| Pair::new(str_at(h, "name"), text(&h["value"])))
            .filter(|h| keep_header(&h.name))
            .collect();
        if headers.iter().any(|h| h.name.eq_ignore_ascii_case("cookie") || h.name.eq_ignore_ascii_case("authorization")) {
            warnings.push(
                "в запросах остались заголовки Cookie и Authorization из браузера — это ваши действующие данные входа, не отправляйте их в git".into(),
            );
        }
        let body = body(post, &mut warnings);
        let host = parsed.host_str().unwrap_or_default().to_string();
        kept.push((host, Request { name: Some(path_name(&url)), method, url, query, headers, body, ..Request::default() }));
    }
    if skipped > 0 {
        warnings.push(format!("пропущено записей: {skipped} — файлы страницы (картинки, стили, скрипты, шрифты) и проверки CORS"));
    }
    let hosts: HashSet<&str> = kept.iter().map(|(h, _)| h.as_str()).collect();
    let several = hosts.len() > 1;
    let name = if hosts.len() == 1 { hosts.iter().next().map(|h| h.to_string()).unwrap_or_default() } else { String::new() };
    let requests = kept.into_iter().map(|(host, request)| Placed { folder: if several { vec![host] } else { Vec::new() }, request }).collect();
    Ok(Imported { format: Format::Har, name, requests, vars: Vec::new(), environments: Vec::new(), warnings })
}

fn is_api(e: &Value, url: &url::Url, method: &str, typed: bool) -> bool {
    if typed {
        let kind = str_at(e, "_resourceType");
        return API.contains(&kind) || (kind == "document" && method != "GET");
    }
    let mime = e.pointer("/response/content/mimeType").and_then(Value::as_str).unwrap_or_default().to_lowercase();
    let asset_type = ["image/", "font/", "video/", "audio/", "text/css", "javascript"].iter().any(|t| mime.contains(t));
    let path = url.path().to_lowercase();
    let asset_path = ASSETS.iter().any(|ext| path.ends_with(ext));
    !(asset_type || asset_path || (method == "GET" && mime.starts_with("text/html")))
}

/// Headers the browser adds for itself: the client sets its own or they mean nothing here.
fn keep_header(name: &str) -> bool {
    let n = name.to_lowercase();
    !(n.starts_with(':')
        || n.starts_with("sec-")
        || ["content-length", "host", "connection", "accept-encoding", "priority", "upgrade-insecure-requests"].contains(&n.as_str()))
}

fn body(post: &Value, warnings: &mut Vec<String>) -> Body {
    let mime = str_at(post, "mimeType");
    let base = mime.split(';').next().unwrap_or_default().trim().to_lowercase();
    let params = array_at(post, "params");
    match base.as_str() {
        "application/x-www-form-urlencoded" => {
            let fields = if params.is_empty() {
                split_query(&format!("?{}", str_at(post, "text"))).1
            } else {
                params.iter().map(|p| Pair::new(str_at(p, "name"), text(&p["value"]))).collect()
            };
            Body::Form { fields }
        }
        "multipart/form-data" if !params.is_empty() => {
            let parts: Vec<Part> = params
                .iter()
                .map(|p| {
                    let file = str_at(p, "fileName");
                    Part { name: str_at(p, "name").into(), value: if file.is_empty() { text(&p["value"]) } else { file.into() }, is_file: !file.is_empty(), enabled: true }
                })
                .collect();
            if parts.iter().any(|p| p.is_file) {
                warnings.push("в HAR нет содержимого отправленных файлов: у полей-файлов указано только имя, выберите файл заново".into());
            }
            Body::Multipart { parts }
        }
        _ => body_of(if base.is_empty() { "" } else { mime }, str_at(post, "text").into()),
    }
}
