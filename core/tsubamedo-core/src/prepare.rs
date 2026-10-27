//! From a request with `{{variables}}` to exactly what goes on the wire.

use std::fs;
use std::path::Path;

use base64::Engine;
use bytes::Bytes;
use url::Url;

use crate::model::{ApiKeyLocation, Auth, Body, Pair, Request, RequestOptions};
use crate::template::{Renderer, Vars};
use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct Prepared {
    pub method: String,
    pub url: Url,
    /// In the order the user wrote them; the transport adds its own only when missing.
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
    pub options: RequestOptions,
}

impl Prepared {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// `base_dir` resolves relative file paths of the body (the request file's folder).
pub fn prepare(request: &Request, vars: &Vars, base_dir: &Path) -> Result<Prepared> {
    let mut r = Renderer::new(vars);
    let url = r.render(&request.url);
    let mut query = render_pairs(&mut r, &request.query);
    let mut headers = render_pairs(&mut r, &request.headers);

    match &request.auth {
        Auth::None => {}
        Auth::Basic { username, password } => {
            let raw = format!("{}:{}", r.render(username), r.render(password));
            let encoded = base64::engine::general_purpose::STANDARD.encode(raw);
            push_default(&mut headers, "Authorization", format!("Basic {encoded}"));
        }
        Auth::Bearer { token } => push_default(&mut headers, "Authorization", format!("Bearer {}", r.render(token))),
        Auth::ApiKey { key, value, location } => {
            let pair = (r.render(key), r.render(value));
            match location {
                ApiKeyLocation::Header => push_default(&mut headers, &pair.0, pair.1),
                ApiKeyLocation::Query => query.push(pair),
            }
        }
    }

    // Files are read only after every variable is known, so a typo is reported before IO.
    enum Plan {
        Ready(Bytes),
        Multipart(Vec<(String, String, bool)>),
        File(std::path::PathBuf),
    }
    let plan = match &request.body {
        Body::None => Plan::Ready(Bytes::new()),
        Body::Json { content } => {
            push_default(&mut headers, "Content-Type", "application/json".into());
            Plan::Ready(Bytes::from(r.render(content)))
        }
        Body::Text { content, content_type } => {
            let content_type = content_type.as_deref().map(|c| r.render(c)).unwrap_or_else(|| "text/plain; charset=utf-8".into());
            push_default(&mut headers, "Content-Type", content_type);
            Plan::Ready(Bytes::from(r.render(content)))
        }
        Body::Form { fields } => {
            push_default(&mut headers, "Content-Type", "application/x-www-form-urlencoded".into());
            let fields = render_pairs(&mut r, fields);
            Plan::Ready(Bytes::from(url::form_urlencoded::Serializer::new(String::new()).extend_pairs(fields).finish()))
        }
        Body::Multipart { parts } => Plan::Multipart(
            parts.iter().filter(|p| p.enabled).map(|p| (r.render(&p.name), r.render(&p.value), p.is_file)).collect(),
        ),
        Body::File { path, content_type } => {
            let path = base_dir.join(r.render(path));
            let content_type = content_type.as_deref().map(|c| r.render(c)).unwrap_or_else(|| guess_type(&path).into());
            push_default(&mut headers, "Content-Type", content_type);
            Plan::File(path)
        }
    };
    let method = r.render(&request.method).trim().to_ascii_uppercase();
    r.finish()?;

    let body = match plan {
        Plan::Ready(bytes) => bytes,
        Plan::Multipart(parts) => {
            let boundary = format!("----TsubameDo{}", uuid::Uuid::new_v4().simple());
            push_default(&mut headers, "Content-Type", format!("multipart/form-data; boundary={boundary}"));
            multipart(&boundary, &parts, base_dir)?
        }
        Plan::File(path) => Bytes::from(fs::read(&path).map_err(|e| Error::io(&path, e))?),
    };

    let mut url = parse_url(&url)?;
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query);
    }
    if method.is_empty() || !method.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.!#$%&'*+^`|~".contains(&b)) {
        return Err(Error::Invalid(format!("неверный метод «{method}»")));
    }
    Ok(Prepared { method, url, headers, body, options: request.options.clone() })
}

/// Like the address bar: `example.com/x` means `http://example.com/x`.
pub fn parse_url(text: &str) -> Result<Url> {
    let text = text.trim();
    let bad = |reason: String| Error::BadUrl { url: text.to_string(), reason };
    if text.is_empty() {
        return Err(bad("адрес пустой".into()));
    }
    let with_scheme = if text.contains("://") { text.to_string() } else { format!("http://{text}") };
    let url = Url::parse(&with_scheme).map_err(|e| bad(e.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(bad("поддерживаются только http и https".into()));
    }
    if url.host().is_none() {
        return Err(bad("нет имени сервера".into()));
    }
    Ok(url)
}

fn render_pairs(r: &mut Renderer, pairs: &[Pair]) -> Vec<(String, String)> {
    pairs.iter().filter(|p| p.enabled).map(|p| (r.render(&p.name), r.render(&p.value))).collect()
}

/// Adds a header unless the user already set it: their own value always wins.
fn push_default(headers: &mut Vec<(String, String)>, name: &str, value: String) {
    if !headers.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)) {
        headers.push((name.to_string(), value));
    }
}

