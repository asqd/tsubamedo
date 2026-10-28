//! A request as code: curl, Python, JavaScript, Go and others, as Insomnia offers them.
//!
//! The code is built from what would be sent (variables substituted, the OAuth 2.0 token the
//! caller added), with the body kept as written where a library takes it better: JSON as a
//! dict for requests, form fields as pairs, multipart files as paths. What a library cannot
//! repeat (Digest, a switched-off certificate check, not following redirects) is said in a
//! comment on top instead of being dropped quietly.

mod targets;

use serde_json::Value;
use url::Url;

use crate::model::{AwsV4, Body};
use crate::prepare::Prepared;
use crate::template::{Vars, render};

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct Language {
    pub id: &'static str,
    /// «Python», «JavaScript»: what the window groups by.
    pub group: &'static str,
    pub name: &'static str,
    #[serde(skip)]
    caps: Caps,
}

/// What a target can repeat of the request besides method, address, headers and body.
#[derive(Debug, Clone, Copy, Default)]
struct Caps {
    digest: bool,
    insecure: bool,
    no_follow: bool,
    multipart: bool,
}

const ALL: Caps = Caps { digest: true, insecure: true, no_follow: true, multipart: true };

const fn caps(digest: bool, insecure: bool, no_follow: bool, multipart: bool) -> Caps {
    Caps { digest, insecure, no_follow, multipart }
}

pub const LANGUAGES: &[Language] = &[
    Language { id: "curl", group: "Shell", name: "curl", caps: ALL },
    Language { id: "httpie", group: "Shell", name: "HTTPie", caps: caps(true, true, true, true) },
    Language { id: "wget", group: "Shell", name: "wget", caps: caps(true, true, true, false) },
    Language { id: "powershell", group: "PowerShell", name: "Invoke-WebRequest", caps: caps(false, true, true, true) },
    Language { id: "python-requests", group: "Python", name: "requests", caps: ALL },
    Language { id: "python-http-client", group: "Python", name: "http.client", caps: caps(false, true, true, false) },
    Language { id: "js-fetch", group: "JavaScript", name: "fetch", caps: caps(false, false, true, true) },
    Language { id: "js-axios", group: "JavaScript", name: "axios", caps: caps(false, false, false, true) },
    Language { id: "js-xhr", group: "JavaScript", name: "XMLHttpRequest", caps: caps(false, false, false, true) },
    Language { id: "node-fetch", group: "Node.js", name: "fetch", caps: caps(false, false, true, true) },
    Language { id: "node-axios", group: "Node.js", name: "axios", caps: caps(false, true, true, true) },
    Language { id: "node-http", group: "Node.js", name: "http", caps: caps(false, true, true, false) },
    Language { id: "go", group: "Go", name: "net/http", caps: caps(false, true, true, true) },
    Language { id: "java-httpclient", group: "Java", name: "HttpClient", caps: caps(false, false, true, false) },
    Language { id: "java-okhttp", group: "Java", name: "OkHttp", caps: caps(false, false, true, true) },
    Language { id: "kotlin-okhttp", group: "Kotlin", name: "OkHttp", caps: caps(false, false, true, true) },
    Language { id: "csharp-httpclient", group: "C#", name: "HttpClient", caps: caps(false, true, true, true) },
    Language { id: "php-curl", group: "PHP", name: "cURL", caps: ALL },
    Language { id: "php-guzzle", group: "PHP", name: "Guzzle", caps: ALL },
    Language { id: "ruby", group: "Ruby", name: "Net::HTTP", caps: caps(false, true, true, true) },
    Language { id: "swift", group: "Swift", name: "URLSession", caps: caps(false, false, false, false) },
    Language { id: "rust-reqwest", group: "Rust", name: "reqwest", caps: caps(false, true, true, true) },
    Language { id: "dart-http", group: "Dart", name: "http", caps: caps(false, false, false, true) },
    Language { id: "c-libcurl", group: "C", name: "libcurl", caps: ALL },
    Language { id: "http", group: "HTTP", name: "HTTP/1.1", caps: caps(false, true, true, true) },
];

