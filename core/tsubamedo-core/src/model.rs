//! A request as the user edits it: text fields may hold `{{variables}}`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// Shown in the sidebar; the file name is used when it is empty.
    pub name: Option<String>,
    pub method: String,
    pub url: String,
    pub query: Vec<Pair>,
    pub headers: Vec<Pair>,
    pub auth: Auth,
    pub body: Body,
    pub options: RequestOptions,
}

impl Default for Request {
    fn default() -> Self {
        Request {
            name: None,
            method: "GET".into(),
            url: String::new(),
            query: Vec::new(),
            headers: Vec::new(),
            auth: Auth::None,
            body: Body::None,
            options: RequestOptions::default(),
        }
    }
}

/// A query parameter, header or form field. Disabled ones stay in the file but are not sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pair {
    pub name: String,
    pub value: String,
    pub enabled: bool,
}

impl Pair {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Pair { name: name.into(), value: value.into(), enabled: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Auth {
    #[default]
    None,
    Basic { username: String, password: String },
    Bearer { token: String },
    #[serde(rename = "apikey")]
    ApiKey { key: String, value: String, location: ApiKeyLocation },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiKeyLocation {
    #[default]
    Header,
    Query,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Body {
    #[default]
    None,
    Json { content: String },
    Text { content: String, content_type: Option<String> },
    /// `application/x-www-form-urlencoded`.
    Form { fields: Vec<Pair> },
    Multipart { parts: Vec<Part> },
    /// The file's bytes as is; a relative path is resolved against the request file's folder.
    File { path: String, content_type: Option<String> },
}

impl Body {
    pub fn kind(&self) -> &'static str {
        match self {
            Body::None => "none",
            Body::Json { .. } => "json",
            Body::Text { .. } => "text",
            Body::Form { .. } => "form",
            Body::Multipart { .. } => "multipart",
            Body::File { .. } => "file",
        }
    }
}

/// A multipart field: text, or a file when `is_file` is set (then `value` is its path).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Part {
    pub name: String,
    pub value: String,
    pub is_file: bool,
    pub enabled: bool,
}

/// Per-request overrides of the send options; `None` keeps the default.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RequestOptions {
    pub follow_redirects: Option<bool>,
    pub timeout_ms: Option<u64>,
    pub verify_tls: Option<bool>,
}