fn multipart(boundary: &str, parts: &[(String, String, bool)], base_dir: &Path) -> Result<Bytes> {
    let mut out = Vec::new();
    let quote = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"").replace(['\r', '\n'], " ");
    for (name, value, is_file) in parts {
        out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        if *is_file {
            let path = base_dir.join(value);
            let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            out.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n",
                    quote(name),
                    quote(&file_name),
                    guess_type(&path)
                )
                .as_bytes(),
            );
            out.extend_from_slice(&fs::read(&path).map_err(|e| Error::io(&path, e))?);
        } else {
            out.extend_from_slice(format!("Content-Disposition: form-data; name=\"{}\"\r\n\r\n", quote(name)).as_bytes());
            out.extend_from_slice(value.as_bytes());
        }
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Ok(Bytes::from(out))
}

fn guess_type(path: &Path) -> &'static str {
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "json" => "application/json",
        "xml" => "application/xml",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Part;

    fn vars() -> Vars {
        [("base", "https://api.test"), ("user", "kot"), ("pass", "мяу")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn builds_url_query_auth_and_headers() {
        let req = Request {
            method: "post".into(),
            url: "{{base}}/search?q=1".into(),
            query: vec![Pair::new("city", "Москва"), Pair { name: "off".into(), value: "x".into(), enabled: false }],
            headers: vec![Pair::new("content-type", "application/vnd.api+json")],
            auth: Auth::Basic { username: "{{user}}".into(), password: "{{pass}}".into() },
            body: Body::Json { content: "{}".into() },
            ..Request::default()
        };
        let p = prepare(&req, &vars(), Path::new(".")).unwrap();
        assert_eq!(p.method, "POST");
        assert_eq!(p.url.as_str(), "https://api.test/search?q=1&city=%D0%9C%D0%BE%D1%81%D0%BA%D0%B2%D0%B0");
        assert_eq!(p.header("Content-Type"), Some("application/vnd.api+json"));
        assert_eq!(p.header("authorization"), Some("Basic a290OtC80Y/Rgw=="));
    }

    #[test]
    fn reports_every_missing_variable() {
        let req = Request { url: "{{host}}/x".into(), headers: vec![Pair::new("X", "{{token}}")], ..Request::default() };
        let err = prepare(&req, &Vars::new(), Path::new(".")).unwrap_err();
        assert_eq!(err.to_string(), "не заданы переменные: host, token");
    }

    #[test]
    fn form_and_multipart_bodies() {
        let form = Request {
            url: "localhost:8080".into(),
            body: Body::Form { fields: vec![Pair::new("name", "Кот Матроскин"), Pair::new("a&b", "1=2")] },
            ..Request::default()
        };
        let p = prepare(&form, &vars(), Path::new(".")).unwrap();
        assert_eq!(p.url.as_str(), "http://localhost:8080/");
        assert_eq!(&p.body[..], b"name=%D0%9A%D0%BE%D1%82+%D0%9C%D0%B0%D1%82%D1%80%D0%BE%D1%81%D0%BA%D0%B8%D0%BD&a%26b=1%3D2");

        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("kot.json"), b"{\"kot\":1}").unwrap();
        let multi = Request {
            url: "{{base}}/upload".into(),
            body: Body::Multipart {
                parts: vec![
                    Part { name: "title".into(), value: "{{user}}".into(), is_file: false, enabled: true },
                    Part { name: "data".into(), value: "kot.json".into(), is_file: true, enabled: true },
                ],
            },
            ..Request::default()
        };
        let p = prepare(&multi, &vars(), dir.path()).unwrap();
        let boundary = p.header("content-type").unwrap().split("boundary=").nth(1).unwrap().to_string();
        let body = String::from_utf8(p.body.to_vec()).unwrap();
        assert_eq!(
            body,
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nkot\r\n\
                 --{boundary}\r\nContent-Disposition: form-data; name=\"data\"; filename=\"kot.json\"\r\nContent-Type: application/json\r\n\r\n{{\"kot\":1}}\r\n\
                 --{boundary}--\r\n"
            )
        );
    }

    #[test]
    fn rejects_bad_urls_and_methods() {
        assert!(parse_url("ftp://x").is_err());
        assert!(parse_url("  ").is_err());
        let req = Request { method: "GE T".into(), url: "x.test".into(), ..Request::default() };
        assert!(prepare(&req, &Vars::new(), Path::new(".")).is_err());
    }
}