/// The request, ready to be written in any language.
#[derive(Debug, Clone)]
pub struct Spec {
    pub method: String,
    pub url: Url,
    /// Without `Content-Type` for multipart: libraries write it with their boundary.
    pub headers: Vec<(String, String)>,
    pub body: Payload,
    pub insecure: bool,
    pub follow: bool,
    pub digest: Option<(String, String)>,
    /// Signed by curl itself (`--aws-sigv4`); elsewhere a note, since the signature depends on
    /// the moment of sending.
    pub aws: Option<AwsV4>,
}

#[derive(Debug, Clone)]
pub enum Payload {
    None,
    Text(String),
    /// Parsed, for languages that take a structure; the text as it was for the rest.
    Json(Value, String),
    Form(Vec<(String, String)>),
    Multipart(Vec<Field>),
    /// A path; the code reads the file.
    File(String),
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    /// The text, or the path of a file.
    pub value: String,
    pub file: bool,
}

impl Spec {
    pub fn new(prepared: &Prepared, body: &Body, vars: &Vars) -> Spec {
        let r = |s: &str| render(s, vars).unwrap_or_else(|_| s.to_string());
        let payload = match body {
            Body::None => Payload::None,
            Body::Json { .. } | Body::Text { .. } => {
                let text = String::from_utf8_lossy(&prepared.body).into_owned();
                match serde_json::from_str::<Value>(&text) {
                    Ok(value) if matches!(body, Body::Json { .. }) => Payload::Json(value, text),
                    _ if text.is_empty() => Payload::None,
                    _ => Payload::Text(text),
                }
            }
            Body::Form { .. } => Payload::Form(url::form_urlencoded::parse(&prepared.body).into_owned().collect()),
            Body::Multipart { parts } => {
                Payload::Multipart(parts.iter().filter(|p| p.enabled).map(|p| Field { name: r(&p.name), value: r(&p.value), file: p.is_file }).collect())
            }
            Body::File { path, .. } => Payload::File(r(path)),
        };
        let multipart = matches!(payload, Payload::Multipart(_));
        let headers = prepared.headers.iter().filter(|(n, _)| !(multipart && n.eq_ignore_ascii_case("content-type"))).cloned().collect();
        Spec {
            method: prepared.method.clone(),
            url: prepared.url.clone(),
            headers,
            body: payload,
            insecure: prepared.options.verify_tls == Some(false),
            follow: prepared.options.follow_redirects != Some(false),
            digest: prepared.digest.clone(),
            // A header the user wrote wins, as when sending.
            aws: prepared.aws.clone().filter(|_| prepared.header("authorization").is_none()),
        }
    }

    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub(crate) fn content_type(&self) -> Option<&str> {
        self.header("content-type")
    }

    /// Headers other than `Content-Type`, for libraries that take that one apart.
    pub(crate) fn other_headers(&self) -> impl Iterator<Item = &(String, String)> {
        self.headers.iter().filter(|(n, _)| !n.eq_ignore_ascii_case("content-type"))
    }

    /// A text body as one literal: JSON compact (a string literal of pretty JSON is a wall of
    /// `\n`), anything else exactly.
    pub(crate) fn text(&self) -> Option<String> {
        match &self.body {
            Payload::Text(t) => Some(t.clone()),
            Payload::Json(v, _) => Some(v.to_string()),
            Payload::Form(fields) => Some(url::form_urlencoded::Serializer::new(String::new()).extend_pairs(fields).finish()),
            _ => None,
        }
    }

    /// Path and query, as in the request line.
    pub(crate) fn target(&self) -> &str {
        &self.url[url::Position::BeforePath..url::Position::AfterQuery]
    }

    pub(crate) fn is_https(&self) -> bool {
        self.url.scheme() == "https"
    }
}

