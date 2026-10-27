//! A request as the user edits it: text fields may hold `{{variables}}`.
//! Serialized in camelCase for the app's interface.

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
    /// Variables kept from the response for other requests: name → where it is (`$.token`).
    #[serde(default)]
    pub captures: Vec<Pair>,
    /// What the response must be: `status == 200`, `$.items count > 0`.
    #[serde(default)]
    pub checks: Vec<Check>,
    #[serde(default)]
    pub scripts: Scripts,
}

/// JavaScript before and after the request; see [`crate::scripts`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Scripts {
    pub before: String,
    pub after: String,
}

impl Scripts {
    pub fn is_empty(&self) -> bool {
        self.before.trim().is_empty() && self.after.trim().is_empty()
    }
}

/// One line of the response checks; see [`crate::checks`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub text: String,
    pub enabled: bool,
}

impl Check {
    pub fn new(text: impl Into<String>) -> Self {
        Check { text: text.into(), enabled: true }
    }
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
            captures: Vec::new(),
            checks: Vec::new(),
            scripts: Scripts::default(),
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
    /// Answered after the server's 401 challenge; see [`crate::digest`].
    Digest { username: String, password: String },
    /// The token is got and renewed on its own; see [`crate::oauth`].
    #[serde(rename = "oauth2")]
    OAuth2(OAuth2),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OAuth2 {
    pub grant: Grant,
    /// Where the user signs in (authorization code).
    pub auth_url: String,
    pub token_url: String,
    pub client_id: String,
    /// Empty for a public client (an app with PKCE).
    pub client_secret: String,
    pub scope: String,
    /// Asked by Auth0 and some others; sent when given.
    pub audience: String,
    /// `http://127.0.0.1:<port>/…` or `localhost`; empty — a free port is picked.
    pub redirect_uri: String,
    pub pkce: bool,
    pub client_auth: ClientAuth,
    /// Password grant.
    pub username: String,
    pub password: String,
}

impl Default for OAuth2 {
    fn default() -> Self {
        OAuth2 {
            grant: Grant::default(),
            auth_url: String::new(),
            token_url: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            scope: String::new(),
            audience: String::new(),
            redirect_uri: String::new(),
            pkce: true,
            client_auth: ClientAuth::default(),
            username: String::new(),
            password: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Grant {
    /// The user signs in in the browser; the app catches the code on this machine.
    #[default]
    AuthorizationCode,
    ClientCredentials,
    Password,
}

impl Grant {
    pub fn as_str(self) -> &'static str {
        match self {
            Grant::AuthorizationCode => "authorization_code",
            Grant::ClientCredentials => "client_credentials",
            Grant::Password => "password",
        }
    }
}

/// How the client proves itself to the token endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClientAuth {
    /// `Authorization: Basic` (client_secret_basic).
    #[default]
    Basic,
    /// `client_id` and `client_secret` in the form (client_secret_post).
    Body,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiKeyLocation {
    #[default]
    Header,
    Query,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", rename_all_fields = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub struct Part {
    pub name: String,
    pub value: String,
    pub is_file: bool,
    pub enabled: bool,
}

/// Per-request overrides of the send options; `None` keeps the default.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestOptions {
    pub follow_redirects: Option<bool>,
    pub timeout_ms: Option<u64>,
    pub verify_tls: Option<bool>,
    /// Send the environment's cookies and keep those the server sets; on unless `false`.
    #[serde(default)]
    pub cookies: Option<bool>,
}