/// The request in a language of [`LANGUAGES`].
pub fn generate(language: &str, spec: &Spec) -> Result<String, String> {
    let lang = LANGUAGES.iter().find(|l| l.id == language).ok_or_else(|| {
        let known = LANGUAGES.iter().map(|l| l.id).collect::<Vec<_>>().join(", ");
        tr!("неизвестный язык «{language}», есть: {known}", "unknown language \"{language}\", there are: {known}")
    })?;
    let mut notes = Vec::new();
    if spec.digest.is_some() && !lang.caps.digest {
        notes.push(tr_static!(
            "Digest-авторизация здесь не повторяется: добавьте её средствами библиотеки.",
            "Digest authorization is not repeated here: add it with the library's own means."
        ));
    }
    if spec.aws.is_some() && lang.id != "curl" {
        notes.push(tr_static!(
            "Подпись AWS Signature v4 здесь не повторяется: она делается в момент отправки — подпишите запрос SDK AWS или библиотекой подписи.",
            "The AWS Signature v4 is not repeated here: it is made at the moment of sending — sign the request with the AWS SDK or a signing library."
        ));
    }
    if spec.insecure && !lang.caps.insecure {
        notes.push(tr_static!(
            "В запросе выключена проверка сертификата, здесь она включена.",
            "The request has certificate verification off; here it is on."
        ));
    }
    if !spec.follow && !lang.caps.no_follow {
        notes.push(tr_static!(
            "Запрос не переходит по перенаправлениям, а этот код переходит.",
            "The request does not follow redirects, but this code does."
        ));
    }
    if matches!(spec.body, Payload::Multipart(_)) && !lang.caps.multipart {
        notes.push(tr_static!(
            "Тело multipart/form-data здесь не собирается — для него удобнее curl, requests или OkHttp.",
            "A multipart/form-data body is not built here — curl, requests or OkHttp suit it better."
        ));
    }
    let code = targets::write(lang.id, spec);
    Ok(with_notes(lang.id, code, &notes))
}

fn with_notes(id: &str, code: String, notes: &[&str]) -> String {
    if notes.is_empty() || id == "http" {
        return code;
    }
    let mark = match id {
        "curl" | "httpie" | "wget" | "powershell" | "python-requests" | "python-http-client" | "ruby" => "#",
        _ => "//",
    };
    let comments: String = notes.iter().map(|n| format!("{mark} {n}\n")).collect();
    // PHP comments go after the opening tag.
    match code.strip_prefix("<?php\n") {
        Some(rest) => format!("<?php\n{comments}{rest}"),
        None => format!("{comments}{code}"),
    }
}

// ---- string literals ----

#[derive(Clone, Copy)]
enum Ctrl {
    /// `\u0001`: JavaScript, Python, Go, Java, C#, Kotlin.
    U4,
    /// `\u{1}`: Swift, Rust.
    Braced,
    /// `\001`: C.
    Octal,
}

/// A double-quoted literal; `extra` are characters the language would otherwise interpolate.
fn dq(s: &str, ctrl: Ctrl, extra: &[char]) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => out.push_str(&match ctrl {
                Ctrl::U4 => format!("\\u{:04x}", c as u32),
                Ctrl::Braced => format!("\\u{{{:x}}}", c as u32),
                Ctrl::Octal => format!("\\{:03o}", c as u32),
            }),
            c if extra.contains(&c) => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn js(s: &str) -> String {
    dq(s, Ctrl::U4, &[])
}

/// Shell: in single quotes, a quote as `'\''`.
fn sh(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// PHP and Ruby: single quotes keep `$`, `#{}` and newlines as they are.
fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// The file name of a path, for multipart.
fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string())
}

/// JSON as a Python literal: `True`, `None`, four-space indents.
fn python_value(v: &Value, indent: usize) -> String {
    let pad = " ".repeat(indent + 4);
    let end = " ".repeat(indent);
    match v {
        Value::Null => "None".into(),
        Value::Bool(b) => if *b { "True" } else { "False" }.into(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => js(s),
        Value::Array(a) if a.is_empty() => "[]".into(),
        Value::Object(o) if o.is_empty() => "{}".into(),
        Value::Array(a) => format!("[\n{}{end}]", a.iter().map(|x| format!("{pad}{},\n", python_value(x, indent + 4))).collect::<String>()),
        Value::Object(o) => format!("{{\n{}{end}}}", o.iter().map(|(k, x)| format!("{pad}{}: {},\n", js(k), python_value(x, indent + 4))).collect::<String>()),
    }
}

/// JSON as a JavaScript literal, indented to sit at `indent`.
fn js_value(v: &Value, indent: usize) -> String {
    let pretty = serde_json::to_string_pretty(v).unwrap_or_default();
    pretty.lines().enumerate().map(|(i, l)| if i == 0 { l.to_string() } else { format!("\n{}{l}", " ".repeat(indent)) }).collect()
}

#[cfg(test)]
mod tests;
